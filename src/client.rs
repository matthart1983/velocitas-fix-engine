/// High-level FIX initiator client.
///
/// Connects to a remote FIX acceptor, performs Logon, and runs the session.
///
/// # Example
///
/// ```ignore
/// let config = FixClientConfig {
///     remote_host: "10.0.1.50".into(),
///     remote_port: 9878,
///     sender_comp_id: "BANK_OMS".into(),
///     target_comp_id: "NYSE".into(),
///     ..Default::default()
/// };
/// let client = FixClient::new(config);
/// client.connect_and_run(&mut MyApp::new()).unwrap();
/// ```

use std::io;
use std::time::Duration;

use crate::engine::{FixApp, FixEngine};
use crate::session::{Session, SessionConfig, SessionRole, SequenceResetPolicy};
use crate::transport::Transport;
use crate::transport::TransportConfig;
use crate::transport_tcp::StdTcpTransport;

/// Client configuration.
#[derive(Debug, Clone)]
pub struct FixClientConfig {
    pub remote_host: String,
    pub remote_port: u16,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub fix_version: String,
    pub heartbeat_interval: Duration,
    pub reconnect_attempts: u32,
}

impl Default for FixClientConfig {
    fn default() -> Self {
        Self {
            remote_host: "127.0.0.1".into(),
            remote_port: 9878,
            sender_comp_id: String::new(),
            target_comp_id: String::new(),
            fix_version: "FIX.4.4".into(),
            heartbeat_interval: Duration::from_secs(30),
            reconnect_attempts: 3,
        }
    }
}

/// High-level FIX initiator client.
pub struct FixClient {
    config: FixClientConfig,
}

impl FixClient {
    pub fn new(config: FixClientConfig) -> Self {
        Self { config }
    }

    /// Connect to the remote acceptor and run the FIX session.
    /// Blocks until the session ends (Logout or disconnect).
    pub fn connect_and_run(&self, app: &mut dyn FixApp) -> io::Result<()> {
        let mut transport = StdTcpTransport::new(TransportConfig::default());
        transport.connect(&self.config.remote_host, self.config.remote_port)?;

        let session_config = SessionConfig {
            session_id: format!("{}-{}", self.config.sender_comp_id, self.config.target_comp_id),
            fix_version: self.config.fix_version.clone(),
            sender_comp_id: self.config.sender_comp_id.clone(),
            target_comp_id: self.config.target_comp_id.clone(),
            role: SessionRole::Initiator,
            heartbeat_interval: self.config.heartbeat_interval,
            reconnect_interval: Duration::from_secs(1),
            max_reconnect_attempts: self.config.reconnect_attempts,
            sequence_reset_policy: SequenceResetPolicy::Daily,
            validate_comp_ids: true,
            max_msg_rate: 50_000,
        };

        let session = Session::new(session_config);
        let mut engine = FixEngine::new_initiator(transport, session);
        engine.run_initiator(app)
    }
}
