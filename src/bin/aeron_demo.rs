/// Independent-runner Aeron/FIX venue demo.
///
/// This demo runs three separate OS processes:
/// 1. A mock FIX venue client speaking FIX over TCP.
/// 2. A FIX gateway endpoint that accepts FIX, normalizes it, and publishes it
///    over Aeron inside the shared SBE envelope.
/// 3. A mock internal execution service that consumes the normalized SBE
///    payload, decides to fill the order, and replies in SBE.
///
/// The gateway then converts the internal execution payload back into a FIX
/// ExecutionReport and sends it to the venue client.
#[allow(dead_code)]
#[path = "../aeron_c.rs"]
mod aeron_c;
#[allow(dead_code)]
#[path = "../aeron_sbe.rs"]
mod aeron_sbe;

use std::env;
use std::fs;
use std::hint::black_box;
use std::io;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{self, Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use aeron_c::{AeronError, AeronErrorKind, Client as RawAeronClient, Publication, Subscription};
use aeron_sbe::{decode_fix_payload_from_sbe, encode_fix_payload_as_sbe};
use hdrhistogram::Histogram;

use velocitas_fix::client::{FixClient, FixClientConfig};
use velocitas_fix::engine::{EngineContext, FixApp, FixEngine};
use velocitas_fix::message::MessageView;
use velocitas_fix::normalized_sbe::{
    decode_normalized_execution_report_from_sbe, decode_normalized_order_from_sbe,
    encode_normalized_execution_report_as_sbe, encode_normalized_order_as_sbe,
    encoded_normalized_execution_report_len, encoded_normalized_order_len,
    NormalizedExecutionReport, NormalizedOrder,
};
use velocitas_fix::parser::FixParser;
use velocitas_fix::serializer;
use velocitas_fix::session::{SequenceResetPolicy, Session, SessionConfig, SessionRole};
use velocitas_fix::tags;
use velocitas_fix::timestamp::{HrTimestamp, TimestampSource};
use velocitas_fix::transport::TransportConfig;
use velocitas_fix::transport_tcp::StdTcpTransport;

const RESOURCE_TIMEOUT: Duration = Duration::from_secs(5);
const RECEIVE_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_RETRY_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_FRAGMENT_LIMIT: usize = 8;
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const RECEIVE_BUSY_SPIN_THRESHOLD: Duration = Duration::from_micros(50);
const RECEIVE_YIELD_THRESHOLD: Duration = Duration::from_millis(1);
const RECEIVE_PARK_INTERVAL: Duration = Duration::from_micros(50);
const CODEC_BENCHMARK_ITERS: u64 = 20_000;
const LIVE_BENCHMARK_ROUND_TRIPS: u64 = 250;
const BENCHMARK_STREAM_ID: i32 = 7101;

const GATEWAY_COMP_ID: &str = "VELOCITAS_GATEWAY";
const VENUE_COMP_ID: &str = "BLOOMBERG_FX";

const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const MAGENTA: &str = "\x1b[35m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("gateway") => run_gateway(
            required_arg(&mut args, "--port")?
                .parse()
                .map_err(invalid_input)?,
            required_arg(&mut args, "--channel")?,
            required_arg(&mut args, "--aeron-dir")?,
            required_arg(&mut args, "--stream-id")?
                .parse()
                .map_err(invalid_input)?,
            PathBuf::from(required_arg(&mut args, "--ready-file")?),
        ),
        Some("internal-executor") => run_internal_executor(
            required_arg(&mut args, "--channel")?,
            required_arg(&mut args, "--aeron-dir")?,
            required_arg(&mut args, "--stream-id")?
                .parse()
                .map_err(invalid_input)?,
            PathBuf::from(required_arg(&mut args, "--ready-file")?),
        ),
        Some("media-driver") => run_media_driver_from_args(&mut args),
        Some("venue") => run_mock_venue(
            required_arg(&mut args, "--host")?,
            required_arg(&mut args, "--port")?
                .parse()
                .map_err(invalid_input)?,
        ),
        Some("benchmark") => run_benchmarks(),
        Some(other) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown aeron_demo mode: {other}"),
        )),
        None => run_orchestrator(),
    }
}

fn banner() {
    println!();
    println!(
        "{BOLD}{CYAN}╔══════════════════════════════════════════════════════════════════════╗{RESET}"
    );
    println!(
        "{BOLD}{CYAN}║            VELOCITAS FIX VENUE -> AERON -> EXECUTION DEMO          ║{RESET}"
    );
    println!(
        "{BOLD}{CYAN}║      independent runners: venue client, FIX gateway, executor      ║{RESET}"
    );
    println!(
        "{BOLD}{CYAN}╚══════════════════════════════════════════════════════════════════════╝{RESET}"
    );
}

fn section(title: &str) {
    let bar = "═".repeat(74);
    println!();
    println!("{BOLD}{CYAN}{bar}{RESET}");
    println!("{BOLD}{CYAN}  {title}{RESET}");
    println!("{BOLD}{CYAN}{bar}{RESET}");
}

fn step(role: &str, message: &str) {
    println!("{GREEN}▸{RESET} [{role}] {message}");
}

fn detail(label: &str, value: impl std::fmt::Display) {
    println!("  {DIM}{label}:{RESET} {value}");
}

fn highlight(label: &str, value: impl std::fmt::Display) {
    println!("  {YELLOW}{label}:{RESET} {BOLD}{MAGENTA}{value}{RESET}");
}

fn invalid_input(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, error.to_string())
}

fn parser_error(error: impl std::fmt::Debug) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}"))
}

fn aeron_client_error(context: &str, error: AeronError) -> io::Error {
    let kind = match error.kind() {
        AeronErrorKind::TimedOut
        | AeronErrorKind::ClientErrorDriverTimeout
        | AeronErrorKind::ClientErrorClientTimeout
        | AeronErrorKind::ClientErrorConductorServiceTimeout => io::ErrorKind::TimedOut,
        AeronErrorKind::PublicationClosed => io::ErrorKind::BrokenPipe,
        AeronErrorKind::PublicationBackPressured
        | AeronErrorKind::PublicationAdminAction
        | AeronErrorKind::ClientErrorBufferFull => io::ErrorKind::WouldBlock,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, format!("{context}: {error}"))
}

fn required_arg(args: &mut impl Iterator<Item = String>, name: &str) -> io::Result<String> {
    args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing required argument {name}"),
        )
    })
}

fn fix_wire(data: &[u8]) -> String {
    String::from_utf8_lossy(data).replace('\x01', "|")
}

fn write_ready_file(path: &Path, contents: &str) -> io::Result<()> {
    fs::write(path, contents)
}

fn wait_for_file(path: &Path, timeout: Duration) -> io::Result<()> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("timed out waiting for ready file {}", path.display()),
    ))
}

fn format_duration(duration: Duration) -> String {
    let ns = duration.as_nanos();
    if ns >= 1_000_000 {
        format!("{:.2} ms", ns as f64 / 1_000_000.0)
    } else if ns >= 1_000 {
        format!("{:.2} us", ns as f64 / 1_000.0)
    } else {
        format!("{ns} ns")
    }
}

fn print_flow(channel: &str, stream_id: i32, port: u16, aeron_dir: &str) {
    section("Independent Runner Topology");
    detail("FIX venue TCP endpoint", format!("127.0.0.1:{port}"));
    detail("Aeron channel", channel);
    detail("request stream", stream_id);
    detail("reply stream", stream_id + 1);
    detail("aeron directory", aeron_dir);
    println!();
    println!(
        "{BOLD}Mock Venue{RESET}          {BOLD}Gateway{RESET}               {BOLD}Media Driver{RESET}          {BOLD}Internal Executor{RESET}"
    );
    println!(
        "FIX/TCP client        FIX Engine + bridge     standalone Aeron core    Aeron/SBE worker"
    );
    println!("     |                      |                        |                        |");
    println!("     | FIX Logon / NOS ---->|                        |                        |");
    println!("     |                      | normalize FIX fields   |                        |");
    println!(
        "     |                      | publish stream {} ---->|                        |",
        stream_id
    );
    println!("     |                      |                        | forward order -------->|");
    println!(
        "     |                      |                        |                        | decode normalized order"
    );
    println!(
        "     |                      |                        |                        | decide fill / build report"
    );
    println!(
        "     |                      |<--- reply stream {} ---|                        |",
        stream_id + 1
    );
    println!("     |                      |<-----------------------| publish execution <-----|");
    println!("     |                      | encode FIX ExecutionReport                       |");
    println!("     |<----- FIX ExecutionReport -----|                        |               |");
    println!("     | FIX Logout --------->|                        |                        |");
    println!();
}

struct RawAeronLink {
    _client: RawAeronClient,
    publication: Publication,
    subscription: Subscription,
    send_frame_buf: Vec<u8>,
}

impl RawAeronLink {
    fn connect(
        aeron_dir: &str,
        channel: &str,
        publish_stream: i32,
        subscribe_stream: i32,
    ) -> io::Result<Self> {
        let client = RawAeronClient::connect(Some(aeron_dir))
            .map_err(|error| aeron_client_error("failed to create Aeron client", error))?;
        let publication = client
            .add_publication(channel, publish_stream, RESOURCE_TIMEOUT)
            .map_err(|error| aeron_client_error("failed to create Aeron publication", error))?;
        let subscription = client
            .add_subscription(channel, subscribe_stream, RESOURCE_TIMEOUT)
            .map_err(|error| aeron_client_error("failed to create Aeron subscription", error))?;

        Ok(Self {
            _client: client,
            publication,
            subscription,
            send_frame_buf: Vec::new(),
        })
    }

    fn publish_payload(&mut self, base_stream_id: i32, payload: &[u8]) -> io::Result<usize> {
        let encoded_len =
            encode_fix_payload_as_sbe(&mut self.send_frame_buf, base_stream_id, payload)?;
        let frame = &self.send_frame_buf[..encoded_len];
        let start = Instant::now();

        loop {
            let result = self.publication.offer(frame);
            if result >= 0 {
                return Ok(encoded_len);
            }

            let error_kind = AeronErrorKind::from_code(result as i32);
            if error_kind.is_back_pressured_or_admin_action()
                && start.elapsed() < SEND_RETRY_TIMEOUT
            {
                thread::yield_now();
                continue;
            }

            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("failed to publish SBE payload: {error_kind} ({result})"),
            ));
        }
    }

    fn recv_payload(&self, timeout: Duration) -> io::Result<(i32, Vec<u8>)> {
        let start = Instant::now();
        loop {
            let mut payload = None;
            let mut decode_error = None;
            self.subscription
                .poll_once(POLL_FRAGMENT_LIMIT, |msg| {
                    if payload.is_some() || decode_error.is_some() {
                        return;
                    }

                    match decode_fix_payload_from_sbe(msg) {
                        Ok(envelope) => {
                            payload = Some((envelope.base_stream_id, envelope.payload.to_vec()));
                        }
                        Err(error) => {
                            decode_error = Some(error);
                        }
                    }
                })
                .map_err(|error| aeron_client_error("failed to poll Aeron subscription", error))?;

            if let Some(error) = decode_error {
                return Err(error);
            }

            if let Some(payload) = payload {
                return Ok(payload);
            }

            let elapsed = start.elapsed();
            if elapsed >= timeout {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out waiting for Aeron payload",
                ));
            }

            if elapsed < RECEIVE_BUSY_SPIN_THRESHOLD {
                std::hint::spin_loop();
            } else if elapsed < RECEIVE_YIELD_THRESHOLD {
                thread::yield_now();
            } else {
                thread::sleep(RECEIVE_PARK_INTERVAL);
            }
        }
    }
}

fn sample_normalized_order() -> NormalizedOrder {
    NormalizedOrder {
        cl_ord_id: "BPIPE-ORD-0001".into(),
        symbol: "EUR/USD".into(),
        side: b'1',
        qty: 5_000_000,
        price: "1.08255".into(),
        sender_comp_id: VENUE_COMP_ID.into(),
        target_comp_id: GATEWAY_COMP_ID.into(),
    }
}

fn average_duration(total: Duration, iterations: u64) -> Duration {
    Duration::from_nanos((total.as_nanos() / iterations.max(1) as u128) as u64)
}

fn print_throughput_result(label: &str, total: Duration, iterations: u64) {
    detail(label, format_duration(average_duration(total, iterations)));
    detail(
        &format!("{label} throughput"),
        format!("{:.0} ops/sec", iterations as f64 / total.as_secs_f64()),
    );
}

fn run_benchmarks() -> io::Result<()> {
    banner();
    section("Benchmark Configuration");
    detail("local codec iterations", CODEC_BENCHMARK_ITERS);
    detail("live round trips", LIVE_BENCHMARK_ROUND_TRIPS);

    run_local_codec_benchmark(CODEC_BENCHMARK_ITERS)?;
    run_live_round_trip_benchmark(LIVE_BENCHMARK_ROUND_TRIPS)?;
    Ok(())
}

fn run_local_codec_benchmark(iterations: u64) -> io::Result<()> {
    let order = sample_normalized_order();
    let report = NormalizedExecutionReport::from_order(&order);

    let order_wire_len = encoded_normalized_order_len(&order)?;
    let report_wire_len = encoded_normalized_execution_report_len(&report)?;

    let mut order_buf = Vec::new();
    let mut report_buf = Vec::new();
    let mut envelope_buf = Vec::new();

    section("Local Codec Benchmark");
    detail(
        "normalized order payload",
        format!("{order_wire_len} bytes"),
    );
    detail(
        "normalized execution payload",
        format!("{report_wire_len} bytes"),
    );

    let order_elapsed = {
        let started = Instant::now();
        for _ in 0..iterations {
            let len = encode_normalized_order_as_sbe(&mut order_buf, &order)?;
            let decoded = decode_normalized_order_from_sbe(&order_buf[..len])?;
            black_box(decoded.qty);
        }
        started.elapsed()
    };
    print_throughput_result("order SBE encode+decode", order_elapsed, iterations);

    let report_elapsed = {
        let started = Instant::now();
        for _ in 0..iterations {
            let len = encode_normalized_execution_report_as_sbe(&mut report_buf, &report)?;
            let decoded = decode_normalized_execution_report_from_sbe(&report_buf[..len])?;
            black_box(decoded.cum_qty);
        }
        started.elapsed()
    };
    print_throughput_result("execution SBE encode+decode", report_elapsed, iterations);

    let envelope_elapsed = {
        let business_len = encode_normalized_order_as_sbe(&mut order_buf, &order)?;
        let business_payload = &order_buf[..business_len];
        let started = Instant::now();
        for _ in 0..iterations {
            let frame_len = encode_fix_payload_as_sbe(
                &mut envelope_buf,
                BENCHMARK_STREAM_ID,
                business_payload,
            )?;
            let decoded = decode_fix_payload_from_sbe(&envelope_buf[..frame_len])?;
            black_box(decoded.payload.len());
        }
        started.elapsed()
    };
    print_throughput_result("envelope wrap+unwrap", envelope_elapsed, iterations);

    Ok(())
}

fn run_live_round_trip_benchmark(round_trips: u64) -> io::Result<()> {
    let channel = format!("aeron:ipc?alias=velocitas-fix-benchmark-{BENCHMARK_STREAM_ID}");
    let run_id = format!("{}-{BENCHMARK_STREAM_ID}", process::id());
    let aeron_dir = env::temp_dir()
        .join(format!("velocitas-fix-benchmark-{run_id}"))
        .display()
        .to_string();
    let driver_ready = env::temp_dir().join(format!("velocitas-driver-ready-{run_id}"));
    let _ = fs::remove_dir_all(&aeron_dir);
    let _ = fs::remove_file(&driver_ready);

    section("Live Aeron Round Trip Benchmark");
    detail("Aeron channel", &channel);
    detail("request stream", BENCHMARK_STREAM_ID);
    detail("reply stream", BENCHMARK_STREAM_ID + 1);

    let result = (|| -> io::Result<()> {
        let current_exe = env::current_exe()?;
        let mut driver = spawn_media_driver(&current_exe, &aeron_dir, &driver_ready)?;
        wait_for_file(&driver_ready, READY_TIMEOUT)?;

        let benchmark_result = (|| -> io::Result<()> {
            let gateway = RawAeronLink::connect(
                &aeron_dir,
                &channel,
                BENCHMARK_STREAM_ID,
                BENCHMARK_STREAM_ID + 1,
            )?;
            let executor = RawAeronLink::connect(
                &aeron_dir,
                &channel,
                BENCHMARK_STREAM_ID + 1,
                BENCHMARK_STREAM_ID,
            )?;

            let executor_handle = thread::spawn(move || -> io::Result<()> {
                let mut executor = executor;
                let mut report_buf = Vec::new();

                for _ in 0..round_trips {
                    let (_, order_payload) = executor.recv_payload(RECEIVE_TIMEOUT)?;
                    let order = decode_normalized_order_from_sbe(&order_payload)?;
                    let report = NormalizedExecutionReport::from_order(&order);
                    let len = encode_normalized_execution_report_as_sbe(&mut report_buf, &report)?;
                    executor.publish_payload(BENCHMARK_STREAM_ID, &report_buf[..len])?;
                }

                Ok(())
            });

            let mut gateway = gateway;
            let order = sample_normalized_order();
            let mut order_buf = Vec::new();
            let order_len = encode_normalized_order_as_sbe(&mut order_buf, &order)?;
            let order_wire = &order_buf[..order_len];
            let reply_wire_len = encoded_normalized_execution_report_len(
                &NormalizedExecutionReport::from_order(&order),
            )?;

            detail(
                "benchmark order payload",
                format!("{} bytes", order_wire.len()),
            );
            detail("benchmark reply payload", format!("{reply_wire_len} bytes"));

            let mut histogram =
                Histogram::<u64>::new_with_bounds(1, 60_000_000_000, 3).map_err(invalid_input)?;
            let mut total = Duration::ZERO;

            for _ in 0..round_trips {
                let started = Instant::now();
                gateway.publish_payload(BENCHMARK_STREAM_ID, order_wire)?;
                let (_, reply_payload) = gateway.recv_payload(RECEIVE_TIMEOUT)?;
                let reply = decode_normalized_execution_report_from_sbe(&reply_payload)?;
                black_box(reply.exec_id.as_str());

                let elapsed = started.elapsed();
                total += elapsed;
                histogram
                    .record(elapsed.as_nanos().min(u64::MAX as u128) as u64)
                    .map_err(invalid_input)?;
            }

            executor_handle
                .join()
                .map_err(|_| io::Error::other("benchmark executor thread panicked"))??;

            highlight(
                "avg round trip",
                format_duration(average_duration(total, round_trips)),
            );
            detail(
                "p50 round trip",
                format_duration(Duration::from_nanos(histogram.value_at_quantile(0.50))),
            );
            detail(
                "p99 round trip",
                format_duration(Duration::from_nanos(histogram.value_at_quantile(0.99))),
            );
            detail(
                "max round trip",
                format_duration(Duration::from_nanos(histogram.max())),
            );
            detail(
                "throughput",
                format!(
                    "{:.0} round-trips/sec",
                    round_trips as f64 / total.as_secs_f64()
                ),
            );

            Ok(())
        })();

        stop_child(&mut driver);
        benchmark_result
    })();

    let _ = fs::remove_dir_all(&aeron_dir);
    let _ = fs::remove_file(&driver_ready);
    result
}

fn run_orchestrator() -> io::Result<()> {
    let stream_id = 6101;
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.local_addr()?.port()
    };
    let channel = format!("aeron:ipc?alias=velocitas-fix-venue-flow-{stream_id}");
    let run_id = format!("{}-{}", process::id(), stream_id);
    let aeron_dir = env::temp_dir()
        .join(format!("velocitas-fix-venue-flow-{run_id}"))
        .display()
        .to_string();
    let driver_ready = env::temp_dir().join(format!("velocitas-driver-ready-{run_id}"));
    let internal_ready = env::temp_dir().join(format!("velocitas-internal-ready-{run_id}"));
    let gateway_ready = env::temp_dir().join(format!("velocitas-gateway-ready-{run_id}"));

    let _ = fs::remove_dir_all(&aeron_dir);
    let _ = fs::remove_file(&driver_ready);
    let _ = fs::remove_file(&internal_ready);
    let _ = fs::remove_file(&gateway_ready);

    banner();
    print_flow(&channel, stream_id, port, &aeron_dir);

    let exe = env::current_exe()?;
    section("Launching Independent Runners");

    let mut driver = spawn_media_driver(&exe, &aeron_dir, &driver_ready)?;
    wait_for_file(&driver_ready, READY_TIMEOUT)?;
    step("orchestrator", "standalone Aeron media driver is ready");

    let run_result = (|| -> io::Result<()> {
        let mut internal = spawn_role(
            &exe,
            "internal-executor",
            &[
                channel.as_str(),
                aeron_dir.as_str(),
                &stream_id.to_string(),
                internal_ready.to_string_lossy().as_ref(),
            ],
        )?;
        wait_for_file(&internal_ready, READY_TIMEOUT)?;
        step("orchestrator", "internal execution service is ready");

        let mut gateway = spawn_role(
            &exe,
            "gateway",
            &[
                &port.to_string(),
                channel.as_str(),
                aeron_dir.as_str(),
                &stream_id.to_string(),
                gateway_ready.to_string_lossy().as_ref(),
            ],
        )?;
        wait_for_file(&gateway_ready, READY_TIMEOUT)?;
        step("orchestrator", "FIX gateway listener is ready");

        let start = Instant::now();
        let venue_status = spawn_role(&exe, "venue", &["127.0.0.1", &port.to_string()])?.wait()?;
        ensure_success("venue", venue_status)?;

        let gateway_status = gateway.wait()?;
        ensure_success("gateway", gateway_status)?;

        let internal_status = internal.wait()?;
        ensure_success("internal-executor", internal_status)?;

        let elapsed = start.elapsed();
        section("Orchestrator Summary");
        detail("venue to gateway", "real FIX/TCP session");
        detail(
            "gateway to executor",
            "real Aeron channel carrying SBE-wrapped normalized payloads through a standalone media driver",
        );
        highlight("wall clock", format_duration(elapsed));
        println!();
        println!("{BOLD}{GREEN}independent runner demo completed successfully{RESET}");

        Ok(())
    })();

    stop_child(&mut driver);
    let _ = fs::remove_dir_all(&aeron_dir);
    let _ = fs::remove_file(&driver_ready);
    let _ = fs::remove_file(&internal_ready);
    let _ = fs::remove_file(&gateway_ready);
    run_result
}

fn spawn_role(exe: &Path, role: &str, values: &[&str]) -> io::Result<Child> {
    let mut command = Command::new(exe);
    command.arg(role);
    for value in values {
        command.arg(value);
    }
    command.spawn()
}

fn ensure_success(role: &str, status: ExitStatus) -> io::Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "{role} runner exited with status {status}"
        )))
    }
}

fn spawn_media_driver(current_exe: &Path, aeron_dir: &str, ready_file: &Path) -> io::Result<Child> {
    Command::new(current_exe)
        .arg("media-driver")
        .arg("--aeron-dir")
        .arg(aeron_dir)
        .arg("--ready-file")
        .arg(ready_file)
        .spawn()
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn run_media_driver(aeron_dir: String, ready_file: PathBuf) -> io::Result<()> {
    let _driver = aeron_c::EmbeddedMediaDriver::shared(&aeron_dir, true).map_err(|error| {
        aeron_client_error("failed to start standalone Aeron media driver", error)
    })?;
    write_ready_file(&ready_file, "driver-ready")?;
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn run_media_driver_from_args(args: &mut impl Iterator<Item = String>) -> io::Result<()> {
    let mut aeron_dir = None;
    let mut ready_file = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--aeron-dir" => aeron_dir = Some(required_arg(args, "--aeron-dir")?),
            "--ready-file" => ready_file = Some(PathBuf::from(required_arg(args, "--ready-file")?)),
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown media-driver argument {other}"),
                ))
            }
        }
    }

    run_media_driver(
        aeron_dir.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing required argument --aeron-dir",
            )
        })?,
        ready_file.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing required argument --ready-file",
            )
        })?,
    )
}

fn run_gateway(
    port: u16,
    channel: String,
    aeron_dir: String,
    stream_id: i32,
    ready_file: PathBuf,
) -> io::Result<()> {
    step(
        "gateway",
        &format!("starting FIX gateway on 127.0.0.1:{port}"),
    );

    let listener = TcpListener::bind(("127.0.0.1", port))?;
    write_ready_file(&ready_file, "gateway-ready")?;

    let (stream, remote) = listener.accept()?;
    step(
        "gateway",
        &format!("accepted venue connection from {remote}"),
    );

    let transport = StdTcpTransport::from_stream(stream, TransportConfig::kernel_tcp())?;
    let session = Session::new(SessionConfig {
        session_id: "FIX-GATEWAY-ACC".into(),
        fix_version: "FIX.4.4".into(),
        sender_comp_id: GATEWAY_COMP_ID.into(),
        target_comp_id: VENUE_COMP_ID.into(),
        role: SessionRole::Acceptor,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::ZERO,
        max_reconnect_attempts: 0,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: true,
        max_msg_rate: 50_000,
    });

    let mut app = GatewayBridgeApp::new(&aeron_dir, &channel, stream_id)?;
    let mut engine = FixEngine::new_acceptor(transport, session);
    engine.handle_inbound_logon()?;
    step(
        "gateway",
        "FIX Logon acknowledged, gateway session is active",
    );
    engine.run_acceptor(&mut app)
}

struct GatewayBridgeApp {
    aeron: RawAeronLink,
    parser: FixParser,
    stream_id: i32,
}

impl GatewayBridgeApp {
    fn new(aeron_dir: &str, channel: &str, stream_id: i32) -> io::Result<Self> {
        let aeron = RawAeronLink::connect(aeron_dir, channel, stream_id, stream_id + 1)?;
        Ok(Self {
            aeron,
            parser: FixParser::new(),
            stream_id,
        })
    }
}

impl FixApp for GatewayBridgeApp {
    fn on_logon(&mut self, _ctx: &mut EngineContext<'_>) -> io::Result<()> {
        step(
            "gateway",
            "venue logon completed; waiting for NewOrderSingle",
        );
        Ok(())
    }

    fn on_message(
        &mut self,
        msg_type: &[u8],
        msg: &MessageView<'_>,
        ctx: &mut EngineContext<'_>,
    ) -> io::Result<()> {
        if msg_type != b"D" {
            return Ok(());
        }

        step("gateway", "received FIX NewOrderSingle from venue");
        detail("fix wire", fix_wire(msg.buffer()));

        let normalized_order = NormalizedOrder::from_fix(msg)?;
        detail(
            "normalized order",
            format!(
                "ClOrdID={} Symbol={} Qty={} Price={} Sender={} Target={}",
                normalized_order.cl_ord_id,
                normalized_order.symbol,
                normalized_order.qty,
                normalized_order.price,
                normalized_order.sender_comp_id,
                normalized_order.target_comp_id,
            ),
        );

        let mut payload = Vec::new();
        let payload_len = encode_normalized_order_as_sbe(&mut payload, &normalized_order)?;
        let started = Instant::now();
        let encoded_len = self
            .aeron
            .publish_payload(self.stream_id, &payload[..payload_len])?;
        detail(
            "sbe publish",
            format!(
                "encoded_len={encoded_len} bytes stream_id={}",
                self.stream_id
            ),
        );

        let (_, reply_payload) = self.aeron.recv_payload(RECEIVE_TIMEOUT)?;
        let reply = decode_normalized_execution_report_from_sbe(&reply_payload)?;
        detail(
            "sbe reply",
            format!(
                "ExecType={} OrdStatus={} OrderID={} ExecID={}",
                char::from(reply.exec_type),
                char::from(reply.ord_status),
                reply.order_id,
                reply.exec_id,
            ),
        );

        let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
        let seq = ctx.next_seq_num();
        let sender = ctx.session().config().sender_comp_id.clone();
        let target = ctx.session().config().target_comp_id.clone();
        let mut buf = [0u8; 2048];
        let len = serializer::build_execution_report(
            &mut buf,
            b"FIX.4.4",
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            reply.order_id.as_bytes(),
            reply.exec_id.as_bytes(),
            reply.cl_ord_id.as_bytes(),
            reply.symbol.as_bytes(),
            reply.side,
            reply.order_qty,
            reply.last_qty,
            reply.last_px.as_bytes(),
            reply.leaves_qty,
            reply.cum_qty,
            reply.avg_px.as_bytes(),
            reply.exec_type,
            reply.ord_status,
        );

        let response_wire = &buf[..len];
        assert_execution_report(&self.parser, response_wire, &reply.cl_ord_id)?;
        ctx.send_raw(response_wire)?;

        step(
            "gateway",
            "converted normalized SBE execution into FIX ExecutionReport",
        );
        detail("execution report wire", fix_wire(response_wire));
        highlight("gateway round trip", format_duration(started.elapsed()));
        Ok(())
    }

    fn on_logout(&mut self) -> io::Result<()> {
        step(
            "gateway",
            "venue logout received; shutting down gateway runner",
        );
        Ok(())
    }
}

fn run_internal_executor(
    channel: String,
    aeron_dir: String,
    stream_id: i32,
    ready_file: PathBuf,
) -> io::Result<()> {
    step(
        "internal",
        "starting executor service against standalone Aeron media driver",
    );
    let mut aeron = RawAeronLink::connect(&aeron_dir, &channel, stream_id + 1, stream_id)?;
    write_ready_file(&ready_file, "internal-ready")?;
    step("internal", "subscribed to normalized order stream");

    let (_, order_payload) = aeron.recv_payload(RECEIVE_TIMEOUT)?;
    let order = decode_normalized_order_from_sbe(&order_payload)?;
    step("internal", "received normalized order over SBE");
    detail(
        "normalized order",
        format!(
            "ClOrdID={} Symbol={} Qty={} Price={} Side={}",
            order.cl_ord_id,
            order.symbol,
            order.qty,
            order.price,
            char::from(order.side),
        ),
    );

    let report = NormalizedExecutionReport::from_order(&order);
    let mut report_payload = Vec::new();
    let report_payload_len =
        encode_normalized_execution_report_as_sbe(&mut report_payload, &report)?;
    let encoded_len = aeron.publish_payload(stream_id, &report_payload[..report_payload_len])?;
    step("internal", "published normalized execution report over SBE");
    detail(
        "normalized execution",
        format!(
            "OrderID={} ExecID={} LastQty={} AvgPx={} encoded_len={} bytes",
            report.order_id, report.exec_id, report.last_qty, report.avg_px, encoded_len,
        ),
    );
    Ok(())
}

fn run_mock_venue(host: String, port: u16) -> io::Result<()> {
    step(
        "venue",
        &format!("connecting to FIX gateway at {host}:{port}"),
    );

    let config = FixClientConfig {
        remote_host: host,
        remote_port: port,
        sender_comp_id: VENUE_COMP_ID.into(),
        target_comp_id: GATEWAY_COMP_ID.into(),
        ..Default::default()
    };

    let client = FixClient::new(config);
    let mut app = VenueApp::default();
    client.connect_and_run(&mut app)
}

#[derive(Default)]
struct VenueApp {
    order_sent_at: Option<Instant>,
}

impl FixApp for VenueApp {
    fn on_logon(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        step("venue", "logon complete; sending FIX NewOrderSingle");

        let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
        let seq = ctx.next_seq_num();
        let sender = ctx.session().config().sender_comp_id.clone();
        let target = ctx.session().config().target_comp_id.clone();

        let mut buf = [0u8; 1024];
        let len = serializer::build_new_order_single(
            &mut buf,
            b"FIX.4.4",
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            b"BPIPE-ORD-0001",
            b"EUR/USD",
            b'1',
            5_000_000,
            b'2',
            b"1.08255",
        );

        let wire = &buf[..len];
        detail("venue order wire", fix_wire(wire));
        self.order_sent_at = Some(Instant::now());
        ctx.send_raw(wire)
    }

    fn on_message(
        &mut self,
        msg_type: &[u8],
        msg: &MessageView<'_>,
        ctx: &mut EngineContext<'_>,
    ) -> io::Result<()> {
        if msg_type != b"8" {
            return Ok(());
        }

        let cl_ord_id = msg.get_field_str(tags::CL_ORD_ID).unwrap_or("?");
        let order_id = msg.get_field_str(tags::ORDER_ID).unwrap_or("?");
        let exec_id = msg.get_field_str(tags::EXEC_ID).unwrap_or("?");
        let exec_type = msg.get_field_str(tags::EXEC_TYPE).unwrap_or("?");
        let cum_qty = msg.get_field_i64(tags::CUM_QTY).unwrap_or(0);
        let avg_px = msg.get_field_str(tags::AVG_PX).unwrap_or("?");
        step("venue", "received FIX ExecutionReport from gateway");
        detail("execution report wire", fix_wire(msg.buffer()));
        detail(
            "execution summary",
            format!(
                "ClOrdID={cl_ord_id} OrderID={order_id} ExecID={exec_id} ExecType={exec_type} CumQty={cum_qty} AvgPx={avg_px}"
            ),
        );
        if let Some(sent) = self.order_sent_at.take() {
            highlight("venue order round trip", format_duration(sent.elapsed()));
        }

        ctx.request_stop();
        Ok(())
    }

    fn on_logout(&mut self) -> io::Result<()> {
        step("venue", "logout complete; venue runner exiting");
        Ok(())
    }
}

fn assert_execution_report(
    parser: &FixParser,
    fix: &[u8],
    expected_cl_ord_id: &str,
) -> io::Result<()> {
    let (view, _) = parser.parse(fix).map_err(parser_error)?;
    if view.msg_type() != Some(b"8") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "gateway did not build an ExecutionReport",
        ));
    }
    let cl_ord_id = view.get_field_str(tags::CL_ORD_ID).unwrap_or("?");
    if cl_ord_id != expected_cl_ord_id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected ClOrdID in ExecutionReport: {cl_ord_id}"),
        ));
    }
    Ok(())
}
