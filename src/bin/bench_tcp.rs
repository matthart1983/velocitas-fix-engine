/// TCP round-trip benchmark — measures real Logon + N×(NOS → ExecRpt) + Logout
/// over localhost TCP, for direct comparison with QuickFIX/J TCP benchmark.
///
/// Usage: cargo run --release --bin bench_tcp [-- --count 10000]

use std::io;
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use velocitas_fix::engine::{EngineContext, FixApp, FixEngine};
use velocitas_fix::message::MessageView;
use velocitas_fix::serializer;
use velocitas_fix::session::{Session, SessionConfig, SessionRole, SequenceResetPolicy};
use velocitas_fix::tags;
use velocitas_fix::timestamp::{HrTimestamp, TimestampSource};
use velocitas_fix::transport::Transport;
use velocitas_fix::transport::TransportConfig;
use velocitas_fix::transport_tcp::StdTcpTransport;

fn main() {
    let count = parse_count();

    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║         Velocitas FIX Engine — TCP Round-Trip Benchmark         ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    println!("  Messages: {count}");
    println!("  Flow:     Logon → {count}×(NOS → ExecRpt) → Logout");
    println!();

    // Bind acceptor
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind failed");
    let port = listener.local_addr().unwrap().port();

    let (ready_tx, ready_rx) = mpsc::channel();

    // Spawn acceptor
    let acceptor_handle = thread::spawn(move || {
        run_acceptor(listener, ready_tx);
    });

    ready_rx.recv().unwrap();

    // Run initiator on main thread for accurate timing
    let (session_elapsed, latencies) = run_initiator(port, count);

    acceptor_handle.join().unwrap();

    // Report results
    let total_msgs = count as u64 * 2; // NOS + ExecRpt
    let msgs_per_sec = total_msgs as f64 / session_elapsed.as_secs_f64();

    latencies_report(&latencies);

    println!();
    println!("  Session total:             {:.1} ms", session_elapsed.as_secs_f64() * 1000.0);
    println!("  Application messages:      {} ({} round-trips)", total_msgs, count);
    println!("  Throughput:                {:.0} msgs/sec", msgs_per_sec);
    println!("  Round-trips/sec:           {:.0}", count as f64 / session_elapsed.as_secs_f64());
    println!();
}

fn latencies_report(latencies: &[Duration]) {
    if latencies.is_empty() {
        return;
    }
    let mut sorted: Vec<u64> = latencies.iter().map(|d| d.as_nanos() as u64).collect();
    sorted.sort_unstable();

    let len = sorted.len();
    let sum: u64 = sorted.iter().sum();
    let mean = sum / len as u64;
    let p50 = sorted[len * 50 / 100];
    let p90 = sorted[len * 90 / 100];
    let p99 = sorted[len * 99 / 100];
    let p999 = sorted[len.saturating_sub(1).min(len * 999 / 1000)];
    let min = sorted[0];
    let max = sorted[len - 1];

    println!("  Round-trip latency (NOS → ExecRpt):");
    println!("    min:    {:>10} µs", format_us(min));
    println!("    mean:   {:>10} µs", format_us(mean));
    println!("    p50:    {:>10} µs", format_us(p50));
    println!("    p90:    {:>10} µs", format_us(p90));
    println!("    p99:    {:>10} µs", format_us(p99));
    println!("    p99.9:  {:>10} µs", format_us(p999));
    println!("    max:    {:>10} µs", format_us(max));
}

fn format_us(nanos: u64) -> String {
    format!("{:.1}", nanos as f64 / 1000.0)
}

fn parse_count() -> usize {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--count" {
            if let Some(c) = args.get(i + 1) {
                return c.parse().unwrap_or(10_000);
            }
        }
    }
    10_000
}

// ─────────────────────────────────────────────────────────────────────
// Acceptor — auto-responds with ExecutionReport for every NOS
// ─────────────────────────────────────────────────────────────────────

fn run_acceptor(listener: TcpListener, ready_tx: mpsc::Sender<()>) {
    ready_tx.send(()).unwrap();

    let (stream, _) = listener.accept().expect("accept failed");
    stream.set_nodelay(true).unwrap();

    let transport = StdTcpTransport::from_stream(stream, TransportConfig::default())
        .expect("wrap failed");

    let session = Session::new(SessionConfig {
        session_id: "BENCH-ACC".into(),
        fix_version: "FIX.4.4".into(),
        sender_comp_id: "EXCHANGE".into(),
        target_comp_id: "TRADER".into(),
        role: SessionRole::Acceptor,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(0),
        max_reconnect_attempts: 0,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: false,
        max_msg_rate: 1_000_000,
    });

    let mut engine = FixEngine::new_acceptor(transport, session);
    engine.handle_inbound_logon().unwrap();

    let mut app = BenchAcceptorApp;
    let _ = engine.run_acceptor(&mut app);
}

struct BenchAcceptorApp;

impl FixApp for BenchAcceptorApp {
    fn on_message(&mut self, msg_type: &[u8], msg: &MessageView<'_>, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        if msg_type == b"D" {
            let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
            let seq = ctx.next_seq_num();
            let sender = ctx.session().config().sender_comp_id.clone();
            let target = ctx.session().config().target_comp_id.clone();
            let cl_ord_id = msg.get_field(tags::CL_ORD_ID).unwrap_or(b"?");
            let symbol = msg.get_field(tags::SYMBOL).unwrap_or(b"?");
            let qty = msg.get_field_i64(tags::ORDER_QTY).unwrap_or(100);

            let mut buf = [0u8; 2048];
            let len = serializer::build_execution_report(
                &mut buf,
                b"FIX.4.4", sender.as_bytes(), target.as_bytes(), seq, &ts,
                b"ORD-001", b"EXEC-001", cl_ord_id, symbol,
                b'1', qty, qty, b"100.00", 0, qty, b"100.00", b'F', b'2',
            );
            ctx.send_raw(&buf[..len])?;
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
// Initiator — sends N NOS, measures round-trip for each
// ─────────────────────────────────────────────────────────────────────

fn run_initiator(port: u16, count: usize) -> (Duration, Vec<Duration>) {
    thread::sleep(Duration::from_millis(50));

    let mut transport = StdTcpTransport::new(TransportConfig::default());
    transport.connect("127.0.0.1", port).expect("connect failed");

    let session = Session::new(SessionConfig {
        session_id: "BENCH-INIT".into(),
        fix_version: "FIX.4.4".into(),
        sender_comp_id: "TRADER".into(),
        target_comp_id: "EXCHANGE".into(),
        role: SessionRole::Initiator,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(1),
        max_reconnect_attempts: 3,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: false,
        max_msg_rate: 1_000_000,
    });

    let mut engine = FixEngine::new_initiator(transport, session);
    let mut app = BenchInitiatorApp {
        count,
        sent: 0,
        latencies: Vec::with_capacity(count),
        send_time: Instant::now(),
        session_start: Instant::now(),
        session_elapsed: Duration::ZERO,
    };

    let _ = engine.run_initiator(&mut app);

    (app.session_elapsed, app.latencies)
}

struct BenchInitiatorApp {
    count: usize,
    sent: usize,
    latencies: Vec<Duration>,
    send_time: Instant,
    session_start: Instant,
    session_elapsed: Duration,
}

impl BenchInitiatorApp {
    fn send_nos(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
        let seq = ctx.next_seq_num();
        let sender = ctx.session().config().sender_comp_id.clone();
        let target = ctx.session().config().target_comp_id.clone();
        let mut buf = [0u8; 1024];
        let cl_ord_id = format!("ORD-{:08}", self.sent);
        let len = serializer::build_new_order_single(
            &mut buf,
            b"FIX.4.4", sender.as_bytes(), target.as_bytes(), seq, &ts,
            cl_ord_id.as_bytes(), b"AAPL", b'1', 100, b'2', b"100.00",
        );
        self.send_time = Instant::now();
        ctx.send_raw(&buf[..len])?;
        self.sent += 1;
        Ok(())
    }
}

impl FixApp for BenchInitiatorApp {
    fn on_logon(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        self.session_start = Instant::now();
        // Send first NOS
        self.send_nos(ctx)
    }

    fn on_message(&mut self, msg_type: &[u8], _msg: &MessageView<'_>, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        if msg_type == b"8" {
            // Record latency
            let rtt = self.send_time.elapsed();
            self.latencies.push(rtt);

            if self.sent < self.count {
                // Send next NOS
                self.send_nos(ctx)?;
            } else {
                // Done — record elapsed and stop
                self.session_elapsed = self.session_start.elapsed();
                ctx.request_stop();
            }
        }
        Ok(())
    }
}
