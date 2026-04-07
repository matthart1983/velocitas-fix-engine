use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::aeron_c::{
    AeronError, AeronErrorKind, Client as AeronClient, EmbeddedMediaDriver, Publication,
    Subscription,
};
use crate::aeron_sbe::{
    decode_fix_payload_from_sbe, encode_fix_payload_as_sbe, FIX_AERON_ENVELOPE_OVERHEAD,
};
use crate::transport::{Transport, TransportConfig, TransportEvent};

const RESOURCE_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_RETRY_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_FRAGMENT_LIMIT: usize = 8;

static DEFAULT_EMBEDDED_DIR: OnceLock<String> = OnceLock::new();

fn default_embedded_dir() -> String {
    DEFAULT_EMBEDDED_DIR
        .get_or_init(|| {
            std::env::temp_dir()
                .join(format!("velocitas-fix-aeron-{}", std::process::id()))
                .display()
                .to_string()
        })
        .clone()
}

fn aeron_context_error(context: &str, error: &AeronError) -> io::Error {
    let kind = match error.kind() {
        AeronErrorKind::ClientErrorDriverTimeout
        | AeronErrorKind::ClientErrorClientTimeout
        | AeronErrorKind::ClientErrorConductorServiceTimeout
        | AeronErrorKind::TimedOut => io::ErrorKind::TimedOut,
        AeronErrorKind::ClientErrorBufferFull
        | AeronErrorKind::PublicationBackPressured
        | AeronErrorKind::PublicationAdminAction => io::ErrorKind::WouldBlock,
        AeronErrorKind::PublicationClosed => io::ErrorKind::BrokenPipe,
        _ => io::ErrorKind::Other,
    };

    io::Error::new(kind, format!("{context}: {error}"))
}

fn aeron_offer_error(context: &str, code: i32) -> io::Error {
    let kind = AeronErrorKind::from_code(code);
    let io_kind = match kind {
        AeronErrorKind::PublicationBackPressured
        | AeronErrorKind::PublicationAdminAction
        | AeronErrorKind::ClientErrorBufferFull => io::ErrorKind::WouldBlock,
        AeronErrorKind::PublicationClosed => io::ErrorKind::BrokenPipe,
        AeronErrorKind::TimedOut => io::ErrorKind::TimedOut,
        _ => io::ErrorKind::Other,
    };

    io::Error::new(io_kind, format!("{context}: {} ({code})", kind.to_string()))
}

struct AeronEndpoint {
    _embedded_driver: Option<Arc<EmbeddedMediaDriver>>,
    _client: AeronClient,
    publication: Publication,
    subscription: Subscription,
}

#[derive(Clone, Copy)]
enum AeronEndpointRole {
    Initiator,
    Acceptor,
}

/// Aeron transport backed by direct bindings to Aeron's official C client API.
///
/// The transport keeps a single configured Aeron channel and derives a paired
/// response stream so initiator and acceptor can exchange FIX messages over one
/// Aeron channel without socket semantics.
pub struct AeronTransport {
    connected: bool,
    config: TransportConfig,
    endpoint: Option<AeronEndpoint>,
    recv_queue: VecDeque<Vec<u8>>,
    send_frame_buf: Vec<u8>,
}

impl AeronTransport {
    pub fn new(config: TransportConfig) -> Self {
        Self {
            connected: false,
            send_frame_buf: Vec::with_capacity(
                config.send_buffer_size + FIX_AERON_ENVELOPE_OVERHEAD,
            ),
            config,
            endpoint: None,
            recv_queue: VecDeque::new(),
        }
    }

    fn endpoint(&self) -> io::Result<&AeronEndpoint> {
        self.endpoint.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "Aeron transport is not connected",
            )
        })
    }

    fn resolve_channel(&self, address: &str, port: u16) -> String {
        self.config
            .aeron_channel
            .clone()
            .unwrap_or_else(|| format!("aeron:udp?endpoint={address}:{port}"))
    }

    fn resolve_dir(&self) -> Option<String> {
        self.config
            .aeron_dir
            .clone()
            .or_else(|| self.config.aeron_embedded_driver.then(default_embedded_dir))
    }

    fn open_client(&self, dir: Option<&str>) -> io::Result<AeronClient> {
        AeronClient::connect(dir)
            .map_err(|error| aeron_context_error("failed to create Aeron client", &error))
    }

    fn stream_pair(&self, role: AeronEndpointRole) -> io::Result<(i32, i32)> {
        let base_stream = self.config.aeron_stream_id;
        let reply_stream = base_stream.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Aeron stream id is too large to derive a reply stream",
            )
        })?;

        Ok(match role {
            AeronEndpointRole::Initiator => (base_stream, reply_stream),
            AeronEndpointRole::Acceptor => (reply_stream, base_stream),
        })
    }

    fn open(&mut self, address: &str, port: u16, role: AeronEndpointRole) -> io::Result<()> {
        let channel = self.resolve_channel(address, port);
        let (publication_stream_id, subscription_stream_id) = self.stream_pair(role)?;
        let dir = self.resolve_dir();
        let embedded_driver = if self.config.aeron_embedded_driver {
            let dir = dir.as_deref().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "embedded Aeron driver requires a directory",
                )
            })?;
            Some(
                EmbeddedMediaDriver::shared(dir, self.config.aeron_dir.is_none()).map_err(
                    |error| aeron_context_error("failed to start embedded Aeron driver", &error),
                )?,
            )
        } else {
            None
        };

        let client = self.open_client(dir.as_deref())?;
        let publication = client
            .add_publication(&channel, publication_stream_id, RESOURCE_TIMEOUT)
            .map_err(|error| aeron_context_error("failed to create Aeron publication", &error))?;
        let subscription = client
            .add_subscription(&channel, subscription_stream_id, RESOURCE_TIMEOUT)
            .map_err(|error| aeron_context_error("failed to create Aeron subscription", &error))?;

        self.endpoint = Some(AeronEndpoint {
            _embedded_driver: embedded_driver,
            _client: client,
            publication,
            subscription,
        });
        self.recv_queue.clear();
        self.connected = true;
        Ok(())
    }

    fn poll_subscription(&mut self, fragment_limit: usize) -> io::Result<()> {
        let subscription = self.endpoint()?.subscription.clone();
        let mut inbound = Vec::new();
        let mut decode_error = None;

        subscription
            .poll_once(fragment_limit, |msg| {
                match decode_fix_payload_from_sbe(msg) {
                    Ok(envelope) => {
                        let _ = envelope.base_stream_id;
                        inbound.push(envelope.payload.to_vec());
                    }
                    Err(error) => {
                        if decode_error.is_none() {
                            decode_error = Some(error);
                        }
                    }
                }
            })
            .map_err(|error| aeron_context_error("failed to poll Aeron subscription", &error))?;

        if let Some(error) = decode_error {
            return Err(error);
        }

        self.recv_queue.extend(inbound);
        Ok(())
    }
}

impl Transport for AeronTransport {
    fn connect(&mut self, address: &str, port: u16) -> io::Result<()> {
        self.open(address, port, AeronEndpointRole::Initiator)
    }

    fn bind(&mut self, address: &str, port: u16) -> io::Result<()> {
        self.open(address, port, AeronEndpointRole::Acceptor)
    }

    fn send(&mut self, data: &[u8]) -> io::Result<usize> {
        let publication = self.endpoint()?.publication.clone();
        let encoded_len =
            encode_fix_payload_as_sbe(&mut self.send_frame_buf, self.config.aeron_stream_id, data)?;
        let frame = &self.send_frame_buf[..encoded_len];
        let start = Instant::now();

        loop {
            let result = publication.offer(frame);
            if result >= 0 {
                return Ok(data.len());
            }

            let error_kind = AeronErrorKind::from_code(result as i32);
            if error_kind.is_back_pressured_or_admin_action()
                && start.elapsed() < SEND_RETRY_TIMEOUT
            {
                thread::yield_now();
                continue;
            }

            return Err(aeron_offer_error(
                "failed to publish SBE FIX envelope to Aeron channel",
                result as i32,
            ));
        }
    }

    fn recv(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.recv_queue.is_empty() {
            self.poll_subscription(POLL_FRAGMENT_LIMIT)?;
        }

        let Some(frame) = self.recv_queue.pop_front() else {
            return Ok(0);
        };

        if frame.len() > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "receive buffer too small for Aeron frame",
            ));
        }

        buffer[..frame.len()].copy_from_slice(&frame);
        Ok(frame.len())
    }

    fn close(&mut self) -> io::Result<()> {
        self.endpoint = None;
        self.recv_queue.clear();
        self.connected = false;
        Ok(())
    }

    fn poll(&mut self) -> io::Result<Option<TransportEvent>> {
        if self.recv_queue.is_empty() {
            self.poll_subscription(POLL_FRAGMENT_LIMIT)?;
        }

        Ok(self
            .recv_queue
            .front()
            .map(|frame| TransportEvent::DataReceived(frame.len())))
    }

    fn is_connected(&self) -> bool {
        self.connected
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    static NEXT_STREAM_ID: AtomicI32 = AtomicI32::new(20_000);

    fn test_config() -> (TransportConfig, Arc<EmbeddedMediaDriver>) {
        let stream_id = NEXT_STREAM_ID.fetch_add(2, Ordering::Relaxed);
        let aeron_dir = std::env::temp_dir()
            .join(format!("velocitas-fix-transport-test-{stream_id}"))
            .display()
            .to_string();
        let driver = EmbeddedMediaDriver::shared(&aeron_dir, true).unwrap();

        (
            TransportConfig {
                aeron_stream_id: stream_id,
                aeron_dir: Some(aeron_dir),
                aeron_embedded_driver: false,
                ..TransportConfig::default()
            },
            driver,
        )
    }

    fn recv_with_timeout(transport: &mut AeronTransport, buf: &mut [u8]) -> usize {
        let start = Instant::now();
        loop {
            let n = transport.recv(buf).unwrap();
            if n > 0 {
                return n;
            }

            assert!(
                start.elapsed() < Duration::from_secs(2),
                "timed out waiting for Aeron frame"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn test_aeron_transport_round_trip() {
        let (config, _driver) = test_config();
        let mut initiator = AeronTransport::new(config.clone());
        let mut acceptor = AeronTransport::new(config);

        initiator.connect("127.0.0.1", 0).unwrap();
        acceptor.bind("127.0.0.1", 0).unwrap();

        initiator
            .send(b"8=FIX.4.4\x019=5\x0135=0\x0110=000\x01")
            .unwrap();

        let mut buf = [0u8; 128];
        let n = recv_with_timeout(&mut acceptor, &mut buf);
        assert_eq!(&buf[..n], b"8=FIX.4.4\x019=5\x0135=0\x0110=000\x01");
    }

    #[test]
    fn test_aeron_transport_does_not_receive_own_frames() {
        let (config, _driver) = test_config();
        let mut initiator = AeronTransport::new(config.clone());
        let mut acceptor = AeronTransport::new(config);

        initiator.connect("127.0.0.1", 0).unwrap();
        acceptor.bind("127.0.0.1", 0).unwrap();

        initiator.send(b"ping").unwrap();

        let mut buf = [0u8; 16];
        assert_eq!(initiator.recv(&mut buf).unwrap(), 0);
        assert_eq!(recv_with_timeout(&mut acceptor, &mut buf), 4);
    }

    #[test]
    fn test_aeron_transport_uses_explicit_channel() {
        let stream_id = NEXT_STREAM_ID.fetch_add(2, Ordering::Relaxed);
        let aeron_dir = std::env::temp_dir()
            .join(format!("velocitas-fix-transport-test-{stream_id}"))
            .display()
            .to_string();
        let _driver = EmbeddedMediaDriver::shared(&aeron_dir, true).unwrap();
        let config = TransportConfig {
            aeron_channel: Some(format!("aeron:ipc?alias=velocitas-fix-{stream_id}")),
            aeron_stream_id: stream_id,
            aeron_dir: Some(aeron_dir),
            aeron_embedded_driver: false,
            ..TransportConfig::default()
        };
        let mut initiator = AeronTransport::new(config.clone());
        let mut acceptor = AeronTransport::new(config);

        initiator.connect("127.0.0.1", 0).unwrap();
        acceptor.bind("127.0.0.1", 0).unwrap();

        initiator.send(b"explicit-channel").unwrap();

        let mut buf = [0u8; 32];
        let n = recv_with_timeout(&mut acceptor, &mut buf);
        assert_eq!(&buf[..n], b"explicit-channel");
    }
}
