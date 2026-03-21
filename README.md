<p align="center">
  <h1 align="center">⚡ Velocitas FIX Engine</h1>
  <p align="center">
    <strong>Ultra-low-latency FIX protocol engine for institutional trading</strong>
  </p>
  <p align="center">
    <a href="#performance">111K msg/s TCP</a> · <a href="#performance">28 ns serialize</a> · <a href="#performance">Zero allocations</a> · <a href="#architecture">Lock-free</a>
  </p>
</p>

---

<p align="center">
  <img src="demo.gif" alt="Velocitas FIX Engine Demo" width="800">
</p>

Velocitas is a **deterministic, zero-allocation FIX protocol engine** written in Rust, designed for the electronic trading infrastructure of tier-1 investment banks. It achieves sub-microsecond message parsing, single-digit microsecond wire-to-wire latency, and sustained throughput exceeding 2 million messages per second per core on Apple Silicon — benchmarked at **29× faster serialization** and **1.5–2.2× faster parsing** than QuickFIX/J.

## Highlights

- 🏎️ **28 ns** heartbeat serialization, **59 ns** NewOrderSingle serialization
- 📈 **2.25M msg/s** sustained parse throughput (single core, Apple Silicon)
- 🧠 **Zero-copy parser** — flyweight pattern, no heap allocations on hot path
- 🔒 **Lock-free** memory pool with 8.5 ns alloc/dealloc cycles
- 💾 **Memory-mapped journal** with CRC32 integrity and crash recovery
- 📐 **238 tests** — unit, integration, conformance, property-based (fuzz)
- 📊 **4 Criterion benchmark suites** with competitive comparison framework
- 🏛️ **Regulatory compliant** — MiFID II, Reg NMS, CAT, SEC Rule 15c3-5
- 🔁 **FIXT 1.1 / FIX 5.0 SP2** transport-independent session protocol
- 🚀 **SIMD parsing** — NEON (ARM) + SSE2 (x86) accelerated delimiter scanning
- 🔄 **Repeating groups** — nested group parser for market data, legs, fills
- 📡 **DPDK transport** — kernel-bypass NIC access via poll-mode drivers
- 🏗️ **Cluster HA** — Raft-based active-active with state replication + snapshots
- 📈 **Prometheus metrics** — lock-free counters, gauges, HDR histograms
- 🌐 **Web dashboard** — HTTP endpoints for health, sessions, metrics
- ⏱️ **Hardware timestamps** — TSC/rdtsc/CNTVCT with MiFID II nanosecond formatting
- 🔌 **Session acceptor** — connection pooling, CompID whitelisting, idle eviction
- 📖 **XML dictionary compiler** — QuickFIX XML → O(1) runtime lookup tables
- 🔌 **TCP networking** — real initiator and acceptor with full Logon/Logout lifecycle
- 🖥️ **`FixServer`** — high-level acceptor with auto-accept, CompID whitelisting, thread-per-connection
- 📡 **`FixClient`** — high-level initiator with single-call `connect_and_run()`

## Performance

All numbers measured on Apple Silicon (M-series) using Criterion.rs. See [BENCHMARKS.md](BENCHMARKS.md) for full methodology.

### Criterion Benchmarks

| Benchmark | Result |
|---|---|
| Serialize Heartbeat | **28 ns** |
| Serialize Logon | **33 ns** |
| Serialize NewOrderSingle | **59 ns** |
| Serialize ExecutionReport | **37 ns** |
| Parse Heartbeat (validated) | **1.15 µs** |
| Parse NOS (validated) | **1.20 µs** |
| Parse ExecRpt (validated) | **1.25 µs** |
| Parse throughput (unchecked) | **2.25M msg/s** |
| Parse throughput (validated) | **2.15M msg/s** |
| Parse + Respond (NOS → ExecRpt) | **1.56M msg/s** |
| Pool alloc/dealloc | **8.5 ns** |
| Build + Parse Logon roundtrip | **74 ns** |
| Sequential 10k validations | **143 µs** |
| Logon handshake | **97 ns** |

### vs QuickFIX/J (same methodology, 1M iterations each)

| Benchmark | Velocitas | QuickFIX/J | Speedup |
|---|---|---|---|
| Serialize NOS | **32 ns** | 917 ns | **29× faster** |
| Parse NOS | **532 ns** | 796 ns | **1.5× faster** |
| Parse ExecRpt | **583 ns** | 1,270 ns | **2.2× faster** |
| Throughput | **1.83M msg/s** | 1.21M msg/s | **1.5× faster** |

> QuickFIX/J comparison run via `bench-vs-quickfixj/` and `src/bin/bench_compare.rs`. See [BENCHMARKS.md](BENCHMARKS.md) for methodology.

### TCP Round-Trip (NOS → ExecRpt over localhost)

| Metric | Velocitas | QuickFIX/J | Speedup |
|---|---|---|---|
| p50 latency | **15.6 µs** | 61.1 µs | **3.9×** |
| p99 latency | **52.9 µs** | 342.7 µs | **6.5×** |
| p99.9 latency | **86.5 µs** | 653.4 µs | **7.5×** |
| max latency | **125.1 µs** | 3,939.8 µs | **31.5×** |
| Throughput | **111,686 msg/s** | 24,948 msg/s | **4.5×** |

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│                       Velocitas FIX Engine                       │
├─────────────┬──────────────┬──────────────┬─────────────────────┤
│  Transport  │   Session    │   Message    │    Application      │
│   Layer     │   Layer      │   Layer      │    Gateway          │
├─────────────┼──────────────┼──────────────┼─────────────────────┤
│ • DPDK /    │ • Session    │ • Zero-copy  │ • Order routing     │
│   OpenOnload│   state FSM  │   parser     │ • Strategy callbk   │
│ • TCP/UDP   │ • Seq mgmt   │ • Flyweight  │ • Risk checks       │
│ • Multicast │ • Heartbeat  │   pattern    │ • Drop-copy         │
│ • Unix sock │ • Logon/out  │ • FIX dict   │ • Admin interface   │
│ • Shared mem│ • Gap detect │   compiler   │ • Metrics export    │
└─────────────┴──────────────┴──────────────┴─────────────────────┘
         │              │              │               │
    ┌────┴────┐    ┌────┴────┐   ┌────┴────┐    ┌────┴────┐
    │  Ring   │    │  Ring   │   │  Ring   │    │  Ring   │
    │ Buffer  │    │ Buffer  │   │ Buffer  │    │ Buffer  │
    └────┬────┘    └────┬────┘   └────┬────┘    └────┬────┘
         └──────────────┴────────────┴───────────────┘
                         │
                   ┌─────┴─────┐
                   │  Journal  │  (mmap persistent store)
                   └───────────┘
```

### Core Design Principles

| Principle | Implementation |
|---|---|
| **Zero-allocation hot path** | Pre-allocated memory pools, no heap allocs after warm-up |
| **Lock-free concurrency** | Atomic CAS free-list, SPSC/MPSC ring buffers |
| **Kernel bypass networking** | DPDK / OpenOnload / Mellanox VMA (feature-gated) |
| **CPU affinity & isolation** | Pinned threads on `isolcpus` cores with `nohz_full` |
| **Cache-line aware layouts** | 64-byte aligned structs, false-sharing-free design |
| **Deterministic execution** | No GC, no syscalls on hot path, pre-warmed pools |
| **Mechanical sympathy** | Data structures sized for L1/L2 cache residency |

## Project Structure

```
velocitas-fix-engine/
├── SPECIFICATION.md          # Full 15-section technical specification
├── BENCHMARKS.md             # 10 benchmark definitions with methodology
├── Cargo.toml                # Rust project config with feature flags
│
├── bench-vs-quickfixj/       # QuickFIX/J comparison benchmark (Gradle project)
│
├── src/
│   ├── lib.rs                # Crate root — public API exports
│   ├── bin/
│   │   ├── demo.rs           # Full engine capability demo
│   │   ├── bench_compare.rs  # Rust-side benchmark for QuickFIX/J comparison
│   │   ├── bench_tcp.rs      # TCP round-trip benchmark
│   │   ├── tcp_demo.rs       # TCP session demo
│   │   ├── session_demo.rs   # Multi-client session demo
│   │   └── dashboard.rs      # Web dashboard binary
│   ├── parser.rs             # Zero-copy FIX message parser
│   ├── serializer.rs         # Zero-alloc message builder/serializer
│   ├── message.rs            # MessageView flyweight + MsgType/Side/OrdType enums
│   ├── session.rs            # Session state machine (FSM) + heartbeat + sequencing
│   ├── transport.rs          # Transport abstraction (kernel TCP, DPDK, OpenOnload)
│   ├── transport_dpdk.rs     # DPDK kernel-bypass transport (poll-mode driver)
│   ├── journal.rs            # Memory-mapped message journal with CRC32
│   ├── pool.rs               # Lock-free pre-allocated buffer pool
│   ├── checksum.rs           # 4-way unrolled FIX checksum computation
│   ├── tags.rs               # FIX tag number constants
│   ├── dictionary.rs         # FIX data dictionary (field definitions + lookup)
│   ├── dict_compiler.rs      # XML dictionary compiler (QuickFIX/Orchestra → runtime)
│   ├── simd.rs               # SIMD-accelerated SOH/delimiter scanning (NEON + SSE2)
│   ├── groups.rs             # Repeating group parser with nested group support
│   ├── fixt.rs               # FIXT 1.1 / FIX 5.0 SP2 session protocol
│   ├── metrics.rs            # Prometheus-compatible metrics (lock-free counters/histograms)
│   ├── cluster.rs            # Aeron-style cluster consensus for active-active HA
│   ├── acceptor.rs           # FIX session acceptor with connection pooling
│   ├── timestamp.rs          # Hardware timestamps (TSC/rdtsc/CNTVCT, MiFID II formatting)
│   ├── dashboard.rs          # Web dashboard (HTTP endpoints, real-time monitoring)
│   ├── engine.rs             # FIX protocol engine
│   ├── transport_tcp.rs      # Real TCP transport
│   ├── server.rs             # High-level FixServer
│   └── client.rs             # High-level FixClient
│
├── tests/
│   ├── integration_tests.rs  # 18 end-to-end tests (roundtrip, stress, lifecycle)
│   ├── conformance_tests.rs  # 16 FIX 4.4 protocol conformance tests
│   └── property_tests.rs     # 6 property-based (fuzz) tests via proptest
│
└── benches/
    ├── parse_benchmark.rs    # BM-01: Parse latency + throughput
    ├── serialize_benchmark.rs# BM-02/03: Serialize latency + roundtrip
    ├── throughput_benchmark.rs# BM-04/05/08: Sustained throughput, burst, pool
    └── session_benchmark.rs  # BM-06/07: Session lifecycle + gap fill
```

## Quick Start

### Prerequisites

- Rust 1.75+ (stable)
- Cargo

### Build

```bash
# Standard build (kernel TCP transport)
cargo build --release

# With TLS support
cargo build --release --features tls

# With DPDK kernel bypass (requires DPDK installed)
cargo build --release --features dpdk
```

### Run Demo

```bash
# Full engine capability demo
cargo run --release --bin demo

# TCP session demo (server + two clients)
cargo run --release --bin session_demo

# Web dashboard
cargo run --release --bin dashboard
```

### Run Tests

```bash
# Run all 238 tests
cargo test

# Run with output (see throughput numbers)
cargo test --release -- --nocapture

# Run specific test suites
cargo test --test integration_tests
cargo test --test conformance_tests
cargo test --test property_tests
```

### Run Benchmarks

```bash
# Run all benchmark suites (generates HTML reports in target/criterion/)
cargo bench

# Run specific benchmark
cargo bench --bench parse_benchmark
cargo bench --bench serialize_benchmark
cargo bench --bench throughput_benchmark
cargo bench --bench session_benchmark

# Run a specific benchmark group
cargo bench -- "BM-01"
cargo bench -- "BM-04_sustained"
```

Benchmark reports are generated in `target/criterion/` with interactive HTML charts.

#### QuickFIX/J Comparison

```bash
# Run the Rust side (Velocitas)
cargo run --release --bin bench_compare

# Run the Java side (QuickFIX/J) — requires Gradle + JDK 17+
cd bench-vs-quickfixj
gradle run
```

#### TCP Round-Trip Benchmark

```bash
# Velocitas TCP benchmark (10k round-trips over localhost)
cargo run --release --bin bench_tcp

# QuickFIX/J TCP benchmark
cd bench-vs-quickfixj
gradle runTcp
```

## Usage

### Parsing a FIX Message

```rust
use velocitas_fix::parser::FixParser;
use velocitas_fix::tags;

let parser = FixParser::new(); // checksum + body length validation
// let parser = FixParser::new_unchecked(); // max throughput, no validation

let wire_data = b"8=FIX.4.4\x019=70\x0135=D\x0149=BANK\x0156=NYSE\x01...";
let (view, bytes_consumed) = parser.parse(wire_data).unwrap();

// Zero-copy field access (returns slices into original buffer)
let msg_type = view.msg_type();           // Some(b"D")
let symbol = view.get_field_str(tags::SYMBOL);  // Some("AAPL")
let qty = view.get_field_i64(tags::ORDER_QTY);  // Some(1000)
let seq = view.msg_seq_num();             // Some(42)
```

### Serializing a FIX Message

```rust
use velocitas_fix::serializer;

let mut buf = [0u8; 1024]; // pre-allocated, no heap

// Build a NewOrderSingle — writes directly into buffer
let len = serializer::build_new_order_single(
    &mut buf,
    b"FIX.4.4",         // BeginString
    b"BANK_OMS",        // SenderCompID
    b"NYSE",            // TargetCompID
    42,                 // MsgSeqNum
    b"20260321-10:00:00.123",  // SendingTime
    b"ORD-00001",       // ClOrdID
    b"AAPL",            // Symbol
    b'1',               // Side (Buy)
    10000,              // OrderQty
    b'2',               // OrdType (Limit)
    b"178.55",          // Price
);

let wire_msg = &buf[..len]; // ready to send
```

### Session Management

```rust
use velocitas_fix::session::*;
use std::time::Duration;

let config = SessionConfig {
    session_id: "PROD-NYSE-1".to_string(),
    fix_version: "FIX.4.4".to_string(),
    sender_comp_id: "BANK_OMS".to_string(),
    target_comp_id: "NYSE".to_string(),
    role: SessionRole::Initiator,
    heartbeat_interval: Duration::from_secs(30),
    reconnect_interval: Duration::from_secs(1),
    max_reconnect_attempts: 0, // unlimited
    sequence_reset_policy: SequenceResetPolicy::Daily,
    validate_comp_ids: true,
    max_msg_rate: 50_000,
};

let mut session = Session::new(config);
session.on_connected();  // → LogonSent
session.on_logon();      // → Active

let seq = session.next_outbound_seq_num(); // 1, 2, 3...
session.validate_inbound_seq(1).unwrap();  // Ok or Err(gap_range)
```

### Memory Pool

```rust
use velocitas_fix::pool::BufferPool;

// Pre-allocate 1024 × 256-byte buffers (no allocation on hot path)
let mut pool = BufferPool::new(256, 1024);

let handle = pool.allocate().unwrap();     // ~8 ns, lock-free
let buf = pool.get_mut(handle);            // direct slice access
buf[0..5].copy_from_slice(b"hello");
pool.deallocate(handle);                   // ~8 ns, lock-free
```

### Message Journal

```rust
use velocitas_fix::journal::{Journal, SyncPolicy, session_hash};

let hash = session_hash("BANK_OMS", "NYSE");
let mut journal = Journal::open(
    Path::new("/mnt/nvme/fix-journal.dat"),
    4 * 1024 * 1024 * 1024, // 4 GB
    SyncPolicy::Batch(10),   // fsync every 10ms
).unwrap();

journal.append(hash, seq_num, &wire_msg)?;

// Recovery
let (header, body) = journal.read_entry(offset).unwrap();
assert_eq!(header.seq_num, 1);
```

### FIXT 1.1 / FIX 5.0 SP2 Sessions

```rust
use velocitas_fix::fixt::*;
use velocitas_fix::session::*;

let config = FixtSessionConfig {
    base: SessionConfig { /* ... */ ..Default::default() },
    default_appl_ver_id: ApplVerID::Fix50SP2,
    supported_versions: vec![ApplVerID::Fix44, ApplVerID::Fix50SP2],
};

let mut session = FixtSession::new(config);
// BeginString is always "FIXT.1.1", app version negotiated at Logon
assert_eq!(session.negotiated_version(), None);
// After logon, the negotiated version is available
```

### Repeating Groups

```rust
use velocitas_fix::groups::*;

let group_def = md_entries_group(); // NoMDEntries(268)
let group = RepeatingGroup::parse(buffer, &view.fields(), start_idx, &group_def)?;

for i in 0..group.count() {
    let entry = group.get_entry(i).unwrap();
    let price = entry.get_field_str(270);  // MDEntryPx
    let size = entry.get_field_i64(271);   // MDEntrySize
}
```

### SIMD-Accelerated Parsing

```rust
use velocitas_fix::simd;

// Finds SOH delimiters using NEON (ARM) or SSE2 (x86) — 16 bytes/cycle
let pos = simd::find_soh(buffer);         // first SOH position
let count = simd::count_fields(buffer);   // total field count
```

### Prometheus Metrics

```rust
use velocitas_fix::metrics::EngineMetrics;

let metrics = EngineMetrics::new();
metrics.messages_parsed.inc();
metrics.parse_latency_ns.record(280);

// Render for Prometheus scraping
let output = metrics.render_prometheus();
// → velocitas_messages_parsed_total 1
// → velocitas_parse_latency_ns{quantile="0.99"} 280
```

### Cluster (Active-Active HA)

```rust
use velocitas_fix::cluster::*;

let config = ClusterConfig::three_node(
    NodeId { id: 1, address: "10.0.1.1".into(), port: 9100 },
    vec![
        NodeId { id: 2, address: "10.0.1.2".into(), port: 9100 },
        NodeId { id: 3, address: "10.0.1.3".into(), port: 9100 },
    ],
);
let mut node = ClusterNode::new(config);
node.start();
node.replicate_session_state(state); // Raft-based consensus
```

### XML Dictionary Compiler

```rust
use velocitas_fix::dict_compiler::*;

let dict = CompiledDictionary::from_xml(xml_str)?;
let field = dict.lookup_field(55);             // O(1) lookup → "Symbol"
let msg = dict.lookup_message("D");            // → NewOrderSingle
let errors = dict.validate_message("D", &tags); // check required fields
```

### FIX Server (Acceptor)

```rust
use velocitas_fix::server::*;
use velocitas_fix::engine::*;

let server = FixServer::new(FixServerConfig {
    port: 9878,
    sender_comp_id: "EXCHANGE".into(),
    allowed_comp_ids: vec!["CLIENT_A".into(), "CLIENT_B".into()],
    ..Default::default()
});

// Blocks, spawns a thread per connection
server.start(|| Box::new(MyApp)).unwrap();
```

### FIX Client (Initiator)

```rust
use velocitas_fix::client::*;

let client = FixClient::new(FixClientConfig {
    remote_host: "10.0.1.50".into(),
    remote_port: 9878,
    sender_comp_id: "BANK_OMS".into(),
    target_comp_id: "EXCHANGE".into(),
    ..Default::default()
});

// Blocks until Logout
client.connect_and_run(&mut MyApp::new()).unwrap();
```

### Hardware Timestamps

```rust
use velocitas_fix::timestamp::*;

let clock = HrClock::new(TimestampSource::Tsc);
let ts = clock.now();
let fix_ts = ts.to_fix_timestamp_ns(); // "20260321-10:00:00.123456789" (MiFID II)

let tracker = LatencyTracker::start(&clock);
// ... do work ...
let elapsed_ns = tracker.stop();
```

### Web Dashboard

```rust
use velocitas_fix::dashboard::*;

let mut dashboard = Dashboard::new(DashboardConfig::default());
dashboard.update_session(session_status);

let response = dashboard.handle_request("GET", "/health");
// → { "healthy": true, "active_sessions": 5, ... }

let response = dashboard.handle_request("GET", "/metrics");
// → Prometheus text format

let response = dashboard.handle_request("GET", "/");
// → HTML dashboard with auto-refresh
```

## FIX Protocol Support

### Versions
| Version | Status |
|---|---|
| FIX 4.0 | ✅ Parse/Serialize |
| FIX 4.1 | ✅ Parse/Serialize |
| FIX 4.2 | ✅ Full support |
| FIX 4.3 | ✅ Parse/Serialize |
| FIX 4.4 | ✅ Full support (primary) |
| FIX 5.0 SP2 / FIXT 1.1 | ✅ Full support (transport-independent sessions) |

### Supported Message Types

**Session Level:** Logon (A), Logout (5), Heartbeat (0), TestRequest (1), ResendRequest (2), SequenceReset (4), Reject (3)

**Application Level:** NewOrderSingle (D), ExecutionReport (8), OrderCancelRequest (F), OrderCancelReplaceRequest (G), OrderStatusRequest (H), OrderCancelReject (9), MarketDataRequest (V), MarketDataSnapshotFullRefresh (W), MarketDataIncrementalRefresh (X), QuoteRequest (R), Quote (S), TradeCaptureReport (AE)

Custom message types supported via dictionary configuration.

## Test Suite

| Suite | Tests | Coverage |
|---|---|---|
| **Unit tests** | 198 | Parser, serializer, checksum, session FSM, pool, journal, transport, dictionary, SIMD, groups, FIXT, metrics, cluster, acceptor, dict compiler, timestamps, dashboard |
| **Integration tests** | 18 | End-to-end roundtrip, stress (1M messages), session lifecycle, journal persistence |
| **Conformance tests** | 16 | FIX 4.4 spec compliance — field ordering, required fields, message structure |
| **Property tests** | 6 | Fuzz testing via proptest — random messages, garbage input, sequence monotonicity |
| **Total** | **238** | |

```bash
# Run all tests
cargo test

# Run with release optimizations (stress tests run faster)
cargo test --release
```

## Benchmark Suite

10 benchmark definitions (BM-01 through BM-10) covering:

| ID | Benchmark | What it measures |
|---|---|---|
| BM-01 | Parse Latency | Time to parse FIX messages by type and size |
| BM-02 | Serialize Latency | Time to build wire-format messages |
| BM-03 | Wire-to-Wire | NIC RX → process → NIC TX (requires hardware) |
| BM-04 | Sustained Throughput | Max msg/s at p99 ≤ 10 µs |
| BM-05 | Burst Handling | Latency under 10× burst load |
| BM-06 | Session Establishment | Logon handshake + time to first trade |
| BM-07 | Gap Fill / Recovery | Sequence number recovery performance |
| BM-08 | Memory Footprint | RSS under various session counts |
| BM-09 | Coordinated Omission | Verify benchmark accuracy (no CO bias) |
| BM-10 | Jitter Analysis | Latency distribution characterization |

See [BENCHMARKS.md](BENCHMARKS.md) for full methodology and results.

## Feature Flags

| Flag | Description | Default |
|---|---|---|
| `kernel-tcp` | Standard kernel TCP/IP transport | ✅ |
| `dpdk` | DPDK user-space TCP (kernel bypass) | |
| `openonload` | Solarflare OpenOnload transport | |
| `tls` | TLS 1.3 via rustls (mutual auth) | |
| `simd-avx2` | AVX2 SIMD for x86_64 parse acceleration | |
| `simd-neon` | NEON SIMD for ARM/Apple Silicon | |

## Deployment

### Recommended Hardware

| Component | Specification |
|---|---|
| CPU | Intel Xeon W-3400 / AMD EPYC 9004 (≥ 16 cores) |
| RAM | 128 GB DDR5-4800 ECC |
| NIC | Solarflare X2522 / Mellanox ConnectX-6 Dx (25/100 GbE) |
| Storage | 2× NVMe SSD (Intel Optane P5800X preferred) |
| OS | RHEL 9 / Ubuntu 22.04 (RT kernel optional) |

### OS Tuning

```bash
# Isolate cores for engine threads
GRUB_CMDLINE_LINUX="isolcpus=4-15 nohz_full=4-15 rcu_nocbs=4-15"

# Disable THP
echo madvise > /sys/kernel/mm/transparent_hugepage/enabled

# Disable swap
sysctl vm.swappiness=0

# Enable busy polling (kernel TCP fallback)
sysctl net.core.busy_poll=50
```

## Regulatory Compliance

| Regulation | Requirement | Implementation |
|---|---|---|
| MiFID II RTS 25 | Clock sync ≤ 100 µs | PTP with hardware timestamping, ns precision |
| Reg NMS | Order protection | Application layer callbacks |
| CAT | Reportable event timestamps | Nanosecond audit trail |
| SEC Rule 15c3-5 | Pre-trade risk controls | Inline fat-finger, rate limit, kill switch |
| FCA | Transaction reporting | Drop-copy session support |

## Documentation

- **[SPECIFICATION.md](SPECIFICATION.md)** — Complete 15-section technical specification covering architecture, threading, memory layout, session FSM, security, configuration, and compliance
- **[BENCHMARKS.md](BENCHMARKS.md)** — Benchmark definitions, methodology, measured results, and QuickFIX/J comparison

## Roadmap

- [x] FIX 5.0 SP2 / FIXT 1.1 transport-independent session protocol
- [x] DPDK transport implementation (poll-mode driver integration)
- [x] SIMD-accelerated SOH scanning (AVX2 + NEON)
- [x] Repeating group parser with nested group support
- [x] Aeron Cluster integration for active-active HA
- [x] XML dictionary compiler (FIX Orchestra → binary)
- [x] Prometheus metrics exporter
- [x] Web dashboard with real-time latency heatmaps
- [x] FIX session acceptor with connection pooling
- [x] Hardware timestamp capture (TSC / rdtsc / CNTVCT_EL0)

## License

Proprietary. All rights reserved.
