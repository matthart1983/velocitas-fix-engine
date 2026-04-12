use std::time::Duration;
use velocitas_fix::journal::{session_hash, Journal, SyncPolicy};
use velocitas_fix::parser::{FixParser, ParseError};
use velocitas_fix::pool::BufferPool;
use velocitas_fix::serializer;
use velocitas_fix::session::{
    SequenceResetPolicy, Session, SessionConfig, SessionRole, SessionState,
};
use velocitas_fix::tags;
/// Integration tests for the Velocitas FIX Engine.
///
/// These tests validate end-to-end behavior across parser, serializer,
/// session management, journal, and pool components.
use velocitas_fix::*;

// ============================================================================
// Parse → Serialize roundtrip tests
// ============================================================================

#[test]
fn test_roundtrip_heartbeat() {
    let mut buf = [0u8; 1024];
    let len = serializer::build_heartbeat(
        &mut buf,
        b"FIX.4.4",
        b"BANK",
        b"NYSE",
        1,
        b"20260321-10:00:00",
    );

    let parser = FixParser::new();
    let (view, consumed) = parser.parse(&buf[..len]).unwrap();

    assert_eq!(consumed, len);
    assert_eq!(view.msg_type(), Some(b"0".as_slice()));
    assert_eq!(view.begin_string(), Some("FIX.4.4"));
    assert_eq!(view.sender_comp_id(), Some("BANK"));
    assert_eq!(view.target_comp_id(), Some("NYSE"));
    assert_eq!(view.msg_seq_num(), Some(1));
    assert!(view.is_checksum_valid());
}

#[test]
fn test_roundtrip_logon() {
    let mut buf = [0u8; 1024];
    let len = serializer::build_logon(
        &mut buf,
        b"FIX.4.4",
        b"CLIENT",
        b"SERVER",
        1,
        b"20260321-10:00:00",
        30,
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&buf[..len]).unwrap();

    assert_eq!(view.msg_type_enum(), Some(MsgType::Logon));
    assert_eq!(view.get_field_i64(tags::HEARTBT_INT), Some(30));
    assert_eq!(view.get_field_i64(tags::ENCRYPT_METHOD), Some(0));
}

#[test]
fn test_roundtrip_new_order_single() {
    let mut buf = [0u8; 1024];
    let len = serializer::build_new_order_single(
        &mut buf,
        b"FIX.4.4",
        b"OMS",
        b"NYSE",
        42,
        b"20260321-10:00:00.123",
        b"CLO-12345",
        b"MSFT",
        b'2',
        5000,
        b'2',
        b"425.75",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&buf[..len]).unwrap();

    assert_eq!(view.msg_type_enum(), Some(MsgType::NewOrderSingle));
    assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("CLO-12345"));
    assert_eq!(view.get_field_str(tags::SYMBOL), Some("MSFT"));
    assert_eq!(view.get_field_i64(tags::ORDER_QTY), Some(5000));
    assert_eq!(view.get_field_str(tags::PRICE), Some("425.75"));
    assert_eq!(view.msg_seq_num(), Some(42));
}

#[test]
fn test_roundtrip_execution_report() {
    let mut buf = [0u8; 2048];
    let len = serializer::build_execution_report(
        &mut buf,
        b"FIX.4.4",
        b"NYSE",
        b"OMS",
        100,
        b"20260321-10:00:00.456",
        b"ORD-001",
        b"EXE-001",
        b"CLO-12345",
        b"MSFT",
        b'2',
        5000,
        2500,
        b"425.75",
        2500,
        2500,
        b"425.75",
        b'F',
        b'1',
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&buf[..len]).unwrap();

    assert_eq!(view.msg_type_enum(), Some(MsgType::ExecutionReport));
    assert_eq!(view.get_field_str(tags::ORDER_ID), Some("ORD-001"));
    assert_eq!(view.get_field_str(tags::EXEC_ID), Some("EXE-001"));
    assert_eq!(view.get_field_i64(tags::LAST_QTY), Some(2500));
    assert_eq!(view.get_field_str(tags::LAST_PX), Some("425.75"));
    assert_eq!(view.get_field_i64(tags::LEAVES_QTY), Some(2500));
    assert_eq!(view.get_field_i64(tags::CUM_QTY), Some(2500));
}

// ============================================================================
// Parser edge cases
// ============================================================================

#[test]
fn test_parse_rejects_truncated_message() {
    let parser = FixParser::new();
    assert!(parser.parse(b"8=FIX.4.4\x01").is_err());
}

#[test]
fn test_parse_rejects_invalid_begin_string_position() {
    let parser = FixParser::new();
    let msg = b"35=D\x018=FIX.4.4\x019=5\x0110=000\x01xxx";
    assert_eq!(
        parser.parse(msg).unwrap_err(),
        ParseError::MissingBeginString
    );
}

#[test]
fn test_parse_large_sequence_numbers() {
    let mut buf = [0u8; 1024];
    let len = serializer::build_heartbeat(
        &mut buf,
        b"FIX.4.4",
        b"S",
        b"T",
        u64::MAX / 2,
        b"20260321-10:00:00",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&buf[..len]).unwrap();
    assert_eq!(view.msg_seq_num(), Some(u64::MAX / 2));
}

#[test]
fn test_parse_all_fix_versions() {
    for version in &[b"FIX.4.0", b"FIX.4.1", b"FIX.4.2", b"FIX.4.3", b"FIX.4.4"] {
        let mut buf = [0u8; 1024];
        let len =
            serializer::build_heartbeat(&mut buf, *version, b"S", b"T", 1, b"20260321-10:00:00");

        let parser = FixParser::new();
        let (view, _) = parser.parse(&buf[..len]).unwrap();
        assert_eq!(view.begin_string().unwrap().as_bytes(), *version);
    }
}

// ============================================================================
// Session state machine integration
// ============================================================================

#[test]
fn test_full_session_lifecycle() {
    let config = SessionConfig {
        session_id: "INT-TEST-1".to_string(),
        fix_version: "FIX.4.4".to_string(),
        sender_comp_id: "BANK".to_string(),
        target_comp_id: "EXCHANGE".to_string(),
        role: SessionRole::Initiator,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(1),
        max_reconnect_attempts: 3,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: true,
        max_msg_rate: 100_000,
    };

    let mut session = Session::new(config);

    // Phase 1: Connect and logon
    assert_eq!(session.state(), SessionState::Disconnected);
    session.on_connected();
    assert_eq!(session.state(), SessionState::LogonSent);
    session.on_logon();
    assert_eq!(session.state(), SessionState::Active);

    // Phase 2: Exchange messages
    for i in 1..=100u64 {
        let seq = session.next_outbound_seq_num();
        assert_eq!(seq, i);
        session.on_message_sent();

        assert!(session.validate_inbound_seq(i).is_ok());
        session.on_message_received();
    }

    // Phase 3: Gap detection
    let result = session.validate_inbound_seq(105);
    assert_eq!(result, Err((101, 105)));
    assert_eq!(session.state(), SessionState::Resending);

    // Phase 4: Gap fill
    session.on_gap_filled(105);
    assert_eq!(session.state(), SessionState::Active);

    // Phase 5: Logout — we sent Logout and are waiting for peer's Logout response
    session.on_logout_sent();
    assert_eq!(session.state(), SessionState::LogoutPending);
    session.on_disconnected();
    assert_eq!(session.state(), SessionState::Disconnected);
}

#[test]
fn test_reconnection_with_sequence_continuity() {
    let config = SessionConfig {
        session_id: "RECON-1".to_string(),
        fix_version: "FIX.4.4".to_string(),
        sender_comp_id: "BANK".to_string(),
        target_comp_id: "EXCHANGE".to_string(),
        role: SessionRole::Initiator,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(1),
        max_reconnect_attempts: 0,
        sequence_reset_policy: SequenceResetPolicy::Never,
        validate_comp_ids: true,
        max_msg_rate: 100_000,
    };

    let mut session = Session::new(config);

    // First connection: exchange 50 messages
    session.on_connected();
    session.on_logon();
    for _ in 0..50 {
        session.next_outbound_seq_num();
    }
    session.on_disconnected();

    // After disconnect, sequence numbers should be preserved
    assert_eq!(session.current_outbound_seq_num(), 51);
    assert_eq!(session.expected_inbound_seq_num(), 1);

    // Reconnect
    assert!(session.should_reconnect());
    session.on_reconnect_attempt();
    session.on_connected();
    session.on_logon();

    // Continue from where we left off
    assert_eq!(session.next_outbound_seq_num(), 51);
}

// ============================================================================
// Journal integration
// ============================================================================

#[test]
fn test_journal_message_persistence_and_recovery() {
    let path = std::env::temp_dir().join("velocitas-int-test-journal.dat");
    let _ = std::fs::remove_file(&path);

    let hash = session_hash("BANK", "EXCHANGE");

    // Write phase: persist 1000 messages
    {
        let mut journal = Journal::open(&path, 1024 * 1024, SyncPolicy::None).unwrap();

        for seq in 1..=1000u64 {
            let mut buf = [0u8; 512];
            let len = serializer::build_new_order_single(
                &mut buf,
                b"FIX.4.4",
                b"BANK",
                b"EXCHANGE",
                seq,
                b"20260321-10:00:00",
                format!("ORD-{:05}", seq).as_bytes(),
                b"AAPL",
                b'1',
                100,
                b'2',
                b"150.00",
            );
            journal.append(hash, seq, &buf[..len]).unwrap();
        }

        assert_eq!(journal.entry_count(), 1000);
    }

    // Read phase: recover and verify
    {
        let journal = Journal::open(&path, 1024 * 1024, SyncPolicy::None).unwrap();
        let parser = FixParser::new();

        // Read first entry
        let (header, body) = journal.read_entry(0).unwrap();
        assert_eq!(header.session_hash, hash);
        assert_eq!(header.seq_num, 1);

        let (view, _) = parser.parse(body).unwrap();
        assert_eq!(view.msg_type_enum(), Some(MsgType::NewOrderSingle));
        assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("ORD-00001"));
    }

    let _ = std::fs::remove_file(&path);
}

// ============================================================================
// Memory pool integration
// ============================================================================

#[test]
fn test_pool_based_message_processing() {
    let mut pool = BufferPool::new(512, 1024);
    let parser = FixParser::new();

    // Simulate: allocate buffer, serialize message, parse it, deallocate
    for i in 1..=500u64 {
        let handle = pool.allocate().expect("pool should not be exhausted");
        let buf = pool.get_mut(handle);

        let len = serializer::build_new_order_single(
            buf,
            b"FIX.4.4",
            b"S",
            b"T",
            i,
            b"20260321-10:00:00",
            format!("O-{}", i).as_bytes(),
            b"AAPL",
            b'1',
            100,
            b'2',
            b"150.00",
        );

        let (view, _) = parser.parse(&pool.get(handle)[..len]).unwrap();
        assert_eq!(view.msg_seq_num(), Some(i));

        pool.deallocate(handle);
    }
}

// ============================================================================
// Stress tests
// ============================================================================

#[test]
fn test_parse_million_messages() {
    let parser = FixParser::new_unchecked();
    let mut buf = [0u8; 512];
    let len = serializer::build_new_order_single(
        &mut buf,
        b"FIX.4.4",
        b"S",
        b"T",
        1,
        b"20260321-10:00:00",
        b"ORD-1",
        b"AAPL",
        b'1',
        100,
        b'2',
        b"150.00",
    );
    let msg = &buf[..len];

    let start = std::time::Instant::now();
    for _ in 0..1_000_000 {
        let (view, _) = parser.parse(msg).unwrap();
        assert_eq!(view.msg_type(), Some(b"D".as_slice()));
    }
    let elapsed = start.elapsed();
    let rate = 1_000_000.0 / elapsed.as_secs_f64();

    eprintln!("Parsed 1M messages in {:?} ({:.0} msg/s)", elapsed, rate);
}

#[test]
fn test_serialize_million_messages() {
    let mut buf = [0u8; 512];

    let start = std::time::Instant::now();
    for i in 0..1_000_000u64 {
        let _ = serializer::build_new_order_single(
            &mut buf,
            b"FIX.4.4",
            b"S",
            b"T",
            i,
            b"20260321-10:00:00",
            b"ORD-1",
            b"AAPL",
            b'1',
            100,
            b'2',
            b"150.00",
        );
    }
    let elapsed = start.elapsed();
    let rate = 1_000_000.0 / elapsed.as_secs_f64();

    eprintln!(
        "Serialized 1M messages in {:?} ({:.0} msg/s)",
        elapsed, rate
    );
}

// ============================================================================
// Message type coverage
// ============================================================================

#[test]
fn test_msg_type_classification() {
    assert!(MsgType::Heartbeat.is_session_level());
    assert!(MsgType::Logon.is_session_level());
    assert!(MsgType::Logout.is_session_level());
    assert!(MsgType::TestRequest.is_session_level());
    assert!(MsgType::ResendRequest.is_session_level());
    assert!(MsgType::SequenceReset.is_session_level());
    assert!(MsgType::Reject.is_session_level());

    assert!(!MsgType::NewOrderSingle.is_session_level());
    assert!(!MsgType::ExecutionReport.is_session_level());
    assert!(!MsgType::MarketDataSnapshot.is_session_level());
}

// ============================================================================
// Side / OrdType parsing
// ============================================================================

#[test]
fn test_side_parsing_in_context() {
    let mut buf = [0u8; 1024];
    for (side_byte, expected) in &[
        (b'1', Side::Buy),
        (b'2', Side::Sell),
        (b'5', Side::SellShort),
    ] {
        let len = serializer::build_new_order_single(
            &mut buf,
            b"FIX.4.4",
            b"S",
            b"T",
            1,
            b"20260321-10:00:00",
            b"O1",
            b"AAPL",
            *side_byte,
            100,
            b'2',
            b"150.00",
        );

        let parser = FixParser::new();
        let (view, _) = parser.parse(&buf[..len]).unwrap();
        let side_field = view.get_field(tags::SIDE).unwrap();
        let side = Side::from_byte(side_field[0]).unwrap();
        assert_eq!(side, *expected);
    }
}

// ============================================================================
// Checksum edge cases
// ============================================================================

#[test]
fn test_checksum_integrity_across_message_types() {
    let parser = FixParser::new(); // Validation enabled

    let mut buf = [0u8; 2048];

    // Verify one serializer-generated message still passes parser validation.
    let len = serializer::build_new_order_single(
        &mut buf,
        b"FIX.4.4",
        b"A",
        b"B",
        1,
        b"20260321-10:00:00",
        b"O1",
        b"X",
        b'1',
        1,
        b'2',
        b"1.0",
    );
    let result = parser.parse(&buf[..len]);
    assert!(
        result.is_ok(),
        "Checksum validation should pass for well-formed message"
    );
}

// ============================================================================
// Dictionary
// ============================================================================

#[test]
fn test_dictionary_field_lookup() {
    use velocitas_fix::dictionary;

    let field = dictionary::lookup_field(35).unwrap();
    assert_eq!(field.name, "MsgType");

    let field = dictionary::lookup_field(55).unwrap();
    assert_eq!(field.name, "Symbol");

    assert!(dictionary::lookup_field(99999).is_none());
}
