/// BM-02: Message Serialization Latency Benchmark
///
/// Measures the time to build and serialize FIX messages from field values.
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use velocitas_fix::serializer;

fn bench_serialize_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("BM-02_serialize_latency");

    // Target: ≤ 50 ns (industry: ~150 ns)
    group.bench_function("heartbeat", |b| {
        let mut buf = [0u8; 1024];
        b.iter(|| {
            serializer::build_heartbeat(
                black_box(&mut buf),
                b"FIX.4.4",
                b"SENDER",
                b"TARGET",
                1,
                b"20260321-10:00:00.000",
            )
        })
    });

    // Target: ≤ 100 ns
    group.bench_function("logon", |b| {
        let mut buf = [0u8; 1024];
        b.iter(|| {
            serializer::build_logon(
                black_box(&mut buf),
                b"FIX.4.4",
                b"CLIENT",
                b"SERVER",
                1,
                b"20260321-10:00:00.000",
                30,
            )
        })
    });

    // Target: ≤ 250 ns (industry: ~700 ns)
    group.bench_function("new_order_single", |b| {
        let mut buf = [0u8; 1024];
        b.iter(|| {
            serializer::build_new_order_single(
                black_box(&mut buf),
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
            )
        })
    });

    // Target: ≤ 400 ns (industry: ~1,200 ns)
    group.bench_function("execution_report", |b| {
        let mut buf = [0u8; 2048];
        b.iter(|| {
            serializer::build_execution_report(
                black_box(&mut buf),
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
            )
        })
    });

    group.finish();
}

fn bench_serialize_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("BM-02_serialize_throughput");
    let count = 10_000u64;
    group.throughput(Throughput::Elements(count));

    group.bench_function("10k_NOS", |b| {
        let mut buf = [0u8; 1024];
        b.iter(|| {
            for seq in 1..=count {
                let _ = serializer::build_new_order_single(
                    black_box(&mut buf),
                    b"FIX.4.4",
                    b"BANK",
                    b"NYSE",
                    seq,
                    b"20260321-10:00:00",
                    b"ORD-00001",
                    b"AAPL",
                    b'1',
                    1000,
                    b'2',
                    b"150.00",
                );
            }
        })
    });

    group.finish();
}

fn bench_serialize_roundtrip(c: &mut Criterion) {
    let parser = velocitas_fix::parser::FixParser::new();
    let mut group = c.benchmark_group("BM-03_serialize_then_parse_roundtrip");

    group.bench_function("NOS_roundtrip", |b| {
        let mut buf = [0u8; 1024];
        b.iter(|| {
            let len = serializer::build_new_order_single(
                &mut buf,
                b"FIX.4.4",
                b"BANK",
                b"NYSE",
                1,
                b"20260321-10:00:00",
                b"ORD-00001",
                b"AAPL",
                b'1',
                1000,
                b'2',
                b"150.00",
            );
            let _ = parser.parse(black_box(&buf[..len]));
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_serialize_latency,
    bench_serialize_throughput,
    bench_serialize_roundtrip,
);
criterion_main!(benches);
