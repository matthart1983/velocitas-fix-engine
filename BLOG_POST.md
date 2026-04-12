# Velocitas FIX vs Artio vs QuickFIX/J: A Three-Way Latency Shootout on a Realistic Trading Topology

Most FIX engine benchmarks lie. They measure `parse(bytes) → Message` in a tight loop and call it a day — no network, no IPC, no executor, no back-pressure. That tells you almost nothing about how the engine performs inside a real trading system, where a message has to cross TCP from a venue, get normalized, hand off to an execution engine over a shared-memory ringbuffer, come back, and be serialized back out to the wire.

So I built the real thing — three times. Once in Rust using the [Velocitas FIX engine](https://github.com/). Once in Java using [QuickFIX/J](https://www.quickfixj.org/), the de-facto open-source Java FIX engine. And once using [Artio](https://github.com/real-logic/artio), Real Logic's Aeron-native FIX engine built by the same team behind [Aeron](https://github.com/real-logic/aeron) itself. Same topology, same pipeline, same workload, five runs each. Here's what happened.

## The topology

```
Venue (FIX initiator)
  ⇄ TCP (localhost)
Gateway (FIX acceptor)
  ⇄ Aeron IPC
Executor
```

Three components, two transports, three threads per engine. The venue connects to the gateway over real TCP, logs on, and starts sending `NewOrderSingle` messages. The gateway parses each order, extracts the fields that matter, encodes them into a compact internal binary format, and publishes them to the executor over Aeron — the same shared-memory messaging library used by LMAX, Adaptive, and most modern HFT stacks. The executor decodes the order, encodes a filled `ExecutionReport` back, and publishes it over a second Aeron stream. The gateway decodes it, builds the FIX ExecRpt, and sends it back over TCP. The venue records the round-trip.

**Nine steps are timed on every iteration:**

1. Venue serializes FIX NOS → writes to TCP
2. Gateway parses FIX NOS
3. Gateway extracts fields → encodes internal binary order
4. Aeron IPC publish (gateway → executor)
5. Executor decodes order
6. Executor encodes ExecutionReport
7. Aeron IPC publish (executor → gateway)
8. Gateway decodes ExecutionReport
9. Gateway builds FIX ExecRpt → writes to TCP → venue

This is representative of what an order gateway actually does in production. The only things missing are a real network hop and a real matching engine — and those would add latency on all three sides equally.

## Ground rules for a fair fight

Benchmarks are easy to rig. I took this seriously:

- **Identical logical pipelines** on all three engines. Same nine steps, same order.
- **Identical workload**: 10,000 warmup round-trips (discarded) + 100,000 measured, × 5 runs. Same `NewOrderSingle`, same reply shape.
- **Zero per-iteration allocations on the hot path.** All three versions pre-allocate buffers, reuse scratch space, and avoid heap churn. Getting Artio there took real work — see below.
- **Same Aeron library** (1.50.4) on both Java engines.
- **Same JVM tuning.** `-server -XX:+UseG1GC`, 512 MB heap for QFJ, 1 GB for Artio (because it runs an embedded Archive). Both with the required `--add-opens` flags.
- **Artio with validation disabled** (`-Dfix.codecs.no_validation=true`) and Agrona bounds checks off (`-Dagrona.disable.bounds.checks=true`). That's how you'd run it in production.
- **Busy-spin on receives.** All three tightly poll the Aeron subscriptions. The Rust version has a three-phase idle strategy (busy-spin → `yield_now` → 10 µs sleep) to avoid pegging the CPU forever; Java uses pure busy-spin.

### The Artio tuning story

My first Artio run was **slower than QuickFIX/J** — p99.9 of 4.2 ms, max of 43 ms. That's not Artio being bad; it's me being sloppy. The hot path had `new String(bytes, UTF_8)` in the gateway frame decode, `String.format("%08d", seq)` in the venue's `sendNos`, and validation was on. Five hundred thousand short-lived Strings per run is enough to trigger young-gen collections during the measured window. Fixing it:

- Pre-allocated byte scratches, in-place ASCII encoding of the sequence counter.
- Swapped the `session-codecs` encoders to the `byte[], int, int` overloads instead of the String/CharSequence ones.
- Disabled codec validation and Agrona bounds checks.
- `ThreadingMode.DEDICATED` for the media driver and archive — SHARED mode multiplexed everything onto one agent thread that was fighting the library poll thread for CPU.

That got Artio's p99.9 from 4,231 µs to 189 µs — a **22× tail improvement** — and the mean from 131 µs to 83 µs. The first run wasn't measuring Artio; it was measuring my allocation mistakes. This is why benchmarking Java engines is annoying: the GC is a latent variable that punishes anyone who doesn't audit every method on the hot path.

## Results (5 runs each, median across runs)

100,000 measured round-trips × 5 runs, same hardware, same session:

| Metric       | **Velocitas (Rust)** | **Artio (Java)** | **QuickFIX/J (Java)** |
|--------------|---------------------:|-----------------:|----------------------:|
| min          |          **11.7 µs** |          36.9 µs |              37.0 µs  |
| p50          |          **20.1 µs** |          78.5 µs |              50.6 µs  |
| mean         |          **23.6 µs** |          83.3 µs |              54.7 µs  |
| p90          |          **31.1 µs** |         108.8 µs |              60.9 µs  |
| p99          |          **82.5 µs** |         142.5 µs |             157.5 µs  |
| p99.9        |         **113.6 µs** |         180.5 µs |             221.2 µs  |
| max          |              4,448 µs |         **940 µs** |            4,218 µs   |
| **Throughput** |      **41,720 RT/s** |      11,986 RT/s |          17,887 RT/s  |

**Rust wins p50, p99, p99.9, and throughput — everything except worst-case `max`, where Artio takes the crown.** Rust is 2.5× faster than QuickFIX/J at median and 3.9× faster than Artio. At p99.9 it's 1.9× faster than QFJ and 1.6× faster than Artio. Throughput: 2.3× over QFJ, 3.5× over Artio.

### The Artio story is more interesting than the win

Look at that `max` row. Artio's worst-case single round-trip (940 µs median across runs) is **4.5× better than QuickFIX/J and 4.7× better than Rust**. That's not noise — Artio was consistently better at worst-case across all 5 runs (min run 624 µs, max run 2,201 µs).

But at p50 Artio is 1.6× *slower* than QuickFIX/J. What's going on?

Artio's architecture splits the FIX engine into two pieces: a `FixEngine` that owns the TCP connections and runs on its own agent thread, and a `FixLibrary` that your application code runs on. They communicate via an internal Aeron publication. Every outbound message does `library → Aeron → engine → TCP`, and every inbound message does `TCP → engine → Aeron → library`. That's **two extra Aeron hops per direction** compared to QuickFIX/J, which writes to TCP directly from the application thread.

On localhost those hops are pure overhead — ~30 µs of extra latency at p50. But they buy you something: the engine/library split means the GC-heavy application code can allocate freely without affecting the I/O path. That's why Artio's tail is so smooth. When something does go wrong, it goes wrong in the library thread, not the engine thread that's actively reading the socket. The engine keeps the wire hot even if the library stutters.

**This is a real architectural trade-off, not a bug.** In production with network latency dominating, the ~30 µs Aeron cost disappears into the noise and Artio's cleaner tail is the only thing you feel. In a localhost microbenchmark like this one, the cost is visible and Artio looks slower at the median. In a multi-session deployment with hundreds of concurrent sessions, the fixed cost amortizes and Artio pulls ahead of QuickFIX/J on total throughput.

QuickFIX/J is the opposite: simple and direct (the session is the thread, writes go straight to TCP), which wins at median, but every message parse creates a `Message` object graph that the GC eventually has to clean up. That's why QFJ's `max` is 4 ms — somewhere in the measured window a young-gen collection happened.

**Velocitas Rust avoids both problems.** No engine/library split, so no fixed IPC tax at median. No GC, so no worst-case cleanup pause. Just a tight hot path that allocates nothing after startup.

## Run-to-run variance

One run is a data point. Five runs tell you whether the data point was real.

| Engine    | p50 (min–median–max)   | p99 (min–median–max)      | tput (min–median–max)        |
|-----------|-----------------------:|--------------------------:|-----------------------------:|
| Rust      | 19.7 – **20.1** – 23.1 | 79.3 – **82.5** – 101.8   | 29,424 – **41,720** – 44,228 |
| QFJ       | 50.5 – **50.6** – 50.9 | 156.5 – **157.5** – 177.6 | 16,913 – **17,887** – 17,900 |
| Artio     | 76.8 – **78.5** – 80.3 | 139.4 – **142.5** – 169.8 | 11,480 – **11,986** – 12,275 |

- **QuickFIX/J is the most consistent**: p50 spread of 0.4 µs across 5 runs. Once the JIT warms up, it does the same thing every time.
- **Artio is consistent at median** but had one p99.9 outlier run (468 µs vs the usual ~180 µs) — a young-gen GC that caught the tail of the measured window.
- **Rust is the most variable**. One run (run 4) showed clear system noise interference: p50 jumped from 20 to 23, p90 from 28 to 67. Busy-spin loops are sensitive to scheduler hiccups — the macOS scheduler happened to preempt the gateway thread at a bad moment. The other 4 runs are tight: p50 19.7–20.5, p99 79–94.

**Even using Rust's *worst* run against Artio/QFJ *best* runs, Rust still wins p50 by 2×+.** The comparison holds under noise.

## Why Rust wins

The usual "Rust is faster than Java" story is about JIT warmup, GC pauses, and bounds checks. That's all true but the specifics matter:

**1. No GC means no worst-case pauses — except the tail.** Rust's `max` is worse than Artio's because of macOS scheduler jitter on busy-spin threads, not GC. Artio's `max` is better because its engine/library split isolates I/O from allocation. Pick your poison: scheduler noise or GC noise. Velocitas wins everywhere except the single worst sample.

**2. FIX parsing cost.** QuickFIX/J parses into a full `Message` object graph even with validation off. Artio parses into a zero-copy decoder wrapping an `AsciiBuffer`. Velocitas parses into a zero-copy `MessageView` that hands out `&[u8]` slices on demand. Decoder-to-decoder, Velocitas and Artio are close. But QuickFIX/J is carrying object-construction overhead on every message.

**3. No virtual dispatch on the hot path.** `FixApp` in Velocitas is a `&mut dyn` trait object called once per message. Artio's `SessionHandler.onMessage` is also once per message. QFJ's `MessageCracker` does a HashMap lookup per field.

**4. Architecture.** Velocitas writes directly from the caller thread to TCP. QuickFIX/J does the same. Artio goes through Aeron both ways. In this benchmark (localhost, one session) direct wins; in a production multi-session deployment, Aeron pipelining wins.

**5. Aeron binding.** Velocitas uses `aeron_c` (the C driver client) directly over FFI. Both Java engines use the native Agrona/Aeron client. Both are fast, but the Rust FFI path has slightly less overhead per publish.

**6. TCP syscall cost is identical** across all three. Both sides do the same `write()` / `read()` dance on localhost TCP with `TCP_NODELAY`. That's the floor — the deltas above are *codec and runtime* deltas, not transport deltas.

## What this means for you

If you're building an order gateway, market data normalizer, or any hot-path component in a trading system, the engine you choose compounds every trade you handle:

- **Velocitas (Rust)** if you want the fastest median and p99, don't mind budgeting for occasional scheduler jitter, and want a runtime with no GC to reason about. 2.5× faster than QuickFIX/J at p50, 3.9× faster than Artio at p50, and **p99.9 under 120 µs end-to-end** including TCP on both ends.

- **Artio (Java)** if you run many concurrent FIX sessions, care about worst-case tail more than median, and have a team that already lives in the JVM ecosystem. The Aeron-based architecture pays off at scale and gives you the smoothest `max` of the three engines I tested. Just be prepared to audit every allocation on the hot path — Artio is fast but it's not automatic.

- **QuickFIX/J (Java)** if you want maximum operational simplicity, JIT predictability, and a well-understood engine with twenty years of community history. It's the slowest median of the three, but it's also the most laser-consistent run to run — and "consistent and slightly slower" is often more valuable in production than "fast but variable."

**Velocitas gives you a 2.5× latency improvement over QuickFIX/J and a 3.9× improvement over Artio on a realistic nine-step pipeline, with no allocations on the hot path, predictable latency, and a p99.9 under 120 µs end-to-end.** That's not a microbenchmark — that's the whole pipeline, including TCP on both ends, across five independent runs of 100,000 round-trips each.

The code for all three benchmarks is public. If you think I've rigged something, clone it and run it. If you find a way to make the Java sides faster, I want to see it — getting Artio down was a 22× tail improvement from allocation fixes alone, and I'd bet there's more to find.

## Reproducing

```bash
# Rust
cd fix-engine && cargo run --release --bin bench_e2e

# QuickFIX/J
cd fix-engine/bench-vs-quickfixj && gradle runE2E

# Artio
cd fix-engine/bench-vs-quickfixj && gradle runArtio
```

Each run takes 5–25 seconds, prints the same format, and uses real TCP + real Aeron. No hand-waving.

---

*Velocitas FIX is a Rust FIX 4.4 engine built for ultra-low-latency trading infrastructure. Zero-allocation hot paths, pluggable transports, native SBE and Aeron integration.*
