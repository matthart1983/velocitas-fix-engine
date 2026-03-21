/// Lightweight web dashboard for the Velocitas FIX engine.
///
/// Serves real-time metrics, session status, and health checks via HTTP.
/// No external dependencies — generates JSON and HTML by hand.

use std::fmt::Write;

// ---------------------------------------------------------------------------
// DashboardConfig
// ---------------------------------------------------------------------------

/// Dashboard configuration.
#[derive(Debug, Clone)]
pub struct DashboardConfig {
    pub bind_address: String,
    pub port: u16,
    pub refresh_interval_ms: u64,
    pub enable_metrics_endpoint: bool,
    pub enable_sessions_endpoint: bool,
    pub enable_health_endpoint: bool,
}

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".to_string(),
            port: 8080,
            refresh_interval_ms: 5000,
            enable_metrics_endpoint: true,
            enable_sessions_endpoint: true,
            enable_health_endpoint: true,
        }
    }
}

// ---------------------------------------------------------------------------
// SessionStatus
// ---------------------------------------------------------------------------

/// Session status for dashboard display.
#[derive(Debug, Clone)]
pub struct SessionStatus {
    pub session_id: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub state: String,
    pub outbound_seq: u64,
    pub inbound_seq: u64,
    pub messages_sent: u64,
    pub messages_received: u64,
    pub last_activity_ms: u64,
    pub uptime_secs: u64,
}

// ---------------------------------------------------------------------------
// HealthStatus
// ---------------------------------------------------------------------------

/// Health check response.
#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub healthy: bool,
    pub version: String,
    pub uptime_secs: u64,
    pub active_sessions: usize,
    pub messages_processed: u64,
    pub engine_state: String,
}

impl Default for HealthStatus {
    fn default() -> Self {
        Self {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            uptime_secs: 0,
            active_sessions: 0,
            messages_processed: 0,
            engine_state: "starting".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// HttpResponse
// ---------------------------------------------------------------------------

/// Dashboard HTTP response.
#[derive(Debug)]
pub struct HttpResponse {
    pub status_code: u16,
    pub content_type: String,
    pub body: String,
}

impl HttpResponse {
    fn json(body: String) -> Self {
        Self {
            status_code: 200,
            content_type: "application/json".to_string(),
            body,
        }
    }

    fn html(body: String) -> Self {
        Self {
            status_code: 200,
            content_type: "text/html; charset=utf-8".to_string(),
            body,
        }
    }

    fn text(body: String) -> Self {
        Self {
            status_code: 200,
            content_type: "text/plain; charset=utf-8".to_string(),
            body,
        }
    }

    fn not_found() -> Self {
        Self {
            status_code: 404,
            content_type: "text/plain".to_string(),
            body: "404 Not Found".to_string(),
        }
    }

    fn method_not_allowed() -> Self {
        Self {
            status_code: 405,
            content_type: "text/plain".to_string(),
            body: "405 Method Not Allowed".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

/// Dashboard server — lightweight HTTP endpoint for monitoring.
pub struct Dashboard {
    config: DashboardConfig,
    sessions: Vec<SessionStatus>,
    health: HealthStatus,
    _start_time_ms: u64,
}

impl Dashboard {
    /// Create a new dashboard with the given configuration.
    pub fn new(config: DashboardConfig) -> Self {
        let start_time_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            config,
            sessions: Vec::new(),
            health: HealthStatus::default(),
            _start_time_ms: start_time_ms,
        }
    }

    /// Add or update a session status. Updates in-place if session_id matches.
    pub fn update_session(&mut self, status: SessionStatus) {
        if let Some(existing) = self
            .sessions
            .iter_mut()
            .find(|s| s.session_id == status.session_id)
        {
            *existing = status;
        } else {
            self.sessions.push(status);
        }
    }

    /// Remove a session by ID.
    pub fn remove_session(&mut self, session_id: &str) {
        self.sessions.retain(|s| s.session_id != session_id);
    }

    /// Update the health status.
    pub fn update_health(&mut self, health: HealthStatus) {
        self.health = health;
    }

    /// Return the current number of tracked sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Route an HTTP request and return a response.
    pub fn handle_request(&self, method: &str, path: &str) -> HttpResponse {
        if method != "GET" {
            return HttpResponse::method_not_allowed();
        }

        match path {
            "/health" => {
                if self.config.enable_health_endpoint {
                    HttpResponse::json(self.render_json_health())
                } else {
                    HttpResponse::not_found()
                }
            }
            "/metrics" => {
                if self.config.enable_metrics_endpoint {
                    HttpResponse::text(self.render_prometheus_metrics())
                } else {
                    HttpResponse::not_found()
                }
            }
            "/sessions" => {
                if self.config.enable_sessions_endpoint {
                    HttpResponse::json(self.render_json_sessions())
                } else {
                    HttpResponse::not_found()
                }
            }
            "/" => HttpResponse::html(self.render_html_dashboard()),
            "/api/latency" => HttpResponse::json(self.render_json_latency()),
            _ => HttpResponse::not_found(),
        }
    }

    /// Render health status as hand-built JSON.
    pub fn render_json_health(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push('{');
        let _ = write!(out, "\"healthy\":{}", self.health.healthy);
        let _ = write!(out, ",\"version\":\"{}\"", json_escape(&self.health.version));
        let _ = write!(out, ",\"uptime_secs\":{}", self.health.uptime_secs);
        let _ = write!(out, ",\"active_sessions\":{}", self.health.active_sessions);
        let _ = write!(
            out,
            ",\"messages_processed\":{}",
            self.health.messages_processed
        );
        let _ = write!(
            out,
            ",\"engine_state\":\"{}\"",
            json_escape(&self.health.engine_state)
        );
        out.push('}');
        out
    }

    /// Render sessions as hand-built JSON array.
    pub fn render_json_sessions(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push('[');
        for (i, s) in self.sessions.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('{');
            let _ = write!(out, "\"session_id\":\"{}\"", json_escape(&s.session_id));
            let _ = write!(
                out,
                ",\"sender_comp_id\":\"{}\"",
                json_escape(&s.sender_comp_id)
            );
            let _ = write!(
                out,
                ",\"target_comp_id\":\"{}\"",
                json_escape(&s.target_comp_id)
            );
            let _ = write!(out, ",\"state\":\"{}\"", json_escape(&s.state));
            let _ = write!(out, ",\"outbound_seq\":{}", s.outbound_seq);
            let _ = write!(out, ",\"inbound_seq\":{}", s.inbound_seq);
            let _ = write!(out, ",\"messages_sent\":{}", s.messages_sent);
            let _ = write!(out, ",\"messages_received\":{}", s.messages_received);
            let _ = write!(out, ",\"last_activity_ms\":{}", s.last_activity_ms);
            let _ = write!(out, ",\"uptime_secs\":{}", s.uptime_secs);
            out.push('}');
        }
        out.push(']');
        out
    }

    /// Render HTML dashboard page with auto-refresh.
    pub fn render_html_dashboard(&self) -> String {
        let refresh_secs = self.config.refresh_interval_ms / 1000;
        let mut html = String::with_capacity(4096);

        let _ = write!(
            html,
            r#"<!DOCTYPE html>
<html>
<head>
<meta charset="utf-8">
<meta http-equiv="refresh" content="{}">
<title>Velocitas FIX Engine Dashboard</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, sans-serif; margin: 2em; background: #f5f5f5; }}
h1 {{ color: #333; }}
.status {{ display: inline-block; padding: 4px 12px; border-radius: 4px; color: #fff; font-weight: bold; }}
.status.healthy {{ background: #27ae60; }}
.status.unhealthy {{ background: #e74c3c; }}
table {{ border-collapse: collapse; width: 100%; margin-top: 1em; background: #fff; }}
th, td {{ border: 1px solid #ddd; padding: 8px; text-align: left; }}
th {{ background: #2c3e50; color: #fff; }}
tr:nth-child(even) {{ background: #f9f9f9; }}
.metrics {{ display: flex; gap: 1em; margin-top: 1em; flex-wrap: wrap; }}
.metric-card {{ background: #fff; border: 1px solid #ddd; border-radius: 4px; padding: 1em; min-width: 180px; }}
.metric-value {{ font-size: 1.8em; font-weight: bold; color: #2c3e50; }}
.metric-label {{ color: #666; font-size: 0.9em; }}
</style>
</head>
<body>
<h1>Velocitas FIX Engine</h1>"#,
            refresh_secs
        );

        // Engine status
        let status_class = if self.health.healthy {
            "healthy"
        } else {
            "unhealthy"
        };
        let _ = write!(
            html,
            r#"<p>Engine State: <span class="status {}">{}</span></p>"#,
            status_class, self.health.engine_state,
        );

        // Key metrics summary
        let _ = write!(
            html,
            r#"<div class="metrics">
<div class="metric-card"><div class="metric-value">{}</div><div class="metric-label">Active Sessions</div></div>
<div class="metric-card"><div class="metric-value">{}</div><div class="metric-label">Messages Processed</div></div>
<div class="metric-card"><div class="metric-value">{}</div><div class="metric-label">Uptime (secs)</div></div>
<div class="metric-card"><div class="metric-value">{}</div><div class="metric-label">Version</div></div>
</div>"#,
            self.health.active_sessions,
            self.health.messages_processed,
            self.health.uptime_secs,
            self.health.version,
        );

        // Session table
        html.push_str("<h2>Sessions</h2>");
        if self.sessions.is_empty() {
            html.push_str("<p>No active sessions.</p>");
        } else {
            html.push_str(
                "<table>\
                 <tr><th>Session ID</th><th>Sender</th><th>Target</th><th>State</th>\
                 <th>Out Seq</th><th>In Seq</th><th>Sent</th><th>Received</th>\
                 <th>Last Activity (ms)</th><th>Uptime (s)</th></tr>",
            );
            for s in &self.sessions {
                let _ = write!(
                    html,
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
                     <td>{}</td><td>{}</td><td>{}</td><td>{}</td>\
                     <td>{}</td><td>{}</td></tr>",
                    s.session_id,
                    s.sender_comp_id,
                    s.target_comp_id,
                    s.state,
                    s.outbound_seq,
                    s.inbound_seq,
                    s.messages_sent,
                    s.messages_received,
                    s.last_activity_ms,
                    s.uptime_secs,
                );
            }
            html.push_str("</table>");
        }

        html.push_str("</body></html>");
        html
    }

    /// Render placeholder Prometheus text-format metrics.
    fn render_prometheus_metrics(&self) -> String {
        let mut out = String::with_capacity(512);
        let _ = writeln!(out, "# HELP velocitas_up Engine health (1=up, 0=down)");
        let _ = writeln!(out, "# TYPE velocitas_up gauge");
        let _ = writeln!(
            out,
            "velocitas_up {}",
            if self.health.healthy { 1 } else { 0 }
        );
        let _ = writeln!(
            out,
            "# HELP velocitas_active_sessions Number of active FIX sessions"
        );
        let _ = writeln!(out, "# TYPE velocitas_active_sessions gauge");
        let _ = writeln!(
            out,
            "velocitas_active_sessions {}",
            self.health.active_sessions
        );
        let _ = writeln!(
            out,
            "# HELP velocitas_messages_processed_total Total messages processed"
        );
        let _ = writeln!(out, "# TYPE velocitas_messages_processed_total counter");
        let _ = writeln!(
            out,
            "velocitas_messages_processed_total {}",
            self.health.messages_processed
        );
        let _ = writeln!(out, "# HELP velocitas_uptime_seconds Engine uptime");
        let _ = writeln!(out, "# TYPE velocitas_uptime_seconds gauge");
        let _ = writeln!(out, "velocitas_uptime_seconds {}", self.health.uptime_secs);
        out
    }

    /// Render latency summary as JSON (placeholder for integration with metrics.rs).
    fn render_json_latency(&self) -> String {
        let mut out = String::with_capacity(128);
        out.push('{');
        let _ = write!(out, "\"p50_ns\":0");
        let _ = write!(out, ",\"p90_ns\":0");
        let _ = write!(out, ",\"p99_ns\":0");
        let _ = write!(out, ",\"p999_ns\":0");
        let _ = write!(out, ",\"count\":0");
        out.push('}');
        out
    }
}

// ---------------------------------------------------------------------------
// JSON string escaping (minimal, no serde)
// ---------------------------------------------------------------------------

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> DashboardConfig {
        DashboardConfig::default()
    }

    fn test_session(id: &str) -> SessionStatus {
        SessionStatus {
            session_id: id.to_string(),
            sender_comp_id: "SENDER".to_string(),
            target_comp_id: "TARGET".to_string(),
            state: "Active".to_string(),
            outbound_seq: 42,
            inbound_seq: 37,
            messages_sent: 1000,
            messages_received: 950,
            last_activity_ms: 100,
            uptime_secs: 3600,
        }
    }

    fn test_health() -> HealthStatus {
        HealthStatus {
            healthy: true,
            version: "0.1.0".to_string(),
            uptime_secs: 7200,
            active_sessions: 2,
            messages_processed: 50000,
            engine_state: "active".to_string(),
        }
    }

    // -- DashboardConfig defaults --

    #[test]
    fn default_config_bind_address() {
        let cfg = DashboardConfig::default();
        assert_eq!(cfg.bind_address, "0.0.0.0");
        assert_eq!(cfg.port, 8080);
        assert!(cfg.enable_health_endpoint);
        assert!(cfg.enable_metrics_endpoint);
        assert!(cfg.enable_sessions_endpoint);
    }

    // -- Request routing --

    #[test]
    fn handle_request_health_returns_200() {
        let mut dash = Dashboard::new(test_config());
        dash.update_health(test_health());
        let resp = dash.handle_request("GET", "/health");
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.content_type, "application/json");
        assert!(resp.body.contains("\"healthy\":true"));
    }

    #[test]
    fn handle_request_sessions_returns_200() {
        let mut dash = Dashboard::new(test_config());
        dash.update_session(test_session("S1"));
        let resp = dash.handle_request("GET", "/sessions");
        assert_eq!(resp.status_code, 200);
        assert_eq!(resp.content_type, "application/json");
        assert!(resp.body.contains("\"session_id\":\"S1\""));
    }

    #[test]
    fn handle_request_root_returns_html() {
        let dash = Dashboard::new(test_config());
        let resp = dash.handle_request("GET", "/");
        assert_eq!(resp.status_code, 200);
        assert!(resp.content_type.contains("text/html"));
        assert!(resp.body.contains("Velocitas FIX Engine"));
    }

    #[test]
    fn handle_request_metrics_returns_prometheus() {
        let mut dash = Dashboard::new(test_config());
        dash.update_health(test_health());
        let resp = dash.handle_request("GET", "/metrics");
        assert_eq!(resp.status_code, 200);
        assert!(resp.content_type.contains("text/plain"));
        assert!(resp.body.contains("velocitas_up 1"));
    }

    #[test]
    fn handle_request_latency_returns_json() {
        let dash = Dashboard::new(test_config());
        let resp = dash.handle_request("GET", "/api/latency");
        assert_eq!(resp.status_code, 200);
        assert!(resp.body.contains("\"p50_ns\""));
        assert!(resp.body.contains("\"p99_ns\""));
    }

    #[test]
    fn handle_request_unknown_path_returns_404() {
        let dash = Dashboard::new(test_config());
        let resp = dash.handle_request("GET", "/nonexistent");
        assert_eq!(resp.status_code, 404);
    }

    #[test]
    fn handle_request_post_returns_405() {
        let dash = Dashboard::new(test_config());
        let resp = dash.handle_request("POST", "/health");
        assert_eq!(resp.status_code, 405);
    }

    // -- Session management --

    #[test]
    fn update_session_adds_new() {
        let mut dash = Dashboard::new(test_config());
        assert_eq!(dash.session_count(), 0);
        dash.update_session(test_session("S1"));
        assert_eq!(dash.session_count(), 1);
    }

    #[test]
    fn update_session_updates_existing() {
        let mut dash = Dashboard::new(test_config());
        dash.update_session(test_session("S1"));
        let mut updated = test_session("S1");
        updated.messages_sent = 9999;
        dash.update_session(updated);
        assert_eq!(dash.session_count(), 1);

        let json = dash.render_json_sessions();
        assert!(json.contains("\"messages_sent\":9999"));
    }

    #[test]
    fn remove_session_works() {
        let mut dash = Dashboard::new(test_config());
        dash.update_session(test_session("S1"));
        dash.update_session(test_session("S2"));
        assert_eq!(dash.session_count(), 2);
        dash.remove_session("S1");
        assert_eq!(dash.session_count(), 1);

        let json = dash.render_json_sessions();
        assert!(!json.contains("\"session_id\":\"S1\""));
        assert!(json.contains("\"session_id\":\"S2\""));
    }

    #[test]
    fn remove_nonexistent_session_is_noop() {
        let mut dash = Dashboard::new(test_config());
        dash.update_session(test_session("S1"));
        dash.remove_session("NOPE");
        assert_eq!(dash.session_count(), 1);
    }

    // -- JSON format validation --

    #[test]
    fn json_health_is_valid() {
        let mut dash = Dashboard::new(test_config());
        dash.update_health(test_health());
        let json = dash.render_json_health();
        // Basic structural checks (no serde available).
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"healthy\":true"));
        assert!(json.contains("\"version\":\"0.1.0\""));
        assert!(json.contains("\"uptime_secs\":7200"));
        assert!(json.contains("\"active_sessions\":2"));
        assert!(json.contains("\"messages_processed\":50000"));
        assert!(json.contains("\"engine_state\":\"active\""));
    }

    #[test]
    fn json_sessions_empty_array() {
        let dash = Dashboard::new(test_config());
        let json = dash.render_json_sessions();
        assert_eq!(json, "[]");
    }

    #[test]
    fn json_sessions_multiple() {
        let mut dash = Dashboard::new(test_config());
        dash.update_session(test_session("A"));
        dash.update_session(test_session("B"));
        let json = dash.render_json_sessions();
        assert!(json.starts_with('['));
        assert!(json.ends_with(']'));
        assert!(json.contains("\"session_id\":\"A\""));
        assert!(json.contains("\"session_id\":\"B\""));
    }

    // -- HTML dashboard --

    #[test]
    fn html_dashboard_contains_expected_elements() {
        let mut dash = Dashboard::new(test_config());
        dash.update_health(test_health());
        dash.update_session(test_session("S1"));
        let html = dash.render_html_dashboard();
        assert!(html.contains("Velocitas FIX Engine"));
        assert!(html.contains("meta http-equiv=\"refresh\""));
        assert!(html.contains("<table>"));
        assert!(html.contains("S1"));
        assert!(html.contains("SENDER"));
        assert!(html.contains("TARGET"));
        assert!(html.contains("Active Sessions"));
    }

    #[test]
    fn html_dashboard_no_sessions() {
        let dash = Dashboard::new(test_config());
        let html = dash.render_html_dashboard();
        assert!(html.contains("No active sessions."));
    }

    // -- JSON escaping --

    #[test]
    fn json_escape_special_chars() {
        assert_eq!(json_escape("hello"), "hello");
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
        assert_eq!(json_escape("a\nb"), "a\\nb");
    }

    // -- Disabled endpoints return 404 --

    #[test]
    fn disabled_health_returns_404() {
        let mut cfg = test_config();
        cfg.enable_health_endpoint = false;
        let dash = Dashboard::new(cfg);
        let resp = dash.handle_request("GET", "/health");
        assert_eq!(resp.status_code, 404);
    }

    #[test]
    fn disabled_sessions_returns_404() {
        let mut cfg = test_config();
        cfg.enable_sessions_endpoint = false;
        let dash = Dashboard::new(cfg);
        let resp = dash.handle_request("GET", "/sessions");
        assert_eq!(resp.status_code, 404);
    }

    #[test]
    fn disabled_metrics_returns_404() {
        let mut cfg = test_config();
        cfg.enable_metrics_endpoint = false;
        let dash = Dashboard::new(cfg);
        let resp = dash.handle_request("GET", "/metrics");
        assert_eq!(resp.status_code, 404);
    }
}
