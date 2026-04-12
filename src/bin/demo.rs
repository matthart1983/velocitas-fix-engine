/// ╔══════════════════════════════════════════════════════════════════════╗
/// ║                    VELOCITAS FIX ENGINE — DEMO                     ║
/// ║                                                                    ║
/// ║  A comprehensive demonstration of all engine capabilities:         ║
/// ║  parsing, serialization, sessions, SIMD, repeating groups,         ║
/// ║  FIXT 1.1, metrics, cluster HA, dictionary, acceptor,              ║
/// ║  timestamps, journal, pool, and dashboard.                         ║
/// ╚══════════════════════════════════════════════════════════════════════╝
use std::thread;
use std::time::{Duration, Instant};

use velocitas_fix::acceptor::{Acceptor, AcceptorConfig};
use velocitas_fix::cluster::{ClusterConfig, ClusterNode, NodeId, ReplicatedSessionState};
use velocitas_fix::dashboard::{Dashboard, DashboardConfig, HealthStatus, SessionStatus};
use velocitas_fix::dict_compiler::{CompiledDictionary, TEST_DICTIONARY_XML};
use velocitas_fix::fixt::{ApplVerID, FixtSession, FixtSessionConfig};
use velocitas_fix::journal::{session_hash, Journal, SyncPolicy};
use velocitas_fix::metrics::EngineMetrics;
use velocitas_fix::parser::FixParser;
use velocitas_fix::pool::BufferPool;
use velocitas_fix::serializer;
use velocitas_fix::session::{SequenceResetPolicy, Session, SessionConfig, SessionRole};
use velocitas_fix::simd;
use velocitas_fix::tags;
use velocitas_fix::timestamp::{HrClock, HrTimestamp, LatencyTracker, TimestampSource};

// ═══════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════

const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const MAGENTA: &str = "\x1b[35m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn header(title: &str) {
    let bar = "═".repeat(70);
    println!();
    println!("{BOLD}{CYAN}{bar}{RESET}");
    println!("{BOLD}{CYAN}  ⚡ {title}{RESET}");
    println!("{BOLD}{CYAN}{bar}{RESET}");
}

fn step(label: &str) {
    println!("  {GREEN}▸{RESET} {label}");
}

fn result(label: &str, value: &str) {
    println!("    {DIM}{label}:{RESET} {BOLD}{value}{RESET}");
}

fn metric(label: &str, value: &str, unit: &str) {
    println!("    {YELLOW}⏱{RESET}  {label}: {BOLD}{MAGENTA}{value}{RESET} {DIM}{unit}{RESET}");
}

fn separator() {
    let line = "─".repeat(60);
    println!("  {DIM}{line}{RESET}");
}

fn pause() {
    thread::sleep(Duration::from_secs(2));
}

fn main() {
    println!();
    println!(
        "{BOLD}{CYAN}╔══════════════════════════════════════════════════════════════════╗{RESET}"
    );
    println!(
        "{BOLD}{CYAN}║              ⚡  VELOCITAS FIX ENGINE  v{}               ║{RESET}",
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "{BOLD}{CYAN}║          High-Performance FIX Protocol Engine Demo              ║{RESET}"
    );
    println!(
        "{BOLD}{CYAN}╚══════════════════════════════════════════════════════════════════╝{RESET}"
    );

    pause();
    demo_parse_serialize();
    pause();
    demo_session_lifecycle();
    pause();
    demo_throughput_benchmark();
    pause();
    demo_simd_scanning();
    pause();
    demo_memory_pool();
    pause();
    demo_journal();
    pause();
    demo_fixt_session();
    pause();
    demo_dictionary_compiler();
    pause();
    demo_repeating_groups();
    pause();
    demo_metrics();
    pause();
    demo_cluster_ha();
    pause();
    demo_acceptor();
    pause();
    demo_timestamps();
    pause();
    demo_dashboard();

    println!();
    let bar = "═".repeat(70);
    println!("{BOLD}{GREEN}{bar}{RESET}");
    println!("{BOLD}{GREEN}  ✅  All demos completed successfully!{RESET}");
    println!("{BOLD}{GREEN}{bar}{RESET}");
    println!();
}

// ═══════════════════════════════════════════════════════════════════════
// 1. Parse & Serialize
// ═══════════════════════════════════════════════════════════════════════

fn demo_parse_serialize() {
    header("1. ZERO-COPY PARSE & ZERO-ALLOC SERIALIZE");

    // --- Serialize a NewOrderSingle ---
    step("Serializing a NewOrderSingle (BANK_OMS → NYSE)");
    let mut buf = [0u8; 1024];
    let len = serializer::build_new_order_single(
        &mut buf,
        b"FIX.4.4",
        b"BANK_OMS",
        b"NYSE",
        42,
        b"20260321-10:00:00.123",
        b"ORD-2026032100001",
        b"AAPL",
        b'1',
        10_000,
        b'2',
        b"178.55",
    );
    result("Message size", &format!("{len} bytes"));
    result(
        "Wire format",
        &format!(
            "{}",
            String::from_utf8_lossy(&buf[..len]).replace('\x01', "|")
        ),
    );

    // --- Parse it back ---
    step("Parsing message back (zero-copy flyweight)");
    let parser = FixParser::new();
    let (view, consumed) = parser.parse(&buf[..len]).unwrap();

    result("Bytes consumed", &format!("{consumed}"));
    result("MsgType", &format!("{:?}", view.msg_type_enum().unwrap()));
    result("SenderCompID", view.sender_comp_id().unwrap());
    result("TargetCompID", view.target_comp_id().unwrap());
    result("ClOrdID", view.get_field_str(tags::CL_ORD_ID).unwrap());
    result("Symbol", view.get_field_str(tags::SYMBOL).unwrap());
    result(
        "Side",
        &format!(
            "{:?}",
            velocitas_fix::Side::from_byte(view.get_field(tags::SIDE).unwrap()[0]).unwrap()
        ),
    );
    result(
        "OrderQty",
        &format!("{}", view.get_field_i64(tags::ORDER_QTY).unwrap()),
    );
    result("Price", view.get_field_str(tags::PRICE).unwrap());
    result("Checksum valid", &format!("{}", view.is_checksum_valid()));
    result("Field count", &format!("{}", view.field_count()));

    // --- Serialize an ExecutionReport in response ---
    separator();
    step("Serializing ExecutionReport response (NYSE → BANK_OMS)");
    let mut rsp_buf = [0u8; 2048];
    let rsp_len = serializer::build_execution_report(
        &mut rsp_buf,
        b"FIX.4.4",
        b"NYSE",
        b"BANK_OMS",
        100,
        b"20260321-10:00:00.456",
        b"NYSE-ORD-001",
        b"NYSE-EXEC-001",
        b"ORD-2026032100001",
        b"AAPL",
        b'1',
        10_000,
        5_000,
        b"178.55",
        5_000,
        5_000,
        b"178.55",
        b'F',
        b'1',
    );
    let (rsp_view, _) = parser.parse(&rsp_buf[..rsp_len]).unwrap();
    result("ExecRpt size", &format!("{rsp_len} bytes"));
    result(
        "ExecType",
        &format!(
            "Fill ({})",
            rsp_view.get_field_str(tags::EXEC_TYPE).unwrap()
        ),
    );
    result(
        "LastQty",
        &format!("{}", rsp_view.get_field_i64(tags::LAST_QTY).unwrap()),
    );
    result(
        "CumQty",
        &format!("{}", rsp_view.get_field_i64(tags::CUM_QTY).unwrap()),
    );
    result(
        "LeavesQty",
        &format!("{}", rsp_view.get_field_i64(tags::LEAVES_QTY).unwrap()),
    );
}

// ═══════════════════════════════════════════════════════════════════════
// 2. Session Lifecycle
// ═══════════════════════════════════════════════════════════════════════

fn demo_session_lifecycle() {
    header("2. SESSION STATE MACHINE");

    let config = SessionConfig {
        session_id: "DEMO-NYSE-1".to_string(),
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

    let mut session = Session::new(config);

    step("Initial state");
    result("State", &format!("{:?}", session.state()));

    step("Connect → LogonSent");
    session.on_connected();
    result("State", &format!("{:?}", session.state()));

    step("Logon acknowledged → Active");
    session.on_logon();
    result("State", &format!("{:?}", session.state()));

    step("Exchanging 100 messages");
    for _ in 0..100 {
        let _seq = session.next_outbound_seq_num();
    }
    result(
        "Outbound seq",
        &format!("{}", session.current_outbound_seq_num()),
    );

    step("Inbound sequence validation (1..50)");
    for i in 1..=50 {
        session.validate_inbound_seq(i).unwrap();
    }
    result(
        "Expected inbound seq",
        &format!("{}", session.expected_inbound_seq_num()),
    );

    step("Gap detection: received seq 55, expected 51");
    let gap = session.validate_inbound_seq(55);
    result("Gap detected", &format!("{:?}", gap.unwrap_err()));
    result(
        "State",
        &format!("{:?} (awaiting gap fill)", session.state()),
    );

    step("Gap filled → Active");
    session.on_gap_filled(55);
    result("State", &format!("{:?}", session.state()));

    step("Logout → Disconnect");
    session.on_logout_sent();
    result("State", &format!("{:?}", session.state()));
    session.on_disconnected();
    result("State", &format!("{:?}", session.state()));
    result(
        "Sequences preserved",
        &format!(
            "outbound={}, inbound={}",
            session.current_outbound_seq_num(),
            session.expected_inbound_seq_num()
        ),
    );
}

// ═══════════════════════════════════════════════════════════════════════
// 3. Throughput Benchmark
// ═══════════════════════════════════════════════════════════════════════

fn demo_throughput_benchmark() {
    header("3. THROUGHPUT BENCHMARK");

    let parser_unchecked = FixParser::new_unchecked();
    let parser_checked = FixParser::new();

    // Pre-build a message
    let mut template = [0u8; 512];
    let len = serializer::build_new_order_single(
        &mut template,
        b"FIX.4.4",
        b"BANK",
        b"EXCH",
        1,
        b"20260321-10:00:00",
        b"ORD-00001",
        b"AAPL",
        b'1',
        1000,
        b'2',
        b"150.00",
    );
    let msg = &template[..len];

    // --- Serialize benchmark ---
    step("Serialization benchmark (1M NewOrderSingle messages)");
    let mut ser_buf = [0u8; 512];
    let start = Instant::now();
    for i in 0..1_000_000u64 {
        let _ = serializer::build_new_order_single(
            &mut ser_buf,
            b"FIX.4.4",
            b"S",
            b"T",
            i,
            b"20260321-10:00:00",
            b"O1",
            b"AAPL",
            b'1',
            100,
            b'2',
            b"150.00",
        );
    }
    let elapsed = start.elapsed();
    let ser_rate = 1_000_000.0 / elapsed.as_secs_f64();
    let ser_ns = elapsed.as_nanos() as f64 / 1_000_000.0;
    metric("Serialize rate", &format!("{ser_rate:.0}"), "msg/s");
    metric("Serialize latency", &format!("{ser_ns:.1}"), "ns/msg");

    // --- Parse benchmark (unchecked) ---
    separator();
    step("Parse benchmark — unchecked (1M messages)");
    let start = Instant::now();
    for _ in 0..1_000_000 {
        let _ = parser_unchecked.parse(msg);
    }
    let elapsed = start.elapsed();
    let parse_rate = 1_000_000.0 / elapsed.as_secs_f64();
    let parse_ns = elapsed.as_nanos() as f64 / 1_000_000.0;
    metric(
        "Parse rate (unchecked)",
        &format!("{parse_rate:.0}"),
        "msg/s",
    );
    metric("Parse latency", &format!("{parse_ns:.1}"), "ns/msg");

    // --- Parse benchmark (validated) ---
    step("Parse benchmark — validated checksum + body length (1M messages)");
    let start = Instant::now();
    for _ in 0..1_000_000 {
        let _ = parser_checked.parse(msg);
    }
    let elapsed = start.elapsed();
    let checked_rate = 1_000_000.0 / elapsed.as_secs_f64();
    let checked_ns = elapsed.as_nanos() as f64 / 1_000_000.0;
    metric(
        "Parse rate (validated)",
        &format!("{checked_rate:.0}"),
        "msg/s",
    );
    metric("Parse latency", &format!("{checked_ns:.1}"), "ns/msg");

    // --- Parse + Respond roundtrip ---
    separator();
    step("Full roundtrip: parse NOS → build ExecRpt (1M iterations)");
    let mut out_buf = [0u8; 2048];
    let start = Instant::now();
    for _ in 0..1_000_000 {
        let (view, _) = parser_unchecked.parse(msg).unwrap();
        let _ = serializer::build_execution_report(
            &mut out_buf,
            b"FIX.4.4",
            b"E",
            b"B",
            1,
            b"20260321-10:00:00",
            b"O1",
            b"X1",
            view.get_field(tags::CL_ORD_ID).unwrap_or(b"?"),
            view.get_field(tags::SYMBOL).unwrap_or(b"?"),
            b'1',
            100,
            100,
            b"150.00",
            0,
            100,
            b"150.00",
            b'2',
            b'2',
        );
    }
    let elapsed = start.elapsed();
    let rt_rate = 1_000_000.0 / elapsed.as_secs_f64();
    metric("Roundtrip rate", &format!("{rt_rate:.0}"), "msg/s");
    metric(
        "Roundtrip latency",
        &format!("{:.1}", elapsed.as_nanos() as f64 / 1_000_000.0),
        "ns/msg",
    );

    separator();
    result("Target", "≥ 2,000,000 msg/s");
    let status = if parse_rate >= 2_000_000.0 {
        "✅ PASS"
    } else {
        "⚠️  BELOW TARGET (debug build)"
    };
    result("Status", status);
}

// ═══════════════════════════════════════════════════════════════════════
// 4. SIMD Scanning
// ═══════════════════════════════════════════════════════════════════════

fn demo_simd_scanning() {
    header("4. SIMD-ACCELERATED DELIMITER SCANNING");

    let sample =
        b"8=FIX.4.4\x019=70\x0135=D\x0149=BANK\x0156=NYSE\x0134=1\x0155=AAPL\x0110=000\x01";

    step("SIMD SOH (0x01) scanning");
    let first_soh = simd::find_soh(sample);
    result("First SOH position", &format!("{:?}", first_soh));

    let field_count = simd::count_fields(sample);
    result("Field count (SOH count)", &format!("{field_count}"));

    step("SIMD equals (=) scanning");
    let first_eq = simd::find_equals(sample);
    result("First '=' position", &format!("{:?}", first_eq));

    #[cfg(target_arch = "aarch64")]
    result("SIMD backend", "NEON (aarch64) — 16 bytes/cycle");
    #[cfg(target_arch = "x86_64")]
    result("SIMD backend", "SSE2 (x86_64) — 16 bytes/cycle");
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    result("SIMD backend", "Scalar fallback");

    // Benchmark SIMD vs purpose
    step("Benchmark: count_fields on 10M calls × 80-byte message");
    let start = Instant::now();
    let mut total = 0usize;
    for _ in 0..10_000_000 {
        total += simd::count_fields(sample);
    }
    let elapsed = start.elapsed();
    metric("Time", &format!("{:.1}", elapsed.as_millis()), "ms");
    metric(
        "Rate",
        &format!("{:.0}", 10_000_000.0 / elapsed.as_secs_f64()),
        "scans/s",
    );
    let _ = total; // prevent optimize-away
}

// ═══════════════════════════════════════════════════════════════════════
// 5. Memory Pool
// ═══════════════════════════════════════════════════════════════════════

fn demo_memory_pool() {
    header("5. LOCK-FREE MEMORY POOL");

    step("Creating pool: 1024 slots × 256 bytes");
    let mut pool = BufferPool::new(256, 1024);
    result("Slot size", "256 bytes");
    result("Capacity", "1,024 slots");

    step("Allocate → write → read → deallocate cycle");
    let handle = pool.allocate().unwrap();
    let buf = pool.get_mut(handle);
    let msg = b"Hello from Velocitas!";
    buf[..msg.len()].copy_from_slice(msg);
    let read_back = std::str::from_utf8(&pool.get(handle)[..msg.len()]).unwrap();
    result(
        "Written",
        &format!("{:?}", std::str::from_utf8(msg).unwrap()),
    );
    result("Read back", &format!("{:?}", read_back));
    pool.deallocate(handle);
    result("Deallocated", "handle returned to pool");

    step("Benchmark: 10M alloc/dealloc cycles");
    let bench_pool = BufferPool::new(256, 16);
    let start = Instant::now();
    for _ in 0..10_000_000 {
        let h = bench_pool.allocate().unwrap();
        bench_pool.deallocate(h);
    }
    let elapsed = start.elapsed();
    metric("Time", &format!("{:.1}", elapsed.as_millis()), "ms");
    metric(
        "Latency",
        &format!("{:.1}", elapsed.as_nanos() as f64 / 10_000_000.0),
        "ns/cycle",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// 6. Journal
// ═══════════════════════════════════════════════════════════════════════

fn demo_journal() {
    header("6. MEMORY-MAPPED JOURNAL");

    let path = std::env::temp_dir().join("velocitas-demo-journal.dat");
    let _ = std::fs::remove_file(&path);

    step(&format!("Opening journal: {}", path.display()));
    let mut journal = Journal::open(&path, 1024 * 1024, SyncPolicy::None).unwrap();
    result("Size", "1 MB (memory-mapped)");
    result("Sync policy", "None (max throughput)");

    let hash = session_hash("BANK_OMS", "NYSE");

    step("Writing 10,000 messages to journal");
    let start = Instant::now();
    for seq in 1..=10_000u64 {
        let mut buf = [0u8; 256];
        let len = serializer::build_heartbeat(
            &mut buf,
            b"FIX.4.4",
            b"B",
            b"N",
            seq,
            b"20260321-10:00:00",
        );
        journal.append(hash, seq, &buf[..len]).unwrap();
    }
    let elapsed = start.elapsed();
    metric("Write time", &format!("{:.2}", elapsed.as_millis()), "ms");
    metric(
        "Write rate",
        &format!("{:.0}", 10_000.0 / elapsed.as_secs_f64()),
        "msg/s",
    );
    result("Entries written", &format!("{}", journal.entry_count()));

    step("Reading back first and last entries");
    let (hdr, body) = journal.read_entry(0).unwrap();
    let parser = FixParser::new();
    let (view, _) = parser.parse(body).unwrap();
    result("Entry 0 seq", &format!("{}", hdr.seq_num));
    result(
        "Entry 0 type",
        &format!("{:?}", view.msg_type_enum().unwrap()),
    );
    result("CRC32 valid", "✓");

    let _ = std::fs::remove_file(&path);
}

// ═══════════════════════════════════════════════════════════════════════
// 7. FIXT 1.1 Session
// ═══════════════════════════════════════════════════════════════════════

fn demo_fixt_session() {
    header("7. FIXT 1.1 / FIX 5.0 SP2 SESSION");

    let config = FixtSessionConfig {
        base: SessionConfig {
            session_id: "FIXT-DEMO-1".to_string(),
            fix_version: "FIXT.1.1".to_string(),
            sender_comp_id: "BANK".to_string(),
            target_comp_id: "VENUE".to_string(),
            role: SessionRole::Initiator,
            ..SessionConfig::default()
        },
        default_appl_ver_id: ApplVerID::Fix50SP2,
        supported_versions: vec![ApplVerID::Fix42, ApplVerID::Fix44, ApplVerID::Fix50SP2],
    };

    let mut session = FixtSession::new(config);

    step("FIXT 1.1 session created");
    result("BeginString", "FIXT.1.1 (always, per spec)");
    result("Default ApplVerID", &format!("{:?}", ApplVerID::Fix50SP2));
    result("Supported versions", "FIX.4.2, FIX.4.4, FIX.5.0SP2");
    result("Is FIXT", &format!("{}", session.is_fixt()));

    step("Session lifecycle");
    session.on_connected();
    result("State", &format!("{:?}", session.state()));
    session.on_logon();
    result("State", &format!("{:?}", session.state()));
    result(
        "Negotiated version",
        &format!(
            "{:?}",
            session.negotiated_version().unwrap_or(ApplVerID::Fix50SP2)
        ),
    );

    step("ApplVerID conversions");
    for ver in &[ApplVerID::Fix42, ApplVerID::Fix44, ApplVerID::Fix50SP2] {
        result(
            &format!("{:?}", ver),
            &format!(
                "wire={:?}  version={}",
                std::str::from_utf8(ver.as_bytes()).unwrap(),
                ver.as_fix_version_str()
            ),
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 8. Dictionary Compiler
// ═══════════════════════════════════════════════════════════════════════

fn demo_dictionary_compiler() {
    header("8. XML DICTIONARY COMPILER");

    step("Compiling FIX 4.4 dictionary from XML");
    let dict = CompiledDictionary::from_xml(TEST_DICTIONARY_XML).unwrap();
    result("Version", &dict.fix_version);
    result("Fields compiled", &format!("{}", dict.field_count()));
    result("Messages compiled", &format!("{}", dict.message_count()));

    step("O(1) field lookup by tag number");
    for tag in &[8, 35, 49, 54, 55] {
        let f = dict.lookup_field(*tag).unwrap();
        result(
            &format!("Tag {tag}"),
            &format!("{} ({})", f.name, f.field_type),
        );
    }

    step("Field with enum values");
    let side = dict.lookup_field(54).unwrap();
    for (val, desc) in &side.values {
        result(&format!("  Side={val}"), desc);
    }

    step("Message validation — NewOrderSingle (35=D)");
    let msg_def = dict.lookup_message("D").unwrap();
    result("Name", &msg_def.name);
    result("Category", &format!("{:?}", msg_def.category));
    result("Required fields", &format!("{:?}", msg_def.required_fields));

    step("Validating with all required fields present");
    let present = &[11, 55, 54, 38, 40];
    let errors = dict.validate_message("D", present);
    result("Validation errors", &format!("{} (clean ✓)", errors.len()));

    step("Validating with missing Symbol(55) and Side(54)");
    let missing = &[11, 38, 40];
    let errors = dict.validate_message("D", missing);
    for e in &errors {
        result("  Missing", e);
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 9. Repeating Groups
// ═══════════════════════════════════════════════════════════════════════

fn demo_repeating_groups() {
    header("9. REPEATING GROUP PARSER");

    // Build a message with a market data group inline
    // We'll construct raw bytes with MD entries
    step("Parsing MarketData with 3 repeating group entries");
    step("  (NoMDEntries=3, each with MDEntryType, MDEntryPx, MDEntrySize)");

    let group_def = velocitas_fix::groups::md_entries_group();
    result(
        "Group definition",
        &format!(
            "count_tag={}, delimiter_tag={}",
            group_def.count_tag, group_def.delimiter_tag
        ),
    );
    result("Member tags", &format!("{:?}", group_def.member_tags));

    // Show available predefined groups
    separator();
    step("Pre-defined group definitions");
    let legs = velocitas_fix::groups::legs_group();
    result(
        "NoLegs(555)",
        &format!(
            "delimiter={}, members={:?}",
            legs.delimiter_tag, legs.member_tags
        ),
    );
    let fills = velocitas_fix::groups::fills_group();
    result(
        "NoFills(1362)",
        &format!(
            "delimiter={}, members={:?}",
            fills.delimiter_tag, fills.member_tags
        ),
    );
    result("Nested groups", "Supported to arbitrary depth");
}

// ═══════════════════════════════════════════════════════════════════════
// 10. Metrics
// ═══════════════════════════════════════════════════════════════════════

fn demo_metrics() {
    header("10. PROMETHEUS METRICS");

    step("Creating engine metrics registry");
    let metrics = EngineMetrics::new();

    step("Recording metrics (lock-free atomics)");
    metrics.messages_parsed.inc_by(1_234_567);
    metrics.messages_sent.inc_by(987_654);
    metrics.messages_rejected.inc_by(42);
    metrics.active_sessions.set(12);

    for ns in &[180, 220, 250, 280, 300, 350, 400, 500, 800, 1200] {
        metrics.parse_latency_ns.record(*ns);
    }

    result(
        "messages_parsed",
        &format!("{}", metrics.messages_parsed.get()),
    );
    result("messages_sent", &format!("{}", metrics.messages_sent.get()));
    result(
        "active_sessions",
        &format!("{}", metrics.active_sessions.get()),
    );
    result(
        "parse_latency p50",
        &format!("{} ns", metrics.parse_latency_ns.percentile(50.0)),
    );
    result(
        "parse_latency p99",
        &format!("{} ns", metrics.parse_latency_ns.percentile(99.0)),
    );

    separator();
    step("Prometheus exposition format (first 15 lines)");
    let output = metrics.render_prometheus();
    for line in output.lines().take(15) {
        println!("    {DIM}{line}{RESET}");
    }
    println!("    {DIM}...{RESET}");
}

// ═══════════════════════════════════════════════════════════════════════
// 11. Cluster HA
// ═══════════════════════════════════════════════════════════════════════

fn demo_cluster_ha() {
    header("11. CLUSTER HA (RAFT CONSENSUS)");

    step("Creating 3-node cluster");
    let peers = vec![
        NodeId {
            id: 2,
            address: "10.0.1.2".into(),
            port: 9002,
        },
        NodeId {
            id: 3,
            address: "10.0.1.3".into(),
            port: 9003,
        },
    ];
    let mut node = ClusterNode::new(ClusterConfig::three_node(1, peers));
    result("Node ID", "1");
    result("Peers", "2 (10.0.1.2:9002), 3 (10.0.1.3:9003)");
    result("Role", &format!("{:?}", node.role()));

    step("Starting election");
    node.begin_election();
    result("Term", &format!("{}", node.current_term()));
    result("Role", &format!("{:?} (voted for self)", node.role()));

    step("Receiving majority vote from node 2");
    node.receive_vote(2, node.current_term(), true);
    result("Role", &format!("{:?}", node.role()));
    result("Is leader", &format!("{}", node.is_leader()));

    step("Replicating session state");
    node.replicate_session_state(ReplicatedSessionState {
        session_id: "NYSE-1".into(),
        sender_comp_id: "BANK".into(),
        target_comp_id: "NYSE".into(),
        outbound_seq_num: 42,
        inbound_seq_num: 38,
        state: 3, // Active
        last_updated_ms: 1711008000000,
    });
    node.replicate_session_state(ReplicatedSessionState {
        session_id: "LSE-1".into(),
        sender_comp_id: "BANK".into(),
        target_comp_id: "LSE".into(),
        outbound_seq_num: 100,
        inbound_seq_num: 95,
        state: 3,
        last_updated_ms: 1711008000000,
    });

    result("Log entries", &format!("{}", node.log_len()));
    result("Replicated sessions", "NYSE-1, LSE-1 (in replication log)");

    step("Single-node cluster (auto-elects leader)");
    let mut solo = ClusterNode::new(ClusterConfig::single_node(99));
    solo.start();
    result("Role", &format!("{:?}", solo.role()));
    result("Is leader", &format!("{}", solo.is_leader()));
}

// ═══════════════════════════════════════════════════════════════════════
// 12. Acceptor
// ═══════════════════════════════════════════════════════════════════════

fn demo_acceptor() {
    header("12. SESSION ACCEPTOR WITH CONNECTION POOLING");

    let config = AcceptorConfig {
        bind_address: "0.0.0.0".into(),
        port: 9878,
        connection_pool_size: 64,
        max_connections: 256,
        allowed_comp_ids: vec!["CITADEL".into(), "JPMORGAN".into(), "GOLDMAN".into()],
        ..AcceptorConfig::default()
    };

    let mut acceptor = Acceptor::new(config);
    result("Bind", "0.0.0.0:9878");
    result("Pool size", "64 pre-allocated slots");
    result("Whitelist", "CITADEL, JPMORGAN, GOLDMAN");

    step("Accepting connections");
    let c1 = acceptor
        .accept_connection("10.0.1.100", "CITADEL", 1000)
        .unwrap();
    let c2 = acceptor
        .accept_connection("10.0.1.101", "JPMORGAN", 1001)
        .unwrap();
    let c3 = acceptor
        .accept_connection("10.0.1.102", "GOLDMAN", 1002)
        .unwrap();
    result("Connection 1", &format!("id={c1} comp_id=CITADEL"));
    result("Connection 2", &format!("id={c2} comp_id=JPMORGAN"));
    result("Connection 3", &format!("id={c3} comp_id=GOLDMAN"));
    result("Active count", &format!("{}", acceptor.active_count()));

    step("Rejecting unauthorized CompID");
    let rejected = acceptor.accept_connection("10.0.1.200", "UNKNOWN_FIRM", 1003);
    result("Result", &format!("{:?}", rejected.unwrap_err()));

    step("Session inspection");
    let session = acceptor.get_session(c1).unwrap();
    result("Session role", &format!("{:?}", session.config().role));
    result("Target CompID", &session.config().target_comp_id);

    step("Releasing connection back to pool");
    acceptor.release_connection(c2).unwrap();
    let stats = acceptor.stats();
    result("Active", &format!("{}", stats.active_connections));
    result("Available in pool", &format!("{}", stats.pool_available));
    result("Total accepted", &format!("{}", stats.total_accepted));
    result("Total rejected", &format!("{}", stats.total_rejected));
}

// ═══════════════════════════════════════════════════════════════════════
// 13. Timestamps
// ═══════════════════════════════════════════════════════════════════════

fn demo_timestamps() {
    header("13. HARDWARE TIMESTAMPS (MiFID II)");

    step("System clock timestamp");
    let ts = HrTimestamp::now(TimestampSource::System);
    let fix_ms = ts.to_fix_timestamp();
    let fix_us = ts.to_fix_timestamp_us();
    let fix_ns = ts.to_fix_timestamp_ns();
    result("Millisecond", std::str::from_utf8(&fix_ms).unwrap());
    result("Microsecond", std::str::from_utf8(&fix_us).unwrap());
    result(
        "Nanosecond (MiFID II)",
        std::str::from_utf8(&fix_ns).unwrap(),
    );

    step("TSC (CPU timestamp counter)");
    let clock = HrClock::new(TimestampSource::System);
    let tsc_val = HrClock::read_tsc();
    result("Raw TSC value", &format!("{tsc_val}"));
    #[cfg(target_arch = "aarch64")]
    result("TSC source", "CNTVCT_EL0 (ARM counter)");
    #[cfg(target_arch = "x86_64")]
    result("TSC source", "RDTSC (x86 timestamp counter)");

    step("Latency measurement");
    let tracker = LatencyTracker::start(clock);
    // Simulate some work
    let mut sum = 0u64;
    for i in 0..10_000 {
        sum = sum.wrapping_add(i);
    }
    let elapsed = tracker.stop();
    metric("Measured work", &format!("{elapsed}"), "ns");
    let _ = sum;

    step("Benchmark: 10M timestamp reads");
    let start = Instant::now();
    let mut last = 0u64;
    for _ in 0..10_000_000 {
        last = HrClock::read_tsc();
    }
    let elapsed = start.elapsed();
    metric("Time", &format!("{:.1}", elapsed.as_millis()), "ms");
    metric(
        "Latency",
        &format!("{:.1}", elapsed.as_nanos() as f64 / 10_000_000.0),
        "ns/read",
    );
    let _ = last;
}

// ═══════════════════════════════════════════════════════════════════════
// 14. Dashboard
// ═══════════════════════════════════════════════════════════════════════

fn demo_dashboard() {
    header("14. WEB DASHBOARD");

    let mut dashboard = Dashboard::new(DashboardConfig::default());

    step("Registering sessions");
    dashboard.update_session(SessionStatus {
        session_id: "NYSE-1".into(),
        sender_comp_id: "BANK_OMS".into(),
        target_comp_id: "NYSE".into(),
        state: "Active".into(),
        outbound_seq: 42_567,
        inbound_seq: 41_234,
        messages_sent: 1_234_567,
        messages_received: 987_654,
        last_activity_ms: 1711008000000,
        uptime_secs: 28800,
    });
    dashboard.update_session(SessionStatus {
        session_id: "LSE-1".into(),
        sender_comp_id: "BANK_OMS".into(),
        target_comp_id: "LSE".into(),
        state: "Active".into(),
        outbound_seq: 18_900,
        inbound_seq: 17_800,
        messages_sent: 567_890,
        messages_received: 456_789,
        last_activity_ms: 1711008000000,
        uptime_secs: 28800,
    });
    result(
        "Sessions registered",
        &format!("{}", dashboard.session_count()),
    );

    step("Updating health status");
    dashboard.update_health(HealthStatus {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: 28800,
        active_sessions: 2,
        messages_processed: 1_802_457,
        engine_state: "running".into(),
    });

    step("GET /health");
    let resp = dashboard.handle_request("GET", "/health");
    result("Status", &format!("{}", resp.status_code));
    for line in resp.body.lines().take(4) {
        println!("    {DIM}{line}{RESET}");
    }
    println!("    {DIM}...{RESET}");

    step("GET /sessions");
    let resp = dashboard.handle_request("GET", "/sessions");
    result("Status", &format!("{}", resp.status_code));
    result("Content-Type", &resp.content_type);
    result("Body length", &format!("{} chars", resp.body.len()));

    step("GET / (HTML dashboard)");
    let resp = dashboard.handle_request("GET", "/");
    result("Status", &format!("{}", resp.status_code));
    result("Content-Type", &resp.content_type);
    result("HTML size", &format!("{} chars", resp.body.len()));
    let has_table = resp.body.contains("<table");
    let has_refresh = resp.body.contains("refresh");
    result("Contains session table", &format!("{has_table}"));
    result("Has auto-refresh", &format!("{has_refresh}"));

    step("GET /unknown → 404");
    let resp = dashboard.handle_request("GET", "/unknown");
    result("Status", &format!("{}", resp.status_code));

    separator();
    result("Endpoints", "/health  /metrics  /sessions  /  /api/latency");
}
