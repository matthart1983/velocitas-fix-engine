/// High-level FIX acceptor server.
///
/// Listens on a TCP port, auto-accepts connections, validates CompIDs,
/// reads inbound Logon, and spawns a per-connection engine thread.
///
/// # Example
///
/// ```ignore
/// let config = FixServerConfig {
///     bind_address: "0.0.0.0".into(),
///     port: 9878,
///     sender_comp_id: "EXCHANGE".into(),
///     ..Default::default()
/// };
/// let server = FixServer::new(config);
/// server.start(|| Box::new(MyApp::new()));
/// ```

use std::io::{self, Read};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::acceptor::{Acceptor, AcceptorConfig};
use crate::engine::{FixApp, FixEngine};
use crate::parser::FixParser;
use crate::session::{Session, SessionConfig, SessionRole, SequenceResetPolicy};
use crate::transport::TransportConfig;
use crate::transport_tcp::StdTcpTransport;

/// Server configuration.
#[derive(Debug, Clone)]
pub struct FixServerConfig {
    pub bind_address: String,
    pub port: u16,
    pub sender_comp_id: String,
    pub fix_version: String,
    pub heartbeat_interval: Duration,
    pub allowed_comp_ids: Vec<String>,
    pub max_connections: usize,
    pub connection_pool_size: usize,
}

impl Default for FixServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".into(),
            port: 9878,
            sender_comp_id: "VELOCITAS".into(),
            fix_version: "FIX.4.4".into(),
            heartbeat_interval: Duration::from_secs(30),
            allowed_comp_ids: Vec::new(),
            max_connections: 256,
            connection_pool_size: 64,
        }
    }
}

/// High-level FIX acceptor server.
pub struct FixServer {
    config: FixServerConfig,
}

impl FixServer {
    pub fn new(config: FixServerConfig) -> Self {
        Self { config }
    }

    /// Start accepting connections. Blocks the calling thread.
    ///
    /// `app_factory` is called once per accepted connection to create a fresh
    /// `FixApp` for that session. The factory must be `Send` and `Sync`.
    pub fn start<F>(&self, app_factory: F) -> io::Result<()>
    where
        F: Fn() -> Box<dyn FixApp + Send> + Send + Sync + 'static,
    {
        let bind = format!("{}:{}", self.config.bind_address, self.config.port);
        let listener = TcpListener::bind(&bind)?;

        eprintln!("⚡ FIX Server listening on {bind}");
        eprintln!("  SenderCompID: {}", self.config.sender_comp_id);
        if !self.config.allowed_comp_ids.is_empty() {
            eprintln!("  Allowed CompIDs: {:?}", self.config.allowed_comp_ids);
        }

        let acceptor = Arc::new(Mutex::new(Acceptor::new(AcceptorConfig {
            bind_address: self.config.bind_address.clone(),
            port: self.config.port,
            max_connections: self.config.max_connections,
            connection_pool_size: self.config.connection_pool_size,
            allowed_comp_ids: self.config.allowed_comp_ids.clone(),
            ..AcceptorConfig::default()
        })));

        let app_factory = Arc::new(app_factory);
        let config = self.config.clone();

        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("  Accept error: {e}");
                    continue;
                }
            };

            let remote = stream.peer_addr()
                .map(|a| a.to_string())
                .unwrap_or_else(|_| "unknown".into());

            let acceptor = Arc::clone(&acceptor);
            let app_factory = Arc::clone(&app_factory);
            let config = config.clone();

            thread::spawn(move || {
                if let Err(e) = handle_connection(stream, &remote, &config, &acceptor, &*app_factory) {
                    eprintln!("  [{remote}] Error: {e}");
                }
                // Release connection from pool
                // (best effort — connection may not have been registered)
            });
        }

        Ok(())
    }
}

fn handle_connection(
    mut stream: std::net::TcpStream,
    remote: &str,
    config: &FixServerConfig,
    acceptor: &Arc<Mutex<Acceptor>>,
    app_factory: &dyn Fn() -> Box<dyn FixApp + Send>,
) -> io::Result<()> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;

    // Read the first FIX message (must be Logon)
    let parser = FixParser::new();
    let mut buf = vec![0u8; 4096];
    let mut pos = 0;

    let logon_comp_id = loop {
        let n = stream.read(&mut buf[pos..])?;
        if n == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "connection closed before Logon"));
        }
        pos += n;

        if let Some(boundary) = parser.find_message_boundary(&buf[..pos]) {
            let (view, _) = parser.parse(&buf[..boundary])
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("parse error: {:?}", e)))?;

            let msg_type = view.msg_type()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no MsgType"))?;

            if msg_type != b"A" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("expected Logon (A), got MsgType={}", String::from_utf8_lossy(msg_type)),
                ));
            }

            let comp_id = view.sender_comp_id()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no SenderCompID in Logon"))?
                .to_string();

            break comp_id;
        }
    };

    // Register with Acceptor (CompID whitelisting + pool management)
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let conn_id = {
        let mut acc = acceptor.lock().unwrap();
        acc.accept_connection(remote, &logon_comp_id, now_ms)
            .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, format!("{:?}", e)))?
    };

    eprintln!("  [{remote}] Accepted: CompID={logon_comp_id} conn_id={conn_id}");

    // Create session and engine
    let session_config = SessionConfig {
        session_id: format!("ACC-{conn_id}"),
        fix_version: config.fix_version.clone(),
        sender_comp_id: config.sender_comp_id.clone(),
        target_comp_id: logon_comp_id.clone(),
        role: SessionRole::Acceptor,
        heartbeat_interval: config.heartbeat_interval,
        reconnect_interval: Duration::from_secs(0),
        max_reconnect_attempts: 0,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: true,
        max_msg_rate: 50_000,
    };

    let transport = StdTcpTransport::from_stream(stream, TransportConfig::default())?;
    let session = Session::new(session_config);
    let mut engine = FixEngine::new_acceptor(transport, session);

    // Send Logon response
    engine.handle_inbound_logon()?;

    // Run engine with user-provided app
    let mut app = app_factory();
    let result = engine.run_acceptor(&mut *app);

    // Release from pool
    {
        let mut acc = acceptor.lock().unwrap();
        let _ = acc.release_connection(conn_id);
    }

    eprintln!("  [{remote}] Disconnected: CompID={logon_comp_id}");
    result
}
