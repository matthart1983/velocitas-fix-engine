/// FIX protocol engine — drives session protocol over a real transport.
///
/// Ties together Transport, Session, Parser, and Serializer to handle the
/// full FIX session lifecycle: Logon, heartbeat, application messages, Logout.

use std::io;

use crate::parser::FixParser;
use crate::serializer;
use crate::session::{Session, SessionAction, SessionState};
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
    fn on_message(&mut self, msg_type: &[u8], msg: &crate::message::MessageView<'_>, ctx: &mut EngineContext<'_>) -> io::Result<()>;

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
        self.session.on_logout_sent(); // marks state as LogoutSent
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

    /// Send a Logon message.
    fn send_logon(&mut self) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = self.session.next_outbound_seq_num();
        let ver = self.session.config().fix_version.clone();
        let sender = self.session.config().sender_comp_id.clone();
        let target = self.session.config().target_comp_id.clone();
        let hb = self.session.config().heartbeat_interval.as_secs() as i64;
        let len = serializer::build_logon(
            &mut self.send_buf, ver.as_bytes(), sender.as_bytes(), target.as_bytes(),
            seq, &ts, hb,
        );
        self.transport.send(&self.send_buf[..len])?;
        self.session.on_message_sent();
        Ok(())
    }

    /// Send a Heartbeat message.
    fn send_heartbeat(&mut self) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = self.session.next_outbound_seq_num();
        let ver = self.session.config().fix_version.clone();
        let sender = self.session.config().sender_comp_id.clone();
        let target = self.session.config().target_comp_id.clone();
        let len = serializer::build_heartbeat(
            &mut self.send_buf, ver.as_bytes(), sender.as_bytes(), target.as_bytes(),
            seq, &ts,
        );
        self.transport.send(&self.send_buf[..len])?;
        self.session.on_message_sent();
        Ok(())
    }

    /// Send a Logout message.
    fn send_logout(&mut self) -> io::Result<()> {
        let ts = Self::now_timestamp();
        let seq = self.session.next_outbound_seq_num();
        let ver = self.session.config().fix_version.clone();
        let sender = self.session.config().sender_comp_id.clone();
        let target = self.session.config().target_comp_id.clone();
        let len = serializer::build_logout(
            &mut self.send_buf, ver.as_bytes(), sender.as_bytes(), target.as_bytes(),
            seq, &ts,
        );
        self.transport.send(&self.send_buf[..len])?;
        self.session.on_logout_sent();
        self.session.on_message_sent();
        Ok(())
    }

    /// Run the initiator engine: connect, logon, process messages, and stop.
    pub fn run_initiator(&mut self, app: &mut dyn FixApp) -> io::Result<()> {
        // Connect
        let addr = self.session.config().target_comp_id.clone();
        self.session.on_connected();
        self.send_logon()?;

        self.run_loop(app)?;
        let _ = addr;
        Ok(())
    }

    /// Run the acceptor engine: process an already-connected socket.
    /// The first Logon message should have already been read and the session
    /// transitioned to Connecting. Call `handle_inbound_logon` first if needed.
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

    fn run_loop(&mut self, app: &mut dyn FixApp) -> io::Result<()> {
        let mut scratch = vec![0u8; 8192];

        loop {
            if self.should_stop {
                break;
            }

            // Drive heartbeat timer
            let action = self.session.check_heartbeat(std::time::Instant::now());
            match action {
                SessionAction::Send(_) => {
                    if self.session.state() == SessionState::Active {
                        self.send_heartbeat()?;
                    }
                }
                SessionAction::Disconnect => {
                    self.should_stop = true;
                    break;
                }
                SessionAction::None => {}
            }

            // Receive data
            let n = match self.transport.recv(&mut scratch) {
                Ok(0) => continue,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset
                    || e.kind() == io::ErrorKind::BrokenPipe
                    || e.kind() == io::ErrorKind::UnexpectedEof => {
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
                let boundary = match self.parser.find_message_boundary(data) {
                    Some(b) => b,
                    None => break,
                };

                // Copy message out so we can compact the buffer before parsing
                let mut msg_copy = vec![0u8; boundary];
                msg_copy.copy_from_slice(&self.recv_buf[..boundary]);

                // Compact buffer
                self.recv_buf.copy_within(boundary..self.recv_pos, 0);
                self.recv_pos -= boundary;

                let (view, _) = match self.parser.parse(&msg_copy) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                self.session.on_message_received();

                // Validate sequence
                if let Some(seq) = view.msg_seq_num() {
                    let _ = self.session.validate_inbound_seq(seq);
                }

                // Dispatch by MsgType
                let msg_type = match view.msg_type() {
                    Some(t) => t,
                    None => continue,
                };

                match msg_type {
                    b"A" => {
                        // Logon
                        if self.session.state() == SessionState::LogonSent {
                            self.session.on_logon();
                            let mut ctx = EngineContext {
                                transport: &mut self.transport,
                                session: &mut self.session,
                            };
                            app.on_logon(&mut ctx)?;
                        }
                    }
                    b"0" => {
                        // Heartbeat — no action needed
                    }
                    b"1" => {
                        // TestRequest — respond with Heartbeat
                        self.send_heartbeat()?;
                    }
                    b"5" => {
                        // Logout
                        app.on_logout()?;
                        if self.session.state() != SessionState::LogoutSent {
                            self.send_logout()?;
                        }
                        self.session.on_disconnected();
                        self.should_stop = true;
                    }
                    _ => {
                        // Application message
                        let mut ctx = EngineContext {
                            transport: &mut self.transport,
                            session: &mut self.session,
                        };
                        app.on_message(msg_type, &view, &mut ctx)?;
                    }
                }
            }

            // Check if session ended or app requested stop
            match self.session.state() {
                SessionState::Disconnected => {
                    self.should_stop = true;
                }
                SessionState::LogoutSent => {
                    // App called request_stop() — send Logout wire message and close
                    let ts = Self::now_timestamp();
                    let seq = self.session.next_outbound_seq_num();
                    let ver = self.session.config().fix_version.clone();
                    let sender = self.session.config().sender_comp_id.clone();
                    let target = self.session.config().target_comp_id.clone();
                    let len = serializer::build_logout(
                        &mut self.send_buf, ver.as_bytes(), sender.as_bytes(), target.as_bytes(),
                        seq, &ts,
                    );
                    let _ = self.transport.send(&self.send_buf[..len]);
                    self.session.on_disconnected();
                    self.should_stop = true;
                }
                _ => {}
            }
        }

        let _ = self.transport.close();
        Ok(())
    }

    /// Send a Logout and stop the engine.
    pub fn initiate_logout(&mut self) -> io::Result<()> {
        self.send_logout()?;
        Ok(())
    }
}
