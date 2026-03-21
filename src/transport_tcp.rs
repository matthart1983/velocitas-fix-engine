use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::Duration;

use crate::transport::{Transport, TransportConfig, TransportEvent};

pub struct StdTcpTransport {
    stream: Option<TcpStream>,
    connected: bool,
    config: TransportConfig,
}

impl StdTcpTransport {
    pub fn new(config: TransportConfig) -> Self {
        StdTcpTransport {
            stream: None,
            connected: false,
            config,
        }
    }

    pub fn from_stream(stream: TcpStream, config: TransportConfig) -> io::Result<Self> {
        stream.set_nodelay(config.tcp_nodelay)?;
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        Ok(StdTcpTransport {
            stream: Some(stream),
            connected: true,
            config,
        })
    }
}

impl Transport for StdTcpTransport {
    fn connect(&mut self, address: &str, port: u16) -> io::Result<()> {
        let stream = TcpStream::connect((address, port))?;
        stream.set_nodelay(self.config.tcp_nodelay)?;
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        self.stream = Some(stream);
        self.connected = true;
        Ok(())
    }

    fn bind(&mut self, _address: &str, _port: u16) -> io::Result<()> {
        Ok(())
    }

    fn send(&mut self, data: &[u8]) -> io::Result<usize> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "not connected")
        })?;
        stream.write_all(data)?;
        Ok(data.len())
    }

    fn recv(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let stream = self.stream.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotConnected, "not connected")
        })?;
        match stream.read(buffer) {
            Ok(n) => Ok(n),
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
            {
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }

    fn close(&mut self) -> io::Result<()> {
        if let Some(stream) = self.stream.take() {
            let _ = stream.shutdown(Shutdown::Both);
        }
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
