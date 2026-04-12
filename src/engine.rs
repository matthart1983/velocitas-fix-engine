/// FIX protocol engine — drives session protocol over a real transport.
///
/// Ties together Transport, Session, Parser, and Serializer to handle the
/// full FIX session lifecycle: Logon, heartbeat, application messages, Logout.
use std::io;

use crate::parser::{FixParser, ParseError};
use crate::serializer;
use crate::session::{Session, SessionAction, SessionRole, SessionState};
use crate::tags;
use crate::timestamp::{HrTimestamp, TimestampSource};
use crate::transport::Transport;

/// Callback trait for application-level FIX message handling.
pub trait FixApp {
    /// Called when a Logon is acknowledged and the session becomes Active.
    fn on_logon(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        let _ = ctx;
        Ok(())
    }

    /// Called when an application-level message is received (not session-level).
    fn on_message(
        &mut self,
        msg_type: &[u8],
        msg: &crate::message::MessageView<'_>,
        ctx: &mut EngineContext<'_>,
    ) -> io::Result<()>;

    /// Called when a Logout is received.
    fn on_logout(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Context passed to FixApp callbacks, allowing the app to send messages.
pub struct EngineContext<'a> {
    transport: &'a mut dyn Transport,
    session: &'a mut Session,
}

impl<'a> EngineContext<'a> {
    /// Send a raw FIX message (already serialized).
    pub fn send_raw(&mut self, data: &[u8]) -> io::Result<()> {
        self.transport.send(data)?;
        self.session.on_message_sent();
        let _ = self.session.next_outbound_seq_num();
        Ok(())
    }

    /// Get the current outbound sequence number (without incrementing).
    pub fn next_seq_num(&self) -> u64 {
        self.session.current_outbound_seq_num()
    }

    /// Get the session configuration.
    pub fn session(&self) -> &Session {
        self.session
    }

    /// Signal that the engine should send Logout and stop.
    pub fn request_stop(&mut self) {
        self.session.on_logout_sent(); // marks state as LogoutPending
    }
}

/// FIX protocol engine for a single connection.
pub struct FixEngine<T: Transport> {
    transport: T,
    parser: FixParser,
    session: Session,
    recv_buf: Vec<u8>,
    recv_pos: usize,
    send_buf: [u8; 4096],
    should_stop: bool,
}

impl<T: Transport> FixEngine<T> {
    /// Create a new engine for an initiator session.
    pub fn new_initiator(transport: T, session: Session) -> Self {
        Self {
            transport,
            parser: FixParser::new(),
            session,
            recv_buf: vec![0u8; 65536],
            recv_pos: 0,
            send_buf: [0u8; 4096],
            should_stop: false,
        }
    }

    /// Create a new engine for an acceptor session (socket already accepted).
    pub fn new_acceptor(transport: T, session: Session) -> Self {
        Self::new_initiator(transport, session)
    }

    fn now_timestamp() -> [u8; 21] {
        HrTimestamp::now(TimestampSource::System).to_fix_timestamp()
    }

    fn parser_error(error: ParseError) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, format!("{error:?}"))
    }

    // ---- Send helpers -------------------------------------------------------

    fn send_logon(&mut self) -> io::Result<()> {
        Self::send_logon_parts(&mut self.transport, &mut self.session, &mut self.send_buf)
    }

    fn send_logon_parts(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
    ) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = session.next_outbound_seq_num();
        let ver = session.config().fix_version.clone();
        let sender = session.config().sender_comp_id.clone();
        let target = session.config().target_comp_id.clone();
        let hb = session.config().heartbeat_interval.as_secs() as i64;
        let len = serializer::build_logon(
            send_buf,
            ver.as_bytes(),
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            hb,
        );
        transport.send(&send_buf[..len])?;
        session.on_message_sent();
        Ok(())
    }

    fn send_logon_with_reset(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
    ) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = session.next_outbound_seq_num();
        let ver = session.config().fix_version.clone();
        let sender = session.config().sender_comp_id.clone();
        let target = session.config().target_comp_id.clone();
        let hb = session.config().heartbeat_interval.as_secs() as i64;
        let len = serializer::build_logon_with_reset(
            send_buf,
            ver.as_bytes(),
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            hb,
        );
        transport.send(&send_buf[..len])?;
        session.on_message_sent();
        Ok(())
    }

    fn send_heartbeat_with_id(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
        test_req_id: Option<&[u8]>,
    ) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = session.next_outbound_seq_num();
        let ver = session.config().fix_version.clone();
        let sender = session.config().sender_comp_id.clone();
        let target = session.config().target_comp_id.clone();
        let len = serializer::build_heartbeat_with_test_req_id(
            send_buf,
            ver.as_bytes(),
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            test_req_id,
        );
        transport.send(&send_buf[..len])?;
        session.on_message_sent();
        Ok(())
    }

    fn send_test_request(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
        test_req_id: &[u8],
    ) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = session.next_outbound_seq_num();
        let ver = session.config().fix_version.clone();
        let sender = session.config().sender_comp_id.clone();
        let target = session.config().target_comp_id.clone();
        let len = serializer::build_test_request(
            send_buf,
            ver.as_bytes(),
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            test_req_id,
        );
        transport.send(&send_buf[..len])?;
        session.on_message_sent();
        Ok(())
    }

    fn send_resend_request(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
        begin_seq: u64,
        end_seq: u64,
    ) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = session.next_outbound_seq_num();
        let ver = session.config().fix_version.clone();
        let sender = session.config().sender_comp_id.clone();
        let target = session.config().target_comp_id.clone();
        let len = serializer::build_resend_request(
            send_buf,
            ver.as_bytes(),
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            begin_seq,
            end_seq,
        );
        transport.send(&send_buf[..len])?;
        session.on_message_sent();
        Ok(())
    }

    fn send_reject(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
        ref_seq_num: u64,
        reason: u32,
        text: Option<&str>,
    ) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = session.next_outbound_seq_num();
        let ver = session.config().fix_version.clone();
        let sender = session.config().sender_comp_id.clone();
        let target = session.config().target_comp_id.clone();
        let len = serializer::build_reject(
            send_buf,
            ver.as_bytes(),
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            ref_seq_num,
            reason,
            text.map(|s| s.as_bytes()),
        );
        transport.send(&send_buf[..len])?;
        session.on_message_sent();
        Ok(())
    }

    fn send_logout_with_text(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
        text: Option<&str>,
    ) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = session.next_outbound_seq_num();
        let ver = session.config().fix_version.clone();
        let sender = session.config().sender_comp_id.clone();
        let target = session.config().target_comp_id.clone();
        let len = serializer::build_logout_with_text(
            send_buf,
            ver.as_bytes(),
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            text.map(|s| s.as_bytes()),
        );
        transport.send(&send_buf[..len])?;
        session.on_logout_sent();
        session.on_message_sent();
        Ok(())
    }

    /// Send a SequenceReset-GapFill to skip a range of sequence numbers.
    fn send_gap_fill(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
        gap_fill_seq: u64,
        new_seq_no: u64,
    ) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let ver = session.config().fix_version.clone();
        let sender = session.config().sender_comp_id.clone();
        let target = session.config().target_comp_id.clone();
        let len = serializer::build_sequence_reset_gap_fill(
            send_buf,
            ver.as_bytes(),
            sender.as_bytes(),
            target.as_bytes(),
            gap_fill_seq,
            &ts,
            new_seq_no,
        );
        transport.send(&send_buf[..len])?;
        session.on_message_sent();
        Ok(())
    }

    /// Execute a session action returned by the session state machine.
    fn execute_action(
        transport: &mut T,
        session: &mut Session,
        send_buf: &mut [u8; 4096],
        action: SessionAction,
    ) -> io::Result<bool> {
        match action {
            SessionAction::SendHeartbeat { test_req_id } => {
                Self::send_heartbeat_with_id(
                    transport,
                    session,
                    send_buf,
                    test_req_id.as_deref(),
                )?;
                Ok(false)
            }
            SessionAction::SendTestRequest { test_req_id } => {
                Self::send_test_request(transport, session, send_buf, &test_req_id)?;
                Ok(false)
            }
            SessionAction::SendResendRequest { begin_seq, end_seq } => {
                Self::send_resend_request(transport, session, send_buf, begin_seq, end_seq)?;
                Ok(false)
            }
            SessionAction::SendReject {
                ref_seq_num,
                reason,
                text,
            } => {
                Self::send_reject(
                    transport,
                    session,
                    send_buf,
                    ref_seq_num,
                    reason,
                    text.as_deref(),
                )?;
                Ok(false)
            }
            SessionAction::SendLogout { text } => {
                Self::send_logout_with_text(transport, session, send_buf, text.as_deref())?;
                Ok(false)
            }
            SessionAction::Disconnect => Ok(true),
            SessionAction::None => Ok(false),
        }
    }

    // ---- Public API ---------------------------------------------------------

    /// Run the initiator engine: connect, logon, process messages, and stop.
    pub fn run_initiator(&mut self, app: &mut dyn FixApp) -> io::Result<()> {
        self.session.on_connected();
        self.send_logon()?;
        self.run_loop(app)?;
        Ok(())
    }

    /// Run the acceptor engine.
    pub fn run_acceptor(&mut self, app: &mut dyn FixApp) -> io::Result<()> {
        self.run_loop(app)
    }

    /// Handle an inbound Logon: transition session to Active, send Logon response.
    pub fn handle_inbound_logon(&mut self) -> io::Result<()> {
        self.session.on_connected();
        self.session.on_logon();
        self.send_logon()?;
        Ok(())
    }

    /// Send a Logout and stop the engine.
    pub fn initiate_logout(&mut self) -> io::Result<()> {
        Self::send_logout_with_text(
            &mut self.transport,
            &mut self.session,
            &mut self.send_buf,
            None,
        )?;
        Ok(())
    }

    // ---- Main loop ----------------------------------------------------------

    fn run_loop(&mut self, app: &mut dyn FixApp) -> io::Result<()> {
        let mut scratch = vec![0u8; 8192];

        loop {
            if self.should_stop {
                break;
            }

            let now = std::time::Instant::now();

            // Phase 5: Check logout timeout
            if self.session.check_logout_timeout(now) {
                // Peer didn't respond to our Logout — force disconnect
                self.session.on_disconnected();
                self.should_stop = true;
                break;
            }

            // Drive heartbeat / test request timer
            let action = self.session.check_heartbeat(now);
            let should_disconnect = Self::execute_action(
                &mut self.transport,
                &mut self.session,
                &mut self.send_buf,
                action,
            )?;
            if should_disconnect {
                self.should_stop = true;
                break;
            }

            // Receive data
            let n = match self.transport.recv(&mut scratch) {
                Ok(0) => continue,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // Non-blocking: no data available, yield
                    continue;
                }
                Err(e)
                    if e.kind() == io::ErrorKind::ConnectionReset
                        || e.kind() == io::ErrorKind::BrokenPipe
                        || e.kind() == io::ErrorKind::UnexpectedEof =>
                {
                    self.session.on_disconnected();
                    self.should_stop = true;
                    break;
                }
                Err(e) => return Err(e),
            };

            // Append to recv buffer
            let end = self.recv_pos + n;
            if end > self.recv_buf.len() {
                self.recv_buf.resize(end * 2, 0);
            }
            self.recv_buf[self.recv_pos..end].copy_from_slice(&scratch[..n]);
            self.recv_pos = end;

            // Process complete messages
            loop {
                let data = &self.recv_buf[..self.recv_pos];
                let (view, consumed) = match self
                    .parser
                    .try_parse_prefix(data)
                    .map_err(Self::parser_error)?
                {
                    Some(parsed) => parsed,
                    None => break,
                };

                // Update receive timestamp
                self.session.on_message_received();

                // Phase 6: Rate limiting
                if !self.session.check_rate_limit() {
                    // Rate limit exceeded — send Logout
                    Self::send_logout_with_text(
                        &mut self.transport,
                        &mut self.session,
                        &mut self.send_buf,
                        Some("Rate limit exceeded"),
                    )?;
                    self.session.on_disconnected();
                    self.should_stop = true;
                    self.recv_buf.copy_within(consumed..self.recv_pos, 0);
                    self.recv_pos -= consumed;
                    break;
                }

                // Dispatch by MsgType
                let msg_type = match view.msg_type() {
                    Some(t) => t,
                    None => {
                        // No MsgType — consume bytes to avoid infinite loop
                        self.recv_buf.copy_within(consumed..self.recv_pos, 0);
                        self.recv_pos -= consumed;
                        continue;
                    }
                };

                // Check PossDupFlag for duplicate handling
                let is_poss_dup = view
                    .get_field(tags::POSS_DUP_FLAG)
                    .map(|v| v == b"Y")
                    .unwrap_or(false);

                // Validate sequence number (skip for session-level during resend)
                let mut seq_gap = false;
                if let Some(seq) = view.msg_seq_num() {
                    if !is_poss_dup {
                        if let Err((gap_start, gap_end)) =
                            self.session.validate_inbound_seq(seq)
                        {
                            // Gap detected — send ResendRequest
                            Self::send_resend_request(
                                &mut self.transport,
                                &mut self.session,
                                &mut self.send_buf,
                                gap_start,
                                gap_end,
                            )?;
                            seq_gap = true;
                        }
                    }
                    // PossDup messages: update receive time but don't advance seq
                }

                match msg_type {
                    b"A" => {
                        // Logon — validate before activating
                        let is_acceptor_awaiting =
                            self.session.config().role == SessionRole::Acceptor
                                && matches!(
                                    self.session.state(),
                                    SessionState::Disconnected | SessionState::Connecting
                                );
                        let is_initiator_awaiting =
                            self.session.state() == SessionState::LogonSent;

                        if is_acceptor_awaiting || is_initiator_awaiting {
                            let reset_flag = view.get_field(tags::RESET_SEQ_NUM_FLAG);
                            match self.session.validate_logon(
                                view.begin_string(),
                                view.sender_comp_id(),
                                view.target_comp_id(),
                                reset_flag,
                            ) {
                                Ok(reset_sequences) => {
                                    if is_acceptor_awaiting
                                        && self.session.state() == SessionState::Disconnected
                                    {
                                        self.session.on_connected();
                                    }
                                    if reset_sequences {
                                        self.session.on_logon_with_reset();
                                        if is_acceptor_awaiting {
                                            Self::send_logon_with_reset(
                                                &mut self.transport,
                                                &mut self.session,
                                                &mut self.send_buf,
                                            )?;
                                        }
                                    } else {
                                        self.session.on_logon();
                                        if is_acceptor_awaiting {
                                            Self::send_logon_parts(
                                                &mut self.transport,
                                                &mut self.session,
                                                &mut self.send_buf,
                                            )?;
                                        }
                                    }
                                    let mut ctx = EngineContext {
                                        transport: &mut self.transport,
                                        session: &mut self.session,
                                    };
                                    app.on_logon(&mut ctx)?;
                                }
                                Err(reason) => {
                                    Self::send_logout_with_text(
                                        &mut self.transport,
                                        &mut self.session,
                                        &mut self.send_buf,
                                        Some(&reason),
                                    )?;
                                    self.session.on_disconnected();
                                    self.should_stop = true;
                                }
                            }
                        }
                    }
                    b"0" => {
                        // Heartbeat — check for TestReqID echo
                        let test_req_id = view.get_field(tags::TEST_REQ_ID);
                        self.session.on_heartbeat_received(test_req_id);
                    }
                    b"1" => {
                        // TestRequest — respond with Heartbeat echoing TestReqID
                        let test_req_id = view.get_field(tags::TEST_REQ_ID).unwrap_or(b"");
                        let action = self.session.on_test_request_received(test_req_id);
                        Self::execute_action(
                            &mut self.transport,
                            &mut self.session,
                            &mut self.send_buf,
                            action,
                        )?;
                    }
                    b"2" => {
                        // ResendRequest — peer wants us to replay messages.
                        // Without journal replay, respond with a GapFill to
                        // skip the requested range (honest: we don't have them).
                        let begin = view.get_field_u64(tags::BEGIN_SEQ_NO).unwrap_or(1);
                        let end = view.get_field_u64(tags::END_SEQ_NO).unwrap_or(0);
                        let fill_to = if end == 0 {
                            self.session.current_outbound_seq_num()
                        } else {
                            end + 1
                        };
                        Self::send_gap_fill(
                            &mut self.transport,
                            &mut self.session,
                            &mut self.send_buf,
                            begin,
                            fill_to,
                        )?;
                    }
                    b"3" => {
                        // Reject — log but continue session
                    }
                    b"4" => {
                        // SequenceReset
                        if let Some(new_seq) = view.get_field_u64(tags::NEW_SEQ_NO) {
                            let is_gap_fill = view
                                .get_field(tags::GAP_FILL_FLAG)
                                .map(|v| v == b"Y")
                                .unwrap_or(false);
                            if is_gap_fill {
                                // GapFill — advance expected inbound seq
                                self.session.on_gap_filled(new_seq);
                            } else {
                                // Hard reset — set inbound seq directly
                                self.session.on_gap_filled(new_seq);
                            }
                        }
                    }
                    b"5" => {
                        // Logout
                        app.on_logout()?;

                        if self.session.state() == SessionState::LogoutPending {
                            // We initiated logout, peer responded — clean close
                            self.session.on_logout_complete();
                        } else {
                            // Peer initiated logout — respond with Logout
                            Self::send_logout_with_text(
                                &mut self.transport,
                                &mut self.session,
                                &mut self.send_buf,
                                None,
                            )?;
                        }
                        self.session.on_disconnected();
                        self.should_stop = true;
                    }
                    _ => {
                        // Application message
                        if self.session.state() != SessionState::Active
                            && self.session.state() != SessionState::Resending
                        {
                            // Not in a state to accept app messages — ignore
                        } else if seq_gap && !is_poss_dup {
                            // Phase 4: Gap detected on this message —
                            // don't dispatch to app until gap is filled.
                            // The message will be replayed via ResendRequest.
                        } else if is_poss_dup {
                            // PossDup: already processed — skip app dispatch
                        } else {
                            let mut ctx = EngineContext {
                                transport: &mut self.transport,
                                session: &mut self.session,
                            };
                            app.on_message(msg_type, &view, &mut ctx)?;
                        }
                    }
                }

                self.recv_buf.copy_within(consumed..self.recv_pos, 0);
                self.recv_pos -= consumed;
            }

            // Check if session ended or app requested stop
            match self.session.state() {
                SessionState::Disconnected => {
                    self.should_stop = true;
                }
                SessionState::LogoutPending => {
                    // Waiting for peer Logout response — don't send another,
                    // just wait (timeout handled at top of loop)
                }
                SessionState::LogoutSent => {
                    // Logout exchange complete — disconnect
                    self.session.on_disconnected();
                    self.should_stop = true;
                }
                _ => {}
            }
        }

        let _ = self.transport.close();
        Ok(())
    }
}
