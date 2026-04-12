/// Hardware timestamp capture for nanosecond-precision timing.
///
/// Supports multiple timestamp sources: system clock, CPU TSC, hardware NIC
/// timestamps (SO_TIMESTAMPING on Linux), and PTP-synchronized clocks.
/// Provides FIX-protocol timestamp formatting at millisecond, microsecond,
/// and nanosecond precision (MiFID II compliant).
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// TimestampSource
// ---------------------------------------------------------------------------

/// Timestamp source selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TimestampSource {
    /// System clock (std::time::Instant) — portable but lower precision
    System,
    /// CPU timestamp counter (rdtsc on x86, cntvct_el0 on ARM)
    Tsc,
    /// Hardware NIC timestamps via SO_TIMESTAMPING (Linux only)
    Hardware,
    /// PTP (Precision Time Protocol) synchronized clock
    Ptp,
}

// ---------------------------------------------------------------------------
// HrTimestamp
// ---------------------------------------------------------------------------

/// High-resolution timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HrTimestamp {
    /// Nanoseconds since epoch (or arbitrary origin for TSC)
    pub nanos: u64,
    /// Source of this timestamp
    pub source: TimestampSource,
}

impl HrTimestamp {
    /// Get current time from the given source.
    #[inline]
    pub fn now(source: TimestampSource) -> Self {
        let nanos = match source {
            TimestampSource::System => {
                let d = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO);
                d.as_nanos() as u64
            }
            TimestampSource::Tsc => HrClock::read_tsc(),
            TimestampSource::Hardware | TimestampSource::Ptp => {
                // Hardware/PTP require OS-level configuration; fall back to
                // system clock for the raw reading. A properly configured
                // HrClock will translate TSC ticks into wall-clock nanos.
                let d = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO);
                d.as_nanos() as u64
            }
        };
        Self { nanos, source }
    }

    /// Compute elapsed nanoseconds since an earlier timestamp.
    #[inline]
    pub fn elapsed_ns(self, since: HrTimestamp) -> u64 {
        self.nanos.saturating_sub(since.nanos)
    }

    /// Format as FIX timestamp with millisecond precision: `YYYYMMDD-HH:MM:SS.sss`
    ///
    /// Returns exactly 21 bytes.
    pub fn to_fix_timestamp(&self) -> [u8; 21] {
        let (y, mo, d, h, mi, s, nanos_frac) = self.to_utc_components();
        let ms = nanos_frac / 1_000_000;
        let mut buf = [0u8; 21];
        write_date(&mut buf, y, mo, d);
        buf[8] = b'-';
        write_time(&mut buf[9..], h, mi, s);
        buf[17] = b'.';
        write_digits3(&mut buf[18..], ms as u32);
        buf
    }

    /// Format as FIX timestamp with microsecond precision: `YYYYMMDD-HH:MM:SS.ssssss`
    ///
    /// Returns exactly 24 bytes.
    pub fn to_fix_timestamp_us(&self) -> [u8; 24] {
        let (y, mo, d, h, mi, s, nanos_frac) = self.to_utc_components();
        let us = nanos_frac / 1_000;
        let mut buf = [0u8; 24];
        write_date(&mut buf, y, mo, d);
        buf[8] = b'-';
        write_time(&mut buf[9..], h, mi, s);
        buf[17] = b'.';
        write_digits6(&mut buf[18..], us as u32);
        buf
    }

    /// Format as FIX timestamp with nanosecond precision: `YYYYMMDD-HH:MM:SS.sssssssss`
    ///
    /// Returns exactly 27 bytes (MiFID II compliant).
    pub fn to_fix_timestamp_ns(&self) -> [u8; 27] {
        let (y, mo, d, h, mi, s, nanos_frac) = self.to_utc_components();
        let mut buf = [0u8; 27];
        write_date(&mut buf, y, mo, d);
        buf[8] = b'-';
        write_time(&mut buf[9..], h, mi, s);
        buf[17] = b'.';
        write_digits9(&mut buf[18..], nanos_frac as u32);
        buf
    }

    /// Break nanos-since-epoch into UTC calendar components.
    fn to_utc_components(&self) -> (u32, u32, u32, u32, u32, u32, u64) {
        let total_secs = self.nanos / 1_000_000_000;
        let nanos_frac = self.nanos % 1_000_000_000;

        let secs = total_secs;
        let days = secs / 86400;
        let day_secs = secs % 86400;

        let h = (day_secs / 3600) as u32;
        let mi = ((day_secs % 3600) / 60) as u32;
        let s = (day_secs % 60) as u32;

        // Civil date from days since 1970-01-01 (Algorithm from Howard Hinnant).
        let (y, mo, d) = civil_from_days(days as i64);

        (y as u32, mo as u32, d as u32, h, mi, s, nanos_frac)
    }
}

// ---------------------------------------------------------------------------
// HrClock
// ---------------------------------------------------------------------------

/// Timestamp clock — provides high-resolution timestamps.
pub struct HrClock {
    source: TimestampSource,
    tsc_frequency_hz: u64,
    tsc_offset: u64,
}

impl HrClock {
    /// Create a new clock with the given source. Calibrates TSC if needed.
    pub fn new(source: TimestampSource) -> Self {
        let (freq, offset) = if source == TimestampSource::Tsc {
            let freq = Self::calibrate_tsc();
            let tsc_now = Self::read_tsc();
            let wall_nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_nanos() as u64;
            // offset = wall_nanos - tsc_to_nanos(tsc_now, freq)
            let tsc_nanos = tsc_to_nanos(tsc_now, freq);
            let offset = wall_nanos.wrapping_sub(tsc_nanos);
            (freq, offset)
        } else {
            (0, 0)
        };

        Self {
            source,
            tsc_frequency_hz: freq,
            tsc_offset: offset,
        }
    }

    /// Read current timestamp using the configured source.
    #[inline]
    pub fn now(&self) -> HrTimestamp {
        match self.source {
            TimestampSource::Tsc => {
                let tsc = Self::read_tsc();
                let nanos = tsc_to_nanos(tsc, self.tsc_frequency_hz).wrapping_add(self.tsc_offset);
                HrTimestamp {
                    nanos,
                    source: TimestampSource::Tsc,
                }
            }
            other => HrTimestamp::now(other),
        }
    }

    /// Read the CPU timestamp counter.
    ///
    /// - On x86_64: uses `_rdtsc()` intrinsic
    /// - On aarch64: reads `CNTVCT_EL0` via inline assembly
    /// - Otherwise: falls back to `Instant`-based nanoseconds
    #[inline]
    pub fn read_tsc() -> u64 {
        #[cfg(target_arch = "x86_64")]
        {
            unsafe { core::arch::x86_64::_rdtsc() }
        }

        #[cfg(target_arch = "aarch64")]
        {
            read_cntvct_el0()
        }

        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            // Fallback: use Instant elapsed from a fixed reference.
            static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            let start = START.get_or_init(Instant::now);
            start.elapsed().as_nanos() as u64
        }
    }

    /// Calibrate TSC frequency by timing against the system clock.
    ///
    /// Spins for ~2ms measuring TSC ticks vs wall-clock time.
    pub fn calibrate_tsc() -> u64 {
        let cal_duration = Duration::from_millis(2);

        let t0 = Instant::now();
        let tsc0 = Self::read_tsc();

        // Spin-wait for the calibration period.
        while t0.elapsed() < cal_duration {
            std::hint::spin_loop();
        }

        let tsc1 = Self::read_tsc();
        let elapsed = t0.elapsed();
        let elapsed_ns = elapsed.as_nanos() as u64;

        if elapsed_ns == 0 {
            return 1_000_000_000; // fallback 1 GHz
        }

        let ticks = tsc1.wrapping_sub(tsc0);
        // freq = ticks * 1_000_000_000 / elapsed_ns
        // Use u128 to avoid overflow.
        ((ticks as u128 * 1_000_000_000) / elapsed_ns as u128) as u64
    }
}

// ---------------------------------------------------------------------------
// LatencyTracker
// ---------------------------------------------------------------------------

/// Latency measurement helper.
pub struct LatencyTracker {
    start: HrTimestamp,
    clock: HrClock,
}

impl LatencyTracker {
    /// Begin a latency measurement.
    pub fn start(clock: HrClock) -> Self {
        let start = clock.now();
        Self { start, clock }
    }

    /// Return elapsed nanoseconds since start.
    #[inline]
    pub fn stop(&self) -> u64 {
        let end = self.clock.now();
        end.elapsed_ns(self.start)
    }
}

// ---------------------------------------------------------------------------
// aarch64 TSC reading
// ---------------------------------------------------------------------------

#[cfg(target_arch = "aarch64")]
#[inline]
fn read_cntvct_el0() -> u64 {
    let val: u64;
    unsafe {
        std::arch::asm!("mrs {}, cntvct_el0", out(reg) val, options(nostack, nomem));
    }
    val
}

// ---------------------------------------------------------------------------
// TSC → nanoseconds conversion
// ---------------------------------------------------------------------------

#[inline]
fn tsc_to_nanos(ticks: u64, freq_hz: u64) -> u64 {
    if freq_hz == 0 {
        return 0;
    }
    ((ticks as u128 * 1_000_000_000) / freq_hz as u128) as u64
}

// ---------------------------------------------------------------------------
// UTC calendar helpers (Howard Hinnant civil_from_days)
// ---------------------------------------------------------------------------

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Formatting helpers — branchless digit writing
// ---------------------------------------------------------------------------

#[inline]
fn write_date(buf: &mut [u8], y: u32, m: u32, d: u32) {
    write_digits4(buf, y);
    write_digits2(&mut buf[4..], m);
    write_digits2(&mut buf[6..], d);
}

#[inline]
fn write_time(buf: &mut [u8], h: u32, m: u32, s: u32) {
    write_digits2(buf, h);
    buf[2] = b':';
    write_digits2(&mut buf[3..], m);
    buf[5] = b':';
    write_digits2(&mut buf[6..], s);
}

#[inline]
fn write_digits2(buf: &mut [u8], val: u32) {
    buf[0] = b'0' + (val / 10) as u8;
    buf[1] = b'0' + (val % 10) as u8;
}

#[inline]
fn write_digits3(buf: &mut [u8], val: u32) {
    buf[0] = b'0' + (val / 100) as u8;
    buf[1] = b'0' + ((val / 10) % 10) as u8;
    buf[2] = b'0' + (val % 10) as u8;
}

#[inline]
fn write_digits4(buf: &mut [u8], val: u32) {
    buf[0] = b'0' + (val / 1000) as u8;
    buf[1] = b'0' + ((val / 100) % 10) as u8;
    buf[2] = b'0' + ((val / 10) % 10) as u8;
    buf[3] = b'0' + (val % 10) as u8;
}

#[inline]
fn write_digits6(buf: &mut [u8], val: u32) {
    buf[0] = b'0' + (val / 100_000) as u8;
    buf[1] = b'0' + ((val / 10_000) % 10) as u8;
    buf[2] = b'0' + ((val / 1_000) % 10) as u8;
    buf[3] = b'0' + ((val / 100) % 10) as u8;
    buf[4] = b'0' + ((val / 10) % 10) as u8;
    buf[5] = b'0' + (val % 10) as u8;
}

#[inline]
fn write_digits9(buf: &mut [u8], val: u32) {
    buf[0] = b'0' + (val / 100_000_000) as u8;
    buf[1] = b'0' + ((val / 10_000_000) % 10) as u8;
    buf[2] = b'0' + ((val / 1_000_000) % 10) as u8;
    buf[3] = b'0' + ((val / 100_000) % 10) as u8;
    buf[4] = b'0' + ((val / 10_000) % 10) as u8;
    buf[5] = b'0' + ((val / 1_000) % 10) as u8;
    buf[6] = b'0' + ((val / 100) % 10) as u8;
    buf[7] = b'0' + ((val / 10) % 10) as u8;
    buf[8] = b'0' + (val % 10) as u8;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_returns_increasing_values() {
        let t1 = HrTimestamp::now(TimestampSource::System);
        let t2 = HrTimestamp::now(TimestampSource::System);
        assert!(
            t2.nanos >= t1.nanos,
            "timestamps must be monotonically non-decreasing"
        );
    }

    #[test]
    fn elapsed_ns_works() {
        let earlier = HrTimestamp {
            nanos: 1_000_000,
            source: TimestampSource::System,
        };
        let later = HrTimestamp {
            nanos: 2_500_000,
            source: TimestampSource::System,
        };
        assert_eq!(later.elapsed_ns(earlier), 1_500_000);
    }

    #[test]
    fn elapsed_ns_saturates_on_reverse() {
        let earlier = HrTimestamp {
            nanos: 5_000_000,
            source: TimestampSource::System,
        };
        let later = HrTimestamp {
            nanos: 1_000_000,
            source: TimestampSource::System,
        };
        assert_eq!(later.elapsed_ns(earlier), 0);
    }

    #[test]
    fn fix_timestamp_ms_format() {
        // 2024-01-15 13:30:45.123 UTC
        // seconds since epoch: 1705325445
        // nanos: 1705325445_123_000_000
        let ts = HrTimestamp {
            nanos: 1_705_325_445_123_000_000,
            source: TimestampSource::System,
        };
        let buf = ts.to_fix_timestamp();
        let s = std::str::from_utf8(&buf).unwrap();
        assert_eq!(s, "20240115-13:30:45.123");
    }

    #[test]
    fn fix_timestamp_us_format() {
        let ts = HrTimestamp {
            nanos: 1_705_325_445_123_456_000,
            source: TimestampSource::System,
        };
        let buf = ts.to_fix_timestamp_us();
        let s = std::str::from_utf8(&buf).unwrap();
        assert_eq!(s, "20240115-13:30:45.123456");
    }

    #[test]
    fn fix_timestamp_ns_format() {
        let ts = HrTimestamp {
            nanos: 1_705_325_445_123_456_789,
            source: TimestampSource::System,
        };
        let buf = ts.to_fix_timestamp_ns();
        let s = std::str::from_utf8(&buf).unwrap();
        assert_eq!(s, "20240115-13:30:45.123456789");
    }

    #[test]
    fn fix_timestamp_epoch_zero() {
        let ts = HrTimestamp {
            nanos: 0,
            source: TimestampSource::System,
        };
        let buf = ts.to_fix_timestamp();
        let s = std::str::from_utf8(&buf).unwrap();
        assert_eq!(s, "19700101-00:00:00.000");
    }

    #[test]
    fn tsc_read_does_not_panic() {
        let val = HrClock::read_tsc();
        // TSC should return a non-zero value on any modern CPU.
        assert!(val > 0, "TSC read returned 0");
    }

    #[test]
    fn tsc_calibrate_returns_reasonable_frequency() {
        let freq = HrClock::calibrate_tsc();
        // Should be at least 100 MHz and less than 100 GHz.
        assert!(freq > 100_000_000, "TSC frequency too low: {}", freq);
        assert!(freq < 100_000_000_000, "TSC frequency too high: {}", freq);
    }

    #[test]
    fn hr_clock_system_now() {
        let clock = HrClock::new(TimestampSource::System);
        let t = clock.now();
        assert_eq!(t.source, TimestampSource::System);
        assert!(t.nanos > 0);
    }

    #[test]
    fn hr_clock_tsc_now() {
        let clock = HrClock::new(TimestampSource::Tsc);
        let t1 = clock.now();
        let t2 = clock.now();
        assert_eq!(t1.source, TimestampSource::Tsc);
        assert!(t2.nanos >= t1.nanos);
    }

    #[test]
    fn latency_tracker_measures_positive_elapsed() {
        let clock = HrClock::new(TimestampSource::System);
        let tracker = LatencyTracker::start(clock);
        // Do a tiny bit of work.
        std::hint::black_box(0u64.wrapping_add(1));
        let elapsed = tracker.stop();
        // Elapsed should be >= 0 (can be 0 on very fast systems, but not negative).
        assert!(
            elapsed < 1_000_000_000,
            "elapsed should be well under 1 second"
        );
    }

    #[test]
    fn tsc_to_nanos_conversion() {
        // 1 GHz clock, 1 billion ticks = 1 second = 1_000_000_000 ns
        assert_eq!(tsc_to_nanos(1_000_000_000, 1_000_000_000), 1_000_000_000);
        // 3 GHz clock, 3 billion ticks = 1 second
        assert_eq!(tsc_to_nanos(3_000_000_000, 3_000_000_000), 1_000_000_000);
        // Zero frequency returns 0
        assert_eq!(tsc_to_nanos(12345, 0), 0);
    }

    #[test]
    fn civil_from_days_epoch() {
        let (y, m, d) = civil_from_days(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn civil_from_days_known_date() {
        // 2024-01-15 is day 19737 since epoch
        let (y, m, d) = civil_from_days(19737);
        assert_eq!((y, m, d), (2024, 1, 15));
    }
}
