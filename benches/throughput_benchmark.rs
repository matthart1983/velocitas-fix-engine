/// BM-04/05: Sustained Throughput and Burst Handling Benchmarks
///
/// Measures maximum message processing rate and behavior under burst load.
use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::time::Instant;
use velocitas_fix::parser::FixParser;
use velocitas_fix::pool::BufferPool;
use velocitas_fix::serializer;

/// Pre-generate a large batch of FIX messages for throughput testing.
fn gen_nos_batch(count: usize) -> Vec<Vec<u8>> {
    let mut messages = Vec::with_capacity(count);
    for i in 0..count {
        let mut buf = vec![0u8; 512];
        let cl_ord_id = format!("ORD-{:08}", i);
        let len = serializer::build_new_order_single(
            &mut buf,
            b"FIX.4.4",
            b"BANK",
            b"EXCH",
            (i + 1) as u64,
            b"20260321-10:00:00",
            cl_ord_id.as_bytes(),
            b"AAPL",
            b'1',
            1000,
            b'2',
            b"150.00",
        );
        buf.truncate(len);
        messages.push(buf);
    }
    messages
}

fn bench_sustained_throughput(c: &mut Criterion) {
    let parser = FixParser::new_unchecked();
    let messages = gen_nos_batch(100_000);

    let mut group = c.benchmark_group("BM-04_sustained_throughput");
    group.throughput(Throughput::Elements(messages.len() as u64));
    group.sample_size(20);

    group.bench_function("100k_NOS_parse_unchecked", |b| {
        b.iter(|| {
            for msg in &messages {
                let _ = parser.parse(black_box(msg));
            }
        })
    });

    let parser_checked = FixParser::new();
    group.bench_function("100k_NOS_parse_validated", |b| {
        b.iter(|| {
            for msg in &messages {
                let _ = parser_checked.parse(black_box(msg));
            }
        })
    });

    group.finish();
}

fn bench_parse_and_respond(c: &mut Criterion) {
    let parser = FixParser::new_unchecked();
    let messages = gen_nos_batch(10_000);

    let mut group = c.benchmark_group("BM-04_parse_and_respond");
    group.throughput(Throughput::Elements(messages.len() as u64));

    group.bench_function("NOS_to_ExecRpt", |b| {
        let mut out_buf = [0u8; 2048];
        b.iter(|| {
            for msg in &messages {
                // Parse inbound NOS
                let (view, _) = parser.parse(black_box(msg)).unwrap();
                // Build outbound ExecutionReport
                let _ = serializer::build_execution_report(
                    &mut out_buf,
                    b"FIX.4.4",
                    b"EXCH",
                    b"BANK",
                    1,
                    b"20260321-10:00:00",
                    b"ORD-001",
                    b"EXEC-001",
                    view.get_field(11).unwrap_or(b"?"),
                    view.get_field(55).unwrap_or(b"?"),
                    b'1',
                    1000,
                    1000,
                    b"150.00",
                    0,
                    1000,
                    b"150.00",
                    b'F',
                    b'2',
                );
            }
        })
    });

    group.finish();
}

fn bench_pool_allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("BM-08_pool_allocation");

    group.bench_function("allocate_deallocate_256B", |b| {
        let pool = BufferPool::new(256, 1024);
        b.iter(|| {
            let handle = pool.allocate().unwrap();
            pool.deallocate(handle);
        })
    });

    group.bench_function("burst_allocate_1000", |b| {
        let pool = BufferPool::new(256, 2048);
        b.iter(|| {
            let mut handles = Vec::with_capacity(1000);
            for _ in 0..1000 {
                handles.push(pool.allocate().unwrap());
            }
            for h in handles {
                pool.deallocate(h);
            }
        })
    });

    group.finish();
}

/// Raw throughput measurement (outside Criterion for msg/s reporting).
/// Run with: cargo test --release -- --nocapture bench_raw_throughput
#[cfg(test)]
mod raw_throughput {
    use super::*;

    #[test]
    fn bench_raw_throughput() {
        let parser = FixParser::new_unchecked();
        let messages = gen_nos_batch(1_000_000);

        // Warm up
        for msg in messages.iter().take(10_000) {
            let _ = parser.parse(msg);
        }

        let start = Instant::now();
        for msg in &messages {
            let _ = parser.parse(black_box(msg));
        }
        let elapsed = start.elapsed();

        let msgs_per_sec = messages.len() as f64 / elapsed.as_secs_f64();
        let ns_per_msg = elapsed.as_nanos() as f64 / messages.len() as f64;

        println!("\n=== BM-04 Raw Throughput (1M messages) ===");
        println!("  Total time:    {:?}", elapsed);
        println!("  Messages/sec:  {:.0}", msgs_per_sec);
        println!("  ns/message:    {:.1}", ns_per_msg);
        println!("  Target:        ≥ 2,000,000 msg/s");
        println!(
            "  Status:        {}",
            if msgs_per_sec >= 2_000_000.0 {
                "✅ PASS"
            } else {
                "❌ BELOW TARGET"
            }
        );
    }
}

criterion_group!(
    benches,
    bench_sustained_throughput,
    bench_parse_and_respond,
    bench_pool_allocation,
);
criterion_main!(benches);
