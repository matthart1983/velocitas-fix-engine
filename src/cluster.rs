/// Aeron-aligned cluster model for active-active high availability.
///
/// Replicates FIX session state across multiple engine instances using a
/// consensus protocol modeled after Aeron Cluster / Raft. Each node maintains
/// a replicated log of session state changes, sequence updates, and message
/// journal entries.

/// Cluster node role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    Leader,
    Follower,
    Candidate,
}

/// Cluster node state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeState {
    Joining,
    Active,
    Snapshotting,
    Left,
}

/// Cluster node identity.
#[derive(Debug, Clone)]
pub struct NodeId {
    pub id: u32,
    pub address: String,
    pub port: u16,
}

/// Cluster configuration.
#[derive(Debug, Clone)]
pub struct ClusterConfig {
    pub node_id: NodeId,
    pub peers: Vec<NodeId>,
    pub election_timeout_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub snapshot_interval_msgs: u64,
    pub log_dir: String,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        ClusterConfig {
            node_id: NodeId {
                id: 0,
                address: "127.0.0.1".to_string(),
                port: 9000,
            },
            peers: Vec::new(),
            election_timeout_ms: 300,
            heartbeat_interval_ms: 100,
            snapshot_interval_msgs: 10_000,
            log_dir: "/tmp/velocitas-cluster".to_string(),
        }
    }
}

impl ClusterConfig {
    /// Configuration for a single-node cluster (immediately becomes leader).
    pub fn single_node(id: u32) -> Self {
        ClusterConfig {
            node_id: NodeId {
                id,
                address: "127.0.0.1".to_string(),
                port: 9000 + id as u16,
            },
            peers: Vec::new(),
            ..Default::default()
        }
    }

    /// Configuration for a three-node cluster.
    pub fn three_node(id: u32, peers: Vec<NodeId>) -> Self {
        ClusterConfig {
            node_id: NodeId {
                id,
                address: "127.0.0.1".to_string(),
                port: 9000 + id as u16,
            },
            peers,
            ..Default::default()
        }
    }
}

/// A log entry in the replicated state machine.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub term: u64,
    pub index: u64,
    pub entry_type: LogEntryType,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEntryType {
    SessionStateChange,
    SequenceUpdate,
    MessageJournal,
    Configuration,
    Snapshot,
}

/// Replicated session state — what gets synchronized across the cluster.
#[derive(Debug, Clone)]
pub struct ReplicatedSessionState {
    pub session_id: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub outbound_seq_num: u64,
    pub inbound_seq_num: u64,
    pub state: u8,
    pub last_updated_ms: u64,
}

/// Cluster node — manages consensus and state replication.
pub struct ClusterNode {
    config: ClusterConfig,
    role: NodeRole,
    state: NodeState,
    current_term: u64,
    voted_for: Option<u32>,
    commit_index: u64,
    last_applied: u64,
    log: Vec<LogEntry>,
    session_states: Vec<ReplicatedSessionState>,
    leader_id: Option<u32>,
    votes_received: u32,
}

impl ClusterNode {
    /// Create a new cluster node with the given configuration.
    pub fn new(config: ClusterConfig) -> Self {
        ClusterNode {
            config,
            role: NodeRole::Follower,
            state: NodeState::Joining,
            current_term: 0,
            voted_for: None,
            commit_index: 0,
            last_applied: 0,
            log: Vec::new(),
            session_states: Vec::new(),
            leader_id: None,
            votes_received: 0,
        }
    }

    /// Get the current role.
    #[inline]
    pub fn role(&self) -> NodeRole {
        self.role
    }

    /// Get the current node state.
    #[inline]
    pub fn state(&self) -> NodeState {
        self.state
    }

    /// Get the current term.
    #[inline]
    pub fn current_term(&self) -> u64 {
        self.current_term
    }

    /// Whether this node is the leader.
    #[inline]
    pub fn is_leader(&self) -> bool {
        self.role == NodeRole::Leader
    }

    /// Get the number of log entries.
    #[inline]
    pub fn log_len(&self) -> usize {
        self.log.len()
    }

    /// Get the commit index.
    #[inline]
    pub fn commit_index(&self) -> u64 {
        self.commit_index
    }

    /// Number of log entries not yet committed.
    #[inline]
    pub fn pending_entries(&self) -> usize {
        if self.log.is_empty() {
            return 0;
        }
        let last_index = self.log.last().map(|e| e.index).unwrap_or(0);
        if last_index > self.commit_index {
            (last_index - self.commit_index) as usize
        } else {
            0
        }
    }

    /// Start the node — transition to Active and begin election.
    pub fn start(&mut self) {
        self.state = NodeState::Active;
        self.begin_election();
    }

    /// Step down from Leader to Follower (e.g., on discovering a higher term).
    pub fn step_down(&mut self, new_term: u64) {
        self.role = NodeRole::Follower;
        self.current_term = new_term;
        self.voted_for = None;
        self.votes_received = 0;
        self.leader_id = None;
    }

    /// Transition to Leader. Resets vote state.
    pub fn become_leader(&mut self) {
        self.role = NodeRole::Leader;
        self.leader_id = Some(self.config.node_id.id);
        self.votes_received = 0;
    }

    /// Start an election: increment term, vote for self, become Candidate.
    pub fn begin_election(&mut self) {
        self.current_term += 1;
        self.role = NodeRole::Candidate;
        self.voted_for = Some(self.config.node_id.id);
        self.votes_received = 1; // vote for self
        self.leader_id = None;

        // Single-node cluster wins immediately.
        if self.config.peers.is_empty() {
            self.become_leader();
        }
    }

    /// Process a vote response from another node.
    ///
    /// If the responding term is higher, step down. Otherwise, if the vote is
    /// granted, accumulate it and check for majority.
    pub fn receive_vote(&mut self, _from_node: u32, term: u64, granted: bool) {
        if term > self.current_term {
            self.step_down(term);
            return;
        }

        if self.role != NodeRole::Candidate || term != self.current_term {
            return;
        }

        if granted {
            self.votes_received += 1;
            let cluster_size = self.config.peers.len() as u32 + 1;
            let majority = cluster_size / 2 + 1;
            if self.votes_received >= majority {
                self.become_leader();
            }
        }
    }

    /// Replicate a session state change — append to the log.
    pub fn replicate_session_state(&mut self, state: ReplicatedSessionState) {
        let index = self.log.last().map(|e| e.index).unwrap_or(0) + 1;

        // Encode session state as data.
        let data = encode_session_state(&state);

        self.log.push(LogEntry {
            term: self.current_term,
            index,
            entry_type: LogEntryType::SessionStateChange,
            data,
        });
    }

    /// Apply a committed log entry to the session state table.
    pub fn apply_log_entry(&mut self, entry: &LogEntry) {
        if entry.entry_type == LogEntryType::SessionStateChange {
            if let Some(state) = decode_session_state(&entry.data) {
                // Upsert: update existing or insert new.
                if let Some(existing) = self
                    .session_states
                    .iter_mut()
                    .find(|s| s.session_id == state.session_id)
                {
                    existing.outbound_seq_num = state.outbound_seq_num;
                    existing.inbound_seq_num = state.inbound_seq_num;
                    existing.state = state.state;
                    existing.last_updated_ms = state.last_updated_ms;
                } else {
                    self.session_states.push(state);
                }
            }
        }
        if entry.index > self.last_applied {
            self.last_applied = entry.index;
        }
        if entry.index > self.commit_index {
            self.commit_index = entry.index;
        }
    }

    /// Look up a replicated session state by session ID.
    pub fn get_session_state(&self, session_id: &str) -> Option<&ReplicatedSessionState> {
        self.session_states
            .iter()
            .find(|s| s.session_id == session_id)
    }

    /// Create a snapshot of the current session states.
    pub fn create_snapshot(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        let count = self.session_states.len() as u32;
        buf.extend_from_slice(&count.to_le_bytes());
        for state in &self.session_states {
            let encoded = encode_session_state(state);
            let len = encoded.len() as u32;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(&encoded);
        }
        buf
    }

    /// Restore session states from a snapshot.
    pub fn restore_snapshot(&mut self, data: &[u8]) {
        self.session_states.clear();
        if data.len() < 4 {
            return;
        }
        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut offset = 4;
        for _ in 0..count {
            if offset + 4 > data.len() {
                break;
            }
            let len = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]) as usize;
            offset += 4;
            if offset + len > data.len() {
                break;
            }
            if let Some(state) = decode_session_state(&data[offset..offset + len]) {
                self.session_states.push(state);
            }
            offset += len;
        }
        self.state = NodeState::Active;
    }
}

/// Encode a ReplicatedSessionState into bytes.
fn encode_session_state(state: &ReplicatedSessionState) -> Vec<u8> {
    let mut buf = Vec::new();
    // session_id
    let id_bytes = state.session_id.as_bytes();
    buf.extend_from_slice(&(id_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(id_bytes);
    // sender_comp_id
    let sender_bytes = state.sender_comp_id.as_bytes();
    buf.extend_from_slice(&(sender_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(sender_bytes);
    // target_comp_id
    let target_bytes = state.target_comp_id.as_bytes();
    buf.extend_from_slice(&(target_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(target_bytes);
    // seq nums, state, timestamp
    buf.extend_from_slice(&state.outbound_seq_num.to_le_bytes());
    buf.extend_from_slice(&state.inbound_seq_num.to_le_bytes());
    buf.push(state.state);
    buf.extend_from_slice(&state.last_updated_ms.to_le_bytes());
    buf
}

/// Decode a ReplicatedSessionState from bytes.
fn decode_session_state(data: &[u8]) -> Option<ReplicatedSessionState> {
    let mut offset = 0;

    let read_str = |data: &[u8], offset: &mut usize| -> Option<String> {
        if *offset + 2 > data.len() {
            return None;
        }
        let len = u16::from_le_bytes([data[*offset], data[*offset + 1]]) as usize;
        *offset += 2;
        if *offset + len > data.len() {
            return None;
        }
        let s = std::str::from_utf8(&data[*offset..*offset + len])
            .ok()?
            .to_string();
        *offset += len;
        Some(s)
    };

    let session_id = read_str(data, &mut offset)?;
    let sender_comp_id = read_str(data, &mut offset)?;
    let target_comp_id = read_str(data, &mut offset)?;

    if offset + 8 + 8 + 1 + 8 > data.len() {
        return None;
    }

    let outbound_seq_num = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
    offset += 8;
    let inbound_seq_num = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
    offset += 8;
    let state = data[offset];
    offset += 1;
    let last_updated_ms = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);

    Some(ReplicatedSessionState {
        session_id,
        sender_comp_id,
        target_comp_id,
        outbound_seq_num,
        inbound_seq_num,
        state,
        last_updated_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session_state(session_id: &str, outbound: u64, inbound: u64) -> ReplicatedSessionState {
        ReplicatedSessionState {
            session_id: session_id.to_string(),
            sender_comp_id: "SENDER".to_string(),
            target_comp_id: "TARGET".to_string(),
            outbound_seq_num: outbound,
            inbound_seq_num: inbound,
            state: 3, // Active
            last_updated_ms: 1_700_000_000_000,
        }
    }

    fn make_peers(ids: &[u32]) -> Vec<NodeId> {
        ids.iter()
            .map(|&id| NodeId {
                id,
                address: "127.0.0.1".to_string(),
                port: 9000 + id as u16,
            })
            .collect()
    }

    #[test]
    fn test_single_node_becomes_leader() {
        let config = ClusterConfig::single_node(1);
        let mut node = ClusterNode::new(config);

        assert_eq!(node.role(), NodeRole::Follower);
        assert_eq!(node.state(), NodeState::Joining);

        node.start();

        assert_eq!(node.state(), NodeState::Active);
        assert_eq!(node.role(), NodeRole::Leader);
        assert!(node.is_leader());
        assert_eq!(node.current_term(), 1);
    }

    #[test]
    fn test_three_node_election_with_majority() {
        let peers = make_peers(&[2, 3]);
        let config = ClusterConfig::three_node(1, peers);
        let mut node = ClusterNode::new(config);

        node.start();

        // After start, node is Candidate (needs votes from peers).
        assert_eq!(node.role(), NodeRole::Candidate);
        assert_eq!(node.current_term(), 1);

        // Receive one more vote — majority (2 of 3).
        node.receive_vote(2, 1, true);
        assert_eq!(node.role(), NodeRole::Leader);
        assert!(node.is_leader());
    }

    #[test]
    fn test_election_needs_majority() {
        let peers = make_peers(&[2, 3]);
        let config = ClusterConfig::three_node(1, peers);
        let mut node = ClusterNode::new(config);

        node.start();

        // Rejected vote — still Candidate.
        node.receive_vote(2, 1, false);
        assert_eq!(node.role(), NodeRole::Candidate);

        // Granted vote — now has majority.
        node.receive_vote(3, 1, true);
        assert_eq!(node.role(), NodeRole::Leader);
    }

    #[test]
    fn test_session_state_replication_and_retrieval() {
        let config = ClusterConfig::single_node(1);
        let mut node = ClusterNode::new(config);
        node.start();

        let state = make_session_state("FIX-1", 42, 37);
        node.replicate_session_state(state);

        assert_eq!(node.log_len(), 1);

        // Apply the entry.
        let entry = node.log[0].clone();
        node.apply_log_entry(&entry);

        let retrieved = node.get_session_state("FIX-1").unwrap();
        assert_eq!(retrieved.outbound_seq_num, 42);
        assert_eq!(retrieved.inbound_seq_num, 37);
        assert_eq!(retrieved.sender_comp_id, "SENDER");
    }

    #[test]
    fn test_snapshot_create_and_restore() {
        let config = ClusterConfig::single_node(1);
        let mut node = ClusterNode::new(config);
        node.start();

        node.replicate_session_state(make_session_state("FIX-1", 10, 5));
        node.replicate_session_state(make_session_state("FIX-2", 20, 15));

        for i in 0..node.log.len() {
            let entry = node.log[i].clone();
            node.apply_log_entry(&entry);
        }

        let snapshot = node.create_snapshot();
        assert!(!snapshot.is_empty());

        // Restore into a fresh node.
        let config2 = ClusterConfig::single_node(2);
        let mut node2 = ClusterNode::new(config2);
        node2.restore_snapshot(&snapshot);

        assert_eq!(node2.state(), NodeState::Active);

        let s1 = node2.get_session_state("FIX-1").unwrap();
        assert_eq!(s1.outbound_seq_num, 10);
        assert_eq!(s1.inbound_seq_num, 5);

        let s2 = node2.get_session_state("FIX-2").unwrap();
        assert_eq!(s2.outbound_seq_num, 20);
        assert_eq!(s2.inbound_seq_num, 15);
    }

    #[test]
    fn test_leader_step_down_on_higher_term() {
        let config = ClusterConfig::single_node(1);
        let mut node = ClusterNode::new(config);
        node.start();
        assert!(node.is_leader());
        assert_eq!(node.current_term(), 1);

        node.step_down(5);
        assert_eq!(node.role(), NodeRole::Follower);
        assert_eq!(node.current_term(), 5);
        assert!(!node.is_leader());
    }

    #[test]
    fn test_receive_vote_with_higher_term_steps_down() {
        let peers = make_peers(&[2, 3]);
        let config = ClusterConfig::three_node(1, peers);
        let mut node = ClusterNode::new(config);
        node.start();
        assert_eq!(node.role(), NodeRole::Candidate);

        // Peer responds with a higher term.
        node.receive_vote(2, 10, false);
        assert_eq!(node.role(), NodeRole::Follower);
        assert_eq!(node.current_term(), 10);
    }

    #[test]
    fn test_log_entry_append_and_commit() {
        let config = ClusterConfig::single_node(1);
        let mut node = ClusterNode::new(config);
        node.start();

        node.replicate_session_state(make_session_state("A", 1, 1));
        node.replicate_session_state(make_session_state("B", 2, 2));
        node.replicate_session_state(make_session_state("C", 3, 3));

        assert_eq!(node.log_len(), 3);
        assert_eq!(node.commit_index(), 0);
        assert_eq!(node.pending_entries(), 3);

        // Apply first two entries.
        let e0 = node.log[0].clone();
        let e1 = node.log[1].clone();
        node.apply_log_entry(&e0);
        node.apply_log_entry(&e1);

        assert_eq!(node.commit_index(), 2);
        assert_eq!(node.pending_entries(), 1);
    }

    #[test]
    fn test_node_state_transitions() {
        let config = ClusterConfig::single_node(1);
        let mut node = ClusterNode::new(config);

        assert_eq!(node.state(), NodeState::Joining);

        node.start();
        assert_eq!(node.state(), NodeState::Active);
    }

    #[test]
    fn test_election_term_increment() {
        let peers = make_peers(&[2, 3]);
        let config = ClusterConfig::three_node(1, peers);
        let mut node = ClusterNode::new(config);

        assert_eq!(node.current_term(), 0);

        node.start(); // first election
        assert_eq!(node.current_term(), 1);

        // Simulate failed election — start another.
        node.begin_election();
        assert_eq!(node.current_term(), 2);
        assert_eq!(node.role(), NodeRole::Candidate);

        node.begin_election();
        assert_eq!(node.current_term(), 3);
    }

    #[test]
    fn test_session_state_upsert() {
        let config = ClusterConfig::single_node(1);
        let mut node = ClusterNode::new(config);
        node.start();

        node.replicate_session_state(make_session_state("FIX-1", 10, 5));
        let entry = node.log[0].clone();
        node.apply_log_entry(&entry);

        assert_eq!(
            node.get_session_state("FIX-1").unwrap().outbound_seq_num,
            10
        );

        // Update the same session.
        node.replicate_session_state(make_session_state("FIX-1", 42, 30));
        let entry = node.log[1].clone();
        node.apply_log_entry(&entry);

        assert_eq!(
            node.get_session_state("FIX-1").unwrap().outbound_seq_num,
            42
        );
        assert_eq!(node.get_session_state("FIX-1").unwrap().inbound_seq_num, 30);
        // Should still be only one entry, not two.
        assert_eq!(node.session_states.len(), 1);
    }

    #[test]
    fn test_get_session_state_not_found() {
        let config = ClusterConfig::single_node(1);
        let node = ClusterNode::new(config);
        assert!(node.get_session_state("nonexistent").is_none());
    }

    #[test]
    fn test_pending_entries_empty_log() {
        let config = ClusterConfig::single_node(1);
        let node = ClusterNode::new(config);
        assert_eq!(node.pending_entries(), 0);
    }
}
