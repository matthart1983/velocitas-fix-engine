/// FIX session state machine.
///
/// Manages session lifecycle: logon, heartbeat, sequencing, gap detection,
/// logout, and reconnection. All state transitions are deterministic.
use std::time::{Duration, Instant};

/// Session state machine states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Disconnected,
    Connecting,
    LogonSent,
    Active,
    /// Waiting for gap fill / resend response from peer.
    Resending,
    /// Logout sent, waiting for peer Logout response (or timeout).
    LogoutPending,
    /// Legacy alias — marks state after logout exchange is complete.
    LogoutSent,
}

/// Session role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRole {
    Initiator,
    Acceptor,
}

/// Sequence number reset policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceResetPolicy {
    Always,
    Daily,
    Weekly,
    Never,
}

/// Session configuration.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub session_id: String,
    pub fix_version: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub role: SessionRole,
    pub heartbeat_interval: Duration,
    pub reconnect_interval: Duration,
    pub max_reconnect_attempts: u32,
    pub sequence_reset_policy: SequenceResetPolicy,
    pub validate_comp_ids: bool,
    pub max_msg_rate: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        SessionConfig {
            session_id: String::new(),
            fix_version: "FIX.4.4".to_string(),
            sender_comp_id: String::new(),
            target_comp_id: String::new(),
            role: SessionRole::Initiator,
            heartbeat_interval: Duration::from_secs(30),
            reconnect_interval: Duration::from_secs(1),
            max_reconnect_attempts: 0,
            sequence_reset_policy: SequenceResetPolicy::Daily,
            validate_comp_ids: true,
            max_msg_rate: 50_000,
        }
    }
}

/// Actions that the session state machine requests the engine to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionAction {
    /// Send a Heartbeat, optionally echoing a TestReqID.
    SendHeartbeat { test_req_id: Option<Vec<u8>> },
    /// Send a TestRequest with the given ID.
    SendTestRequest { test_req_id: Vec<u8> },
    /// Send a ResendRequest for the given sequence range.
    SendResendRequest { begin_seq: u64, end_seq: u64 },
    /// Send a Reject for the given reference sequence number.
    SendReject {
        ref_seq_num: u64,
        reason: u32,
        text: Option<String>,
    },
    /// Send a Logout with optional reason text.
    SendLogout { text: Option<String> },
    /// Disconnect the transport.
    Disconnect,
    /// No action needed.
    None,
}

/// FIX session — manages state, sequencing, and heartbeats.
pub struct Session {
    config: SessionConfig,
    state: SessionState,

    // Sequence numbers
    outbound_seq_num: u64,
    inbound_seq_num: u64,

    // Heartbeat tracking
    last_sent_time: Instant,
    last_received_time: Instant,
    test_request_pending: bool,
    test_request_id: Option<Vec<u8>>,

    // Reconnection
    reconnect_attempts: u32,
    test_req_counter: u64,

    // Rate limiting
    msg_count_window: u32,
    window_start: Instant,

    // Logout timer
    logout_sent_time: Option<Instant>,
    logout_timeout: Duration,
}

impl Session {
    /// Create a new session with the given configuration.
    pub fn new(config: SessionConfig) -> Self {
        let now = Instant::now();
        Session {
            config,
            state: SessionState::Disconnected,
            outbound_seq_num: 1,
            inbound_seq_num: 1,
            last_sent_time: now,
            last_received_time: now,
            test_request_pending: false,
            test_request_id: None,
            reconnect_attempts: 0,
            test_req_counter: 0,
            msg_count_window: 0,
            window_start: now,
            logout_sent_time: None,
            logout_timeout: Duration::from_secs(10),
        }
    }

    /// Get the current session state.
    #[inline]
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// Get the session configuration.
    #[inline]
    pub fn config(&self) -> &SessionConfig {
        &self.config
    }

    /// Get the next outbound sequence number (and increment).
    #[inline]
    pub fn next_outbound_seq_num(&mut self) -> u64 {
        let seq = self.outbound_seq_num;
        self.outbound_seq_num += 1;
        seq
    }

    /// Get the expected inbound sequence number.
    #[inline]
    pub fn expected_inbound_seq_num(&self) -> u64 {
        self.inbound_seq_num
    }

    /// Get the current outbound sequence number (without incrementing).
    #[inline]
    pub fn current_outbound_seq_num(&self) -> u64 {
        self.outbound_seq_num
    }

    /// Handle a state transition event.
    pub fn on_connected(&mut self) {
        match self.state {
            SessionState::Disconnected | SessionState::Connecting => {
                if self.config.role == SessionRole::Initiator {
                    self.state = SessionState::LogonSent;
                } else {
                    // Acceptor waits for inbound Logon
                    self.state = SessionState::Connecting;
                }
                self.last_received_time = Instant::now();
                self.last_sent_time = Instant::now();
                self.reconnect_attempts = 0;
            }
            _ => {}
        }
    }

    /// Handle successful logon.
    pub fn on_logon(&mut self) {
        self.state = SessionState::Active;
        self.last_received_time = Instant::now();
        self.test_request_pending = false;
    }

    /// Validate an inbound Logon message against session configuration.
    ///
    /// Checks BeginString, CompID directionality, and ResetSeqNumFlag.
    /// Returns `Ok(reset_sequences)` where `reset_sequences` is true if
    /// the peer requested sequence reset (tag 141=Y).
    /// Returns `Err(reason)` if validation fails — caller should send
    /// Logout with the reason text and disconnect.
    pub fn validate_logon(
        &self,
        begin_string: Option<&str>,
        sender_comp_id: Option<&str>,
        target_comp_id: Option<&str>,
        reset_seq_num_flag: Option<&[u8]>,
    ) -> Result<bool, String> {
        // Validate BeginString matches config
        if let Some(bs) = begin_string {
            if bs != self.config.fix_version {
                return Err(format!(
                    "BeginString mismatch: expected {}, received {}",
                    self.config.fix_version, bs
                ));
            }
        } else {
            return Err("Missing BeginString".to_string());
        }

        // Validate CompIDs — inbound Logon's SenderCompID should match
        // our config's TargetCompID (and vice versa)
        if self.config.validate_comp_ids {
            if let Some(sender) = sender_comp_id {
                if sender != self.config.target_comp_id {
                    return Err(format!(
                        "SenderCompID mismatch: expected {}, received {}",
                        self.config.target_comp_id, sender
                    ));
                }
            } else {
                return Err("Missing SenderCompID".to_string());
            }

            if let Some(target) = target_comp_id {
                if target != self.config.sender_comp_id {
                    return Err(format!(
                        "TargetCompID mismatch: expected {}, received {}",
                        self.config.sender_comp_id, target
                    ));
                }
            } else {
                return Err("Missing TargetCompID".to_string());
            }
        }

        // Check ResetSeqNumFlag (tag 141)
        let reset = reset_seq_num_flag
            .map(|v| v == b"Y")
            .unwrap_or(false);

        Ok(reset)
    }

    /// Handle logon with sequence reset — resets both sequence numbers and
    /// transitions to Active.
    pub fn on_logon_with_reset(&mut self) {
        self.reset_sequences();
        self.on_logon();
    }

    /// Handle inbound message sequence validation.
    /// Returns `Ok(())` if sequence is correct, or `Err` with the gap range.
    pub fn validate_inbound_seq(&mut self, received_seq: u64) -> Result<(), (u64, u64)> {
        if received_seq == self.inbound_seq_num {
            self.inbound_seq_num += 1;
            self.last_received_time = Instant::now();
            Ok(())
        } else if received_seq > self.inbound_seq_num {
            // Gap detected
            let gap_start = self.inbound_seq_num;
            let gap_end = received_seq;
            self.state = SessionState::Resending;
            Err((gap_start, gap_end))
        } else {
            // Duplicate or already processed — ignore
            Ok(())
        }
    }

    /// Handle gap fill completion.
    pub fn on_gap_filled(&mut self, new_seq: u64) {
        self.inbound_seq_num = new_seq;
        self.state = SessionState::Active;
    }

    /// Handle locally-initiated logout — transitions to LogoutPending and
    /// starts the logout response timer.
    pub fn on_logout_sent(&mut self) {
        self.state = SessionState::LogoutPending;
        self.logout_sent_time = Some(Instant::now());
    }

    /// Check if we're waiting for a logout response and have timed out.
    pub fn check_logout_timeout(&self, now: Instant) -> bool {
        if self.state != SessionState::LogoutPending {
            return false;
        }
        if let Some(sent_time) = self.logout_sent_time {
            return now.duration_since(sent_time) >= self.logout_timeout;
        }
        false
    }

    /// Mark logout exchange as complete.
    pub fn on_logout_complete(&mut self) {
        self.state = SessionState::LogoutSent;
        self.logout_sent_time = None;
    }

    /// Handle disconnect.
    pub fn on_disconnected(&mut self) {
        self.state = SessionState::Disconnected;
        self.test_request_pending = false;
        self.logout_sent_time = None;
    }

    /// Returns true if the session is in gap-fill / resend recovery.
    #[inline]
    pub fn is_resending(&self) -> bool {
        self.state == SessionState::Resending
    }

    /// Check if heartbeat or test request should be sent (called periodically).
    pub fn check_heartbeat(&mut self, now: Instant) -> SessionAction {
        if self.state != SessionState::Active {
            return SessionAction::None;
        }

        let since_received = now.duration_since(self.last_received_time);

        // Check receive timeout first — if we haven't received anything in
        // heartbeat_interval + reasonable transmission time, send TestRequest
        // or disconnect if one is already pending.
        let timeout = self.config.heartbeat_interval + Duration::from_secs(5);
        if since_received >= timeout {
            if self.test_request_pending {
                // Already sent a TestRequest and got no response — disconnect
                return SessionAction::Disconnect;
            } else {
                self.test_req_counter += 1;
                let id = format!("TR-{}", self.test_req_counter).into_bytes();
                self.test_request_pending = true;
                self.test_request_id = Some(id.clone());
                return SessionAction::SendTestRequest { test_req_id: id };
            }
        }

        // Send heartbeat if we haven't sent anything in heartbeat_interval
        let since_sent = now.duration_since(self.last_sent_time);
        if since_sent >= self.config.heartbeat_interval {
            self.last_sent_time = now;
            return SessionAction::SendHeartbeat { test_req_id: None };
        }

        SessionAction::None
    }

    /// Record that a message was sent (for heartbeat timing).
    #[inline]
    pub fn on_message_sent(&mut self) {
        self.last_sent_time = Instant::now();
    }

    /// Record that a message was received (for heartbeat timing).
    ///
    /// Note: this does NOT clear `test_request_pending`. The pending test
    /// request is only cleared by `on_test_request_response()` when a
    /// Heartbeat echoing the correct TestReqID is received.
    #[inline]
    pub fn on_message_received(&mut self) {
        self.last_received_time = Instant::now();
    }

    /// Handle an inbound Heartbeat that may be a TestRequest response.
    /// If `test_req_id` matches the pending TestReqID, clears the pending flag.
    /// A Heartbeat with no TestReqID also clears it (lenient, per common impls).
    pub fn on_heartbeat_received(&mut self, test_req_id: Option<&[u8]>) {
        if !self.test_request_pending {
            return;
        }
        match (&self.test_request_id, test_req_id) {
            (Some(expected), Some(received)) if expected.as_slice() == received => {
                self.test_request_pending = false;
                self.test_request_id = None;
            }
            (_, None) => {
                // Lenient: clear on any heartbeat even without TestReqID
                self.test_request_pending = false;
                self.test_request_id = None;
            }
            _ => {
                // Mismatched TestReqID — don't clear
            }
        }
    }

    /// Handle an inbound TestRequest. Returns the action to echo back
    /// a Heartbeat with the received TestReqID.
    pub fn on_test_request_received(&mut self, test_req_id: &[u8]) -> SessionAction {
        SessionAction::SendHeartbeat {
            test_req_id: Some(test_req_id.to_vec()),
        }
    }

    /// Check if the session should attempt reconnection.
    pub fn should_reconnect(&self) -> bool {
        if self.state != SessionState::Disconnected {
            return false;
        }
        if self.config.role != SessionRole::Initiator {
            return false;
        }
        if self.config.max_reconnect_attempts > 0
            && self.reconnect_attempts >= self.config.max_reconnect_attempts
        {
            return false;
        }
        true
    }

    /// Increment reconnect attempt counter.
    pub fn on_reconnect_attempt(&mut self) {
        self.reconnect_attempts += 1;
        self.state = SessionState::Connecting;
    }

    /// Reset sequence numbers (e.g., at start of day).
    pub fn reset_sequences(&mut self) {
        self.outbound_seq_num = 1;
        self.inbound_seq_num = 1;
    }

    /// Check rate limit. Returns true if the message should be allowed.
    #[inline]
    pub fn check_rate_limit(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.window_start);

        if elapsed >= Duration::from_secs(1) {
            self.msg_count_window = 0;
            self.window_start = now;
        }

        self.msg_count_window += 1;
        self.msg_count_window <= self.config.max_msg_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SessionConfig {
        SessionConfig {
            session_id: "TEST-1".to_string(),
            fix_version: "FIX.4.4".to_string(),
            sender_comp_id: "SENDER".to_string(),
            target_comp_id: "TARGET".to_string(),
            role: SessionRole::Initiator,
            heartbeat_interval: Duration::from_secs(30),
            reconnect_interval: Duration::from_secs(1),
            max_reconnect_attempts: 5,
            sequence_reset_policy: SequenceResetPolicy::Daily,
            validate_comp_ids: true,
            max_msg_rate: 1000,
        }
    }

    #[test]
    fn test_session_initial_state() {
        let session = Session::new(test_config());
        assert_eq!(session.state(), SessionState::Disconnected);
        assert_eq!(session.expected_inbound_seq_num(), 1);
        assert_eq!(session.current_outbound_seq_num(), 1);
    }

    #[test]
    fn test_session_connect_logon_flow() {
        let mut session = Session::new(test_config());

        session.on_connected();
        assert_eq!(session.state(), SessionState::LogonSent);

        session.on_logon();
        assert_eq!(session.state(), SessionState::Active);
    }

    #[test]
    fn test_session_sequence_numbers() {
        let mut session = Session::new(test_config());
        session.on_connected();
        session.on_logon();

        assert_eq!(session.next_outbound_seq_num(), 1);
        assert_eq!(session.next_outbound_seq_num(), 2);
        assert_eq!(session.next_outbound_seq_num(), 3);

        assert!(session.validate_inbound_seq(1).is_ok());
        assert!(session.validate_inbound_seq(2).is_ok());
        assert_eq!(session.expected_inbound_seq_num(), 3);
    }

    #[test]
    fn test_session_gap_detection() {
        let mut session = Session::new(test_config());
        session.on_connected();
        session.on_logon();

        assert!(session.validate_inbound_seq(1).is_ok());
        // Gap: expected 2, received 5
        let result = session.validate_inbound_seq(5);
        assert_eq!(result, Err((2, 5)));
        assert_eq!(session.state(), SessionState::Resending);
    }

    #[test]
    fn test_session_disconnect_reconnect() {
        let mut session = Session::new(test_config());
        session.on_connected();
        session.on_logon();
        session.on_disconnected();

        assert_eq!(session.state(), SessionState::Disconnected);
        assert!(session.should_reconnect());

        session.on_reconnect_attempt();
        assert_eq!(session.state(), SessionState::Connecting);
    }

    #[test]
    fn test_session_max_reconnect_attempts() {
        let mut session = Session::new(test_config());

        for _ in 0..5 {
            session.on_reconnect_attempt();
            session.on_disconnected();
        }

        assert!(!session.should_reconnect());
    }

    #[test]
    fn test_session_reset_sequences() {
        let mut session = Session::new(test_config());
        session.next_outbound_seq_num();
        session.next_outbound_seq_num();
        assert_eq!(session.current_outbound_seq_num(), 3);

        session.reset_sequences();
        assert_eq!(session.current_outbound_seq_num(), 1);
        assert_eq!(session.expected_inbound_seq_num(), 1);
    }

    #[test]
    fn test_session_rate_limit() {
        let mut config = test_config();
        config.max_msg_rate = 3;
        let mut session = Session::new(config);

        assert!(session.check_rate_limit());
        assert!(session.check_rate_limit());
        assert!(session.check_rate_limit());
        assert!(!session.check_rate_limit());
    }

    #[test]
    fn test_acceptor_session() {
        let mut config = test_config();
        config.role = SessionRole::Acceptor;
        let mut session = Session::new(config);

        session.on_connected();
        assert_eq!(session.state(), SessionState::Connecting);
        assert!(!session.should_reconnect());
    }

    // -- Logon validation tests ------------------------------------------------

    #[test]
    fn test_validate_logon_success() {
        let session = Session::new(test_config());
        // Inbound Logon from TARGET (our target) → us (SENDER)
        let result = session.validate_logon(
            Some("FIX.4.4"),
            Some("TARGET"),   // peer's SenderCompID == our TargetCompID
            Some("SENDER"),   // peer's TargetCompID == our SenderCompID
            None,
        );
        assert_eq!(result, Ok(false));
    }

    #[test]
    fn test_validate_logon_bad_begin_string() {
        let session = Session::new(test_config());
        let result = session.validate_logon(
            Some("FIX.4.2"),
            Some("TARGET"),
            Some("SENDER"),
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("BeginString mismatch"));
    }

    #[test]
    fn test_validate_logon_bad_sender_comp_id() {
        let session = Session::new(test_config());
        let result = session.validate_logon(
            Some("FIX.4.4"),
            Some("WRONG"),    // not our expected TargetCompID
            Some("SENDER"),
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("SenderCompID mismatch"));
    }

    #[test]
    fn test_validate_logon_bad_target_comp_id() {
        let session = Session::new(test_config());
        let result = session.validate_logon(
            Some("FIX.4.4"),
            Some("TARGET"),
            Some("WRONG"),    // not our SenderCompID
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TargetCompID mismatch"));
    }

    #[test]
    fn test_validate_logon_missing_begin_string() {
        let session = Session::new(test_config());
        let result = session.validate_logon(None, Some("TARGET"), Some("SENDER"), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Missing BeginString"));
    }

    #[test]
    fn test_validate_logon_skip_comp_id_validation() {
        let mut config = test_config();
        config.validate_comp_ids = false;
        let session = Session::new(config);
        let result = session.validate_logon(
            Some("FIX.4.4"),
            Some("ANYTHING"),
            Some("ANYTHING"),
            None,
        );
        assert_eq!(result, Ok(false));
    }

    #[test]
    fn test_validate_logon_reset_seq_num_flag() {
        let session = Session::new(test_config());
        let result = session.validate_logon(
            Some("FIX.4.4"),
            Some("TARGET"),
            Some("SENDER"),
            Some(b"Y"),
        );
        assert_eq!(result, Ok(true));
    }

    #[test]
    fn test_validate_logon_reset_seq_num_flag_no() {
        let session = Session::new(test_config());
        let result = session.validate_logon(
            Some("FIX.4.4"),
            Some("TARGET"),
            Some("SENDER"),
            Some(b"N"),
        );
        assert_eq!(result, Ok(false));
    }

    #[test]
    fn test_logon_with_reset() {
        let mut session = Session::new(test_config());
        session.next_outbound_seq_num(); // seq now 2
        session.next_outbound_seq_num(); // seq now 3
        assert_eq!(session.current_outbound_seq_num(), 3);

        session.on_logon_with_reset();
        assert_eq!(session.state(), SessionState::Active);
        assert_eq!(session.current_outbound_seq_num(), 1);
        assert_eq!(session.expected_inbound_seq_num(), 1);
    }

    // -- TestRequest / Heartbeat echo tests ------------------------------------

    #[test]
    fn test_heartbeat_received_clears_matching_test_req() {
        let mut session = Session::new(test_config());
        session.on_connected();
        session.on_logon();

        // Simulate sending a TestRequest
        session.test_request_pending = true;
        session.test_request_id = Some(b"TR-1".to_vec());

        // Matching heartbeat clears it
        session.on_heartbeat_received(Some(b"TR-1"));
        assert!(!session.test_request_pending);
    }

    #[test]
    fn test_heartbeat_received_ignores_mismatched_test_req() {
        let mut session = Session::new(test_config());
        session.on_connected();
        session.on_logon();

        session.test_request_pending = true;
        session.test_request_id = Some(b"TR-1".to_vec());

        // Wrong ID — doesn't clear
        session.on_heartbeat_received(Some(b"TR-WRONG"));
        assert!(session.test_request_pending);
    }

    #[test]
    fn test_heartbeat_no_id_clears_pending() {
        let mut session = Session::new(test_config());
        session.on_connected();
        session.on_logon();

        session.test_request_pending = true;
        session.test_request_id = Some(b"TR-1".to_vec());

        // Lenient: no TestReqID still clears
        session.on_heartbeat_received(None);
        assert!(!session.test_request_pending);
    }

    #[test]
    fn test_on_test_request_received_echoes_id() {
        let mut session = Session::new(test_config());
        session.on_connected();
        session.on_logon();

        let action = session.on_test_request_received(b"REQ-42");
        assert_eq!(
            action,
            SessionAction::SendHeartbeat {
                test_req_id: Some(b"REQ-42".to_vec())
            }
        );
    }

    #[test]
    fn test_check_heartbeat_sends_test_request_on_timeout() {
        let mut config = test_config();
        config.heartbeat_interval = Duration::from_millis(10);
        let mut session = Session::new(config);
        session.on_connected();
        session.on_logon();

        // Force receive time beyond heartbeat_interval + 5s grace
        session.last_received_time = Instant::now() - Duration::from_secs(10);
        session.last_sent_time = Instant::now();

        let action = session.check_heartbeat(Instant::now());
        match action {
            SessionAction::SendTestRequest { test_req_id } => {
                assert!(!test_req_id.is_empty());
                assert!(session.test_request_pending);
            }
            other => panic!("Expected SendTestRequest, got {other:?}"),
        }
    }

    #[test]
    fn test_check_heartbeat_disconnects_after_unanswered_test_req() {
        let mut config = test_config();
        config.heartbeat_interval = Duration::from_millis(10);
        let mut session = Session::new(config);
        session.on_connected();
        session.on_logon();

        session.last_received_time = Instant::now() - Duration::from_secs(10);
        session.last_sent_time = Instant::now();
        session.test_request_pending = true;

        let action = session.check_heartbeat(Instant::now());
        assert_eq!(action, SessionAction::Disconnect);
    }
}
