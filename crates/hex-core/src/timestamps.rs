//! Timestamp handling: device clocks, host clocks, and timebase metadata.

use serde::{Deserialize, Serialize};

use crate::Real;

/// Where a sample's time value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeSource {
    /// The device stamped the sample before sending it.
    Device,
    /// The host stamped the sample when it arrived.
    Host,
    /// The source file provided no time and one was synthesised from an index.
    Synthesized,
    /// The source declared a clock that HexaDOF could not classify.
    #[default]
    Unknown,
}

impl TimeSource {
    pub fn label(self) -> &'static str {
        match self {
            TimeSource::Device => "Device clock",
            TimeSource::Host => "Host clock",
            TimeSource::Synthesized => "Synthesized from sample index",
            TimeSource::Unknown => "Unknown",
        }
    }

    /// True when the clock is subject to rollover or drift that the user must
    /// be warned about.
    pub fn is_uncertain(self) -> bool {
        matches!(self, TimeSource::Unknown | TimeSource::Synthesized)
    }
}

/// The units a raw timestamp column is expressed in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimestampUnit {
    #[default]
    Seconds,
    Milliseconds,
    Microseconds,
    /// Device ticks; the tick rate is required to convert.
    Ticks(Real),
}

impl TimestampUnit {
    /// Multiplier converting a raw value to seconds.
    pub fn to_seconds_factor(self) -> Option<Real> {
        match self {
            TimestampUnit::Seconds => Some(1.0),
            TimestampUnit::Milliseconds => Some(1e-3),
            TimestampUnit::Microseconds => Some(1e-6),
            TimestampUnit::Ticks(rate) if rate > 0.0 => Some(1.0 / rate),
            TimestampUnit::Ticks(_) => None,
        }
    }

    pub fn label(self) -> String {
        match self {
            TimestampUnit::Seconds => "seconds".to_string(),
            TimestampUnit::Milliseconds => "milliseconds".to_string(),
            TimestampUnit::Microseconds => "microseconds".to_string(),
            TimestampUnit::Ticks(rate) => format!("device ticks at {} Hz", rate),
        }
    }
}

/// A single timestamp, always stored internally in seconds.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub Real);

impl Timestamp {
    pub fn from_seconds(s: Real) -> Self {
        Self(s)
    }

    pub fn from_millis(ms: Real) -> Self {
        Self(ms * 1e-3)
    }

    pub fn from_micros(us: Real) -> Self {
        Self(us * 1e-6)
    }

    /// Convert from a raw value using an explicit unit.
    pub fn from_raw(value: Real, unit: TimestampUnit) -> Option<Self> {
        unit.to_seconds_factor().map(|f| Self(value * f))
    }

    pub fn seconds(self) -> Real {
        self.0
    }

    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }
}

/// Record of how a timebase was established, stored with every data product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timebase {
    /// Clock the raw values came from.
    pub source: TimeSource,
    /// Unit the raw values were expressed in.
    pub unit: TimestampUnit,
    /// Offset subtracted from raw time so the first sample sits at zero.
    pub start_offset: Real,
    /// Estimated nominal sample interval in seconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nominal_dt: Option<Real>,
    /// True when the clock is known to wrap (for example a 32-bit microsecond
    /// counter) and rollover correction was applied.
    #[serde(default)]
    pub rollover_corrected: bool,
    /// Free-text note shown to the user, for example when host time was used as
    /// a fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Default for Timebase {
    fn default() -> Self {
        Self {
            source: TimeSource::Unknown,
            unit: TimestampUnit::Seconds,
            start_offset: 0.0,
            nominal_dt: None,
            rollover_corrected: false,
            note: None,
        }
    }
}

impl Timebase {
    pub fn device_seconds() -> Self {
        Self {
            source: TimeSource::Device,
            unit: TimestampUnit::Seconds,
            ..Self::default()
        }
    }

    /// Rebase a raw timestamp into the corrected timebase.
    pub fn correct(&self, raw: Real) -> Real {
        raw - self.start_offset
    }

    /// Apply the timebase to a whole raw column, returning corrected seconds.
    pub fn apply(&self, raw: &[Real]) -> Vec<Real> {
        let factor = self.unit.to_seconds_factor().unwrap_or(1.0);
        raw.iter().map(|v| v * factor - self.start_offset).collect()
    }

    /// Estimated sample rate in hertz, when a nominal interval exists.
    pub fn nominal_rate_hz(&self) -> Option<Real> {
        self.nominal_dt.filter(|dt| *dt > 0.0).map(|dt| 1.0 / dt)
    }
}

/// Analysis of a time column: gaps, duplicates, and reversals.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClockAnalysis {
    /// Number of samples examined.
    pub sample_count: usize,
    /// Median sample interval in seconds.
    pub median_dt: Real,
    /// Smallest observed positive interval.
    pub min_dt: Real,
    /// Largest observed interval.
    pub max_dt: Real,
    /// Effective sample rate from the median interval.
    pub effective_rate_hz: Real,
    /// Indices where time did not increase.
    pub non_monotonic_indices: Vec<usize>,
    /// Indices where time jumped backwards by more than the gap threshold.
    pub reversal_indices: Vec<usize>,
    /// Runs of samples separated by more than `gap_threshold * median_dt`.
    pub gaps: Vec<Gap>,
    /// Duplicate timestamp indices.
    pub duplicate_indices: Vec<usize>,
    /// True when the whole series is strictly increasing.
    pub strictly_increasing: bool,
}

/// A detected gap in a time series.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Gap {
    /// Index of the sample before the gap.
    pub after_index: usize,
    /// Time at the start of the gap, in seconds.
    pub start_time: Real,
    /// Time at the end of the gap, in seconds.
    pub end_time: Real,
    /// Duration of the gap in seconds.
    pub duration: Real,
    /// Number of samples missing, estimated from the median interval.
    pub estimated_missing: usize,
}

/// Analyse a corrected time column.
///
/// `gap_factor` is the multiple of the median interval above which an interval
/// is reported as a gap. A typical value is 3.0.
pub fn analyze_clock(times: &[Real], gap_factor: Real) -> ClockAnalysis {
    let mut dts: Vec<Real> = Vec::with_capacity(times.len().saturating_sub(1));
    for w in times.windows(2) {
        let d = w[1] - w[0];
        if d.is_finite() {
            dts.push(d);
        }
    }

    let mut sorted: Vec<Real> = dts.iter().copied().filter(|d| *d > 0.0).collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let median_dt = if sorted.is_empty() {
        0.0
    } else {
        sorted[sorted.len() / 2]
    };
    let min_dt = sorted.first().copied().unwrap_or(0.0);
    let max_dt = sorted.last().copied().unwrap_or(0.0);
    let effective_rate_hz = if median_dt > 0.0 {
        1.0 / median_dt
    } else {
        0.0
    };

    let mut non_monotonic_indices = Vec::new();
    let mut reversal_indices = Vec::new();
    let mut duplicate_indices = Vec::new();
    let mut gaps = Vec::new();

    let gap_threshold = if median_dt > 0.0 {
        median_dt * gap_factor.max(1.0)
    } else {
        Real::INFINITY
    };

    for i in 1..times.len() {
        let d = times[i] - times[i - 1];
        if d <= 0.0 {
            non_monotonic_indices.push(i);
            if d < 0.0 {
                reversal_indices.push(i);
            }
            if d == 0.0 {
                duplicate_indices.push(i);
            }
        } else if d > gap_threshold {
            let estimated_missing = if median_dt > 0.0 {
                ((d / median_dt).round() as isize - 1).max(0) as usize
            } else {
                0
            };
            gaps.push(Gap {
                after_index: i - 1,
                start_time: times[i - 1],
                end_time: times[i],
                duration: d,
                estimated_missing,
            });
        }
    }

    ClockAnalysis {
        sample_count: times.len(),
        median_dt,
        min_dt,
        max_dt,
        effective_rate_hz,
        strictly_increasing: non_monotonic_indices.is_empty(),
        non_monotonic_indices,
        reversal_indices,
        duplicate_indices,
        gaps,
    }
}

/// Correction applied to a wrapping counter clock.
///
/// A 32-bit microsecond counter wraps roughly every 71 minutes, which is short
/// enough to matter for a long flight log.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RolloverCorrection {
    /// Counter modulus, for example `2^32`.
    pub modulus: Real,
    /// Number of wraps detected.
    pub wraps_detected: usize,
}

impl RolloverCorrection {
    pub fn for_bits(bits: u32) -> Self {
        Self {
            modulus: 2f64.powi(bits as i32),
            wraps_detected: 0,
        }
    }

    /// Unwrap a raw counter series into a monotonically increasing series.
    pub fn apply(&mut self, raw: &[Real]) -> Vec<Real> {
        let mut out = Vec::with_capacity(raw.len());
        if raw.is_empty() {
            return out;
        }
        let mut offset = 0.0;
        let mut previous = raw[0];
        out.push(previous);
        for &value in &raw[1..] {
            if value < previous - self.modulus * 0.5 {
                offset += self.modulus;
                self.wraps_detected += 1;
            }
            let corrected = value + offset;
            out.push(corrected);
            previous = value;
        }
        out
    }
}

/// A device clock offset estimate between device time and host time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DeviceClock {
    /// Device time minus host time, in seconds, at the last update.
    pub offset: Real,
    /// Exponential smoothing factor used when updating the offset.
    pub smoothing: Real,
    /// Number of updates applied.
    pub updates: usize,
}

impl Default for DeviceClock {
    fn default() -> Self {
        Self {
            offset: 0.0,
            smoothing: 0.05,
            updates: 0,
        }
    }
}

impl DeviceClock {
    pub fn new(smoothing: Real) -> Self {
        Self {
            offset: 0.0,
            smoothing: smoothing.clamp(1e-4, 1.0),
            updates: 0,
        }
    }

    /// Fold in a paired (device, host) observation.
    pub fn update(&mut self, device_time: Real, host_time: Real) {
        let observed = device_time - host_time;
        if !observed.is_finite() {
            return;
        }
        if self.updates == 0 {
            self.offset = observed;
        } else {
            self.offset += self.smoothing * (observed - self.offset);
        }
        self.updates += 1;
    }

    /// Convert a device time into host time.
    pub fn to_host(&self, device_time: Real) -> Real {
        device_time - self.offset
    }

    /// Convert a host time into device time.
    pub fn to_device(&self, host_time: Real) -> Real {
        host_time + self.offset
    }

    /// True once enough observations exist for the offset to be meaningful.
    pub fn is_locked(&self) -> bool {
        self.updates >= 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_units_convert() {
        assert!((Timestamp::from_millis(1500.0).seconds() - 1.5).abs() < 1e-12);
        assert!((Timestamp::from_micros(2_000_000.0).seconds() - 2.0).abs() < 1e-12);
        let t = Timestamp::from_raw(50.0, TimestampUnit::Ticks(100.0)).unwrap();
        assert!((t.seconds() - 0.5).abs() < 1e-12);
        assert!(Timestamp::from_raw(1.0, TimestampUnit::Ticks(0.0)).is_none());
    }

    #[test]
    fn timebase_applies_scale_and_offset() {
        let tb = Timebase {
            source: TimeSource::Device,
            unit: TimestampUnit::Milliseconds,
            start_offset: 1.0,
            ..Default::default()
        };
        let out = tb.apply(&[1000.0, 1500.0, 2000.0]);
        assert!((out[0] - 0.0).abs() < 1e-12);
        assert!((out[1] - 0.5).abs() < 1e-12);
        assert!((out[2] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn clock_analysis_finds_gaps_and_duplicates() {
        let times = vec![0.0, 0.01, 0.02, 0.02, 0.03, 0.53, 0.54];
        let a = analyze_clock(&times, 3.0);
        assert_eq!(a.sample_count, 7);
        assert!((a.median_dt - 0.01).abs() < 1e-12);
        assert!(a.effective_rate_hz > 99.0 && a.effective_rate_hz < 101.0);
        assert!(!a.strictly_increasing);
        assert_eq!(a.duplicate_indices, vec![3]);
        assert_eq!(a.gaps.len(), 1);
        assert_eq!(a.gaps[0].after_index, 4);
        assert!(a.gaps[0].estimated_missing >= 49);
    }

    #[test]
    fn clock_analysis_reports_reversals() {
        let times = vec![0.0, 0.1, 0.05, 0.15];
        let a = analyze_clock(&times, 3.0);
        assert_eq!(a.reversal_indices, vec![2]);
        assert!(!a.strictly_increasing);
    }

    #[test]
    fn clock_analysis_handles_clean_series() {
        let times: Vec<Real> = (0..100).map(|i| i as Real * 0.005).collect();
        let a = analyze_clock(&times, 3.0);
        assert!(a.strictly_increasing);
        assert!(a.gaps.is_empty());
        assert!(a.duplicate_indices.is_empty());
        assert!((a.effective_rate_hz - 200.0).abs() < 1e-6);
    }

    #[test]
    fn rollover_correction_unwraps_counter() {
        let mut r = RolloverCorrection::for_bits(8);
        let raw = vec![250.0, 254.0, 2.0, 6.0, 10.0];
        let out = r.apply(&raw);
        assert_eq!(r.wraps_detected, 1);
        assert!(out.windows(2).all(|w| w[1] >= w[0]));
        assert!((out[2] - 258.0).abs() < 1e-9);
    }

    #[test]
    fn device_clock_smooths_toward_observation() {
        let mut c = DeviceClock::new(0.5);
        c.update(10.0, 9.0); // offset 1.0
        assert!((c.offset - 1.0).abs() < 1e-12);
        c.update(12.0, 11.5); // observed 0.5, smoothed to 0.75
        assert!((c.offset - 1.25).abs() > 0.0);
        assert!(c.updates == 2);
        assert!((c.to_host(12.0) - (12.0 - c.offset)).abs() < 1e-12);
        assert!((c.to_device(c.to_host(12.0)) - 12.0).abs() < 1e-12);
    }

    #[test]
    fn device_clock_locks_after_several_updates() {
        let mut c = DeviceClock::default();
        assert!(!c.is_locked());
        for i in 0..5 {
            c.update(i as Real + 5.0, i as Real);
        }
        assert!(c.is_locked());
    }
}
