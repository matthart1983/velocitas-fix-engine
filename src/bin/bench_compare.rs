/// Velocitas FIX Engine benchmark — matches QuickFIX/J benchmark scenarios.
///
/// Run with: cargo run --release --bin bench_compare

use std::time::Instant;

use velocitas_fix::parser::FixParser;
use velocitas_fix::serializer;

const WARMUP: usize = 100_000;
const ITERATIONS: usize = 1_000_000;

fn main() {
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║           Velocitas FIX Engine Benchmark  (Rust)                ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();

    let heartbeat = build_heartbeat();
    let nos = build_nos();
    let exec_rpt = build_exec_rpt();

    bench_parse("Heartbeat", &heartbeat);
    bench_parse("NewOrderSingle", &nos);
    bench_parse("ExecutionReport", &exec_rpt);

    println!();
    bench_serialize();

    println!();
    bench_throughput(&nos);
}

fn bench_parse(label: &str, msg: &[u8]) {
    let parser = FixParser::new();

    // Warmup
    for _ in 0..WARMUP {
        let _ = std::hint::black_box(parser.parse(std::hint::black_box(msg)));
    }

    // Measure
    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = std::hint::black_box(parser.parse(std::hint::black_box(msg)));
    }
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / ITERATIONS as f64;
    println!("  Parse {:<20}  {:.0} ns/msg  ({} bytes)", label, ns_per_op, msg.len());
}

fn bench_serialize() {
    let mut buf = [0u8; 1024];

    // Warmup
    for i in 0..WARMUP {
        std::hint::black_box(build_nos_with_seq(&mut buf, i as u64));
    }

    // Measure
    let start = Instant::now();
    for i in 0..ITERATIONS {
        std::hint::black_box(build_nos_with_seq(&mut buf, i as u64));
    }
    let elapsed = start.elapsed();
    let ns_per_op = elapsed.as_nanos() as f64 / ITERATIONS as f64;
    println!("  Serialize NOS              {:.0} ns/msg", ns_per_op);
}

fn bench_throughput(msg: &[u8]) {
    let parser = FixParser::new();

    // Warmup
    for _ in 0..WARMUP {
        let _ = std::hint::black_box(parser.parse(std::hint::black_box(msg)));
    }

    let start = Instant::now();
    for _ in 0..ITERATIONS {
        let _ = std::hint::black_box(parser.parse(std::hint::black_box(msg)));
    }
    let elapsed = start.elapsed();
    let msgs_per_sec = ITERATIONS as f64 / elapsed.as_secs_f64();
    println!("  Throughput (NOS parse)      {:.0} msgs/sec  ({} messages)", msgs_per_sec, ITERATIONS);
}

fn build_heartbeat() -> Vec<u8> {
    let mut buf = vec![0u8; 1024];
    let len = serializer::build_heartbeat(
        &mut buf, b"FIX.4.4", b"SENDER", b"TARGET", 1,
        b"20260321-10:00:00.000",
    );
    buf.truncate(len);
    buf
}

fn build_nos() -> Vec<u8> {
    let mut buf = vec![0u8; 1024];
    let len = serializer::build_new_order_single(
        &mut buf, b"FIX.4.4", b"BANK_OMS", b"NYSE", 42,
        b"20260321-10:00:00.123", b"ORD-2026032100001", b"AAPL",
        b'1', 10_000, b'2', b"178.55",
    );
    buf.truncate(len);
    buf
}

fn build_exec_rpt() -> Vec<u8> {
    let mut buf = vec![0u8; 2048];
    let len = serializer::build_execution_report(
        &mut buf, b"FIX.4.4", b"NYSE", b"BANK_OMS", 100,
        b"20260321-10:00:00.456", b"NYSE-ORD-001", b"NYSE-EXEC-001",
        b"ORD-2026032100001", b"AAPL", b'1', 10_000, 5_000,
        b"178.55", 5_000, 5_000, b"178.55", b'F', b'1',
    );
    buf.truncate(len);
    buf
}

fn build_nos_with_seq(buf: &mut [u8], seq: u64) -> usize {
    serializer::build_new_order_single(
        buf, b"FIX.4.4", b"BANK_OMS", b"NYSE", seq,
        b"20260321-10:00:00.123", b"ORD-2026032100001", b"AAPL",
        b'1', 10_000, b'2', b"178.55",
    )
}
