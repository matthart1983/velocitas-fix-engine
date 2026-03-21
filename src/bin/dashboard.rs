/// Standalone HTTP server for the Velocitas FIX Engine dashboard.
///
/// Starts a TCP listener and serves the dashboard endpoints:
///   /         — HTML dashboard with auto-refresh
///   /health   — JSON health status
///   /metrics  — Prometheus text-format metrics
///   /sessions — JSON session list
///
/// Usage: cargo run --release --bin dashboard [-- --port 8080]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::time::Instant;

use velocitas_fix::dashboard::{Dashboard, DashboardConfig, HealthStatus, SessionStatus};

fn main() {
    let port = parse_port();
    let config = DashboardConfig {
        port,
        ..DashboardConfig::default()
    };

    let bind = format!("{}:{}", config.bind_address, config.port);

    let mut dashboard = Dashboard::new(config);
    seed_demo_data(&mut dashboard);

    let start = Instant::now();

    let listener = TcpListener::bind(&bind).unwrap_or_else(|e| {
        eprintln!("Failed to bind to {bind}: {e}");
        std::process::exit(1);
    });

    println!("⚡ Velocitas FIX Engine Dashboard");
    println!("   Listening on http://{bind}");
    println!("   Endpoints: /  /health  /metrics  /sessions  /api/latency");
    println!("   Press Ctrl+C to stop");
    println!();

    for stream in listener.incoming() {
        let mut stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Accept error: {e}");
                continue;
            }
        };

        // Update uptime on each request, preserving existing health fields.
        let mut health = dashboard.health();
        health.uptime_secs = start.elapsed().as_secs();
        dashboard.update_health(health);

        let reader = BufReader::new(&stream);
        let request_line = match reader.lines().next() {
            Some(Ok(line)) => line,
            _ => continue,
        };

        let (method, path) = match parse_request_line(&request_line) {
            Some(v) => v,
            None => continue,
        };

        let resp = dashboard.handle_request(&method, &path);

        let status_text = match resp.status_code {
            200 => "OK",
            404 => "Not Found",
            405 => "Method Not Allowed",
            _ => "Unknown",
        };

        let header = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            resp.status_code,
            status_text,
            resp.content_type,
            resp.body.len(),
        );

        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(resp.body.as_bytes());
        let _ = stream.flush();

        println!("  {} {} → {}", method, path, resp.status_code);
    }
}

fn parse_port() -> u16 {
    let args: Vec<String> = std::env::args().collect();
    for i in 0..args.len() {
        if args[i] == "--port" {
            if let Some(p) = args.get(i + 1) {
                return p.parse().unwrap_or_else(|_| {
                    eprintln!("Invalid port: {p}");
                    std::process::exit(1);
                });
            }
        }
    }
    8080
}

fn parse_request_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    Some((method, path))
}

fn seed_demo_data(dashboard: &mut Dashboard) {
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

    dashboard.update_health(HealthStatus {
        healthy: true,
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_secs: 0,
        active_sessions: 2,
        messages_processed: 1_802_457,
        engine_state: "running".to_string(),
    });
}
