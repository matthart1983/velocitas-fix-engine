use velocitas_fix::checksum;
/// FIX Protocol Conformance Tests
///
/// Validates compliance with the FIX 4.4 specification requirements.
/// These tests use pre-built wire-format messages to verify correct parsing
/// of real-world FIX message patterns.
use velocitas_fix::parser::FixParser;
use velocitas_fix::tags;

/// Helper: build a raw FIX message from body fields, computing correct
/// BodyLength and Checksum.
fn build_raw_fix(begin_string: &str, body: &str) -> Vec<u8> {
    let soh = '\x01';
    let begin = format!("8={begin_string}{soh}");
    let body_len = body.len();
    let header = format!("{begin}9={body_len}{soh}");
    let pre_checksum = format!("{header}{body}");
    let cs = checksum::compute(pre_checksum.as_bytes());
    let mut cs_buf = [0u8; 3];
    checksum::format(cs, &mut cs_buf);
    let cs_str = std::str::from_utf8(&cs_buf).unwrap();
    format!("{pre_checksum}10={cs_str}{soh}").into_bytes()
}

// ============================================================================
// FIX 4.4 Section 3: Message Format
// ============================================================================

#[test]
fn test_conform_first_three_fields_order() {
    // BeginString (8) must be first, BodyLength (9) second, MsgType (35) third
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=0\x0149=SENDER\x0156=TARGET\x0134=1\x0152=20260321-10:00:00\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();

    let fields = view.fields();
    assert_eq!(fields[0].tag, 8); // BeginString
    assert_eq!(fields[1].tag, 9); // BodyLength
    assert_eq!(fields[2].tag, 35); // MsgType
}

#[test]
fn test_conform_checksum_is_last_field() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=0\x0149=S\x0156=T\x0134=1\x0152=20260321-10:00:00\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();

    let fields = view.fields();
    let last = fields.last().unwrap();
    assert_eq!(last.tag, 10); // Checksum must be last
}

// ============================================================================
// FIX 4.4 Section 4: Session-Level Messages
// ============================================================================

#[test]
fn test_conform_logon_required_fields() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=A\x0149=CLIENT\x0156=SERVER\x0134=1\x0152=20260321-10:00:00\x0198=0\x01108=30\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();

    assert!(view.get_field(tags::MSG_TYPE).is_some());
    assert!(view.get_field(tags::SENDER_COMP_ID).is_some());
    assert!(view.get_field(tags::TARGET_COMP_ID).is_some());
    assert!(view.get_field(tags::MSG_SEQ_NUM).is_some());
    assert!(view.get_field(tags::SENDING_TIME).is_some());
    assert!(view.get_field(tags::ENCRYPT_METHOD).is_some());
    assert!(view.get_field(tags::HEARTBT_INT).is_some());
}

#[test]
fn test_conform_heartbeat_structure() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=0\x0149=S\x0156=T\x0134=5\x0152=20260321-10:00:00.123\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();
    assert_eq!(view.msg_type(), Some(b"0".as_slice()));
}

#[test]
fn test_conform_test_request_with_id() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=1\x0149=S\x0156=T\x0134=10\x0152=20260321-10:00:00\x01112=TEST123\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();
    assert_eq!(view.msg_type(), Some(b"1".as_slice()));
    assert_eq!(view.get_field_str(tags::TEST_REQ_ID), Some("TEST123"));
}

#[test]
fn test_conform_resend_request() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=2\x0149=S\x0156=T\x0134=3\x0152=20260321-10:00:00\x017=1\x0116=100\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();
    assert_eq!(view.msg_type(), Some(b"2".as_slice()));
    assert_eq!(view.get_field_i64(tags::BEGIN_SEQ_NO), Some(1));
    assert_eq!(view.get_field_i64(tags::END_SEQ_NO), Some(100));
}

#[test]
fn test_conform_sequence_reset_gap_fill() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=4\x0149=S\x0156=T\x0134=5\x0152=20260321-10:00:00\x0143=Y\x01123=Y\x0136=10\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();
    assert_eq!(view.msg_type(), Some(b"4".as_slice()));
    assert_eq!(view.get_field_str(tags::GAP_FILL_FLAG), Some("Y"));
    assert_eq!(view.get_field_i64(tags::NEW_SEQ_NO), Some(10));
    assert_eq!(view.get_field_str(tags::POSS_DUP_FLAG), Some("Y"));
}

#[test]
fn test_conform_session_reject() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=3\x0149=S\x0156=T\x0134=7\x0152=20260321-10:00:00\x0145=5\x01373=1\x01372=D\x0158=Invalid tag\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();
    assert_eq!(view.msg_type(), Some(b"3".as_slice()));
    assert_eq!(view.get_field_i64(tags::REF_SEQ_NUM), Some(5));
    assert_eq!(view.get_field_str(tags::TEXT), Some("Invalid tag"));
}

#[test]
fn test_conform_logout() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=5\x0149=S\x0156=T\x0134=100\x0152=20260321-17:00:00\x0158=End of trading day\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();
    assert_eq!(view.msg_type(), Some(b"5".as_slice()));
    assert_eq!(view.get_field_str(tags::TEXT), Some("End of trading day"));
}

// ============================================================================
// FIX 4.4 Application Messages
// ============================================================================

#[test]
fn test_conform_new_order_single_all_fields() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=D\x0149=BANK\x0156=NYSE\x0134=42\x0152=20260321-10:00:00.123\x01\
         11=ORD-00001\x0121=1\x0155=AAPL\x0154=1\x0160=20260321-10:00:00.123\x01\
         38=10000\x0140=2\x0144=178.55\x0159=0\x011=ACCT001\x01207=XNYS\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();

    assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("ORD-00001"));
    assert_eq!(view.get_field_str(tags::HANDL_INST), Some("1"));
    assert_eq!(view.get_field_str(tags::SYMBOL), Some("AAPL"));
    assert_eq!(view.get_field_str(tags::SIDE), Some("1"));
    assert_eq!(view.get_field_i64(tags::ORDER_QTY), Some(10000));
    assert_eq!(view.get_field_str(tags::ORD_TYPE), Some("2"));
    assert_eq!(view.get_field_str(tags::PRICE), Some("178.55"));
    assert_eq!(view.get_field_str(tags::TIME_IN_FORCE), Some("0"));
    assert_eq!(view.get_field_str(tags::ACCOUNT), Some("ACCT001"));
    assert_eq!(view.get_field_str(tags::SECURITY_EXCHANGE), Some("XNYS"));
}

#[test]
fn test_conform_execution_report_new() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=8\x0149=NYSE\x0156=BANK\x0134=50\x0152=20260321-10:00:01\x01\
         37=NYSE-001\x0117=EXEC-001\x01150=0\x0139=0\x0155=AAPL\x0154=1\x01\
         38=10000\x0132=0\x0131=0\x01151=10000\x0114=0\x016=0\x0111=ORD-00001\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();

    assert_eq!(view.get_field_str(tags::EXEC_TYPE), Some("0")); // New
    assert_eq!(view.get_field_str(tags::ORD_STATUS), Some("0")); // New
    assert_eq!(view.get_field_i64(tags::LEAVES_QTY), Some(10000));
    assert_eq!(view.get_field_i64(tags::CUM_QTY), Some(0));
}

#[test]
fn test_conform_order_cancel_request() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=F\x0149=BANK\x0156=NYSE\x0134=43\x0152=20260321-10:01:00\x01\
         11=CXL-00001\x0141=ORD-00001\x0137=NYSE-001\x0155=AAPL\x0154=1\x01\
         38=10000\x0160=20260321-10:01:00\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();

    assert_eq!(view.msg_type(), Some(b"F".as_slice()));
    assert_eq!(view.get_field_str(tags::CL_ORD_ID), Some("CXL-00001"));
    assert_eq!(view.get_field_str(tags::ORIG_CL_ORD_ID), Some("ORD-00001"));
}

#[test]
fn test_conform_order_cancel_replace_request() {
    let msg = build_raw_fix(
        "FIX.4.4",
        "35=G\x0149=BANK\x0156=NYSE\x0134=44\x0152=20260321-10:02:00\x01\
         11=RPL-00001\x0141=ORD-00001\x0137=NYSE-001\x0155=AAPL\x0154=1\x01\
         38=5000\x0140=2\x0144=180.00\x0160=20260321-10:02:00\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();

    assert_eq!(view.msg_type(), Some(b"G".as_slice()));
    assert_eq!(view.get_field_i64(tags::ORDER_QTY), Some(5000));
    assert_eq!(view.get_field_str(tags::PRICE), Some("180.00"));
}

// ============================================================================
// Multiple FIX versions
// ============================================================================

#[test]
fn test_conform_fix42_message() {
    let msg = build_raw_fix(
        "FIX.4.2",
        "35=D\x0149=S\x0156=T\x0134=1\x0152=20260321-10:00:00\x01\
         11=O1\x0121=1\x0155=IBM\x0154=1\x0138=100\x0140=1\x0160=20260321-10:00:00\x01",
    );

    let parser = FixParser::new();
    let (view, _) = parser.parse(&msg).unwrap();
    assert_eq!(view.begin_string(), Some("FIX.4.2"));
    assert_eq!(view.msg_type(), Some(b"D".as_slice()));
}

// ============================================================================
// Negative conformance tests
// ============================================================================

#[test]
fn test_conform_reject_wrong_field_order() {
    // BodyLength before BeginString should fail
    let msg = b"9=10\x018=FIX.4.4\x0135=0\x0110=000\x01xxxx";
    let parser = FixParser::new();
    assert!(parser.parse(msg).is_err());
}

#[test]
fn test_conform_reject_missing_msg_type() {
    // Valid BeginString and BodyLength but third field is not MsgType
    let msg = build_raw_fix_no_validation("FIX.4.4", "49=S\x0156=T\x0134=1\x01");
    let parser = FixParser::new();
    assert!(parser.parse(&msg).is_err());
}

/// Build a message without proper MsgType ordering (for negative tests).
fn build_raw_fix_no_validation(begin_string: &str, body: &str) -> Vec<u8> {
    let soh = '\x01';
    let begin = format!("8={begin_string}{soh}");
    let body_len = body.len();
    let header = format!("{begin}9={body_len}{soh}");
    let pre_checksum = format!("{header}{body}");
    let cs = checksum::compute(pre_checksum.as_bytes());
    let mut cs_buf = [0u8; 3];
    checksum::format(cs, &mut cs_buf);
    let cs_str = std::str::from_utf8(&cs_buf).unwrap();
    format!("{pre_checksum}10={cs_str}{soh}").into_bytes()
}
