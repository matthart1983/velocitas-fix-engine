/// FIXT 1.1 session protocol handler.
///
/// Wraps the base `Session` and adds FIXT-specific negotiation.
/// In FIXT 1.1, BeginString is always "FIXT.1.1" regardless of the
/// application-layer FIX version. The application version is negotiated
/// during Logon via ApplVerID (tag 1128) / DefaultApplVerID (tag 1137),
/// and multiple application versions can be supported simultaneously.
use crate::message::MessageView;
use crate::session::{Session, SessionConfig, SessionState};

// ---------------------------------------------------------------------------
// FIXT tag constants
// ---------------------------------------------------------------------------

/// ApplVerID (tag 1128) — application-level FIX version on a per-message basis.
pub const APPL_VER_ID: u32 = 1128;

/// DefaultApplVerID (tag 1137) — default application-level FIX version for the session.
pub const DEFAULT_APPL_VER_ID: u32 = 1137;

/// CstmApplVerID (tag 1129) — custom application version identifier.
pub const CSTM_APPL_VER_ID: u32 = 1129;

/// ApplExtID (tag 1156) — application extension identifier.
pub const APPL_EXT_ID: u32 = 1156;

/// The FIXT 1.1 BeginString constant.
pub const FIXT_BEGIN_STRING: &str = "FIXT.1.1";

// ---------------------------------------------------------------------------
// ApplVerID
// ---------------------------------------------------------------------------

/// FIX application version identifiers (tag 1128 / 1137 values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplVerID {
    Fix27,    // "0"
    Fix30,    // "1"
    Fix40,    // "2"
    Fix41,    // "3"
    Fix42,    // "4"
    Fix43,    // "5"
    Fix44,    // "6"
    Fix50,    // "7"
    Fix50SP1, // "8"
    Fix50SP2, // "9"
}

impl ApplVerID {
    /// Parse an `ApplVerID` from the wire-format byte value.
    #[inline]
    pub fn from_bytes(bytes: &[u8]) -> Option<ApplVerID> {
        match bytes {
            b"0" => Some(ApplVerID::Fix27),
            b"1" => Some(ApplVerID::Fix30),
            b"2" => Some(ApplVerID::Fix40),
            b"3" => Some(ApplVerID::Fix41),
            b"4" => Some(ApplVerID::Fix42),
            b"5" => Some(ApplVerID::Fix43),
            b"6" => Some(ApplVerID::Fix44),
            b"7" => Some(ApplVerID::Fix50),
            b"8" => Some(ApplVerID::Fix50SP1),
            b"9" => Some(ApplVerID::Fix50SP2),
            _ => None,
        }
    }

    /// Return the wire-format byte representation.
    #[inline]
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            ApplVerID::Fix27 => b"0",
            ApplVerID::Fix30 => b"1",
            ApplVerID::Fix40 => b"2",
            ApplVerID::Fix41 => b"3",
            ApplVerID::Fix42 => b"4",
            ApplVerID::Fix43 => b"5",
            ApplVerID::Fix44 => b"6",
            ApplVerID::Fix50 => b"7",
            ApplVerID::Fix50SP1 => b"8",
            ApplVerID::Fix50SP2 => b"9",
        }
    }

    /// Return a human-readable FIX version string (e.g. `"FIX.5.0SP2"`).
    #[inline]
    pub fn as_fix_version_str(&self) -> &'static str {
        match self {
            ApplVerID::Fix27 => "FIX.2.7",
            ApplVerID::Fix30 => "FIX.3.0",
            ApplVerID::Fix40 => "FIX.4.0",
            ApplVerID::Fix41 => "FIX.4.1",
            ApplVerID::Fix42 => "FIX.4.2",
            ApplVerID::Fix43 => "FIX.4.3",
            ApplVerID::Fix44 => "FIX.4.4",
            ApplVerID::Fix50 => "FIX.5.0",
            ApplVerID::Fix50SP1 => "FIX.5.0SP1",
            ApplVerID::Fix50SP2 => "FIX.5.0SP2",
        }
    }
}

// ---------------------------------------------------------------------------
// FixtSessionConfig
// ---------------------------------------------------------------------------

/// FIXT 1.1 session configuration (extends `SessionConfig`).
pub struct FixtSessionConfig {
    pub base: SessionConfig,
    pub default_appl_ver_id: ApplVerID,
    pub supported_versions: Vec<ApplVerID>,
}

// ---------------------------------------------------------------------------
// FixtSession
// ---------------------------------------------------------------------------

/// FIXT 1.1 session.
///
/// Wraps a base `Session` and layers on FIXT-specific version negotiation.
/// The `BeginString` on the wire is always `FIXT.1.1`; the application-layer
/// FIX version is carried in `DefaultApplVerID` (tag 1137) on the Logon and
/// optionally overridden per-message via `ApplVerID` (tag 1128).
pub struct FixtSession {
    base: Session,
    config: FixtSessionConfig,
    negotiated_version: Option<ApplVerID>,
}

/// A field to be added to an outbound Logon message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogonField {
    pub tag: u32,
    pub value: Vec<u8>,
}

/// Errors produced by FIXT version validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtError {
    MissingDefaultApplVerID,
    UnsupportedApplVerID(Vec<u8>),
    InvalidApplVerID(Vec<u8>),
    MissingApplVerID,
}

impl FixtSession {
    /// Create a new FIXT 1.1 session.
    ///
    /// The base `SessionConfig::fix_version` is forced to `"FIXT.1.1"`.
    pub fn new(mut config: FixtSessionConfig) -> Self {
        config.base.fix_version = FIXT_BEGIN_STRING.to_string();

        if !config
            .supported_versions
            .contains(&config.default_appl_ver_id)
        {
            config.supported_versions.push(config.default_appl_ver_id);
        }

        let base = Session::new(config.base.clone());
        FixtSession {
            base,
            config,
            negotiated_version: None,
        }
    }

    // -- FIXT-specific API --------------------------------------------------

    /// Returns `true` — this is a FIXT session (as opposed to FIX 4.x).
    #[inline]
    pub fn is_fixt(&self) -> bool {
        true
    }

    /// Return the negotiated application version, if logon has completed.
    #[inline]
    pub fn negotiated_version(&self) -> Option<ApplVerID> {
        self.negotiated_version
    }

    /// Process an inbound Logon message: extract and validate `DefaultApplVerID`.
    ///
    /// On success the negotiated application version is stored and can be
    /// retrieved via [`negotiated_version`].
    pub fn on_logon_received(&mut self, msg: &MessageView<'_>) -> Result<(), FixtError> {
        let ver_bytes = msg
            .get_field(DEFAULT_APPL_VER_ID)
            .ok_or(FixtError::MissingDefaultApplVerID)?;

        let ver = ApplVerID::from_bytes(ver_bytes)
            .ok_or_else(|| FixtError::InvalidApplVerID(ver_bytes.to_vec()))?;

        if !self.config.supported_versions.contains(&ver) {
            return Err(FixtError::UnsupportedApplVerID(ver_bytes.to_vec()));
        }

        self.negotiated_version = Some(ver);
        self.base.on_logon();
        Ok(())
    }

    /// Build the FIXT-specific fields that must be appended to an outbound
    /// Logon message (`DefaultApplVerID`).
    pub fn build_logon_fields(&self) -> Vec<LogonField> {
        vec![LogonField {
            tag: DEFAULT_APPL_VER_ID,
            value: self.config.default_appl_ver_id.as_bytes().to_vec(),
        }]
    }

    /// Validate the `ApplVerID` (tag 1128) on an inbound application message.
    ///
    /// Session-level messages (Heartbeat, TestRequest, etc.) do not carry
    /// `ApplVerID` and always pass validation. For application messages the
    /// tag is optional — when absent the negotiated default is assumed.
    pub fn validate_appl_ver(&self, msg: &MessageView<'_>) -> Result<ApplVerID, FixtError> {
        if let Some(mt) = msg.msg_type_enum() {
            if mt.is_session_level() {
                return Ok(self
                    .negotiated_version
                    .unwrap_or(self.config.default_appl_ver_id));
            }
        }

        match msg.get_field(APPL_VER_ID) {
            Some(ver_bytes) => {
                let ver = ApplVerID::from_bytes(ver_bytes)
                    .ok_or_else(|| FixtError::InvalidApplVerID(ver_bytes.to_vec()))?;

                if !self.config.supported_versions.contains(&ver) {
                    return Err(FixtError::UnsupportedApplVerID(ver_bytes.to_vec()));
                }
                Ok(ver)
            }
            None => self.negotiated_version.ok_or(FixtError::MissingApplVerID),
        }
    }

    // -- Delegated base session methods -------------------------------------

    /// Get the current session state.
    #[inline]
    pub fn state(&self) -> SessionState {
        self.base.state()
    }

    /// Get the session configuration.
    #[inline]
    pub fn config(&self) -> &SessionConfig {
        self.base.config()
    }

    /// Handle the transport-connected event.
    #[inline]
    pub fn on_connected(&mut self) {
        self.base.on_connected();
    }

    /// Handle successful logon (use [`on_logon_received`] for FIXT negotiation).
    #[inline]
    pub fn on_logon(&mut self) {
        self.base.on_logon();
    }

    /// Validate inbound sequence number.
    #[inline]
    pub fn validate_inbound_seq(&mut self, received_seq: u64) -> Result<(), (u64, u64)> {
        self.base.validate_inbound_seq(received_seq)
    }

    /// Handle gap fill completion.
    #[inline]
    pub fn on_gap_filled(&mut self, new_seq: u64) {
        self.base.on_gap_filled(new_seq);
    }

    /// Handle logout sent.
    #[inline]
    pub fn on_logout_sent(&mut self) {
        self.base.on_logout_sent();
    }

    /// Handle disconnect.
    #[inline]
    pub fn on_disconnected(&mut self) {
        self.base.on_disconnected();
        self.negotiated_version = None;
    }

    /// Check if heartbeat should be sent.
    #[inline]
    pub fn check_heartbeat(&mut self, now: std::time::Instant) -> crate::session::SessionAction {
        self.base.check_heartbeat(now)
    }

    /// Record that a message was sent.
    #[inline]
    pub fn on_message_sent(&mut self) {
        self.base.on_message_sent();
    }

    /// Record that a message was received.
    #[inline]
    pub fn on_message_received(&mut self) {
        self.base.on_message_received();
    }

    /// Check if the session should attempt reconnection.
    #[inline]
    pub fn should_reconnect(&self) -> bool {
        self.base.should_reconnect()
    }

    /// Increment reconnect attempt counter.
    #[inline]
    pub fn on_reconnect_attempt(&mut self) {
        self.base.on_reconnect_attempt();
    }

    /// Reset sequence numbers.
    #[inline]
    pub fn reset_sequences(&mut self) {
        self.base.reset_sequences();
    }

    /// Get the next outbound sequence number (and increment).
    #[inline]
    pub fn next_outbound_seq_num(&mut self) -> u64 {
        self.base.next_outbound_seq_num()
    }

    /// Get the expected inbound sequence number.
    #[inline]
    pub fn expected_inbound_seq_num(&self) -> u64 {
        self.base.expected_inbound_seq_num()
    }

    /// Get the current outbound sequence number (without incrementing).
    #[inline]
    pub fn current_outbound_seq_num(&self) -> u64 {
        self.base.current_outbound_seq_num()
    }

    /// Check rate limit.
    #[inline]
    pub fn check_rate_limit(&mut self) -> bool {
        self.base.check_rate_limit()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SequenceResetPolicy, SessionConfig, SessionRole, SessionState};
    use crate::tags;
    use std::time::Duration;

    fn fixt_config() -> FixtSessionConfig {
        FixtSessionConfig {
            base: SessionConfig {
                session_id: "FIXT-TEST-1".to_string(),
                fix_version: "FIXT.1.1".to_string(),
                sender_comp_id: "SENDER".to_string(),
                target_comp_id: "TARGET".to_string(),
                role: SessionRole::Initiator,
                heartbeat_interval: Duration::from_secs(30),
                reconnect_interval: Duration::from_secs(1),
                max_reconnect_attempts: 5,
                sequence_reset_policy: SequenceResetPolicy::Daily,
                validate_comp_ids: true,
                max_msg_rate: 1000,
            },
            default_appl_ver_id: ApplVerID::Fix50SP2,
            supported_versions: vec![ApplVerID::Fix50, ApplVerID::Fix50SP1, ApplVerID::Fix50SP2],
        }
    }

    /// Helper: build a minimal MessageView with the supplied fields.
    fn build_msg<'a>(buf: &'a [u8], fields: &[(u32, u32, u16)]) -> MessageView<'a> {
        let mut view = MessageView::new(buf);
        for &(tag, offset, length) in fields {
            view.add_field(tag, offset, length);
        }
        view
    }

    // -- ApplVerID conversion -----------------------------------------------

    #[test]
    fn test_appl_ver_id_roundtrip() {
        let all = [
            ApplVerID::Fix27,
            ApplVerID::Fix30,
            ApplVerID::Fix40,
            ApplVerID::Fix41,
            ApplVerID::Fix42,
            ApplVerID::Fix43,
            ApplVerID::Fix44,
            ApplVerID::Fix50,
            ApplVerID::Fix50SP1,
            ApplVerID::Fix50SP2,
        ];

        for ver in &all {
            let bytes = ver.as_bytes();
            let parsed = ApplVerID::from_bytes(bytes).unwrap();
            assert_eq!(*ver, parsed);
        }
    }

    #[test]
    fn test_appl_ver_id_version_strings() {
        assert_eq!(ApplVerID::Fix50SP2.as_fix_version_str(), "FIX.5.0SP2");
        assert_eq!(ApplVerID::Fix44.as_fix_version_str(), "FIX.4.4");
        assert_eq!(ApplVerID::Fix27.as_fix_version_str(), "FIX.2.7");
    }

    #[test]
    fn test_appl_ver_id_invalid() {
        assert_eq!(ApplVerID::from_bytes(b"X"), None);
        assert_eq!(ApplVerID::from_bytes(b""), None);
        assert_eq!(ApplVerID::from_bytes(b"10"), None);
    }

    // -- FixtSession creation -----------------------------------------------

    #[test]
    fn test_fixt_begin_string() {
        let session = FixtSession::new(fixt_config());
        assert_eq!(session.config().fix_version, "FIXT.1.1");
    }

    #[test]
    fn test_fixt_is_fixt() {
        let session = FixtSession::new(fixt_config());
        assert!(session.is_fixt());
    }

    #[test]
    fn test_fixt_initial_state() {
        let session = FixtSession::new(fixt_config());
        assert_eq!(session.state(), SessionState::Disconnected);
        assert_eq!(session.negotiated_version(), None);
    }

    // -- Version negotiation ------------------------------------------------

    #[test]
    fn test_negotiate_fix50sp2() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        // Simulate inbound Logon with DefaultApplVerID=9 (FIX.5.0SP2)
        //   "A\x019\x01"  — MsgType at offset 0 len 1, DefaultApplVerID at offset 2 len 1
        let buf = b"A\x019\x01";
        let view = build_msg(
            buf,
            &[
                (tags::MSG_TYPE, 0, 1),      // "A"
                (DEFAULT_APPL_VER_ID, 2, 1), // "9"
            ],
        );

        let result = session.on_logon_received(&view);
        assert!(result.is_ok());
        assert_eq!(session.negotiated_version(), Some(ApplVerID::Fix50SP2));
        assert_eq!(session.state(), SessionState::Active);
    }

    #[test]
    fn test_negotiate_fix50() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        let buf = b"A\x017\x01";
        let view = build_msg(buf, &[(tags::MSG_TYPE, 0, 1), (DEFAULT_APPL_VER_ID, 2, 1)]);

        let result = session.on_logon_received(&view);
        assert!(result.is_ok());
        assert_eq!(session.negotiated_version(), Some(ApplVerID::Fix50));
    }

    #[test]
    fn test_reject_unsupported_version() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        // DefaultApplVerID = "4" → FIX.4.2, which is not in supported_versions
        let buf = b"A\x014\x01";
        let view = build_msg(buf, &[(tags::MSG_TYPE, 0, 1), (DEFAULT_APPL_VER_ID, 2, 1)]);

        let result = session.on_logon_received(&view);
        assert_eq!(result, Err(FixtError::UnsupportedApplVerID(b"4".to_vec())));
        assert_eq!(session.negotiated_version(), None);
    }

    #[test]
    fn test_reject_missing_default_appl_ver_id() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        let buf = b"A\x01";
        let view = build_msg(buf, &[(tags::MSG_TYPE, 0, 1)]);

        let result = session.on_logon_received(&view);
        assert_eq!(result, Err(FixtError::MissingDefaultApplVerID));
    }

    #[test]
    fn test_reject_invalid_appl_ver_id() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        let buf = b"A\x01X\x01";
        let view = build_msg(buf, &[(tags::MSG_TYPE, 0, 1), (DEFAULT_APPL_VER_ID, 2, 1)]);

        let result = session.on_logon_received(&view);
        assert_eq!(result, Err(FixtError::InvalidApplVerID(b"X".to_vec())));
    }

    // -- Logon fields -------------------------------------------------------

    #[test]
    fn test_build_logon_fields() {
        let session = FixtSession::new(fixt_config());
        let fields = session.build_logon_fields();

        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].tag, DEFAULT_APPL_VER_ID);
        assert_eq!(fields[0].value, b"9"); // Fix50SP2
    }

    // -- Application message validation -------------------------------------

    #[test]
    fn test_validate_appl_ver_session_msg() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        // Negotiate first
        let buf = b"A\x019\x01";
        let logon = build_msg(buf, &[(tags::MSG_TYPE, 0, 1), (DEFAULT_APPL_VER_ID, 2, 1)]);
        session.on_logon_received(&logon).unwrap();

        // Session-level Heartbeat — no ApplVerID required, always valid
        let hb_buf = b"0\x01";
        let hb = build_msg(hb_buf, &[(tags::MSG_TYPE, 0, 1)]);
        let result = session.validate_appl_ver(&hb);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_appl_ver_app_msg_default() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        let buf = b"A\x019\x01";
        let logon = build_msg(buf, &[(tags::MSG_TYPE, 0, 1), (DEFAULT_APPL_VER_ID, 2, 1)]);
        session.on_logon_received(&logon).unwrap();

        // Application-level NewOrderSingle without explicit ApplVerID → uses default
        let nos_buf = b"D\x01";
        let nos = build_msg(nos_buf, &[(tags::MSG_TYPE, 0, 1)]);
        let result = session.validate_appl_ver(&nos);
        assert_eq!(result, Ok(ApplVerID::Fix50SP2));
    }

    #[test]
    fn test_validate_appl_ver_app_msg_explicit() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        let buf = b"A\x019\x01";
        let logon = build_msg(buf, &[(tags::MSG_TYPE, 0, 1), (DEFAULT_APPL_VER_ID, 2, 1)]);
        session.on_logon_received(&logon).unwrap();

        // NewOrderSingle with explicit ApplVerID = "7" (FIX.5.0)
        let nos_buf = b"D\x017\x01";
        let nos = build_msg(nos_buf, &[(tags::MSG_TYPE, 0, 1), (APPL_VER_ID, 2, 1)]);
        let result = session.validate_appl_ver(&nos);
        assert_eq!(result, Ok(ApplVerID::Fix50));
    }

    #[test]
    fn test_validate_appl_ver_unsupported() {
        let mut session = FixtSession::new(fixt_config());
        session.on_connected();

        let buf = b"A\x019\x01";
        let logon = build_msg(buf, &[(tags::MSG_TYPE, 0, 1), (DEFAULT_APPL_VER_ID, 2, 1)]);
        session.on_logon_received(&logon).unwrap();

        // NewOrderSingle with ApplVerID = "4" (FIX.4.2, not supported)
        let nos_buf = b"D\x014\x01";
        let nos = build_msg(nos_buf, &[(tags::MSG_TYPE, 0, 1), (APPL_VER_ID, 2, 1)]);
        let result = session.validate_appl_ver(&nos);
        assert_eq!(result, Err(FixtError::UnsupportedApplVerID(b"4".to_vec())));
    }

    // -- Full lifecycle -----------------------------------------------------

    #[test]
    fn test_fixt_lifecycle() {
        let mut session = FixtSession::new(fixt_config());

        // 1. Disconnected
        assert_eq!(session.state(), SessionState::Disconnected);

        // 2. Connect
        session.on_connected();
        assert_eq!(session.state(), SessionState::LogonSent);

        // 3. Logon with version negotiation
        let buf = b"A\x019\x01";
        let logon = build_msg(buf, &[(tags::MSG_TYPE, 0, 1), (DEFAULT_APPL_VER_ID, 2, 1)]);
        session.on_logon_received(&logon).unwrap();
        assert_eq!(session.state(), SessionState::Active);
        assert_eq!(session.negotiated_version(), Some(ApplVerID::Fix50SP2));

        // 4. Exchange messages
        assert_eq!(session.next_outbound_seq_num(), 1);
        assert!(session.validate_inbound_seq(1).is_ok());

        // 5. Disconnect clears negotiated version
        session.on_disconnected();
        assert_eq!(session.state(), SessionState::Disconnected);
        assert_eq!(session.negotiated_version(), None);
    }

    // -- Session-level messages always use FIXT.1.1 -------------------------

    #[test]
    fn test_session_messages_use_fixt_begin_string() {
        let session = FixtSession::new(fixt_config());

        // Regardless of the application version, the session config
        // (and therefore all serialized session-level messages) must
        // use "FIXT.1.1" as the BeginString.
        assert_eq!(session.config().fix_version, FIXT_BEGIN_STRING);

        // Build a Heartbeat with the session's BeginString and verify
        let mut buf = [0u8; 1024];
        let len = crate::serializer::build_heartbeat(
            &mut buf,
            session.config().fix_version.as_bytes(),
            session.config().sender_comp_id.as_bytes(),
            session.config().target_comp_id.as_bytes(),
            1,
            b"20260321-10:00:00",
        );

        let parser = crate::parser::FixParser::new();
        let (view, _) = parser.parse(&buf[..len]).expect("should parse heartbeat");
        assert_eq!(view.begin_string(), Some("FIXT.1.1"));
    }
}
