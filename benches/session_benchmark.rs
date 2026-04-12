/// BM-06/07: Session Establishment and Gap Fill Benchmarks
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::time::Duration;
use velocitas_fix::parser::FixParser;
use velocitas_fix::serializer;
use velocitas_fix::session::{
    SequenceResetPolicy, Session, SessionConfig, SessionRole, SessionState,
};

fn session_config() -> SessionConfig {
    SessionConfig {
        session_id: "BENCH-1".to_string(),
        fix_version: "FIX.4.4".to_string(),
        sender_comp_id: "BANK".to_string(),
        target_comp_id: "EXCH".to_string(),
        role: SessionRole::Initiator,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(1),
        max_reconnect_attempts: 0,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: true,
        max_msg_rate: 1_000_000,
    }
}

fn bench_session_lifecycle(c: &mut Criterion) {
    let mut group = c.benchmark_group("BM-06_session_lifecycle");

    group.bench_function("logon_handshake", |b| {
        b.iter(|| {
            let mut session = Session::new(session_config());
            session.on_connected();
            assert_eq!(session.state(), SessionState::LogonSent);
            session.on_logon();
            assert_eq!(session.state(), SessionState::Active);
            black_box(&session);
        })
    });

    group.bench_function("sequence_validation_hit", |b| {
        let mut session = Session::new(session_config());
        session.on_connected();
        session.on_logon();

        b.iter(|| {
            let seq = session.expected_inbound_seq_num();
            let _ = session.validate_inbound_seq(black_box(seq));
        })
    });

    group.bench_function("rate_limit_check", |b| {
        let mut session = Session::new(session_config());
        b.iter(|| {
            let _ = session.check_rate_limit();
        })
    });

    group.finish();
}

fn bench_sequence_recovery(c: &mut Criterion) {
    let mut group = c.benchmark_group("BM-07_sequence_recovery");

    group.bench_function("gap_detect_1000", |b| {
        b.iter(|| {
            let mut session = Session::new(session_config());
            session.on_connected();
            session.on_logon();

            // Simulate receiving seq 1001 when expecting 1 (gap of 1000)
            let result = session.validate_inbound_seq(black_box(1001));
            assert!(result.is_err());

            // Simulate gap fill
            session.on_gap_filled(1001);
            assert_eq!(session.state(), SessionState::Active);
        })
    });

    group.bench_function("sequential_10k_validations", |b| {
        b.iter(|| {
            let mut session = Session::new(session_config());
            session.on_connected();
            session.on_logon();

            for seq in 1..=10_000u64 {
                let _ = session.validate_inbound_seq(black_box(seq));
            }
        })
    });

    group.finish();
}

fn bench_logon_message_build(c: &mut Criterion) {
    let parser = FixParser::new();

    let mut group = c.benchmark_group("BM-06_logon_message_build");

    group.bench_function("build_logon", |b| {
        let mut buf = [0u8; 1024];
        b.iter(|| {
            serializer::build_logon(
                black_box(&mut buf),
                b"FIX.4.4",
                b"BANK",
                b"EXCH",
                1,
                b"20260321-10:00:00",
                30,
            )
        })
    });

    group.bench_function("build_and_parse_logon", |b| {
        let mut buf = [0u8; 1024];
        b.iter(|| {
            let len = serializer::build_logon(
                &mut buf,
                b"FIX.4.4",
                b"BANK",
                b"EXCH",
                1,
                b"20260321-10:00:00",
                30,
            );
            let _ = parser.parse(black_box(&buf[..len]));
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_session_lifecycle,
    bench_sequence_recovery,
    bench_logon_message_build,
);
criterion_main!(benches);
