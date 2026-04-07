use std::io;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use velocitas_fix::engine::{EngineContext, FixApp, FixEngine};
use velocitas_fix::message::MessageView;
use velocitas_fix::serializer;
use velocitas_fix::session::{SequenceResetPolicy, Session, SessionConfig, SessionRole};
use velocitas_fix::tags;
use velocitas_fix::timestamp::{HrTimestamp, TimestampSource};
use velocitas_fix::transport::{build_transport, TransportConfig};

static NEXT_STREAM_ID: AtomicI32 = AtomicI32::new(30_000);

#[test]
fn test_aeron_is_the_default_end_to_end_integration_path() {
    let stream_id = NEXT_STREAM_ID.fetch_add(2, Ordering::Relaxed);
    let aeron_dir = std::env::temp_dir().join(format!("velocitas-fix-test-{stream_id}"));
    let ready_file = std::env::temp_dir().join(format!("velocitas-fix-test-ready-{stream_id}"));
    let _ = std::fs::remove_dir_all(&aeron_dir);
    let _ = std::fs::remove_file(&ready_file);

    let mut driver = spawn_media_driver(&aeron_dir, &ready_file);
    wait_for_file(&ready_file, Duration::from_secs(5));

    let (tx, rx) = mpsc::channel();

    let aeron_dir_for_acceptor = aeron_dir.display().to_string();
    let acceptor = thread::spawn(move || -> io::Result<()> {
        let mut transport = build_transport(TransportConfig {
            aeron_stream_id: stream_id,
            aeron_dir: Some(aeron_dir_for_acceptor),
            ..TransportConfig::default()
        })?;
        transport.bind("127.0.0.1", 0)?;

        let session = Session::new(SessionConfig {
            session_id: "AERON-ACC-TEST".into(),
            fix_version: "FIX.4.4".into(),
            sender_comp_id: "EXCHANGE".into(),
            target_comp_id: "BANK_OMS".into(),
            role: SessionRole::Acceptor,
            heartbeat_interval: Duration::from_secs(30),
            reconnect_interval: Duration::ZERO,
            max_reconnect_attempts: 0,
            sequence_reset_policy: SequenceResetPolicy::Daily,
            validate_comp_ids: true,
            max_msg_rate: 50_000,
        });

        let mut engine = FixEngine::new_acceptor(transport, session);
        let mut app = AcceptorApp;
        engine.run_acceptor(&mut app)
    });

    let aeron_dir_for_initiator = aeron_dir.display().to_string();
    let initiator = thread::spawn(move || -> io::Result<()> {
        thread::sleep(Duration::from_millis(10));

        let config = TransportConfig {
            aeron_stream_id: stream_id,
            aeron_dir: Some(aeron_dir_for_initiator),
            ..TransportConfig::default()
        };
        let mut transport = build_transport(config)?;
        transport.connect("127.0.0.1", 0)?;

        let session = Session::new(SessionConfig {
            session_id: "AERON-INIT-TEST".into(),
            fix_version: "FIX.4.4".into(),
            sender_comp_id: "BANK_OMS".into(),
            target_comp_id: "EXCHANGE".into(),
            role: SessionRole::Initiator,
            heartbeat_interval: Duration::from_secs(30),
            reconnect_interval: Duration::from_secs(1),
            max_reconnect_attempts: 3,
            sequence_reset_policy: SequenceResetPolicy::Daily,
            validate_comp_ids: true,
            max_msg_rate: 50_000,
        });

        let mut engine = FixEngine::new_initiator(transport, session);
        let mut app = InitiatorApp { done_tx: tx };
        engine.run_initiator(&mut app)
    });

    let cl_ord_id = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("timed out waiting for execution report");
    assert_eq!(cl_ord_id, "ORD-AERON-TEST");

    initiator.join().unwrap().unwrap();
    acceptor.join().unwrap().unwrap();

    stop_child(&mut driver);
    let _ = std::fs::remove_dir_all(&aeron_dir);
    let _ = std::fs::remove_file(&ready_file);
}

fn spawn_media_driver(aeron_dir: &std::path::Path, ready_file: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_aeron_media_driver"))
        .arg("--aeron-dir")
        .arg(aeron_dir)
        .arg("--ready-file")
        .arg(ready_file)
        .arg("--delete-dir-on-exit")
        .spawn()
        .expect("failed to start standalone Aeron media driver")
}

fn wait_for_file(path: &std::path::Path, timeout: Duration) {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if path.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("timed out waiting for ready file {}", path.display());
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

struct AcceptorApp;

impl FixApp for AcceptorApp {
    fn on_message(
        &mut self,
        msg_type: &[u8],
        msg: &MessageView<'_>,
        ctx: &mut EngineContext<'_>,
    ) -> io::Result<()> {
        if msg_type == b"D" {
            let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
            let seq = ctx.next_seq_num();
            let sender = ctx.session().config().sender_comp_id.clone();
            let target = ctx.session().config().target_comp_id.clone();
            let cl_ord_id = msg.get_field(tags::CL_ORD_ID).unwrap_or(b"?");
            let symbol = msg.get_field(tags::SYMBOL).unwrap_or(b"?");
            let qty = msg.get_field_i64(tags::ORDER_QTY).unwrap_or(100);

            let mut buf = [0u8; 2048];
            let len = serializer::build_execution_report(
                &mut buf,
                b"FIX.4.4",
                sender.as_bytes(),
                target.as_bytes(),
                seq,
                &ts,
                b"ORD-1",
                b"EXEC-1",
                cl_ord_id,
                symbol,
                b'1',
                qty,
                qty,
                b"178.55",
                0,
                qty,
                b"178.55",
                b'F',
                b'2',
            );
            ctx.send_raw(&buf[..len])?;
        }
        Ok(())
    }
}

struct InitiatorApp {
    done_tx: mpsc::Sender<String>,
}

impl FixApp for InitiatorApp {
    fn on_logon(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
        let seq = ctx.next_seq_num();
        let sender = ctx.session().config().sender_comp_id.clone();
        let target = ctx.session().config().target_comp_id.clone();

        let mut buf = [0u8; 1024];
        let len = serializer::build_new_order_single(
            &mut buf,
            b"FIX.4.4",
            sender.as_bytes(),
            target.as_bytes(),
            seq,
            &ts,
            b"ORD-AERON-TEST",
            b"AAPL",
            b'1',
            100,
            b'2',
            b"178.55",
        );
        ctx.send_raw(&buf[..len])?;
        Ok(())
    }

    fn on_message(
        &mut self,
        msg_type: &[u8],
        msg: &MessageView<'_>,
        ctx: &mut EngineContext<'_>,
    ) -> io::Result<()> {
        if msg_type == b"8" {
            let cl_ord_id = msg
                .get_field_str(tags::CL_ORD_ID)
                .unwrap_or("?")
                .to_string();
            self.done_tx.send(cl_ord_id).unwrap();
            ctx.request_stop();
        }
        Ok(())
    }
}
