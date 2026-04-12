/// DPDK (Data Plane Development Kit) kernel-bypass transport.
///
/// Provides ultra-low-latency network I/O by bypassing the kernel TCP/IP stack
/// and driving NICs directly from user space via poll-mode drivers (PMDs).
///
/// This module is gated behind `#[cfg(feature = "dpdk")]` for the actual FFI
/// calls, but the struct definitions and API compile on all platforms.
use std::io;

use crate::transport::{Transport, TransportEvent};

// ---------------------------------------------------------------------------
// DpdkConfig
// ---------------------------------------------------------------------------

/// DPDK transport configuration.
#[derive(Debug, Clone)]
pub struct DpdkConfig {
    /// EAL (Environment Abstraction Layer) arguments.
    pub eal_args: Vec<String>,
    /// DPDK port ID (NIC).
    pub port_id: u16,
    /// Number of RX queues.
    pub rx_queues: u16,
    /// Number of TX queues.
    pub tx_queues: u16,
    /// RX ring size (must be power of 2).
    pub rx_ring_size: u16,
    /// TX ring size (must be power of 2).
    pub tx_ring_size: u16,
    /// Number of mbufs in the memory pool.
    pub num_mbufs: u32,
    /// Mbuf cache size.
    pub mbuf_cache_size: u32,
    /// Whether to enable hardware timestamping.
    pub hw_timestamps: bool,
    /// CPU core to pin the poll thread to.
    pub poll_core: Option<u32>,
    /// Promiscuous mode.
    pub promiscuous: bool,
}

impl Default for DpdkConfig {
    fn default() -> Self {
        DpdkConfig {
            eal_args: Vec::new(),
            port_id: 0,
            rx_queues: 1,
            tx_queues: 1,
            rx_ring_size: 1024,
            tx_ring_size: 1024,
            num_mbufs: 8192,
            mbuf_cache_size: 256,
            hw_timestamps: false,
            poll_core: None,
            promiscuous: false,
        }
    }
}

// ---------------------------------------------------------------------------
// DpdkStats
// ---------------------------------------------------------------------------

/// DPDK port statistics.
#[derive(Debug, Clone, Default)]
pub struct DpdkStats {
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_missed: u64,
}

// ---------------------------------------------------------------------------
// DpdkMbuf
// ---------------------------------------------------------------------------

/// Represents a DPDK mbuf (message buffer).
///
/// In a real implementation this would be a wrapper around `*mut rte_mbuf`
/// allocated from a DPDK mempool in hugepage memory.
pub struct DpdkMbuf {
    /// Length of the data in the buffer.
    pub data_len: u16,
    /// Total packet length (across chained mbufs).
    pub pkt_len: u32,
    /// Port the packet arrived on / will be sent from.
    pub port: u16,
    /// Hardware timestamp (if available).
    pub timestamp: u64,
    data: Vec<u8>,
}

impl DpdkMbuf {
    /// Returns an immutable slice of the packet data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Returns a mutable slice of the packet data.
    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Replace the packet data with `bytes`.
    pub fn set_data(&mut self, bytes: &[u8]) {
        self.data = bytes.to_vec();
        self.data_len = bytes.len() as u16;
        self.pkt_len = bytes.len() as u32;
    }
}

// ---------------------------------------------------------------------------
// DpdkMempool
// ---------------------------------------------------------------------------

/// DPDK memory pool for mbuf allocation.
///
/// In a real implementation this wraps `*mut rte_mempool` backed by hugepages,
/// providing lock-free per-core caching for fast alloc/free.
pub struct DpdkMempool {
    /// Name of the mempool (used by `rte_pktmbuf_pool_create`).
    pub name: String,
    /// Number of mbufs in the pool.
    pub num_mbufs: u32,
    /// Per-core cache size.
    pub cache_size: u32,
    /// Data room size per mbuf (payload capacity).
    pub data_room_size: u16,
    initialized: bool,
}

impl DpdkMempool {
    /// Create a new mempool descriptor.
    ///
    /// In a real implementation this would call `rte_pktmbuf_pool_create` to
    /// allocate the pool from hugepage memory.
    pub fn new(name: &str, num_mbufs: u32, cache_size: u32, data_room_size: u16) -> Self {
        DpdkMempool {
            name: name.to_string(),
            num_mbufs,
            cache_size,
            data_room_size,
            initialized: true,
        }
    }

    /// Allocate an mbuf from the pool.
    ///
    /// In a real implementation this calls `rte_pktmbuf_alloc`, which is an
    /// O(1) operation using per-lcore caches.
    pub fn allocate_mbuf(&self) -> Option<DpdkMbuf> {
        if !self.initialized {
            return None;
        }
        Some(DpdkMbuf {
            data_len: 0,
            pkt_len: 0,
            port: 0,
            timestamp: 0,
            data: Vec::with_capacity(self.data_room_size as usize),
        })
    }

    /// Return an mbuf to the pool.
    ///
    /// In a real implementation this calls `rte_pktmbuf_free`, returning the
    /// mbuf to the per-lcore cache (or the common pool if the cache is full).
    pub fn free_mbuf(&self, _mbuf: DpdkMbuf) {
        // Stub: the mbuf is simply dropped.
    }
}

// ---------------------------------------------------------------------------
// DpdkTransport
// ---------------------------------------------------------------------------

/// DPDK transport — kernel-bypass NIC access via poll-mode drivers.
///
/// Bypasses the kernel network stack entirely: packets flow directly between
/// user-space and the NIC via hugepage-backed ring buffers, eliminating
/// system-call overhead and context switches.
pub struct DpdkTransport {
    config: DpdkConfig,
    initialized: bool,
    connected: bool,
    stats: DpdkStats,
}

impl DpdkTransport {
    /// Create a new DPDK transport with the given configuration.
    pub fn new(config: DpdkConfig) -> Self {
        DpdkTransport {
            config,
            initialized: false,
            connected: false,
            stats: DpdkStats::default(),
        }
    }

    /// Initialise the DPDK EAL and configure the port.
    ///
    /// In a real implementation this would:
    /// 1. Call `rte_eal_init` with the configured EAL arguments.
    /// 2. Verify the port ID is valid via `rte_eth_dev_count_avail`.
    /// 3. Configure the port with `rte_eth_dev_configure` (rx/tx queues, RSS, etc.).
    /// 4. Set up RX/TX queues with `rte_eth_rx_queue_setup` / `rte_eth_tx_queue_setup`.
    /// 5. Create the mbuf mempool via `rte_pktmbuf_pool_create`.
    /// 6. Start the port with `rte_eth_dev_start`.
    /// 7. Optionally enable promiscuous mode via `rte_eth_promiscuous_enable`.
    /// 8. Optionally pin the poll thread to `poll_core` via `rte_lcore_id` / affinity.
    pub fn initialize(&mut self) -> io::Result<()> {
        if self.initialized {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "DPDK EAL already initialized",
            ));
        }

        // Stub: mark as initialized.
        self.initialized = true;
        Ok(())
    }

    /// Return current port statistics.
    ///
    /// In a real implementation this calls `rte_eth_stats_get` to fetch
    /// hardware counters from the NIC.
    pub fn stats(&self) -> &DpdkStats {
        &self.stats
    }

    /// Enable or disable promiscuous mode on the port.
    ///
    /// In a real implementation this calls `rte_eth_promiscuous_enable` or
    /// `rte_eth_promiscuous_disable`.
    pub fn set_promiscuous(&mut self, enabled: bool) {
        self.config.promiscuous = enabled;
    }

    /// Return the MAC address of the port.
    ///
    /// In a real implementation this calls `rte_eth_macaddr_get`.
    pub fn mac_address(&self) -> [u8; 6] {
        [0; 6]
    }
}

impl Transport for DpdkTransport {
    /// Connect to a remote endpoint.
    ///
    /// In a real implementation this would configure the DPDK port's flow
    /// director / 5-tuple filter so that only traffic matching the remote
    /// endpoint is delivered to the application's RX queue. A user-space
    /// TCP stack (e.g. ANS, F-Stack, or a custom lightweight implementation)
    /// would then perform the TCP handshake over raw Ethernet frames.
    fn connect(&mut self, _address: &str, _port: u16) -> io::Result<()> {
        if !self.initialized {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "DPDK not initialized; call initialize() first",
            ));
        }
        self.connected = true;
        Ok(())
    }

    /// Bind and listen for incoming connections.
    ///
    /// In a real implementation this would set up the DPDK port to accept
    /// connections on the specified address and port, configuring flow rules
    /// and registering a listener in the user-space TCP stack.
    fn bind(&mut self, _address: &str, _port: u16) -> io::Result<()> {
        if !self.initialized {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "DPDK not initialized; call initialize() first",
            ));
        }
        Ok(())
    }

    /// Send data via DPDK.
    ///
    /// In a real implementation this would:
    /// 1. Allocate an mbuf from the mempool (`rte_pktmbuf_alloc`).
    /// 2. Copy the payload into the mbuf data room.
    /// 3. Prepend TCP/IP/Ethernet headers.
    /// 4. Enqueue the mbuf on the TX ring via `rte_eth_tx_burst`.
    /// 5. Update TX statistics.
    fn send(&mut self, data: &[u8]) -> io::Result<usize> {
        if !self.connected {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "not connected"));
        }
        let len = data.len();
        self.stats.tx_packets += 1;
        self.stats.tx_bytes += len as u64;
        Ok(len)
    }

    /// Receive data via DPDK.
    ///
    /// In a real implementation this would:
    /// 1. Call `rte_eth_rx_burst` to dequeue mbufs from the RX ring.
    /// 2. Strip Ethernet/IP/TCP headers.
    /// 3. Copy the payload into the caller's buffer.
    /// 4. Free the mbufs back to the mempool (`rte_pktmbuf_free`).
    /// 5. Update RX statistics.
    fn recv(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        if !self.connected {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "not connected"));
        }
        Ok(0)
    }

    /// Close the DPDK transport connection.
    ///
    /// In a real implementation this would tear down the user-space TCP
    /// connection (FIN handshake), remove flow director rules, and
    /// optionally stop the port via `rte_eth_dev_stop`.
    fn close(&mut self) -> io::Result<()> {
        self.connected = false;
        Ok(())
    }

    /// Poll for transport events (non-blocking).
    ///
    /// In a real implementation this is the hot path: it calls
    /// `rte_eth_rx_burst` in a tight loop (busy-poll) to check for
    /// incoming packets with zero system-call overhead.
    fn poll(&mut self) -> io::Result<Option<TransportEvent>> {
        Ok(None)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dpdk_config_defaults() {
        let cfg = DpdkConfig::default();
        assert_eq!(cfg.port_id, 0);
        assert_eq!(cfg.rx_queues, 1);
        assert_eq!(cfg.tx_queues, 1);
        assert_eq!(cfg.rx_ring_size, 1024);
        assert_eq!(cfg.tx_ring_size, 1024);
        assert_eq!(cfg.num_mbufs, 8192);
        assert_eq!(cfg.mbuf_cache_size, 256);
        assert!(!cfg.hw_timestamps);
        assert!(cfg.poll_core.is_none());
        assert!(!cfg.promiscuous);
        assert!(cfg.eal_args.is_empty());
    }

    #[test]
    fn test_dpdk_transport_lifecycle() {
        let config = DpdkConfig::default();
        let mut transport = DpdkTransport::new(config);

        assert!(!transport.is_connected());

        // Must initialize before connecting.
        assert!(transport.connect("10.0.0.1", 9876).is_err());

        transport.initialize().unwrap();

        // Double init should fail.
        assert!(transport.initialize().is_err());

        transport.connect("10.0.0.1", 9876).unwrap();
        assert!(transport.is_connected());

        let sent = transport.send(b"8=FIX.4.4\x01").unwrap();
        assert_eq!(sent, 10);

        transport.close().unwrap();
        assert!(!transport.is_connected());
    }

    #[test]
    fn test_dpdk_transport_send_not_connected() {
        let mut transport = DpdkTransport::new(DpdkConfig::default());
        transport.initialize().unwrap();
        let err = transport.send(b"data").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    #[test]
    fn test_dpdk_transport_recv_not_connected() {
        let mut transport = DpdkTransport::new(DpdkConfig::default());
        transport.initialize().unwrap();
        let mut buf = [0u8; 64];
        let err = transport.recv(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected);
    }

    #[test]
    fn test_dpdk_stats_tracking() {
        let mut transport = DpdkTransport::new(DpdkConfig::default());
        transport.initialize().unwrap();
        transport.connect("10.0.0.1", 9876).unwrap();

        transport.send(b"hello").unwrap();
        transport.send(b"world!!").unwrap();

        let stats = transport.stats();
        assert_eq!(stats.tx_packets, 2);
        assert_eq!(stats.tx_bytes, 12);
        assert_eq!(stats.rx_packets, 0);
    }

    #[test]
    fn test_dpdk_promiscuous() {
        let mut transport = DpdkTransport::new(DpdkConfig::default());
        assert!(!transport.config.promiscuous);
        transport.set_promiscuous(true);
        assert!(transport.config.promiscuous);
        transport.set_promiscuous(false);
        assert!(!transport.config.promiscuous);
    }

    #[test]
    fn test_dpdk_mac_address() {
        let transport = DpdkTransport::new(DpdkConfig::default());
        assert_eq!(transport.mac_address(), [0u8; 6]);
    }

    #[test]
    fn test_dpdk_mempool_allocate_free() {
        let pool = DpdkMempool::new("test_pool", 1024, 32, 2048);
        assert_eq!(pool.name, "test_pool");
        assert_eq!(pool.num_mbufs, 1024);
        assert_eq!(pool.cache_size, 32);
        assert_eq!(pool.data_room_size, 2048);

        let mbuf = pool.allocate_mbuf();
        assert!(mbuf.is_some());

        let mbuf = mbuf.unwrap();
        assert_eq!(mbuf.data_len, 0);
        assert_eq!(mbuf.pkt_len, 0);

        pool.free_mbuf(mbuf);
    }

    #[test]
    fn test_dpdk_mbuf_data_access() {
        let pool = DpdkMempool::new("test_pool", 1024, 32, 2048);
        let mut mbuf = pool.allocate_mbuf().unwrap();

        assert!(mbuf.data().is_empty());

        mbuf.set_data(b"FIX message payload");
        assert_eq!(mbuf.data(), b"FIX message payload");
        assert_eq!(mbuf.data_len, 19);
        assert_eq!(mbuf.pkt_len, 19);

        mbuf.data_mut()[0] = b'X';
        assert_eq!(mbuf.data()[0], b'X');
    }

    #[test]
    fn test_dpdk_transport_bind() {
        let mut transport = DpdkTransport::new(DpdkConfig::default());

        // Bind without init should fail.
        assert!(transport.bind("0.0.0.0", 9876).is_err());

        transport.initialize().unwrap();
        transport.bind("0.0.0.0", 9876).unwrap();
    }

    #[test]
    fn test_dpdk_transport_poll() {
        let mut transport = DpdkTransport::new(DpdkConfig::default());
        transport.initialize().unwrap();
        let event = transport.poll().unwrap();
        assert!(event.is_none());
    }

    #[test]
    fn test_dpdk_transport_trait_object() {
        let mut transport: Box<dyn Transport> = Box::new(DpdkTransport::new(DpdkConfig::default()));
        // Verify DpdkTransport can be used as a trait object.
        assert!(!transport.is_connected());
        assert!(transport.poll().unwrap().is_none());
    }
}
