/// FIX session acceptor with connection pooling.
///
/// Manages inbound connections, enforces CompID whitelisting, and maintains
/// a pre-allocated pool of reusable connection slots for low-latency accept.
use crate::session::{SequenceResetPolicy, Session, SessionConfig, SessionRole};
use std::fmt;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AcceptorConfig {
    pub bind_address: String,
    pub port: u16,
    pub max_connections: usize,
    pub connection_pool_size: usize,
    pub accept_timeout_ms: u64,
    pub allowed_comp_ids: Vec<String>,
    pub require_auth: bool,
    pub default_heartbeat_interval: Duration,
    pub default_fix_version: String,
}

impl Default for AcceptorConfig {
    fn default() -> Self {
        AcceptorConfig {
            bind_address: "0.0.0.0".to_string(),
            port: 9878,
            max_connections: 1024,
            connection_pool_size: 256,
            accept_timeout_ms: 5000,
            allowed_comp_ids: Vec::new(),
            require_auth: false,
            default_heartbeat_interval: Duration::from_secs(30),
            default_fix_version: "FIX.4.4".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Available,
    InUse,
    Draining,
    Closed,
}

pub struct PooledConnection {
    pub id: u64,
    pub state: ConnectionState,
    pub session: Option<Session>,
    pub remote_comp_id: String,
    pub remote_address: String,
    pub connected_at_ms: u64,
    pub last_active_ms: u64,
    pub bytes_received: u64,
    pub bytes_sent: u64,
}

impl fmt::Debug for PooledConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PooledConnection")
            .field("id", &self.id)
            .field("state", &self.state)
            .field(
                "session",
                &self
                    .session
                    .as_ref()
                    .map(|s| s.config().session_id.as_str()),
            )
            .field("remote_comp_id", &self.remote_comp_id)
            .field("remote_address", &self.remote_address)
            .field("connected_at_ms", &self.connected_at_ms)
            .field("last_active_ms", &self.last_active_ms)
            .field("bytes_received", &self.bytes_received)
            .field("bytes_sent", &self.bytes_sent)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptorError {
    PoolExhausted,
    CompIdNotAllowed(String),
    AlreadyConnected(String),
    ConnectionNotFound(u64),
    AuthRequired,
}

#[derive(Debug, Clone, Default)]
pub struct AcceptorStats {
    pub active_connections: usize,
    pub pool_available: usize,
    pub total_accepted: u64,
    pub total_rejected: u64,
    pub total_bytes_received: u64,
    pub total_bytes_sent: u64,
}

pub struct Acceptor {
    config: AcceptorConfig,
    connections: Vec<PooledConnection>,
    next_conn_id: u64,
    total_accepted: u64,
    total_rejected: u64,
}

impl Acceptor {
    /// Create a new acceptor with a pre-allocated connection pool.
    pub fn new(config: AcceptorConfig) -> Self {
        let pool_size = config.connection_pool_size;
        let mut connections = Vec::with_capacity(pool_size);
        for i in 0..pool_size {
            connections.push(PooledConnection {
                id: i as u64,
                state: ConnectionState::Available,
                session: None,
                remote_comp_id: String::new(),
                remote_address: String::new(),
                connected_at_ms: 0,
                last_active_ms: 0,
                bytes_received: 0,
                bytes_sent: 0,
            });
        }
        Acceptor {
            config,
            connections,
            next_conn_id: pool_size as u64,
            total_accepted: 0,
            total_rejected: 0,
        }
    }

    /// Accept an inbound connection. Validates the CompID whitelist, finds an
    /// available pool slot, creates a Session with Acceptor role, and returns
    /// the connection id.
    pub fn accept_connection(
        &mut self,
        remote_addr: &str,
        comp_id: &str,
        current_time_ms: u64,
    ) -> Result<u64, AcceptorError> {
        if self.config.require_auth {
            // Auth is required but not performed at this layer — reject.
            self.total_rejected += 1;
            return Err(AcceptorError::AuthRequired);
        }

        if !self.is_comp_id_allowed(comp_id) {
            self.total_rejected += 1;
            return Err(AcceptorError::CompIdNotAllowed(comp_id.to_string()));
        }

        // Check if this comp_id is already connected.
        if self.find_by_comp_id(comp_id).is_some() {
            self.total_rejected += 1;
            return Err(AcceptorError::AlreadyConnected(comp_id.to_string()));
        }

        // Find an available slot.
        let slot = self
            .connections
            .iter()
            .position(|c| c.state == ConnectionState::Available);

        let slot_idx = match slot {
            Some(idx) => idx,
            None => {
                self.total_rejected += 1;
                return Err(AcceptorError::PoolExhausted);
            }
        };

        let conn_id = self.next_conn_id;
        self.next_conn_id += 1;

        let session_config = SessionConfig {
            session_id: format!("ACC-{}", conn_id),
            fix_version: self.config.default_fix_version.clone(),
            sender_comp_id: "VELOCITAS".to_string(),
            target_comp_id: comp_id.to_string(),
            role: SessionRole::Acceptor,
            heartbeat_interval: self.config.default_heartbeat_interval,
            reconnect_interval: Duration::from_secs(0),
            max_reconnect_attempts: 0,
            sequence_reset_policy: SequenceResetPolicy::Daily,
            validate_comp_ids: true,
            max_msg_rate: 50_000,
        };

        let conn = &mut self.connections[slot_idx];
        conn.id = conn_id;
        conn.state = ConnectionState::InUse;
        conn.session = Some(Session::new(session_config));
        conn.remote_comp_id = comp_id.to_string();
        conn.remote_address = remote_addr.to_string();
        conn.connected_at_ms = current_time_ms;
        conn.last_active_ms = current_time_ms;
        conn.bytes_received = 0;
        conn.bytes_sent = 0;

        self.total_accepted += 1;
        Ok(conn_id)
    }

    /// Release a connection back to the pool for reuse.
    pub fn release_connection(&mut self, conn_id: u64) -> Result<(), AcceptorError> {
        let conn = self
            .connections
            .iter_mut()
            .find(|c| c.id == conn_id && c.state != ConnectionState::Available);

        match conn {
            Some(c) => {
                c.state = ConnectionState::Available;
                c.session = None;
                c.remote_comp_id.clear();
                c.remote_address.clear();
                c.connected_at_ms = 0;
                c.last_active_ms = 0;
                c.bytes_received = 0;
                c.bytes_sent = 0;
                Ok(())
            }
            None => Err(AcceptorError::ConnectionNotFound(conn_id)),
        }
    }

    /// Get an immutable reference to a pooled connection by id.
    pub fn get_connection(&self, conn_id: u64) -> Option<&PooledConnection> {
        self.connections.iter().find(|c| c.id == conn_id)
    }

    /// Get a mutable reference to a pooled connection by id.
    pub fn get_connection_mut(&mut self, conn_id: u64) -> Option<&mut PooledConnection> {
        self.connections.iter_mut().find(|c| c.id == conn_id)
    }

    /// Get an immutable reference to the session for a connection.
    pub fn get_session(&self, conn_id: u64) -> Option<&Session> {
        self.connections
            .iter()
            .find(|c| c.id == conn_id)
            .and_then(|c| c.session.as_ref())
    }

    /// Get a mutable reference to the session for a connection.
    pub fn get_session_mut(&mut self, conn_id: u64) -> Option<&mut Session> {
        self.connections
            .iter_mut()
            .find(|c| c.id == conn_id)
            .and_then(|c| c.session.as_mut())
    }

    /// Set a connection to Draining state (no new messages, finish in-flight).
    pub fn drain_connection(&mut self, conn_id: u64) -> Result<(), AcceptorError> {
        match self.connections.iter_mut().find(|c| c.id == conn_id) {
            Some(c) if c.state == ConnectionState::InUse => {
                c.state = ConnectionState::Draining;
                Ok(())
            }
            Some(_) => Err(AcceptorError::ConnectionNotFound(conn_id)),
            None => Err(AcceptorError::ConnectionNotFound(conn_id)),
        }
    }

    /// Count of currently active (InUse or Draining) connections.
    pub fn active_count(&self) -> usize {
        self.connections
            .iter()
            .filter(|c| c.state == ConnectionState::InUse || c.state == ConnectionState::Draining)
            .count()
    }

    /// Check if a CompID is allowed. An empty whitelist allows all.
    pub fn is_comp_id_allowed(&self, comp_id: &str) -> bool {
        if self.config.allowed_comp_ids.is_empty() {
            return true;
        }
        self.config.allowed_comp_ids.iter().any(|id| id == comp_id)
    }

    /// Evict idle InUse connections whose last activity exceeds `max_idle_ms`.
    /// Returns the number of connections evicted.
    pub fn evict_idle(&mut self, max_idle_ms: u64, current_time_ms: u64) -> usize {
        let mut evicted = 0;
        for conn in &mut self.connections {
            if conn.state == ConnectionState::InUse
                && current_time_ms.saturating_sub(conn.last_active_ms) > max_idle_ms
            {
                conn.state = ConnectionState::Available;
                conn.session = None;
                conn.remote_comp_id.clear();
                conn.remote_address.clear();
                conn.connected_at_ms = 0;
                conn.last_active_ms = 0;
                conn.bytes_received = 0;
                conn.bytes_sent = 0;
                evicted += 1;
            }
        }
        evicted
    }

    /// Collect current acceptor statistics.
    pub fn stats(&self) -> AcceptorStats {
        let active = self.active_count();
        let available = self
            .connections
            .iter()
            .filter(|c| c.state == ConnectionState::Available)
            .count();
        let total_bytes_received: u64 = self.connections.iter().map(|c| c.bytes_received).sum();
        let total_bytes_sent: u64 = self.connections.iter().map(|c| c.bytes_sent).sum();
        AcceptorStats {
            active_connections: active,
            pool_available: available,
            total_accepted: self.total_accepted,
            total_rejected: self.total_rejected,
            total_bytes_received,
            total_bytes_sent,
        }
    }

    /// Find an active connection by remote CompID.
    pub fn find_by_comp_id(&self, comp_id: &str) -> Option<u64> {
        self.connections
            .iter()
            .find(|c| {
                (c.state == ConnectionState::InUse || c.state == ConnectionState::Draining)
                    && c.remote_comp_id == comp_id
            })
            .map(|c| c.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SessionRole;

    fn test_config() -> AcceptorConfig {
        AcceptorConfig {
            connection_pool_size: 4,
            max_connections: 8,
            ..AcceptorConfig::default()
        }
    }

    #[test]
    fn test_accept_connection_success() {
        let mut acceptor = Acceptor::new(test_config());
        let result = acceptor.accept_connection("10.0.0.1:5000", "CLIENT-A", 1000);
        assert!(result.is_ok());
        let conn_id = result.unwrap();

        let conn = acceptor.get_connection(conn_id).unwrap();
        assert_eq!(conn.state, ConnectionState::InUse);
        assert_eq!(conn.remote_comp_id, "CLIENT-A");
        assert_eq!(conn.remote_address, "10.0.0.1:5000");
        assert_eq!(conn.connected_at_ms, 1000);
    }

    #[test]
    fn test_pool_exhaustion() {
        let mut acceptor = Acceptor::new(test_config());
        for i in 0..4 {
            let result = acceptor.accept_connection(
                &format!("10.0.0.{}:5000", i),
                &format!("CLIENT-{}", i),
                1000,
            );
            assert!(result.is_ok());
        }
        let result = acceptor.accept_connection("10.0.0.99:5000", "CLIENT-99", 1000);
        assert_eq!(result, Err(AcceptorError::PoolExhausted));
    }

    #[test]
    fn test_release_and_reuse() {
        let mut acceptor = Acceptor::new(test_config());

        // Fill all 4 slots.
        let mut ids = Vec::new();
        for i in 0..4 {
            let id = acceptor
                .accept_connection(
                    &format!("10.0.0.{}:5000", i),
                    &format!("CLIENT-{}", i),
                    1000,
                )
                .unwrap();
            ids.push(id);
        }
        assert_eq!(acceptor.active_count(), 4);

        // Release one.
        acceptor.release_connection(ids[0]).unwrap();
        assert_eq!(acceptor.active_count(), 3);

        // Accept a new one — should reuse the slot.
        let new_id = acceptor
            .accept_connection("10.0.0.50:5000", "CLIENT-NEW", 2000)
            .unwrap();
        assert_eq!(acceptor.active_count(), 4);
        assert_ne!(new_id, ids[0]); // new conn_id
    }

    #[test]
    fn test_comp_id_whitelist_allows_valid() {
        let mut config = test_config();
        config.allowed_comp_ids = vec!["ALPHA".to_string(), "BETA".to_string()];
        let mut acceptor = Acceptor::new(config);

        let result = acceptor.accept_connection("10.0.0.1:5000", "ALPHA", 1000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_comp_id_whitelist_rejects_invalid() {
        let mut config = test_config();
        config.allowed_comp_ids = vec!["ALPHA".to_string(), "BETA".to_string()];
        let mut acceptor = Acceptor::new(config);

        let result = acceptor.accept_connection("10.0.0.1:5000", "GAMMA", 1000);
        assert_eq!(
            result,
            Err(AcceptorError::CompIdNotAllowed("GAMMA".to_string()))
        );
    }

    #[test]
    fn test_empty_whitelist_allows_all() {
        let config = test_config(); // empty allowed_comp_ids
        let mut acceptor = Acceptor::new(config);

        assert!(acceptor.is_comp_id_allowed("ANYTHING"));
        let result = acceptor.accept_connection("10.0.0.1:5000", "ANYTHING", 1000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_evict_idle_connections() {
        let mut acceptor = Acceptor::new(test_config());

        let id1 = acceptor
            .accept_connection("10.0.0.1:5000", "CLIENT-A", 1000)
            .unwrap();
        let _id2 = acceptor
            .accept_connection("10.0.0.2:5000", "CLIENT-B", 5000)
            .unwrap();

        // Update last_active_ms for id1 to simulate staleness.
        acceptor.get_connection_mut(id1).unwrap().last_active_ms = 1000;

        // Evict connections idle for more than 3000ms at time 5000.
        let evicted = acceptor.evict_idle(3000, 5000);
        assert_eq!(evicted, 1);
        assert_eq!(acceptor.active_count(), 1);
    }

    #[test]
    fn test_drain_connection() {
        let mut acceptor = Acceptor::new(test_config());
        let id = acceptor
            .accept_connection("10.0.0.1:5000", "CLIENT-A", 1000)
            .unwrap();

        acceptor.drain_connection(id).unwrap();
        let conn = acceptor.get_connection(id).unwrap();
        assert_eq!(conn.state, ConnectionState::Draining);

        // Draining connections still count as active.
        assert_eq!(acceptor.active_count(), 1);
    }

    #[test]
    fn test_stats_tracking() {
        let mut config = test_config();
        config.allowed_comp_ids = vec!["GOOD".to_string()];
        let mut acceptor = Acceptor::new(config);

        let _ = acceptor.accept_connection("10.0.0.1:5000", "GOOD", 1000);
        let _ = acceptor.accept_connection("10.0.0.2:5000", "BAD", 2000);

        let stats = acceptor.stats();
        assert_eq!(stats.total_accepted, 1);
        assert_eq!(stats.total_rejected, 1);
        assert_eq!(stats.active_connections, 1);
        assert_eq!(stats.pool_available, 3);
    }

    #[test]
    fn test_find_by_comp_id() {
        let mut acceptor = Acceptor::new(test_config());
        let id = acceptor
            .accept_connection("10.0.0.1:5000", "CLIENT-A", 1000)
            .unwrap();

        assert_eq!(acceptor.find_by_comp_id("CLIENT-A"), Some(id));
        assert_eq!(acceptor.find_by_comp_id("CLIENT-X"), None);
    }

    #[test]
    fn test_get_session_acceptor_role() {
        let mut acceptor = Acceptor::new(test_config());
        let id = acceptor
            .accept_connection("10.0.0.1:5000", "CLIENT-A", 1000)
            .unwrap();

        let session = acceptor.get_session(id).unwrap();
        assert_eq!(session.config().role, SessionRole::Acceptor);
        assert_eq!(session.config().target_comp_id, "CLIENT-A");
        assert_eq!(session.config().sender_comp_id, "VELOCITAS");
    }

    #[test]
    fn test_multiple_simultaneous_connections() {
        let mut acceptor = Acceptor::new(test_config());

        let id1 = acceptor
            .accept_connection("10.0.0.1:5000", "CLIENT-A", 1000)
            .unwrap();
        let id2 = acceptor
            .accept_connection("10.0.0.2:5000", "CLIENT-B", 1000)
            .unwrap();
        let id3 = acceptor
            .accept_connection("10.0.0.3:5000", "CLIENT-C", 1000)
            .unwrap();

        assert_ne!(id1, id2);
        assert_ne!(id2, id3);
        assert_eq!(acceptor.active_count(), 3);

        // Each has its own session.
        assert_eq!(
            acceptor.get_session(id1).unwrap().config().target_comp_id,
            "CLIENT-A"
        );
        assert_eq!(
            acceptor.get_session(id2).unwrap().config().target_comp_id,
            "CLIENT-B"
        );
        assert_eq!(
            acceptor.get_session(id3).unwrap().config().target_comp_id,
            "CLIENT-C"
        );
    }
}
