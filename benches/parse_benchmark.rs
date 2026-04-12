/// BM-01: Message Parse Latency Benchmark
///
/// Measures the time to parse FIX messages of various sizes using Criterion.
/// Compares against industry benchmark targets from BENCHMARKS.md.
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velocitas_fix::parser::FixParser;
use velocitas_fix::serializer;

/// Generate a valid FIX Heartbeat message.
fn gen_heartbeat() -> Vec<u8> {
    let mut buf = vec![0u8; 1024];
    let len = serializer::build_heartbeat(
        &mut buf,
        b"FIX.4.4",
        b"SENDER",
        b"TARGET",
        1,
        b"20260321-10:00:00.000",
    );
    buf.truncate(len);
    buf
}

/// Generate a valid FIX NewOrderSingle message.
fn gen_new_order_single() -> Vec<u8> {
    let mut buf = vec![0u8; 1024];
    let len = serializer::build_new_order_single(
        &mut buf,
        b"FIX.4.4",
        b"BANK_OMS_DESK1",
        b"NYSE_MATCHING",
        42,
        b"20260321-10:00:00.123456",
        b"ORD-2026032100001",
        b"AAPL",
        b'1',
        10000,
        b'2',
        b"178.5500",
    );
    buf.truncate(len);
    buf
}

/// Generate a valid FIX ExecutionReport message.
fn gen_execution_report() -> Vec<u8> {
    let mut buf = vec![0u8; 2048];
    let len = serializer::build_execution_report(
        &mut buf,
        b"FIX.4.4",
        b"NYSE_MATCHING",
        b"BANK_OMS_DESK1",
        100,
        b"20260321-10:00:00.456789",
        b"NYSE-ORD-20260321-000001",
        b"NYSE-EXEC-20260321-000001",
        b"ORD-2026032100001",
        b"AAPL",
        b'1',
        10000,
        5000,
        b"178.5500",
        5000,
        5000,
        b"178.5500",
        b'F',
        b'1',
    );
    buf.truncate(len);
    buf
}

/// Generate a batch of N pre-built messages for throughput testing.
fn gen_message_batch(count: usize) -> Vec<Vec<u8>> {
    (0..count).map(|_| gen_new_order_single()).collect()
}

fn bench_parse_latency(c: &mut Criterion) {
    let parser = FixParser::new();
    let parser_unchecked = FixParser::new_unchecked();

    let heartbeat = gen_heartbeat();
    let nos = gen_new_order_single();
    let exec_rpt = gen_execution_report();

    let mut group = c.benchmark_group("BM-01_parse_latency");

    // Target: ≤ 80 ns (industry: ~200 ns)
    group.bench_function("heartbeat_validated", |b| {
        b.iter(|| parser.parse(black_box(&heartbeat)))
    });

    group.bench_function("heartbeat_unchecked", |b| {
        b.iter(|| parser_unchecked.parse(black_box(&heartbeat)))
    });

    // Target: ≤ 300 ns (industry: ~800 ns)
    group.bench_function("new_order_single_validated", |b| {
        b.iter(|| parser.parse(black_box(&nos)))
    });

    group.bench_function("new_order_single_unchecked", |b| {
        b.iter(|| parser_unchecked.parse(black_box(&nos)))
    });

    // Target: ≤ 500 ns (industry: ~1,500 ns)
    group.bench_function("execution_report_validated", |b| {
        b.iter(|| parser.parse(black_box(&exec_rpt)))
    });

    group.bench_function("execution_report_unchecked", |b| {
        b.iter(|| parser_unchecked.parse(black_box(&exec_rpt)))
    });

    group.finish();
}

fn bench_parse_throughput(c: &mut Criterion) {
    let parser = FixParser::new_unchecked();
    let batch = gen_message_batch(10_000);

    let mut group = c.benchmark_group("BM-01_parse_throughput");

    group.throughput(Throughput::Elements(batch.len() as u64));
    group.bench_function("10k_messages", |b| {
        b.iter(|| {
            for msg in &batch {
                let _ = parser.parse(black_box(msg));
            }
        })
    });

    group.finish();
}

fn bench_parse_by_size(c: &mut Criterion) {
    let parser = FixParser::new();

    let messages: Vec<(&str, Vec<u8>)> = vec![
        ("heartbeat_60B", gen_heartbeat()),
        ("NOS_220B", gen_new_order_single()),
        ("ExecRpt_450B", gen_execution_report()),
    ];

    let mut group = c.benchmark_group("BM-01_parse_by_size");

    for (name, msg) in &messages {
        group.throughput(Throughput::Bytes(msg.len() as u64));
        group.bench_with_input(BenchmarkId::new("parse", name), msg, |b, msg| {
            b.iter(|| parser.parse(black_box(msg)))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_parse_latency,
    bench_parse_throughput,
    bench_parse_by_size,
);
criterion_main!(benches);
