# Aeron Integration Guide

This is the standard integration path for Velocitas.

Use Aeron when you are wiring colocated services into the FIX engine. Use the TCP client/server wrappers only when you explicitly need socket-based venue or counterparty connectivity.

## What "Aeron" Means Here

The crate exposes an `AeronTransport` backend and makes it the default transport selected by `TransportConfig::default()`.

The current implementation uses direct Rust FFI bindings over the official Aeron C client and publishes each FIX wire message as an SBE envelope onto the configured Aeron channel.

By default the transport expects an independent media driver to already be running, which matches typical Aeron production deployments and keeps transport ownership separate from application ownership:

- one shared driver process for all colocated participants
- a stable default Aeron directory so separate processes meet consistently

Each FIX wire message is wrapped in one SBE envelope and sent as one Aeron message.

## Quick Start

Before building, provide either:

- `AERON_SOURCE_DIR=/path/to/official/aeron` so `build.rs` can compile the C client and driver with CMake
- `AERON_LIB_DIR=/path/to/prebuilt/aeron/lib` to link against prebuilt `libaeron` and `libaeron_driver`

Build the project with the default transport:

```bash
export AERON_SOURCE_DIR=/path/to/aeron
cargo build --release
```

Run the self-contained Aeron demo:

```bash
cargo run --release --bin aeron_demo
```

Run the focused local Aeron benchmarks:

```bash
cargo run --release --bin aeron_demo benchmark
```

If you want to run a shared standalone media driver yourself, start it explicitly:

```bash
cargo run --release --bin aeron_media_driver -- --aeron-dir /tmp/velocitas-fix-aeron
```

Run the focused Aeron end-to-end regression test:

```bash
cargo test --test aeron_integration
```

## Default Behavior

These are the important defaults from `TransportConfig`:

- `TransportConfig::default()` selects `TransportMode::Aeron`
- the default channel is `aeron:ipc`
- the default stream ID is `1001`
- the default transport expects an independent media driver
- the default Aeron directory is `TransportConfig::default_aeron_dir()`

The `aeron_demo` binary is intentionally more ergonomic than the raw transport default: it spawns a standalone media-driver child process automatically so the end-to-end example works as a single command.

The transport envelope schema lives in `schema/fix_aeron_envelope.xml`, and the Rust bindings are generated with the official `aeron-io/simple-binary-encoding` Rust generator into `generated/rust/velocitas_fix_sbe`.

The configured stream ID is the base request stream. The paired transport uses the next stream ID for replies on the same Aeron channel.

If you want an explicit stream ID, use `TransportConfig::aeron_ipc(stream_id)`.

If you want to opt back into an embedded driver for a single process, set `aeron_embedded_driver` to `true`.

If you want multiple processes to share one driver, keep `aeron_embedded_driver` as `false` and point them all at the same `aeron_dir`.

If you need TCP instead, use `TransportConfig::kernel_tcp()`.

## Standard Pattern

The simplest setup is:

1. Create an Aeron transport for the acceptor.
2. Bind it.
3. Create an Aeron transport for the initiator.
4. Connect it.
5. Create `Session` values for each side.
6. Pass both transports into `FixEngine`.

```rust
use std::io;
use std::time::Duration;

use velocitas_fix::engine::{FixApp, FixEngine};
use velocitas_fix::session::{SequenceResetPolicy, Session, SessionConfig, SessionRole};
use velocitas_fix::transport::{build_transport, TransportConfig};

fn build_acceptor() -> io::Result<FixEngine<Box<dyn velocitas_fix::transport::Transport>>> {
    let mut transport = build_transport(TransportConfig {
        aeron_stream_id: 1001,
        aeron_dir: Some("/tmp/velocitas-fix-aeron".into()),
        ..TransportConfig::default()
    })?;
    transport.bind("127.0.0.1", 0)?;

    let session = Session::new(SessionConfig {
        session_id: "AERON-ACCEPTOR".into(),
        fix_version: "FIX.4.4".into(),
        sender_comp_id: "EXCHANGE".into(),
        target_comp_id: "BANK_OMS".into(),
        role: SessionRole::Acceptor,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::ZERO,
        max_reconnect_attempts: 0,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: true,
        max_msg_rate: 50_000,
    });

    Ok(FixEngine::new_acceptor(transport, session))
}

fn build_initiator() -> io::Result<FixEngine<Box<dyn velocitas_fix::transport::Transport>>> {
    let mut transport = build_transport(TransportConfig {
        aeron_stream_id: 1001,
        aeron_dir: Some("/tmp/velocitas-fix-aeron".into()),
        ..TransportConfig::default()
    })?;
    transport.connect("127.0.0.1", 0)?;

    let session = Session::new(SessionConfig {
        session_id: "AERON-INITIATOR".into(),
        fix_version: "FIX.4.4".into(),
        sender_comp_id: "BANK_OMS".into(),
        target_comp_id: "EXCHANGE".into(),
        role: SessionRole::Initiator,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(1),
        max_reconnect_attempts: 3,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: true,
        max_msg_rate: 50_000,
    });

    Ok(FixEngine::new_initiator(transport, session))
}
```

## Initiator And Acceptor Behavior

There is one important difference between the Aeron path and the TCP wrapper path:

- with Aeron, `run_acceptor()` can consume the initial inbound Logon directly
- with the TCP wrappers, the server still uses the socket-oriented pre-read path before handing control to the engine

That means Aeron integrations can stay entirely on the `FixEngine` API without going through `FixServer`.

## Recommended Integration Shape

For most local application integrations:

1. Keep your business logic in a `FixApp` implementation.
2. Use `build_transport(TransportConfig::default())` or `aeron_ipc(stream_id)`.
3. Construct `FixEngine` directly.
4. Reserve `FixClient` and `FixServer` for TCP edge connections.

This keeps the colocated path consistent across tests, demos, and production embeddings.

## When To Use TCP Instead

Use the TCP wrappers when you need:

- a listening socket
- remote connectivity over TCP/IP
- compatibility with existing FIX counterparties

In that case, opt in explicitly:

```rust
use velocitas_fix::transport::TransportConfig;

let tcp = TransportConfig::kernel_tcp();
```

## Reference Files

- `src/transport.rs` — transport defaults, modes, and factory
- `src/transport_aeron.rs` — Aeron transport backend
- `src/aeron_sbe.rs` — SBE envelope encode/decode helpers
- `schema/fix_aeron_envelope.xml` — Aeron transport SBE schema
- `generated/rust/velocitas_fix_sbe` — official generated Rust SBE bindings
- `src/engine.rs` — acceptor/initiator engine behavior
- `src/bin/aeron_demo.rs` — runnable end-to-end Aeron example
- `tests/aeron_integration.rs` — focused regression coverage
