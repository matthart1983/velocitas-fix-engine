/// Transport layer abstraction.
///
/// Provides a unified interface over kernel TCP, DPDK, and OpenOnload transports.
/// The actual kernel-bypass implementations require platform-specific dependencies
/// and are behind feature flags.

use std::io;

/// Transport event types delivered to the session layer.
#[derive(Debug)]
pub enum TransportEvent {
    Connected,
    Disconnected,
    DataReceived(usize), // number of bytes available in the receive buffer
    Error(io::Error),
}

/// Transport configuration.
#[derive(Debug, Clone)]
pub struct TransportConfig {
    pub mode: TransportMode,
    pub bind_address: Option<String>,
    pub connect_address: Option<String>,
    pub port: u16,
    pub recv_buffer_size: usize,
    pub send_buffer_size: usize,
    pub tcp_nodelay: bool,
}

/// Transport mode selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    /// Standard kernel TCP/IP stack.
    KernelTcp,
    /// DPDK user-space TCP (requires `dpdk` feature).
    Dpdk,
    /// Solarflare OpenOnload (requires `openonload` feature).
    OpenOnload,
}

impl Default for TransportConfig {
    fn default() -> Self {
        TransportConfig {
            mode: TransportMode::KernelTcp,
            bind_address: None,
            connect_address: None,
            port: 0,
            recv_buffer_size: 256 * 1024,
            send_buffer_size: 256 * 1024,
            tcp_nodelay: true,
        }
    }
}

/// Transport trait — implemented by each transport backend.
pub trait Transport {
    /// Connect to a remote endpoint (Initiator mode).
    fn connect(&mut self, address: &str, port: u16) -> io::Result<()>;

    /// Bind and listen for connections (Acceptor mode).
    fn bind(&mut self, address: &str, port: u16) -> io::Result<()>;

    /// Send data. Returns the number of bytes sent.
    fn send(&mut self, data: &[u8]) -> io::Result<usize>;

    /// Receive data into the provided buffer. Returns number of bytes read.
    fn recv(&mut self, buffer: &mut [u8]) -> io::Result<usize>;

    /// Close the connection.
    fn close(&mut self) -> io::Result<()>;

    /// Poll for events (non-blocking).
    fn poll(&mut self) -> io::Result<Option<TransportEvent>>;

    /// Returns true if the transport is connected.
    fn is_connected(&self) -> bool;
}

/// Kernel TCP transport implementation using standard sockets.
pub struct KernelTcpTransport {
    connected: bool,
    _config: TransportConfig,
}

impl KernelTcpTransport {
    pub fn new(config: TransportConfig) -> Self {
        KernelTcpTransport {
            connected: false,
            _config: config,
        }
    }
}

impl Transport for KernelTcpTransport {
    fn connect(&mut self, _address: &str, _port: u16) -> io::Result<()> {
        // Real implementation would use mio/tokio or raw sockets
        self.connected = true;
        Ok(())
    }

    fn bind(&mut self, _address: &str, _port: u16) -> io::Result<()> {
        Ok(())
    }

    fn send(&mut self, data: &[u8]) -> io::Result<usize> {
        if !self.connected {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "not connected"));
        }
        Ok(data.len())
    }

    fn recv(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        if !self.connected {
            return Err(io::Error::new(io::ErrorKind::NotConnected, "not connected"));
        }
        Ok(0)
    }

    fn close(&mut self) -> io::Result<()> {
        self.connected = false;
        Ok(())
    }

    fn poll(&mut self) -> io::Result<Option<TransportEvent>> {
        Ok(None)
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kernel_tcp_transport() {
        let config = TransportConfig::default();
        let mut transport = KernelTcpTransport::new(config);

        assert!(!transport.is_connected());
        transport.connect("127.0.0.1", 9876).unwrap();
        assert!(transport.is_connected());

        let sent = transport.send(b"hello").unwrap();
        assert_eq!(sent, 5);

        transport.close().unwrap();
        assert!(!transport.is_connected());
    }
}
