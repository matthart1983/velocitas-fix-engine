/// End-to-end FIX + Aeron round-trip benchmark with real TCP venue.
///
/// Topology:
///   Venue (FIX initiator, TCP client)
///     ↕ TCP (127.0.0.1)
///   FIX Gateway (FIX acceptor)
///     ↕ Aeron IPC (STREAM_GATEWAY_TO_EXEC / STREAM_EXEC_TO_GATEWAY)
///   Executor
///
/// Pipeline (what the RTT timer on the venue measures):
///   [1] Venue sends FIX NOS bytes over TCP
///   [2] Gateway FIX engine parses NOS
///   [3] Gateway extracts fields → encodes SBE NormalizedOrder
///   [4] Aeron IPC publish (gateway → executor)
///   [5] Executor decodes SBE NormalizedOrder
///   [6] Executor encodes SBE NormalizedExecutionReport
///   [7] Aeron IPC publish (executor → gateway)
///   [8] Gateway decodes SBE NormalizedExecutionReport
///   [9] Gateway FIX engine serializes FIX ExecRpt → TCP → venue
///
/// Run with: cargo run --release --bin bench_e2e
#[allow(dead_code)]
#[path = "../aeron_c.rs"]
mod aeron_c;
#[allow(dead_code)]
#[path = "../aeron_sbe.rs"]
mod aeron_sbe;

use std::io;
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use aeron_c::{AeronErrorKind, Client as AeronClient, Publication, Subscription};
use aeron_sbe::{decode_fix_payload_from_sbe, encode_fix_payload_as_sbe};
use hdrhistogram::Histogram;

use velocitas_fix::engine::{EngineContext, FixApp, FixEngine};
use velocitas_fix::message::MessageView;
use velocitas_fix::normalized_sbe::{
    decode_normalized_execution_report_view_from_sbe, decode_normalized_order_view_from_sbe,
    encode_filled_execution_report_from_order_view_as_sbe, encode_normalized_order_as_sbe,
    NormalizedExecutionReportScratch, NormalizedOrder,
};
use velocitas_fix::serializer;
use velocitas_fix::session::{SequenceResetPolicy, Session, SessionConfig, SessionRole};
use velocitas_fix::tags;
use velocitas_fix::timestamp::{HrTimestamp, TimestampSource};
use velocitas_fix::transport::{Transport, TransportConfig};
use velocitas_fix::transport_tcp::StdTcpTransport;

// ── Tuning ───────────────────────────────────────────────────────────────────

const WARMUP: usize = 10_000;
const ITERATIONS: usize = 100_000;

const STREAM_GATEWAY_TO_EXEC: i32 = 1001;
const STREAM_EXEC_TO_GATEWAY: i32 = 1002;

const SEND_RETRY: Duration = Duration::from_secs(1);
const RECV_TIMEOUT: Duration = Duration::from_secs(10);
const RESOURCE_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_LIMIT: usize = 8;

// Idle strategy: busy-spin → yield → sleep.
const BUSY_SPIN_THRESHOLD: Duration = Duration::from_micros(50);
const YIELD_THRESHOLD: Duration = Duration::from_micros(500);

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() -> io::Result<()> {
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  Velocitas FIX — TCP Venue + Aeron End-to-End Benchmark (Rust)  ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!("  Topology:");
    println!("    Venue ⇄ TCP ⇄ Gateway ⇄ Aeron IPC ⇄ Executor");
    println!();
    println!("  Pipeline (measured RTT):");
    println!("    [1] Venue sends FIX NOS over TCP");
    println!("    [2] Gateway parses FIX NOS");
    println!("    [3] Gateway encodes SBE NormalizedOrder");
    println!("    [4] Aeron IPC publish (gateway → executor)");
    println!("    [5] Executor decodes SBE NormalizedOrder");
    println!("    [6] Executor encodes SBE NormalizedExecutionReport");
    println!("    [7] Aeron IPC publish (executor → gateway)");
    println!("    [8] Gateway decodes SBE NormalizedExecutionReport");
    println!("    [9] Gateway serializes FIX ExecRpt → TCP → venue");
    println!();
    println!("  Warmup:    {} round-trips (not measured)", WARMUP);
    println!("  Measured:  {} round-trips", ITERATIONS);
    println!();

    // ── Aeron embedded media driver ──
    let aeron_dir = std::env::temp_dir()
        .join(format!("velocitas-e2e-bench-{}", std::process::id()))
        .display()
        .to_string();
    let _driver = aeron_c::EmbeddedMediaDriver::shared(&aeron_dir, true)
        .map_err(|e| io::Error::other(format!("media driver: {e}")))?;

    let channel = format!("aeron:ipc?alias=e2e-bench-{}", std::process::id());

    // ── Executor Aeron link ──
    let exec_link = AeronLink::connect(
        &aeron_dir,
        &channel,
        STREAM_EXEC_TO_GATEWAY,
        STREAM_GATEWAY_TO_EXEC,
    )?;
    // ── Gateway Aeron link ──
    let gw_link = AeronLink::connect(
        &aeron_dir,
        &channel,
        STREAM_GATEWAY_TO_EXEC,
        STREAM_EXEC_TO_GATEWAY,
    )?;

    let total_rounds = (WARMUP + ITERATIONS) as u64;

    // ── Executor thread ──
    let exec_handle = thread::spawn(move || executor_loop(exec_link, total_rounds));

    // ── TCP listener for the gateway ──
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    let (ready_tx, ready_rx) = mpsc::channel();

    // ── Gateway thread ──
    let gateway_handle = thread::spawn(move || run_gateway(listener, gw_link, ready_tx));

    ready_rx.recv().unwrap();

    // ── Venue on main thread (for accurate timing) ──
    let (session_elapsed, histogram) = run_venue(port);

    gateway_handle
        .join()
        .map_err(|_| io::Error::other("gateway thread panicked"))??;
    exec_handle
        .join()
        .map_err(|_| io::Error::other("executor thread panicked"))??;

    print_results(&histogram, session_elapsed, ITERATIONS as u64);

    Ok(())
}

// ── Venue (FIX initiator, TCP client) ────────────────────────────────────────

fn run_venue(port: u16) -> (Duration, Histogram<u64>) {
    thread::sleep(Duration::from_millis(50));

    let mut transport = StdTcpTransport::new(TransportConfig::kernel_tcp());
    transport
        .connect("127.0.0.1", port)
        .expect("venue connect failed");

    let session = Session::new(SessionConfig {
        session_id: "VENUE".into(),
        fix_version: "FIX.4.4".into(),
        sender_comp_id: "BANK_OMS".into(),
        target_comp_id: "NYSE".into(),
        role: SessionRole::Initiator,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(1),
        max_reconnect_attempts: 3,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: false,
        max_msg_rate: 1_000_000,
    });

    let mut engine = FixEngine::new_initiator(transport, session);
    let mut app = VenueApp {
        total: WARMUP + ITERATIONS,
        sent: 0,
        histogram: Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).unwrap(),
        send_time: Instant::now(),
        measured_start: None,
        session_elapsed: Duration::ZERO,
    };

    let _ = engine.run_initiator(&mut app);

    (app.session_elapsed, app.histogram)
}

struct VenueApp {
    total: usize,
    sent: usize,
    histogram: Histogram<u64>,
    send_time: Instant,
    measured_start: Option<Instant>,
    session_elapsed: Duration,
}

impl VenueApp {
    fn send_nos(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
        let seq = ctx.next_seq_num();
        let sender = ctx.session().config().sender_comp_id.clone();
        let target = ctx.session().config().target_comp_id.clone();
        let mut buf = [0u8; 1024];
        let cl_ord_id = format!("ORD-{:08}", self.sent);
        let len = serializer::build_new_order_single(
            &mut buf,
            b"FIX.4.4",
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            cl_ord_id.as_bytes(),
            b"AAPL",
            b'1',
            10_000,
            b'2',
            b"178.55",
        );
        self.send_time = Instant::now();
        ctx.send_raw(&buf[..len])?;
        self.sent += 1;
        Ok(())
    }
}

impl FixApp for VenueApp {
    fn on_logon(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        self.send_nos(ctx)
    }

    fn on_message(
        &mut self,
        msg_type: &[u8],
        _msg: &MessageView<'_>,
        ctx: &mut EngineContext<'_>,
    ) -> io::Result<()> {
        if msg_type == b"8" {
            let rtt = self.send_time.elapsed();
            if self.sent > WARMUP {
                self.histogram
                    .record(rtt.as_nanos().min(u64::MAX as u128) as u64)
                    .ok();
            } else if self.sent == WARMUP {
                // Last warmup reply — start the measured clock on the next send.
                self.measured_start = Some(Instant::now());
            }

            if self.sent < self.total {
                self.send_nos(ctx)?;
            } else {
                if let Some(start) = self.measured_start {
                    self.session_elapsed = start.elapsed();
                }
                ctx.request_stop();
            }
        }
        Ok(())
    }
}

// ── Gateway (FIX acceptor + Aeron IPC) ───────────────────────────────────────

fn run_gateway(
    listener: TcpListener,
    link: AeronLink,
    ready_tx: mpsc::Sender<()>,
) -> io::Result<()> {
    ready_tx.send(()).unwrap();

    let (stream, _) = listener.accept().expect("gateway accept failed");
    stream.set_nodelay(true).unwrap();

    let transport = StdTcpTransport::from_stream(stream, TransportConfig::kernel_tcp())
        .map_err(|e| io::Error::other(format!("gateway wrap: {e:?}")))?;

    let session = Session::new(SessionConfig {
        session_id: "GATEWAY".into(),
        fix_version: "FIX.4.4".into(),
        sender_comp_id: "NYSE".into(),
        target_comp_id: "BANK_OMS".into(),
        role: SessionRole::Acceptor,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(0),
        max_reconnect_attempts: 0,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: false,
        max_msg_rate: 1_000_000,
    });

    let mut engine = FixEngine::new_acceptor(transport, session);
    engine
        .handle_inbound_logon()
        .map_err(|e| io::Error::other(format!("gateway logon: {e:?}")))?;

    let mut app = GatewayApp {
        link,
        order_sbe_buf: Vec::with_capacity(256),
        fix_out_buf: [0u8; 1024],
        replies: 0,
    };
    engine
        .run_acceptor(&mut app)
        .map_err(|e| io::Error::other(format!("gateway run: {e:?}")))?;

    Ok(())
}

struct GatewayApp {
    link: AeronLink,
    order_sbe_buf: Vec<u8>,
    fix_out_buf: [u8; 1024],
    replies: usize,
}

impl FixApp for GatewayApp {
    fn on_message(
        &mut self,
        msg_type: &[u8],
        msg: &MessageView<'_>,
        ctx: &mut EngineContext<'_>,
    ) -> io::Result<()> {
        if msg_type != b"D" {
            return Ok(());
        }

        // [3] Extract fields from parsed FIX → SBE NormalizedOrder.
        let order = NormalizedOrder {
            cl_ord_id: msg
                .get_field_str(tags::CL_ORD_ID)
                .unwrap_or("")
                .to_owned(),
            symbol: msg.get_field_str(tags::SYMBOL).unwrap_or("").to_owned(),
            side: msg
                .get_field(tags::SIDE)
                .and_then(|b| b.first().copied())
                .unwrap_or(b'1'),
            qty: msg.get_field_i64(tags::ORDER_QTY).unwrap_or(0),
            price: msg.get_field_str(tags::PRICE).unwrap_or("").to_owned(),
            sender_comp_id: msg
                .get_field_str(tags::SENDER_COMP_ID)
                .unwrap_or("")
                .to_owned(),
            target_comp_id: msg
                .get_field_str(tags::TARGET_COMP_ID)
                .unwrap_or("")
                .to_owned(),
        };
        let sbe_len = encode_normalized_order_as_sbe(&mut self.order_sbe_buf, &order)
            .map_err(|e| io::Error::other(format!("SBE encode order: {e}")))?;

        // [4] Publish to executor.
        self.link.publish(&self.order_sbe_buf[..sbe_len])?;

        // [7] Receive reply from executor.
        let reply = self.link.recv(RECV_TIMEOUT)?;

        // [8] Decode ExecutionReport.
        let exec_rpt = decode_normalized_execution_report_view_from_sbe(reply)
            .map_err(|e| io::Error::other(format!("SBE decode reply: {e}")))?;
        let exec_id_bytes = exec_rpt.exec_id.as_bytes().to_vec();

        // [9] Serialize FIX ExecRpt → venue.
        self.replies += 1;
        let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
        let seq = ctx.next_seq_num();
        let sender = ctx.session().config().sender_comp_id.clone();
        let target = ctx.session().config().target_comp_id.clone();
        let fix_len = serializer::build_execution_report(
            &mut self.fix_out_buf,
            b"FIX.4.4",
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            b"NYSE-ORD-001",
            &exec_id_bytes,
            order.cl_ord_id.as_bytes(),
            order.symbol.as_bytes(),
            b'1',
            order.qty,
            order.qty,
            order.price.as_bytes(),
            0,
            order.qty,
            order.price.as_bytes(),
            b'F',
            b'2',
        );
        ctx.send_raw(&self.fix_out_buf[..fix_len])?;

        if self.replies >= WARMUP + ITERATIONS {
            ctx.request_stop();
        }
        Ok(())
    }
}

// ── Executor ─────────────────────────────────────────────────────────────────

fn executor_loop(mut link: AeronLink, rounds: u64) -> io::Result<()> {
    let mut order_copy: Vec<u8> = Vec::with_capacity(256);
    let mut reply_buf: Vec<u8> = Vec::with_capacity(256);
    let mut scratch = NormalizedExecutionReportScratch::default();

    for _ in 0..rounds {
        {
            let sbe = link.recv(RECV_TIMEOUT)?;
            order_copy.clear();
            order_copy.extend_from_slice(sbe);
        }

        let order = decode_normalized_order_view_from_sbe(&order_copy)
            .map_err(|e| io::Error::other(format!("executor SBE decode: {e}")))?;

        let len = encode_filled_execution_report_from_order_view_as_sbe(
            &mut reply_buf,
            &mut scratch,
            &order,
        )
        .map_err(|e| io::Error::other(format!("executor SBE encode: {e}")))?;

        link.publish(&reply_buf[..len])?;
    }
    Ok(())
}

// ── Aeron link ────────────────────────────────────────────────────────────────

struct AeronLink {
    _client: AeronClient,
    pub_: Publication,
    sub_: Subscription,
    frame_buf: Vec<u8>,
    recv_buf: Vec<u8>,
}

impl AeronLink {
    fn connect(
        aeron_dir: &str,
        channel: &str,
        publish_stream: i32,
        subscribe_stream: i32,
    ) -> io::Result<Self> {
        let client = AeronClient::connect(Some(aeron_dir))
            .map_err(|e| io::Error::other(format!("Aeron client: {e}")))?;
        let pub_ = client
            .add_publication(channel, publish_stream, RESOURCE_TIMEOUT)
            .map_err(|e| io::Error::other(format!("Aeron publication: {e}")))?;
        let sub_ = client
            .add_subscription(channel, subscribe_stream, RESOURCE_TIMEOUT)
            .map_err(|e| io::Error::other(format!("Aeron subscription: {e}")))?;
        Ok(Self {
            _client: client,
            pub_,
            sub_,
            frame_buf: Vec::with_capacity(512),
            recv_buf: Vec::with_capacity(512),
        })
    }

    fn publish(&mut self, payload: &[u8]) -> io::Result<()> {
        let encoded = encode_fix_payload_as_sbe(&mut self.frame_buf, 0, payload)
            .map_err(|e| io::Error::other(format!("SBE frame encode: {e}")))?;
        let frame = &self.frame_buf[..encoded];
        let start = Instant::now();
        loop {
            let rc = self.pub_.offer(frame);
            if rc >= 0 {
                return Ok(());
            }
            if AeronErrorKind::from_code(rc as i32).is_back_pressured_or_admin_action()
                && start.elapsed() < SEND_RETRY
            {
                thread::yield_now();
                continue;
            }
            return Err(io::Error::other(format!(
                "Aeron offer failed: {}",
                AeronErrorKind::from_code(rc as i32)
            )));
        }
    }

    fn recv(&mut self, timeout: Duration) -> io::Result<&[u8]> {
        let start = Instant::now();
        loop {
            let mut found = false;
            self.sub_
                .poll_once(POLL_LIMIT, |msg| {
                    if found {
                        return;
                    }
                    if let Ok(env) = decode_fix_payload_from_sbe(msg) {
                        self.recv_buf.clear();
                        self.recv_buf.extend_from_slice(env.payload);
                        found = true;
                    }
                })
                .map_err(|e| io::Error::other(format!("Aeron poll: {e}")))?;

            if found {
                return Ok(&self.recv_buf);
            }

            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "recv timeout"));
            }
            if elapsed < BUSY_SPIN_THRESHOLD {
                std::hint::spin_loop();
            } else if elapsed < YIELD_THRESHOLD {
                thread::yield_now();
            } else {
                thread::sleep(Duration::from_micros(10));
            }
        }
    }
}

// ── Reporting ────────────────────────────────────────────────────────────────

fn print_results(histogram: &Histogram<u64>, total: Duration, iters: u64) {
    if histogram.len() == 0 {
        return;
    }
    println!("  Round-trip latency (venue FIX NOS → venue FIX ExecRpt over TCP):");
    println!("    min:    {:>10.1} µs", histogram.min() as f64 / 1_000.0);
    println!(
        "    mean:   {:>10.1} µs",
        histogram.mean() / 1_000.0
    );
    println!(
        "    p50:    {:>10.1} µs",
        histogram.value_at_quantile(0.50) as f64 / 1_000.0
    );
    println!(
        "    p90:    {:>10.1} µs",
        histogram.value_at_quantile(0.90) as f64 / 1_000.0
    );
    println!(
        "    p99:    {:>10.1} µs",
        histogram.value_at_quantile(0.99) as f64 / 1_000.0
    );
    println!(
        "    p99.9:  {:>10.1} µs",
        histogram.value_at_quantile(0.999) as f64 / 1_000.0
    );
    println!("    max:    {:>10.1} µs", histogram.max() as f64 / 1_000.0);
    println!();
    if total.as_secs_f64() > 0.0 {
        println!(
            "  Throughput:                {:.0} round-trips/sec",
            iters as f64 / total.as_secs_f64()
        );
    }
}
