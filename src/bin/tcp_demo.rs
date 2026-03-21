/// TCP FIX session demo — real initiator ↔ acceptor over localhost.
///
/// Demonstrates: TCP connect, Logon handshake, NewOrderSingle → ExecutionReport,
/// and clean Logout on both sides.
///
/// Usage: cargo run --release --bin tcp_demo

use std::io;
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use velocitas_fix::engine::{EngineContext, FixApp, FixEngine};
use velocitas_fix::message::MessageView;
use velocitas_fix::serializer;
use velocitas_fix::transport::Transport;
use velocitas_fix::session::{Session, SessionConfig, SessionRole, SequenceResetPolicy};
use velocitas_fix::tags;
use velocitas_fix::timestamp::{HrTimestamp, TimestampSource};
use velocitas_fix::transport::TransportConfig;
use velocitas_fix::transport_tcp::StdTcpTransport;

const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn main() {
    println!();
    println!("{BOLD}{CYAN}╔══════════════════════════════════════════════════════════════════╗{RESET}");
    println!("{BOLD}{CYAN}║         ⚡  VELOCITAS FIX ENGINE — TCP SESSION DEMO            ║{RESET}");
    println!("{BOLD}{CYAN}╚══════════════════════════════════════════════════════════════════╝{RESET}");
    println!();

    // Bind acceptor to a random port
    let listener = TcpListener::bind("127.0.0.1:0").expect("Failed to bind");
    let port = listener.local_addr().unwrap().port();
    println!("  {GREEN}▸{RESET} Acceptor listening on 127.0.0.1:{port}");

    let (tx, rx) = mpsc::channel();

    // Spawn acceptor thread
    let acceptor_handle = thread::spawn(move || {
        run_acceptor(listener, tx);
    });

    // Wait for acceptor to be ready
    rx.recv().unwrap();

    // Spawn initiator thread
    let initiator_handle = thread::spawn(move || {
        run_initiator(port);
    });

    initiator_handle.join().unwrap();
    acceptor_handle.join().unwrap();

    println!();
    println!("{BOLD}{GREEN}  ✅  TCP session demo completed successfully!{RESET}");
    println!();
}

// ─────────────────────────────────────────────────────────────────────
// Acceptor
// ─────────────────────────────────────────────────────────────────────

fn run_acceptor(listener: TcpListener, ready_tx: mpsc::Sender<()>) {
    ready_tx.send(()).unwrap();

    let (stream, remote) = listener.accept().expect("Accept failed");
    println!("  {GREEN}▸{RESET} Acceptor: connection from {remote}");

    // Read the first message (should be Logon)
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    stream.set_nodelay(true).unwrap();

    let transport = StdTcpTransport::from_stream(stream, TransportConfig::default())
        .expect("Failed to wrap stream");

    let session_config = SessionConfig {
        session_id: "ACC-DEMO".to_string(),
        fix_version: "FIX.4.4".to_string(),
        sender_comp_id: "NYSE".to_string(),
        target_comp_id: "BANK_OMS".to_string(),
        role: SessionRole::Acceptor,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(0),
        max_reconnect_attempts: 0,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: true,
        max_msg_rate: 50_000,
    };

    let session = Session::new(session_config);
    let mut engine = FixEngine::new_acceptor(transport, session);

    // Wait for inbound Logon, then respond
    let mut app = AcceptorApp;
    // We need to read the first logon manually, then run the engine
    // Actually the engine handles this — we just need to set up the session properly
    engine.handle_inbound_logon().expect("Failed to handle logon");
    println!("  {GREEN}▸{RESET} Acceptor: {BOLD}Logon sent{RESET} (session Active)");

    engine.run_acceptor(&mut app).expect("Acceptor engine error");
}

struct AcceptorApp;

impl FixApp for AcceptorApp {
    fn on_message(&mut self, msg_type: &[u8], msg: &MessageView<'_>, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        match msg_type {
            b"A" => {
                // Inbound logon from initiator — already handled
                println!("  {GREEN}▸{RESET} Acceptor: received {BOLD}Logon{RESET} from {}",
                    msg.sender_comp_id().unwrap_or("?"));
            }
            b"D" => {
                // NewOrderSingle — respond with ExecutionReport
                let cl_ord_id = msg.get_field_str(tags::CL_ORD_ID).unwrap_or("?");
                let symbol = msg.get_field_str(tags::SYMBOL).unwrap_or("?");
                let qty = msg.get_field_i64(tags::ORDER_QTY).unwrap_or(0);
                println!("  {YELLOW}⏱{RESET}  Acceptor: received {BOLD}NewOrderSingle{RESET} ClOrdID={cl_ord_id} Symbol={symbol} Qty={qty}");

                // Build and send ExecutionReport
                let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
                let seq = ctx.next_seq_num();
                let sender = ctx.session().config().sender_comp_id.clone();
                let target = ctx.session().config().target_comp_id.clone();
                let mut buf = [0u8; 2048];
                let len = serializer::build_execution_report(
                    &mut buf,
                    b"FIX.4.4",
                    sender.as_bytes(),
                    target.as_bytes(),
                    seq,
                    &ts,
                    b"NYSE-ORD-001",
                    b"NYSE-EXEC-001",
                    cl_ord_id.as_bytes(),
                    symbol.as_bytes(),
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
                println!("  {GREEN}▸{RESET} Acceptor: sent {BOLD}ExecutionReport{RESET} (Fill)");
            }
            _ => {
                println!("  {DIM}  Acceptor: received MsgType={}{RESET}",
                    std::str::from_utf8(msg_type).unwrap_or("?"));
            }
        }
        Ok(())
    }

    fn on_logout(&mut self) -> io::Result<()> {
        println!("  {GREEN}▸{RESET} Acceptor: received {BOLD}Logout{RESET}");
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
// Initiator
// ─────────────────────────────────────────────────────────────────────

fn run_initiator(port: u16) {
    // Small delay to let acceptor call accept()
    thread::sleep(Duration::from_millis(100));

    let mut transport = StdTcpTransport::new(TransportConfig::default());
    transport.connect("127.0.0.1", port).expect("Connect failed");
    println!("  {GREEN}▸{RESET} Initiator: connected to 127.0.0.1:{port}");

    let session_config = SessionConfig {
        session_id: "INIT-DEMO".to_string(),
        fix_version: "FIX.4.4".to_string(),
        sender_comp_id: "BANK_OMS".to_string(),
        target_comp_id: "NYSE".to_string(),
        role: SessionRole::Initiator,
        heartbeat_interval: Duration::from_secs(30),
        reconnect_interval: Duration::from_secs(1),
        max_reconnect_attempts: 3,
        sequence_reset_policy: SequenceResetPolicy::Daily,
        validate_comp_ids: true,
        max_msg_rate: 50_000,
    };

    let session = Session::new(session_config);
    let mut engine = FixEngine::new_initiator(transport, session);
    let mut app = InitiatorApp { done: false };

    engine.run_initiator(&mut app).expect("Initiator engine error");
}

struct InitiatorApp {
    done: bool,
}

impl FixApp for InitiatorApp {
    fn on_logon(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        println!("  {GREEN}▸{RESET} Initiator: {BOLD}Logon acknowledged{RESET} — session Active");

        // Send a NewOrderSingle
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
            b"ORD-2026032100001",
            b"AAPL",
            b'1',
            10_000,
            b'2',
            b"178.55",
        );
        ctx.send_raw(&buf[..len])?;
        println!("  {GREEN}▸{RESET} Initiator: sent {BOLD}NewOrderSingle{RESET} ClOrdID=ORD-2026032100001 AAPL Buy 10000 @ 178.55");

        Ok(())
    }

    fn on_message(&mut self, msg_type: &[u8], msg: &MessageView<'_>, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        match msg_type {
            b"8" => {
                // ExecutionReport received
                let exec_type = msg.get_field_str(tags::EXEC_TYPE).unwrap_or("?");
                let cum_qty = msg.get_field_i64(tags::CUM_QTY).unwrap_or(0);
                let leaves = msg.get_field_i64(tags::LEAVES_QTY).unwrap_or(0);
                println!("  {YELLOW}⏱{RESET}  Initiator: received {BOLD}ExecutionReport{RESET} ExecType={exec_type} CumQty={cum_qty} LeavesQty={leaves}");

                // Send Logout
                if !self.done {
                    self.done = true;
                    ctx.request_stop();
                }
            }
            _ => {
                println!("  {DIM}  Initiator: received MsgType={}{RESET}",
                    std::str::from_utf8(msg_type).unwrap_or("?"));
            }
        }
        Ok(())
    }

    fn on_logout(&mut self) -> io::Result<()> {
        println!("  {GREEN}▸{RESET} Initiator: received {BOLD}Logout{RESET}");
        Ok(())
    }
}
