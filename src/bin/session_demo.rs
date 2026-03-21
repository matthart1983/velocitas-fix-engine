/// FIX Server + Client demo — demonstrates both startup modes.
///
/// 1. Starts a FixServer (acceptor) on a random port
/// 2. Connects two FixClient (initiator) sessions
/// 3. Each client sends a NewOrderSingle and receives an ExecutionReport
/// 4. Clean Logout on all sessions
///
/// Usage: cargo run --release --bin session_demo

use std::io;
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use velocitas_fix::client::{FixClient, FixClientConfig};
use velocitas_fix::engine::{EngineContext, FixApp};
use velocitas_fix::message::MessageView;
use velocitas_fix::serializer;
use velocitas_fix::server::{FixServer, FixServerConfig};
use velocitas_fix::tags;
use velocitas_fix::timestamp::{HrTimestamp, TimestampSource};

const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn main() {
    println!();
    println!("{BOLD}{CYAN}╔══════════════════════════════════════════════════════════════════╗{RESET}");
    println!("{BOLD}{CYAN}║       ⚡  VELOCITAS — SERVER + CLIENT SESSION DEMO              ║{RESET}");
    println!("{BOLD}{CYAN}╚══════════════════════════════════════════════════════════════════╝{RESET}");
    println!();

    // Find a free port
    let port = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };

    // Start server in background thread
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    thread::spawn(move || {
        let config = FixServerConfig {
            bind_address: "127.0.0.1".into(),
            port,
            sender_comp_id: "EXCHANGE".into(),
            allowed_comp_ids: vec![
                "CITADEL".into(),
                "JPMORGAN".into(),
                "GOLDMAN".into(),
            ],
            ..Default::default()
        };

        let server = FixServer::new(config);

        // Signal ready after binding
        ready_tx.send(()).unwrap();

        server.start(|| Box::new(ExchangeApp)).unwrap();
    });

    ready_rx.recv().unwrap();
    thread::sleep(Duration::from_millis(100)); // let the server bind

    println!("  {GREEN}▸{RESET} Server started on 127.0.0.1:{port}");
    println!("  {DIM}  Allowed: CITADEL, JPMORGAN, GOLDMAN{RESET}");
    println!();

    // Launch two clients concurrently
    let h1 = thread::spawn(move || {
        run_client(port, "CITADEL", "AAPL", 5000);
    });
    let h2 = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        run_client(port, "JPMORGAN", "MSFT", 3000);
    });

    h1.join().unwrap();
    h2.join().unwrap();

    println!();
    println!("{BOLD}{GREEN}  ✅  All sessions completed successfully!{RESET}");
    println!();

    // Server thread runs forever; exit the process
    std::process::exit(0);
}

fn run_client(port: u16, comp_id: &str, symbol: &str, qty: i64) {
    println!("  {GREEN}▸{RESET} [{comp_id}] Connecting to EXCHANGE...");

    let config = FixClientConfig {
        remote_host: "127.0.0.1".into(),
        remote_port: port,
        sender_comp_id: comp_id.into(),
        target_comp_id: "EXCHANGE".into(),
        ..Default::default()
    };

    let mut app = TraderApp {
        comp_id: comp_id.to_string(),
        symbol: symbol.to_string(),
        qty,
        done: false,
    };

    let client = FixClient::new(config);
    client.connect_and_run(&mut app).unwrap();
}

// ─────────────────────────────────────────────────────────────────────
// Exchange (server-side app) — responds to NOS with ExecutionReport
// ─────────────────────────────────────────────────────────────────────

struct ExchangeApp;

impl FixApp for ExchangeApp {
    fn on_message(&mut self, msg_type: &[u8], msg: &MessageView<'_>, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        if msg_type == b"D" {
            let cl_ord_id = msg.get_field(tags::CL_ORD_ID).unwrap_or(b"?");
            let symbol = msg.get_field(tags::SYMBOL).unwrap_or(b"?");
            let qty = msg.get_field_i64(tags::ORDER_QTY).unwrap_or(0);
            let from = msg.sender_comp_id().unwrap_or("?");

            println!("  {YELLOW}⏱{RESET}  [EXCHANGE] {BOLD}NOS{RESET} from {from}: {} {} @ mkt",
                std::str::from_utf8(symbol).unwrap_or("?"), qty);

            let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
            let seq = ctx.next_seq_num();
            let sender = ctx.session().config().sender_comp_id.clone();
            let target = ctx.session().config().target_comp_id.clone();
            let mut buf = [0u8; 2048];
            let len = serializer::build_execution_report(
                &mut buf, b"FIX.4.4", sender.as_bytes(), target.as_bytes(),
                seq, &ts, b"ORD-001", b"EXEC-001", cl_ord_id, symbol,
                b'1', qty, qty, b"178.55", 0, qty, b"178.55", b'F', b'2',
            );
            ctx.send_raw(&buf[..len])?;
        }
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────
// Trader (client-side app) — sends NOS, receives fill, logs out
// ─────────────────────────────────────────────────────────────────────

struct TraderApp {
    comp_id: String,
    symbol: String,
    qty: i64,
    done: bool,
}

impl FixApp for TraderApp {
    fn on_logon(&mut self, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        println!("  {GREEN}▸{RESET} [{}] {BOLD}Logon{RESET} — session Active", self.comp_id);

        let ts = HrTimestamp::now(TimestampSource::System).to_fix_timestamp();
        let seq = ctx.next_seq_num();
        let sender = ctx.session().config().sender_comp_id.clone();
        let target = ctx.session().config().target_comp_id.clone();
        let mut buf = [0u8; 1024];
        let len = serializer::build_new_order_single(
            &mut buf, b"FIX.4.4", sender.as_bytes(), target.as_bytes(),
            seq, &ts, b"ORD-001", self.symbol.as_bytes(),
            b'1', self.qty, b'2', b"178.55",
        );
        ctx.send_raw(&buf[..len])?;
        println!("  {GREEN}▸{RESET} [{}] Sent {BOLD}NOS{RESET}: {} Buy {} @ 178.55", self.comp_id, self.symbol, self.qty);

        Ok(())
    }

    fn on_message(&mut self, msg_type: &[u8], msg: &MessageView<'_>, ctx: &mut EngineContext<'_>) -> io::Result<()> {
        if msg_type == b"8" && !self.done {
            let cum_qty = msg.get_field_i64(tags::CUM_QTY).unwrap_or(0);
            println!("  {GREEN}▸{RESET} [{}] {BOLD}Fill{RESET}: CumQty={cum_qty} — sending Logout", self.comp_id);
            self.done = true;
            ctx.request_stop();
        }
        Ok(())
    }

    fn on_logout(&mut self) -> io::Result<()> {
        println!("  {GREEN}▸{RESET} [{}] {BOLD}Logout{RESET} complete", self.comp_id);
        Ok(())
    }
}
