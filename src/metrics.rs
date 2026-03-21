/// Prometheus-compatible metrics exporter for the Velocitas FIX engine.
///
/// All counters and gauges are lock-free using atomics. Histograms use an HDR
/// histogram behind a `Mutex` (only acquired on reads / resets, never on the
/// recording hot-path — `record()` still takes the lock briefly, but HDR
/// histogram recording is O(1) and sub-microsecond).

use std::fmt::Write as FmtWrite;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;

use hdrhistogram::Histogram;

// ---------------------------------------------------------------------------
// Counter
// ---------------------------------------------------------------------------

/// Atomic counter (monotonically increasing).
pub struct Counter {
    name: &'static str,
    help: &'static str,
    labels: Vec<(&'static str, String)>,
    value: AtomicU64,
}

impl Counter {
    /// Create a counter with the given name and help text.
    pub fn new(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            labels: Vec::new(),
            value: AtomicU64::new(0),
        }
    }

    /// Create a counter with the given name, help text, and labels.
    pub fn with_labels(
        name: &'static str,
        help: &'static str,
        labels: Vec<(&'static str, String)>,
    ) -> Self {
        Self {
            name,
            help,
            labels,
            value: AtomicU64::new(0),
        }
    }

    /// Increment by 1.
    #[inline]
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment by `n`.
    #[inline]
    pub fn inc_by(&self, n: u64) {
        self.value.fetch_add(n, Ordering::Relaxed);
    }

    /// Read the current value.
    #[inline]
    pub fn get(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Reset to zero.
    #[inline]
    pub fn reset(&self) {
        self.value.store(0, Ordering::Relaxed);
    }

    /// Render in Prometheus text format.
    fn render(&self, out: &mut String) {
        let _ = writeln!(out, "# HELP {} {}", self.name, self.help);
        let _ = writeln!(out, "# TYPE {} counter", self.name);
        if self.labels.is_empty() {
            let _ = writeln!(out, "{} {}", self.name, self.get());
        } else {
            let labels = format_labels(&self.labels);
            let _ = writeln!(out, "{}{} {}", self.name, labels, self.get());
        }
    }
}

// ---------------------------------------------------------------------------
// Gauge
// ---------------------------------------------------------------------------

/// Atomic gauge (can go up and down).
pub struct Gauge {
    name: &'static str,
    help: &'static str,
    labels: Vec<(&'static str, String)>,
    value: AtomicI64,
}

impl Gauge {
    /// Create a gauge with the given name and help text.
    pub fn new(name: &'static str, help: &'static str) -> Self {
        Self {
            name,
            help,
            labels: Vec::new(),
            value: AtomicI64::new(0),
        }
    }

    /// Create a gauge with the given name, help text, and labels.
    pub fn with_labels(
        name: &'static str,
        help: &'static str,
        labels: Vec<(&'static str, String)>,
    ) -> Self {
        Self {
            name,
            help,
            labels,
            value: AtomicI64::new(0),
        }
    }

    /// Increment by 1.
    #[inline]
    pub fn inc(&self) {
        self.value.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement by 1.
    #[inline]
    pub fn dec(&self) {
        self.value.fetch_sub(1, Ordering::Relaxed);
    }

    /// Set to an absolute value.
    #[inline]
    pub fn set(&self, val: i64) {
        self.value.store(val, Ordering::Relaxed);
    }

    /// Read the current value.
    #[inline]
    pub fn get(&self) -> i64 {
        self.value.load(Ordering::Relaxed)
    }

    /// Render in Prometheus text format.
    fn render(&self, out: &mut String) {
        let _ = writeln!(out, "# HELP {} {}", self.name, self.help);
        let _ = writeln!(out, "# TYPE {} gauge", self.name);
        if self.labels.is_empty() {
            let _ = writeln!(out, "{} {}", self.name, self.get());
        } else {
            let labels = format_labels(&self.labels);
            let _ = writeln!(out, "{}{} {}", self.name, labels, self.get());
        }
    }
}

// ---------------------------------------------------------------------------
// LatencyHistogram
// ---------------------------------------------------------------------------

/// Lock-free latency histogram using HDR Histogram.
///
/// The `Mutex` is only used to serialise concurrent accesses to the underlying
/// `hdrhistogram::Histogram` which is not `Sync`. Recording is O(1) and
/// extremely fast — typical lock hold times are well under 100 ns.
pub struct LatencyHistogram {
    name: &'static str,
    help: &'static str,
    histogram: Mutex<Histogram<u64>>,
}

impl LatencyHistogram {
    /// Create a new histogram that can record values up to `max_value_ns`.
    pub fn new(name: &'static str, help: &'static str, max_value_ns: u64) -> Self {
        Self {
            name,
            help,
            histogram: Mutex::new(
                Histogram::<u64>::new_with_bounds(1, max_value_ns, 3)
                    .expect("invalid histogram bounds"),
            ),
        }
    }

    /// Record a single value.
    #[inline]
    pub fn record(&self, value_ns: u64) {
        let mut h = self.histogram.lock().unwrap();
        // Clamp to max trackable value to avoid errors.
        let v = value_ns.min(h.high());
        h.record(v).expect("value out of range after clamping");
    }

    /// Return the value at the given percentile (0.0–100.0).
    pub fn percentile(&self, p: f64) -> u64 {
        self.histogram.lock().unwrap().value_at_percentile(p)
    }

    /// Mean recorded value.
    pub fn mean(&self) -> f64 {
        self.histogram.lock().unwrap().mean()
    }

    /// Minimum recorded value (0 if empty).
    pub fn min(&self) -> u64 {
        self.histogram.lock().unwrap().min()
    }

    /// Maximum recorded value (0 if empty).
    pub fn max(&self) -> u64 {
        self.histogram.lock().unwrap().max()
    }

    /// Total count of recorded values.
    pub fn count(&self) -> u64 {
        self.histogram.lock().unwrap().len()
    }

    /// Reset all recorded data.
    pub fn reset(&self) {
        self.histogram.lock().unwrap().reset();
    }

    /// Render in Prometheus summary format.
    fn render(&self, out: &mut String) {
        let h = self.histogram.lock().unwrap();
        let _ = writeln!(out, "# HELP {} {}", self.name, self.help);
        let _ = writeln!(out, "# TYPE {} summary", self.name);

        for &q in &[50.0, 90.0, 99.0, 99.9] {
            let v = h.value_at_percentile(q);
            let _ = writeln!(
                out,
                "{}{{quantile=\"{}\"}} {}",
                self.name,
                q / 100.0,
                v,
            );
        }

        let count = h.len();
        let sum: u64 = h
            .iter_recorded()
            .map(|v| v.value_iterated_to() * v.count_at_value())
            .sum();

        let _ = writeln!(out, "{}_count {}", self.name, count);
        let _ = writeln!(out, "{}_sum {}", self.name, sum);
    }
}

// ---------------------------------------------------------------------------
// MetricsSnapshot
// ---------------------------------------------------------------------------

/// Point-in-time snapshot of all engine metrics.
#[derive(Debug, Clone)]
pub struct MetricsSnapshot {
    // Counters
    pub messages_parsed: u64,
    pub messages_sent: u64,
    pub messages_rejected: u64,
    pub journal_writes: u64,
    pub risk_checks_passed: u64,
    pub risk_checks_blocked: u64,
    pub kill_switch_activations: u64,

    // Gauges
    pub active_sessions: i64,
    pub pending_resends: i64,

    // Histogram summaries
    pub parse_latency_p50_ns: u64,
    pub parse_latency_p99_ns: u64,
    pub serialize_latency_p50_ns: u64,
    pub serialize_latency_p99_ns: u64,
    pub wire_to_wire_latency_p50_ns: u64,
    pub wire_to_wire_latency_p99_ns: u64,
    pub journal_write_latency_p50_ns: u64,
    pub journal_write_latency_p99_ns: u64,
}

// ---------------------------------------------------------------------------
// EngineMetrics
// ---------------------------------------------------------------------------

/// Engine-wide metrics registry.
pub struct EngineMetrics {
    // Message counters
    pub messages_parsed: Counter,
    pub messages_sent: Counter,
    pub messages_rejected: Counter,

    // Session gauges
    pub active_sessions: Gauge,
    pub pending_resends: Gauge,

    // Latency histograms
    pub parse_latency_ns: LatencyHistogram,
    pub serialize_latency_ns: LatencyHistogram,
    pub wire_to_wire_latency_ns: LatencyHistogram,

    // Journal metrics
    pub journal_writes: Counter,
    pub journal_write_latency_ns: LatencyHistogram,

    // Risk metrics
    pub risk_checks_passed: Counter,
    pub risk_checks_blocked: Counter,
    pub kill_switch_activations: Counter,
}

/// Maximum trackable value for latency histograms (10 seconds in nanoseconds).
const MAX_LATENCY_NS: u64 = 10_000_000_000;

impl EngineMetrics {
    /// Create a new metrics registry with all metrics initialised to zero.
    pub fn new() -> Self {
        Self {
            messages_parsed: Counter::new(
                "velocitas_messages_parsed_total",
                "Total FIX messages parsed",
            ),
            messages_sent: Counter::new(
                "velocitas_messages_sent_total",
                "Total FIX messages sent",
            ),
            messages_rejected: Counter::new(
                "velocitas_messages_rejected_total",
                "Total FIX messages rejected",
            ),

            active_sessions: Gauge::new(
                "velocitas_active_sessions",
                "Number of active FIX sessions",
            ),
            pending_resends: Gauge::new(
                "velocitas_pending_resends",
                "Number of pending resend requests",
            ),

            parse_latency_ns: LatencyHistogram::new(
                "velocitas_parse_latency_ns",
                "Parse latency in nanoseconds",
                MAX_LATENCY_NS,
            ),
            serialize_latency_ns: LatencyHistogram::new(
                "velocitas_serialize_latency_ns",
                "Serialize latency in nanoseconds",
                MAX_LATENCY_NS,
            ),
            wire_to_wire_latency_ns: LatencyHistogram::new(
                "velocitas_wire_to_wire_latency_ns",
                "Wire-to-wire latency in nanoseconds",
                MAX_LATENCY_NS,
            ),

            journal_writes: Counter::new(
                "velocitas_journal_writes_total",
                "Total journal write operations",
            ),
            journal_write_latency_ns: LatencyHistogram::new(
                "velocitas_journal_write_latency_ns",
                "Journal write latency in nanoseconds",
                MAX_LATENCY_NS,
            ),

            risk_checks_passed: Counter::new(
                "velocitas_risk_checks_passed_total",
                "Total risk checks that passed",
            ),
            risk_checks_blocked: Counter::new(
                "velocitas_risk_checks_blocked_total",
                "Total risk checks that blocked an order",
            ),
            kill_switch_activations: Counter::new(
                "velocitas_kill_switch_activations_total",
                "Total kill-switch activations",
            ),
        }
    }

    /// Render all metrics in Prometheus text exposition format.
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(4096);

        self.messages_parsed.render(&mut out);
        out.push('\n');
        self.messages_sent.render(&mut out);
        out.push('\n');
        self.messages_rejected.render(&mut out);
        out.push('\n');

        self.active_sessions.render(&mut out);
        out.push('\n');
        self.pending_resends.render(&mut out);
        out.push('\n');

        self.parse_latency_ns.render(&mut out);
        out.push('\n');
        self.serialize_latency_ns.render(&mut out);
        out.push('\n');
        self.wire_to_wire_latency_ns.render(&mut out);
        out.push('\n');

        self.journal_writes.render(&mut out);
        out.push('\n');
        self.journal_write_latency_ns.render(&mut out);
        out.push('\n');

        self.risk_checks_passed.render(&mut out);
        out.push('\n');
        self.risk_checks_blocked.render(&mut out);
        out.push('\n');
        self.kill_switch_activations.render(&mut out);

        out
    }

    /// Take a point-in-time snapshot of all metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            messages_parsed: self.messages_parsed.get(),
            messages_sent: self.messages_sent.get(),
            messages_rejected: self.messages_rejected.get(),
            journal_writes: self.journal_writes.get(),
            risk_checks_passed: self.risk_checks_passed.get(),
            risk_checks_blocked: self.risk_checks_blocked.get(),
            kill_switch_activations: self.kill_switch_activations.get(),

            active_sessions: self.active_sessions.get(),
            pending_resends: self.pending_resends.get(),

            parse_latency_p50_ns: self.parse_latency_ns.percentile(50.0),
            parse_latency_p99_ns: self.parse_latency_ns.percentile(99.0),
            serialize_latency_p50_ns: self.serialize_latency_ns.percentile(50.0),
            serialize_latency_p99_ns: self.serialize_latency_ns.percentile(99.0),
            wire_to_wire_latency_p50_ns: self.wire_to_wire_latency_ns.percentile(50.0),
            wire_to_wire_latency_p99_ns: self.wire_to_wire_latency_ns.percentile(99.0),
            journal_write_latency_p50_ns: self.journal_write_latency_ns.percentile(50.0),
            journal_write_latency_p99_ns: self.journal_write_latency_ns.percentile(99.0),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_labels(labels: &[(&'static str, String)]) -> String {
    let mut out = String::from("{");
    for (i, (k, v)) in labels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{}=\"{}\"", k, v);
    }
    out.push('}');
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn counter_inc_and_get() {
        let c = Counter::new("test_counter", "A test counter");
        assert_eq!(c.get(), 0);
        c.inc();
        assert_eq!(c.get(), 1);
        c.inc_by(9);
        assert_eq!(c.get(), 10);
    }

    #[test]
    fn counter_reset() {
        let c = Counter::new("test_counter", "A test counter");
        c.inc_by(42);
        assert_eq!(c.get(), 42);
        c.reset();
        assert_eq!(c.get(), 0);
    }

    #[test]
    fn counter_with_labels() {
        let c = Counter::with_labels(
            "test_counter",
            "A test counter",
            vec![("session", "FIX.4.4-SENDER-TARGET".to_string())],
        );
        c.inc();
        assert_eq!(c.get(), 1);

        let mut out = String::new();
        c.render(&mut out);
        assert!(out.contains("test_counter{session=\"FIX.4.4-SENDER-TARGET\"} 1"));
    }

    #[test]
    fn gauge_inc_dec_set() {
        let g = Gauge::new("test_gauge", "A test gauge");
        assert_eq!(g.get(), 0);
        g.inc();
        g.inc();
        assert_eq!(g.get(), 2);
        g.dec();
        assert_eq!(g.get(), 1);
        g.set(100);
        assert_eq!(g.get(), 100);
        g.set(-5);
        assert_eq!(g.get(), -5);
    }

    #[test]
    fn histogram_record_and_percentile() {
        let h = LatencyHistogram::new("test_hist", "A test histogram", 1_000_000);
        for v in 100..=1000 {
            h.record(v);
        }
        // p50 should be around 550
        let p50 = h.percentile(50.0);
        assert!(p50 >= 500 && p50 <= 600, "p50 was {}", p50);

        // p99 should be close to 1000
        let p99 = h.percentile(99.0);
        assert!(p99 >= 950 && p99 <= 1050, "p99 was {}", p99);

        let mean = h.mean();
        assert!(mean > 400.0 && mean < 600.0, "mean was {}", mean);

        assert!(h.min() >= 100);
        assert!(h.max() <= 1000);
        assert_eq!(h.count(), 901);
    }

    #[test]
    fn histogram_reset() {
        let h = LatencyHistogram::new("test_hist", "A test histogram", 1_000_000);
        h.record(500);
        assert_eq!(h.count(), 1);
        h.reset();
        assert_eq!(h.count(), 0);
    }

    #[test]
    fn prometheus_format_rendering() {
        let m = EngineMetrics::new();
        m.messages_parsed.inc_by(12345);
        m.active_sessions.set(3);
        m.parse_latency_ns.record(280);
        m.parse_latency_ns.record(350);
        m.parse_latency_ns.record(500);

        let output = m.render_prometheus();

        // Counter lines
        assert!(output.contains("# HELP velocitas_messages_parsed_total Total FIX messages parsed"));
        assert!(output.contains("# TYPE velocitas_messages_parsed_total counter"));
        assert!(output.contains("velocitas_messages_parsed_total 12345"));

        // Gauge lines
        assert!(output.contains("# TYPE velocitas_active_sessions gauge"));
        assert!(output.contains("velocitas_active_sessions 3"));

        // Histogram summary lines
        assert!(output.contains("# HELP velocitas_parse_latency_ns Parse latency in nanoseconds"));
        assert!(output.contains("# TYPE velocitas_parse_latency_ns summary"));
        assert!(output.contains("velocitas_parse_latency_ns{quantile=\"0.5\"}"));
        assert!(output.contains("velocitas_parse_latency_ns{quantile=\"0.99\"}"));
        assert!(output.contains("velocitas_parse_latency_ns_count 3"));
    }

    #[test]
    fn engine_metrics_new_creates_all() {
        let m = EngineMetrics::new();

        // Counters start at zero
        assert_eq!(m.messages_parsed.get(), 0);
        assert_eq!(m.messages_sent.get(), 0);
        assert_eq!(m.messages_rejected.get(), 0);
        assert_eq!(m.journal_writes.get(), 0);
        assert_eq!(m.risk_checks_passed.get(), 0);
        assert_eq!(m.risk_checks_blocked.get(), 0);
        assert_eq!(m.kill_switch_activations.get(), 0);

        // Gauges start at zero
        assert_eq!(m.active_sessions.get(), 0);
        assert_eq!(m.pending_resends.get(), 0);

        // Histograms are empty
        assert_eq!(m.parse_latency_ns.count(), 0);
        assert_eq!(m.serialize_latency_ns.count(), 0);
        assert_eq!(m.wire_to_wire_latency_ns.count(), 0);
        assert_eq!(m.journal_write_latency_ns.count(), 0);
    }

    #[test]
    fn snapshot_captures_current_values() {
        let m = EngineMetrics::new();
        m.messages_parsed.inc_by(100);
        m.active_sessions.set(5);
        m.parse_latency_ns.record(200);
        m.parse_latency_ns.record(400);

        let snap = m.snapshot();
        assert_eq!(snap.messages_parsed, 100);
        assert_eq!(snap.active_sessions, 5);
        assert!(snap.parse_latency_p50_ns >= 200);
        assert!(snap.parse_latency_p99_ns >= 200);
    }

    #[test]
    fn thread_safety_counter() {
        let counter = Arc::new(Counter::new("threaded", "thread test"));
        let num_threads = 8;
        let increments_per_thread = 10_000;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let c = Arc::clone(&counter);
                thread::spawn(move || {
                    for _ in 0..increments_per_thread {
                        c.inc();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            counter.get(),
            num_threads * increments_per_thread,
            "counter should reflect all increments from all threads",
        );
    }

    #[test]
    fn thread_safety_gauge() {
        let gauge = Arc::new(Gauge::new("threaded_gauge", "thread gauge test"));
        let num_threads = 8;
        let ops_per_thread = 5_000;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let g = Arc::clone(&gauge);
                thread::spawn(move || {
                    for _ in 0..ops_per_thread {
                        g.inc();
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(gauge.get(), (num_threads * ops_per_thread) as i64);
    }

    #[test]
    fn thread_safety_histogram() {
        let hist = Arc::new(LatencyHistogram::new("threaded_hist", "thread hist test", 1_000_000));
        let num_threads = 4;
        let records_per_thread = 1_000;

        let handles: Vec<_> = (0..num_threads)
            .map(|t| {
                let h = Arc::clone(&hist);
                thread::spawn(move || {
                    for i in 0..records_per_thread {
                        h.record((t as u64 * 1000) + i + 1);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(
            hist.count(),
            (num_threads * records_per_thread) as u64,
            "histogram should have all recorded values",
        );
    }
}
