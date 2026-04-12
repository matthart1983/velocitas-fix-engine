# Draft: Show HN / r/rust post — Velocitas FIX Engine

---

## Option A — Show HN

**Title:** Show HN: Velocitas – a zero-allocation FIX protocol engine in Rust (2.26M msg/s, 28ns serialize)

**Body:**

I built a FIX protocol engine in Rust aimed at institutional trading infrastructure. The goals were sub-microsecond message parsing, zero allocations on the hot path, and Aeron IPC as the default colocated integration transport.

**Benchmarks (Apple Silicon, Criterion.rs):**

- Parse throughput: ~2.26M msg/s sustained (single core)
- Heartbeat serialization: 28 ns
- NewOrderSingle serialization: 59 ns
- Memory pool alloc/dealloc: 8.5 ns
- 29× faster serialization than QuickFIX/J; 1.5–2.2× faster parsing

**Design decisions I found interesting:**

- **Flyweight parser** — the parser never allocates. It hands back a view over the original byte buffer. Field access is O(1) via a lookup table compiled from a QuickFIX XML dictionary at build time.
- **SIMD delimiter scanning** — NEON (ARM) and SSE2 (x86) for the SOH delimiter scan. This is the inner loop of parsing and the gain is meaningful at high message rates.
- **Aeron over TCP for colocation** — when your matching engine and risk system are on the same box, you don't want kernel TCP in the data path. Aeron IPC gives you a lock-free ring buffer with a separate media driver process, ~100ns latency, and back-pressure. TCP stays available for venue/counterparty connectivity where you don't control both ends.
- **SBE envelopes on Aeron** — each FIX wire message is wrapped in a Simple Binary Encoding envelope before hitting the Aeron channel. Keeps schema evolution manageable.
- **Lock-free memory pool** — fixed-size slab allocator for message buffers. 8.5 ns cycles vs. the allocator. The goal is deterministic latency, not just throughput.
- **Raft HA** — Aeron-aligned Raft model with state replication and snapshots for session failover.

The project also has DPDK transport for kernel-bypass NIC access, hardware timestamps (TSC/rdtsc/CNTVCT) formatted to MiFID II nanosecond precision, a Prometheus metrics endpoint, and a web dashboard for session monitoring.

MIT licensed.

GitHub: https://github.com/matthart1983/fix-engine

Happy to talk through any of the design choices — particularly the Aeron integration, which I found surprisingly underrepresented in Rust tooling.

---

## Option B — r/rust

**Title:** I wrote a FIX protocol engine in Rust — zero allocations, 2.26M msg/s, Aeron IPC transport

**Body:**

Built Velocitas, a FIX engine for institutional trading infrastructure. Here's what made it interesting from a Rust perspective:

**Zero-alloc hot path**

The parser uses a flyweight pattern — it hands back a view over the input buffer, no heap activity on successful parses. The XML FIX dictionary is compiled at build time into O(1) lookup tables via `build.rs`. In production, you never pay for a `HashMap` lookup; it's a direct array index.

**SIMD for the inner loop**

FIX uses SOH (0x01) as a field delimiter. Scanning for it is the innermost loop. I implemented NEON (ARM) and SSE2 (x86_64) paths for the scan. At 2M+ msg/s the difference is measurable.

**Aeron IPC instead of Unix sockets or channels**

For colocated services (OMS → risk engine → FIX gateway), I use Aeron rather than `tokio::mpsc` or Unix domain sockets. Reasons: the media driver runs as a separate process (crash isolation), the ring buffer is lock-free and mapped into both processes, and back-pressure is built in. The latency floor is around 100ns IPC vs. ~1µs for kernel sockets. The downside is the C FFI — the official Aeron client is C, so there's a `build.rs` that either compiles it from source (CMake) or links against a prebuilt `libaeron`.

**SBE on top of Aeron**

Using Simple Binary Encoding for the message framing on the Aeron channel. Fixed-width, no parsing, schema is generated from a definition file. This keeps the wire format stable as the codebase evolves.

**Numbers (Criterion, Apple Silicon):**

```
Parse throughput:       2.26M msg/s sustained
Heartbeat serialize:    28 ns
NewOrderSingle:         59 ns
Pool alloc/dealloc:     8.5 ns
vs QuickFIX/J:          29× faster serialize, 1.5–2.2× faster parse
```

MIT: https://github.com/matthart1983/fix-engine

The part I'd most like feedback on: the Aeron FFI setup is clunky (you have to either point at the Aeron source tree or a prebuilt lib dir). If anyone has done this more cleanly in a Rust crate I'd be interested.

---

## Notes on timing

Post Option A (Show HN) on a weekday morning US time, 8–9am ET.
Post Option B (r/rust) can go anytime — the sub is active and engaged with performance posts.
Don't post both simultaneously. Start with r/rust, then Show HN a week later if the r/rust post gets traction.
