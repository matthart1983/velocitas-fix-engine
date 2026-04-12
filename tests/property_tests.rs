/// Property-based tests for the Velocitas FIX Engine.
///
/// Uses proptest to generate random FIX messages and verify invariants:
/// - Every serialized message can be parsed back
/// - Checksum is always valid after serialization
/// - Field values survive the roundtrip intact
/// - Parser never panics on arbitrary input
use proptest::prelude::*;
use velocitas_fix::parser::FixParser;
use velocitas_fix::serializer;
use velocitas_fix::tags;

/// Strategy for generating valid CompIDs (alphanumeric, 1–16 chars).
fn comp_id_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop::sample::select(
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789".to_vec(),
        ),
        1..=16,
    )
}

/// Strategy for generating valid ClOrdIDs.
fn cl_ord_id_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop::sample::select(
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_".to_vec(),
        ),
        1..=20,
    )
}

/// Strategy for generating valid symbols.
fn symbol_strategy() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop::sample::select(b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.".to_vec()),
        1..=8,
    )
}

/// Strategy for generating valid prices.
fn price_strategy() -> impl Strategy<Value = Vec<u8>> {
    (1u32..999999, 0u32..99).prop_map(|(whole, frac)| format!("{}.{:02}", whole, frac).into_bytes())
}

/// Strategy for generating valid quantities.
fn qty_strategy() -> impl Strategy<Value = i64> {
    1i64..10_000_000
}

/// Strategy for generating valid sequence numbers.
fn seq_num_strategy() -> impl Strategy<Value = u64> {
    1u64..1_000_000_000
}

proptest! {
    /// Every heartbeat we serialize should parse back correctly.
    #[test]
    fn prop_heartbeat_roundtrip(
        sender in comp_id_strategy(),
        target in comp_id_strategy(),
        seq in seq_num_strategy(),
    ) {
        let mut buf = [0u8; 2048];
        let len = serializer::build_heartbeat(
            &mut buf,
            b"FIX.4.4",
            &sender,
            &target,
            seq,
            b"20260321-10:00:00",
        );

        let parser = FixParser::new();
        let (view, consumed) = parser.parse(&buf[..len]).unwrap();

        prop_assert_eq!(consumed, len);
        prop_assert_eq!(view.msg_type(), Some(b"0".as_slice()));
        prop_assert_eq!(view.sender_comp_id().unwrap().as_bytes(), sender.as_slice());
        prop_assert_eq!(view.target_comp_id().unwrap().as_bytes(), target.as_slice());
        prop_assert_eq!(view.msg_seq_num(), Some(seq));
        prop_assert!(view.is_checksum_valid());
    }

    /// Every NOS we serialize should parse back with all fields intact.
    #[test]
    fn prop_nos_roundtrip(
        sender in comp_id_strategy(),
        target in comp_id_strategy(),
        seq in seq_num_strategy(),
        cl_ord_id in cl_ord_id_strategy(),
        symbol in symbol_strategy(),
        side in prop::sample::select(vec![b'1', b'2', b'5']),
        qty in qty_strategy(),
        price in price_strategy(),
    ) {
        let mut buf = [0u8; 4096];
        let len = serializer::build_new_order_single(
            &mut buf,
            b"FIX.4.4",
            &sender,
            &target,
            seq,
            b"20260321-10:00:00.000",
            &cl_ord_id,
            &symbol,
            side,
            qty,
            b'2', // Limit
            &price,
        );

        let parser = FixParser::new();
        let (view, consumed) = parser.parse(&buf[..len]).unwrap();

        prop_assert_eq!(consumed, len);
        prop_assert_eq!(view.msg_type(), Some(b"D".as_slice()));
        prop_assert_eq!(view.get_field(tags::CL_ORD_ID), Some(cl_ord_id.as_slice()));
        prop_assert_eq!(view.get_field(tags::SYMBOL), Some(symbol.as_slice()));
        prop_assert_eq!(view.get_field_i64(tags::ORDER_QTY), Some(qty));
        prop_assert_eq!(view.get_field(tags::PRICE), Some(price.as_slice()));
        prop_assert_eq!(view.msg_seq_num(), Some(seq));
        prop_assert!(view.is_checksum_valid());
    }

    /// Every ExecutionReport we serialize should roundtrip correctly.
    #[test]
    fn prop_exec_report_roundtrip(
        sender in comp_id_strategy(),
        target in comp_id_strategy(),
        seq in seq_num_strategy(),
        order_id in cl_ord_id_strategy(),
        exec_id in cl_ord_id_strategy(),
        cl_ord_id in cl_ord_id_strategy(),
        symbol in symbol_strategy(),
        qty in qty_strategy(),
        price in price_strategy(),
    ) {
        let mut buf = [0u8; 4096];
        let len = serializer::build_execution_report(
            &mut buf,
            b"FIX.4.4",
            &sender,
            &target,
            seq,
            b"20260321-10:00:00.000",
            &order_id,
            &exec_id,
            &cl_ord_id,
            &symbol,
            b'1',
            qty,
            qty / 2,
            &price,
            qty - qty / 2,
            qty / 2,
            &price,
            b'F',
            b'1',
        );

        let parser = FixParser::new();
        let (view, consumed) = parser.parse(&buf[..len]).unwrap();

        prop_assert_eq!(consumed, len);
        prop_assert_eq!(view.msg_type(), Some(b"8".as_slice()));
        prop_assert_eq!(view.get_field(tags::ORDER_ID), Some(order_id.as_slice()));
        prop_assert_eq!(view.get_field(tags::EXEC_ID), Some(exec_id.as_slice()));
        prop_assert!(view.is_checksum_valid());
    }

    /// Parser must never panic on arbitrary byte input.
    #[test]
    fn prop_parser_no_panic_on_garbage(
        data in prop::collection::vec(any::<u8>(), 0..1024),
    ) {
        let parser = FixParser::new();
        // We don't care about the result, only that it doesn't panic
        let _ = parser.parse(&data);
    }

    /// Parser must never panic on inputs that look partially like FIX.
    #[test]
    fn prop_parser_no_panic_on_partial_fix(
        garbage_len in 0usize..512,
    ) {
        let mut data = Vec::new();
        data.extend_from_slice(b"8=FIX.4.4\x01");
        data.extend(vec![0u8; garbage_len]);

        let parser = FixParser::new();
        let _ = parser.parse(&data);
    }

    /// Sequence numbers must always increment correctly.
    #[test]
    fn prop_session_sequence_monotonic(
        msg_count in 1usize..10_000,
    ) {
        use velocitas_fix::session::*;

        let config = SessionConfig {
            session_id: "PROP".to_string(),
            fix_version: "FIX.4.4".to_string(),
            sender_comp_id: "S".to_string(),
            target_comp_id: "T".to_string(),
            role: SessionRole::Initiator,
            heartbeat_interval: std::time::Duration::from_secs(30),
            reconnect_interval: std::time::Duration::from_secs(1),
            max_reconnect_attempts: 0,
            sequence_reset_policy: SequenceResetPolicy::Never,
            validate_comp_ids: true,
            max_msg_rate: 1_000_000,
        };

        let mut session = Session::new(config);
        let mut prev = 0u64;

        for _ in 0..msg_count {
            let seq = session.next_outbound_seq_num();
            prop_assert!(seq > prev, "Sequence must be monotonically increasing");
            prev = seq;
        }
    }
}
