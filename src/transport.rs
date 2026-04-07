/// Transport layer abstraction.
///
/// Provides a unified interface over the standard Aeron IPC integration path,
/// kernel TCP, and the kernel-bypass transports. The actual kernel-bypass
/// implementations require platform-specific dependencies and are behind
/// feature flags.
use std::io;

use crate::transport_aeron::AeronTransport;
use crate::transport_tcp::StdTcpTransport;

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
    pub aeron_channel: Option<String>,
    pub aeron_stream_id: i32,
    pub aeron_dir: Option<String>,
    pub aeron_embedded_driver: bool,
}

/// Transport mode selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    /// Standard Aeron IPC/UDP integration path.
    Aeron,
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
            mode: TransportMode::Aeron,
            bind_address: None,
            connect_address: None,
            port: 0,
            recv_buffer_size: 256 * 1024,
            send_buffer_size: 256 * 1024,
            tcp_nodelay: true,
            aeron_channel: Some("aeron:ipc".to_string()),
            aeron_stream_id: 1001,
            aeron_dir: Some(TransportConfig::default_aeron_dir()),
            aeron_embedded_driver: false,
        }
    }
}

impl TransportConfig {
    /// Shared Aeron directory used by the default independent media driver path.
    pub fn default_aeron_dir() -> String {
        std::env::temp_dir()
            .join("velocitas-fix-aeron")
            .display()
            .to_string()
    }

    /// Default internal integration path for colocated services.
    ///
    /// The provided stream id is the base request stream. The paired Aeron
    /// transport uses the next stream id for traffic in the opposite direction.
    /// This expects an independent media driver to already be running for the
    /// configured Aeron directory.
    pub fn aeron_ipc(stream_id: i32) -> Self {
        TransportConfig {
            aeron_stream_id: stream_id,
            ..Default::default()
        }
    }

    /// Explicitly opt into kernel TCP when talking to venues or counterparties.
    pub fn kernel_tcp() -> Self {
        TransportConfig {
            mode: TransportMode::KernelTcp,
            ..Default::default()
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

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn connect(&mut self, address: &str, port: u16) -> io::Result<()> {
        (**self).connect(address, port)
    }

    fn bind(&mut self, address: &str, port: u16) -> io::Result<()> {
        (**self).bind(address, port)
    }

    fn send(&mut self, data: &[u8]) -> io::Result<usize> {
        (**self).send(data)
    }

    fn recv(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        (**self).recv(buffer)
    }

    fn close(&mut self) -> io::Result<()> {
        (**self).close()
    }

    fn poll(&mut self) -> io::Result<Option<TransportEvent>> {
        (**self).poll()
    }

    fn is_connected(&self) -> bool {
        (**self).is_connected()
    }
}

/// Build the transport selected by `TransportConfig`.
pub fn build_transport(config: TransportConfig) -> io::Result<Box<dyn Transport>> {
    match config.mode {
        TransportMode::Aeron => Ok(Box::new(AeronTransport::new(config))),
        TransportMode::KernelTcp => Ok(Box::new(StdTcpTransport::new(config))),
        TransportMode::Dpdk => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "DPDK transport requires a specialized constructor",
        )),
        TransportMode::OpenOnload => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "OpenOnload transport requires a specialized constructor",
        )),
    }
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
    fn test_default_transport_is_aeron() {
        let config = TransportConfig::default();
        assert_eq!(config.mode, TransportMode::Aeron);
        assert_eq!(config.aeron_channel.as_deref(), Some("aeron:ipc"));
        assert_eq!(
            config.aeron_dir.as_deref(),
            Some(TransportConfig::default_aeron_dir().as_str())
        );
        assert!(!config.aeron_embedded_driver);
    }

    #[test]
    fn test_kernel_tcp_transport() {
        let config = TransportConfig::kernel_tcp();
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
