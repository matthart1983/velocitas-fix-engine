# Velocitas FIX Engine — Performance Benchmark Suite

## 1. Benchmark Methodology

### 1.1 Principles

- Benchmarks measured using [Criterion.rs](https://github.com/bheisler/criterion.rs)
- Results reported as Criterion mean estimates unless otherwise noted
- QuickFIX/J comparison uses simple loop methodology (1M iterations) for apples-to-apples comparison

### 1.2 Reference Hardware

| Component | Specification |
|---|---|
| CPU | Apple Silicon (ARM64) |
| OS | macOS |
| Toolchain | Rust stable, `--release` profile |

### 1.3 Comparison Targets

| Engine | Version | Notes |
|---|---|---|
| QuickFIX/J | 2.3.1 | Open source, Java |
| **Velocitas** | **1.0** | **This engine** |

---

## 2. Measured Results

### BM-01: Message Parse Latency (Criterion)

**Objective:** Measure time to parse a FIX message from a byte buffer into an accessible field structure.

| Message | Validated | Unchecked |
|---|---|---|
| Heartbeat | 1.15 µs | 1.17 µs |
| NewOrderSingle | 1.20 µs | 1.18 µs |
| ExecutionReport | 1.25 µs | 1.22 µs |

**Throughput:** 10k messages parsed in 4.43 ms → **~2.26M msg/s**

---

### BM-02: Message Serialization Latency (Criterion)

**Objective:** Measure time to build a FIX message from field values into a wire-format byte buffer (including BodyLength, CheckSum computation).

| Message | Mean Latency |
|---|---|
| Heartbeat | 28 ns |
| Logon | 33 ns |
| NewOrderSingle | 59 ns |
| ExecutionReport | 37 ns |

**Throughput:** 10k NOS serialized in 463 µs → **~21.6M msg/s**

---

### BM-03: Roundtrip Latency (Criterion)

**Objective:** Measure time to serialize then parse a NewOrderSingle message.

| Operation | Mean Latency |
|---|---|
| NOS serialize → parse roundtrip | 131 ns |

---

### BM-04: Sustained Throughput (Criterion, 100k messages)

**Objective:** Measure sustained message processing rate.

| Mode | Throughput |
|---|---|
| Parse unchecked | 2.25M msg/s |
| Parse validated | 2.15M msg/s |
| Parse + Respond (NOS → ExecRpt) | 1.56M msg/s |

---

### BM-05: Burst Handling

*Not yet measured.*

---

### BM-06: Session Lifecycle (Criterion)

**Objective:** Measure individual session operation latencies.

| Operation | Mean Latency |
|---|---|
| Logon handshake | 97 ns |
| Sequence validation hit | 15 ns |
| Rate limit check | 19 ns |
| Build logon | 11 ns |
| Build and parse logon | 74 ns |

---

### BM-07: Sequence Recovery (Criterion)

**Objective:** Measure sequence number gap detection and validation performance.

| Operation | Mean Latency |
|---|---|
| Gap detect (1000 messages) | 96 ns |
| Sequential 10k validations | 143 µs |

---

### BM-08: Pool Allocation (Criterion)

**Objective:** Measure buffer pool allocation performance.

| Operation | Mean Latency |
|---|---|
| Allocate/deallocate 256B | 8.5 ns |
| Burst allocate 1000 | 8.4 µs |

---

### BM-11: TCP Round-Trip Latency

**Objective:** Measure end-to-end latency of a NewOrderSingle → ExecutionReport round-trip over real TCP on localhost.

**Setup:** Acceptor and initiator on the same host, connected via loopback TCP. 10,000 sequential round-trips (send NOS, wait for ExecRpt, record latency, repeat).

**Velocitas Results:**

| Percentile | Latency |
|---|---|
| min | 10.9 µs |
| mean | 17.6 µs |
| p50 | 15.6 µs |
| p90 | 18.7 µs |
| p99 | 52.9 µs |
| p99.9 | 86.5 µs |
| max | 125.1 µs |

**Throughput:** 111,686 msgs/sec (55,843 round-trips/sec)

---

### BM-09: Coordinated Omission Resistance

*Not yet measured.*

---

### BM-10: Jitter Analysis

*Not yet measured.*

---

## 3. QuickFIX/J Comparison

Both engines benchmarked using the same simple loop methodology with 1M iterations each, on the same hardware.

| Benchmark | Velocitas | QuickFIX/J | Ratio |
|---|---|---|---|
| Parse Heartbeat | 474 ns | 330 ns | 0.70x |
| Parse NewOrderSingle | 532 ns | 796 ns | 1.50x faster |
| Parse ExecutionReport | 583 ns | 1,270 ns | 2.18x faster |
| Serialize NewOrderSingle | 32 ns | 917 ns | 28.7x faster |
| Mixed throughput | 1.83M msg/s | 1.21M msg/s | 1.51x higher |

> **Note:** QuickFIX/J wins on Heartbeat parsing (a very small message). Velocitas is significantly faster on larger messages and serialization.

### TCP Round-Trip (10,000 NOS → ExecRpt, localhost)

| Metric | Velocitas | QuickFIX/J | Ratio |
|---|---|---|---|
| p50 latency | 15.6 µs | 61.1 µs | 3.9x faster |
| p99 latency | 52.9 µs | 342.7 µs | 6.5x faster |
| p99.9 latency | 86.5 µs | 653.4 µs | 7.5x faster |
| max latency | 125.1 µs | 3,939.8 µs | 31.5x faster |
| Throughput | 111,686 msg/s | 24,948 msg/s | 4.5x higher |

> **Note:** QuickFIX/J's max latency (3.9 ms) reflects Java GC pauses. Velocitas has no GC and shows consistent tail latency.

---

## 4. Continuous Benchmark Pipeline

### 4.1 PR Gate Benchmarks

- Run BM-01 (parse) and BM-02 (serialize) on every PR
- Compare against `main` branch baseline
- Block merge if median regresses by ≥ 3%
- Run with `--release` profile and `RUSTFLAGS="-C target-cpu=native"`
