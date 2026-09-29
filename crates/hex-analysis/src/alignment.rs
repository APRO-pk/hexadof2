//! Time alignment between two data series.
//!
//! Alignment is the first-class operation in a comparison, and the method used is
//! recorded with the result. A comparison that silently picks an alignment is
//! worse than no comparison, because the error metrics then measure the alignment
//! choice rather than the physics.

use hex_core::{Real, Vec3};
use serde::{Deserialize, Serialize};

/// How two series are put onto a common timebase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum AlignmentMethod {
    /// Use the absolute timestamps as recorded. Correct only when both series
    /// already share a clock.
    Absolute,
    /// Align the first sample of each series.
    FirstSample,
    /// Align a named event in each series, for example lift-off.
    Event {
        /// Event code in the reference series, for example `lift_off`.
        reference_event: EventRef,
        /// Event code in the comparison series.
        comparison_event: EventRef,
    },
    /// Shift the comparison series so a chosen channel correlates best with the
    /// reference. The lag search is bounded.
    CrossCorrelation {
        /// Channel to correlate, given as a plain value series.
        search_half_width_seconds: Real,
        /// Step of the lag search, seconds.
        step_seconds: Real,
    },
    /// Apply a manual offset entered by the user.
    Manual {
        /// Offset added to the comparison series times, seconds.
        offset_seconds: Real,
    },
}

/// A reference to an event by code, so an alignment survives a re-run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRef {
    /// Event code, for example `apogee`.
    pub code: String,
    /// Optional occurrence index when an event fires more than once.
    pub occurrence: usize,
}

impl EventRef {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            occurrence: 0,
        }
    }

    pub fn nth(code: impl Into<String>, occurrence: usize) -> Self {
        Self {
            code: code.into(),
            occurrence,
        }
    }
}

impl AlignmentMethod {
    pub fn label(&self) -> String {
        match self {
            AlignmentMethod::Absolute => "Absolute timestamps".to_string(),
            AlignmentMethod::FirstSample => "First valid sample".to_string(),
            AlignmentMethod::Event {
                reference_event,
                comparison_event,
            } => format!(
                "Event: {} against {}",
                reference_event.code, comparison_event.code
            ),
            AlignmentMethod::CrossCorrelation {
                search_half_width_seconds,
                ..
            } => format!(
                "Cross-correlation within {:.3} s",
                search_half_width_seconds
            ),
            AlignmentMethod::Manual { offset_seconds } => {
                format!("Manual offset {:.3} s", offset_seconds)
            }
        }
    }

    /// Whether the method relies on the two series sharing a clock.
    pub fn requires_shared_clock(&self) -> bool {
        matches!(self, AlignmentMethod::Absolute)
    }
}

/// The result of an alignment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    /// Method that produced this alignment.
    pub method: AlignmentMethod,
    /// Offset added to the comparison series times, seconds.
    pub offset_seconds: Real,
    /// Quality of the alignment, between 0 and 1, when the method can measure it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<Real>,
    /// Time span over which the two series overlap after alignment, seconds.
    pub overlap_seconds: Real,
    /// Start of the overlap in reference time, seconds.
    pub overlap_start: Real,
    /// End of the overlap in reference time, seconds.
    pub overlap_end: Real,
}

impl Alignment {
    /// Whether the overlap is long enough to be worth comparing.
    pub fn is_usable(&self, minimum_overlap_seconds: Real) -> bool {
        self.overlap_seconds >= minimum_overlap_seconds
    }

    /// A warning when the alignment is weak.
    pub fn warning(&self) -> Option<String> {
        match self.quality {
            Some(q) if q < 0.5 => Some(format!(
                "The alignment quality is {:.2}. The offset may be wrong, which would make every error metric misleading.",
                q
            )),
            _ if self.overlap_seconds <= 0.0 => Some(
                "The two series do not overlap after alignment, so no comparison is possible."
                    .to_string(),
            ),
            _ => None,
        }
    }

    pub fn describe(&self) -> String {
        format!(
            "{}, offset {:+.4} s, overlap {:.3} s",
            self.method.label(),
            self.offset_seconds,
            self.overlap_seconds
        )
    }
}

/// Failures an alignment can hit.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AlignmentError {
    #[error("The reference series is empty.")]
    EmptyReference,
    #[error("The comparison series is empty.")]
    EmptyComparison,
    #[error("The event {code} was not found in the {series} series.")]
    MissingEvent { code: String, series: &'static str },
    #[error("The two series do not overlap in time.")]
    NoOverlap,
    #[error("Cross-correlation needs both series to share a sampled channel.")]
    ChannelMissing,
    #[error("The alignment offset is not finite.")]
    NonFinite,
}

impl AlignmentError {
    pub fn user_message(&self) -> String {
        format!("{}.", self.to_string().trim_end_matches('.'))
    }

    pub fn suggested_action(&self) -> &'static str {
        match self {
            AlignmentError::EmptyReference | AlignmentError::EmptyComparison => {
                "Import a flight log and load a simulation run first."
            }
            AlignmentError::MissingEvent { .. } => {
                "Add the event marker to both series, or choose a different alignment method."
            }
            AlignmentError::NoOverlap => {
                "Check that both series describe the same flight, then try a manual offset."
            }
            AlignmentError::ChannelMissing => {
                "Choose a channel that exists in both series, or align by event instead."
            }
            AlignmentError::NonFinite => "Enter a finite offset in seconds.",
        }
    }
}

/// A minimal time series the aligner can work with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimeSeries {
    /// Time values in seconds, increasing.
    pub times: Vec<Real>,
    /// An optional named value channel used by cross-correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<Vec<Real>>,
    /// Event times by code, in seconds.
    #[serde(default)]
    pub events: Vec<(String, Real)>,
}

impl TimeSeries {
    pub fn from_times(times: Vec<Real>) -> Self {
        Self {
            times,
            values: None,
            events: Vec::new(),
        }
    }

    pub fn with_values(mut self, values: Vec<Real>) -> Self {
        self.values = Some(values);
        self
    }

    pub fn with_event(mut self, code: impl Into<String>, time: Real) -> Self {
        self.events.push((code.into(), time));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    pub fn start(&self) -> Option<Real> {
        self.times.first().copied()
    }

    pub fn end(&self) -> Option<Real> {
        self.times.last().copied()
    }

    pub fn span(&self) -> Real {
        match (self.start(), self.end()) {
            (Some(a), Some(b)) => b - a,
            _ => 0.0,
        }
    }

    /// Event time for a reference, honouring the occurrence index.
    pub fn event_time(&self, event: &EventRef) -> Option<Real> {
        let mut seen = 0usize;
        for (code, time) in &self.events {
            if code == &event.code {
                if seen == event.occurrence {
                    return Some(*time);
                }
                seen += 1;
            }
        }
        None
    }

    /// Median sample interval, used as the natural step for interpolation.
    pub fn median_dt(&self) -> Real {
        if self.times.len() < 2 {
            return 0.0;
        }
        let mut dts: Vec<Real> = self
            .times
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|d| *d > 0.0 && d.is_finite())
            .collect();
        if dts.is_empty() {
            return 0.0;
        }
        dts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        dts[dts.len() / 2]
    }
}

/// Align two series with the chosen method.
pub fn align(
    reference: &TimeSeries,
    comparison: &TimeSeries,
    method: AlignmentMethod,
) -> Result<Alignment, AlignmentError> {
    if reference.is_empty() {
        return Err(AlignmentError::EmptyReference);
    }
    if comparison.is_empty() {
        return Err(AlignmentError::EmptyComparison);
    }

    let (offset, quality) = match method.clone() {
        AlignmentMethod::Absolute => (0.0, Some(1.0)),
        AlignmentMethod::FirstSample => {
            let a = reference.start().unwrap_or(0.0);
            let b = comparison.start().unwrap_or(0.0);
            (a - b, Some(1.0))
        }
        AlignmentMethod::Event {
            reference_event,
            comparison_event,
        } => {
            let a = reference.event_time(&reference_event).ok_or_else(|| {
                AlignmentError::MissingEvent {
                    code: reference_event.code.clone(),
                    series: "reference",
                }
            })?;
            let b = comparison.event_time(&comparison_event).ok_or_else(|| {
                AlignmentError::MissingEvent {
                    code: comparison_event.code.clone(),
                    series: "comparison",
                }
            })?;
            (a - b, Some(1.0))
        }
        AlignmentMethod::Manual { offset_seconds } => {
            if !offset_seconds.is_finite() {
                return Err(AlignmentError::NonFinite);
            }
            (offset_seconds, Some(1.0))
        }
        AlignmentMethod::CrossCorrelation {
            search_half_width_seconds,
            step_seconds,
        } => {
            let a = reference
                .values
                .as_ref()
                .ok_or(AlignmentError::ChannelMissing)?;
            let b = comparison
                .values
                .as_ref()
                .ok_or(AlignmentError::ChannelMissing)?;
            cross_correlate(
                &reference.times,
                a,
                &comparison.times,
                b,
                search_half_width_seconds,
                step_seconds,
            )?
        }
    };

    let ref_start = reference.start().unwrap_or(0.0);
    let ref_end = reference.end().unwrap_or(0.0);
    // The comparison series occupies [start + offset, end + offset] on the
    // reference timebase.
    let cmp_start = comparison.start().unwrap_or(0.0) + offset;
    let cmp_end = comparison.end().unwrap_or(0.0) + offset;

    let overlap_start = ref_start.max(cmp_start);
    let overlap_end = ref_end.min(cmp_end);
    let overlap_seconds = (overlap_end - overlap_start).max(0.0);

    Ok(Alignment {
        method,
        offset_seconds: offset,
        quality,
        overlap_seconds,
        overlap_start,
        overlap_end,
    })
}

/// Find the lag that best correlates two sampled channels.
///
/// The search is bounded by `half_width` and stepped by `step`, and the return
/// value is the offset to add to the comparison times. Values are linearly
/// interpolated at the shifted times, and only lags with at least four
/// overlapping samples are considered, so a short accidental overlap cannot win.
fn cross_correlate(
    ref_times: &[Real],
    ref_values: &[Real],
    cmp_times: &[Real],
    cmp_values: &[Real],
    half_width: Real,
    step: Real,
) -> Result<(Real, Option<Real>), AlignmentError> {
    if ref_times.len() != ref_values.len() || cmp_times.len() != cmp_values.len() {
        return Err(AlignmentError::ChannelMissing);
    }
    if !half_width.is_finite() || half_width <= 0.0 {
        return Err(AlignmentError::NonFinite);
    }
    let step = if step.is_finite() && step > 0.0 {
        step
    } else {
        half_width / 200.0
    };

    let mut best_offset = 0.0;
    let mut best_score = Real::NEG_INFINITY;

    let steps = ((2.0 * half_width) / step).ceil() as usize;
    for i in 0..=steps {
        let offset = -half_width + i as Real * step;
        let score = correlation_score(ref_times, ref_values, cmp_times, cmp_values, offset);
        if let Some(s) = score {
            if s > best_score {
                best_score = s;
                best_offset = offset;
            }
        }
    }

    if !best_score.is_finite() {
        return Err(AlignmentError::NoOverlap);
    }

    // Map the peak correlation onto a rough quality figure. A perfect match gives
    // 1, no correlation gives 0.
    let quality = Some(best_score.clamp(0.0, 1.0));
    Ok((best_offset, quality))
}

/// Normalised correlation coefficient at one lag.
///
/// Returns `None` when fewer than four samples overlap or either series has no
/// variance, which is the honest answer rather than a spurious zero.
fn correlation_score(
    ref_times: &[Real],
    ref_values: &[Real],
    cmp_times: &[Real],
    cmp_values: &[Real],
    offset: Real,
) -> Option<Real> {
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for (t, v) in ref_times.iter().zip(ref_values.iter()) {
        let target = t - offset;
        if let Some(sample) = sample_at(cmp_times, cmp_values, target) {
            if v.is_finite() && sample.is_finite() {
                xs.push(*v);
                ys.push(sample);
            }
        }
    }
    if xs.len() < 4 {
        return None;
    }
    let n = xs.len() as Real;
    let mx = xs.iter().sum::<Real>() / n;
    let my = ys.iter().sum::<Real>() / n;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (x, y) in xs.iter().zip(ys.iter()) {
        let dx = x - mx;
        let dy = y - my;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx < 1e-18 || syy < 1e-18 {
        return None;
    }
    Some(sxy / (sxx.sqrt() * syy.sqrt()))
}

/// Linear interpolation of a sampled channel at a time.
pub fn sample_at(times: &[Real], values: &[Real], t: Real) -> Option<Real> {
    if times.len() != values.len() || times.is_empty() {
        return None;
    }
    if t < times[0] || t > times[times.len() - 1] {
        return None;
    }
    let idx = match times.partition_point(|x| *x <= t) {
        0 => 0,
        i if i >= times.len() => times.len() - 1,
        i => i - 1,
    };
    if idx + 1 >= times.len() {
        return Some(values[times.len() - 1]);
    }
    let span = times[idx + 1] - times[idx];
    if span <= 0.0 {
        return Some(values[idx]);
    }
    let f = (t - times[idx]) / span;
    Some(values[idx] * (1.0 - f) + values[idx + 1] * f)
}

/// Resample a series onto a reference timebase, after applying an offset.
///
/// Times outside the source range produce `None`, so the caller can distinguish
/// extrapolation from a genuine sample. Fabricating a value outside the recorded
/// range would be indistinguishable from data.
pub fn resample(
    source_times: &[Real],
    source_values: &[Real],
    target_times: &[Real],
    offset: Real,
) -> Vec<Option<Real>> {
    target_times
        .iter()
        .map(|t| sample_at(source_times, source_values, t - offset))
        .collect()
}

/// Resample a vector channel onto a reference timebase.
pub fn resample_vectors(
    source_times: &[Real],
    source_values: &[Vec3],
    target_times: &[Real],
    offset: Real,
) -> Vec<Option<Vec3>> {
    target_times
        .iter()
        .map(|t| {
            let target = t - offset;
            if source_times.len() != source_values.len() || source_times.is_empty() {
                return None;
            }
            if target < source_times[0] || target > source_times[source_times.len() - 1] {
                return None;
            }
            let idx = match source_times.partition_point(|x| *x <= target) {
                0 => 0,
                i if i >= source_times.len() => source_times.len() - 1,
                i => i - 1,
            };
            if idx + 1 >= source_times.len() {
                return Some(source_values[source_values.len() - 1]);
            }
            let span = source_times[idx + 1] - source_times[idx];
            if span <= 0.0 {
                return Some(source_values[idx]);
            }
            let f = (target - source_times[idx]) / span;
            Some(source_values[idx] * (1.0 - f) + source_values[idx + 1] * f)
        })
        .collect()
}

/// The time-to-event difference between two series for a matching pair of codes.
pub fn time_to_event_difference(
    reference: &TimeSeries,
    comparison: &TimeSeries,
    reference_event: &EventRef,
    comparison_event: &EventRef,
) -> Option<Real> {
    let a = reference.event_time(reference_event)?;
    let b = comparison.event_time(comparison_event)?;
    Some(a - b)
}

/// The set of alignment methods worth offering for a pair of series.
///
/// A method that cannot work is not offered, so the UI never presents an option
/// that will fail.
pub fn available_methods(reference: &TimeSeries, comparison: &TimeSeries) -> Vec<AlignmentMethod> {
    let mut methods = vec![AlignmentMethod::Absolute, AlignmentMethod::FirstSample];
    let common_events: Vec<String> = reference
        .events
        .iter()
        .filter(|(code, _)| comparison.events.iter().any(|(c, _)| c == code))
        .map(|(code, _)| code.clone())
        .collect();
    if let Some(code) = common_events.first() {
        methods.push(AlignmentMethod::Event {
            reference_event: EventRef::new(code.clone()),
            comparison_event: EventRef::new(code.clone()),
        });
    }
    if reference.values.is_some() && comparison.values.is_some() {
        methods.push(AlignmentMethod::CrossCorrelation {
            search_half_width_seconds: (reference.span().min(comparison.span()) * 0.25).max(0.1),
            step_seconds: 0.01,
        });
    }
    methods.push(AlignmentMethod::Manual {
        offset_seconds: 0.0,
    });
    methods
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(start: Real, dt: Real, n: usize) -> TimeSeries {
        let times: Vec<Real> = (0..n).map(|i| start + i as Real * dt).collect();
        let values: Vec<Real> = times.iter().map(|t| (*t - start) * 2.0).collect();
        TimeSeries::from_times(times).with_values(values)
    }

    #[test]
    fn absolute_alignment_uses_no_offset() {
        let a = ramp(0.0, 0.01, 100);
        let b = ramp(0.0, 0.01, 100);
        let alignment = align(&a, &b, AlignmentMethod::Absolute).unwrap();
        assert!((alignment.offset_seconds).abs() < 1e-15);
        assert!(alignment.is_usable(0.5));
        assert!(alignment.warning().is_none());
        assert!(alignment.describe().contains("Absolute"));
    }

    #[test]
    fn first_sample_alignment_removes_a_constant_start_difference() {
        let a = ramp(10.0, 0.01, 50);
        let b = ramp(4.0, 0.01, 50);
        let alignment = align(&a, &b, AlignmentMethod::FirstSample).unwrap();
        assert!((alignment.offset_seconds - 6.0).abs() < 1e-12);
        // After alignment the spans coincide.
        assert!((alignment.overlap_seconds - b.span()).abs() < 1e-9);
    }

    #[test]
    fn event_alignment_uses_the_matching_events() {
        let a = ramp(0.0, 0.01, 100).with_event("lift_off", 2.0);
        let b = ramp(0.0, 0.01, 100).with_event("lift_off", 2.5);
        let alignment = align(
            &a,
            &b,
            AlignmentMethod::Event {
                reference_event: EventRef::new("lift_off"),
                comparison_event: EventRef::new("lift_off"),
            },
        )
        .unwrap();
        assert!((alignment.offset_seconds + 0.5).abs() < 1e-12);
        assert_eq!(alignment.method.label(), "Event: lift_off against lift_off");
    }

    #[test]
    fn event_alignment_reports_a_missing_event() {
        let a = ramp(0.0, 0.01, 100).with_event("lift_off", 1.0);
        let b = ramp(0.0, 0.01, 100);
        let err = align(
            &a,
            &b,
            AlignmentMethod::Event {
                reference_event: EventRef::new("lift_off"),
                comparison_event: EventRef::new("lift_off"),
            },
        )
        .unwrap_err();
        assert!(matches!(err, AlignmentError::MissingEvent { .. }));
        assert!(err.user_message().contains("lift_off"));
        assert!(!err.suggested_action().is_empty());
    }

    #[test]
    fn event_occurrence_index_selects_the_right_event() {
        let series = TimeSeries::from_times(vec![0.0, 1.0, 2.0, 3.0])
            .with_event("peak", 1.0)
            .with_event("peak", 3.0);
        assert_eq!(series.event_time(&EventRef::new("peak")), Some(1.0));
        assert_eq!(series.event_time(&EventRef::nth("peak", 1)), Some(3.0));
        assert_eq!(series.event_time(&EventRef::nth("peak", 5)), None);
    }

    #[test]
    fn manual_alignment_uses_the_supplied_offset() {
        let a = ramp(0.0, 0.01, 100);
        let b = ramp(0.0, 0.01, 100);
        let alignment = align(
            &a,
            &b,
            AlignmentMethod::Manual {
                offset_seconds: -0.25,
            },
        )
        .unwrap();
        assert!((alignment.offset_seconds + 0.25).abs() < 1e-15);
    }

    #[test]
    fn manual_alignment_rejects_a_non_finite_offset() {
        let a = ramp(0.0, 0.01, 10);
        let b = ramp(0.0, 0.01, 10);
        let err = align(
            &a,
            &b,
            AlignmentMethod::Manual {
                offset_seconds: Real::NAN,
            },
        )
        .unwrap_err();
        assert_eq!(err, AlignmentError::NonFinite);
    }

    #[test]
    fn cross_correlation_recovers_a_known_shift() {
        // Both series carry the same signal; the comparison is 0.3 s late, so the
        // offset that pulls it back is -0.3, meaning the comparison times are
        // shifted by -0.3.
        let n = 400;
        let dt = 0.01;
        let a_times: Vec<Real> = (0..n).map(|i| i as Real * dt).collect();
        let a_values: Vec<Real> = a_times
            .iter()
            .map(|t| (t * 6.0).sin() + (t * 1.3).sin() * 0.4)
            .collect();
        let shift = 0.3;
        let b_times: Vec<Real> = a_times.iter().map(|t| t + shift).collect();
        let b_values: Vec<Real> = a_times
            .iter()
            .map(|t| ((t) * 6.0).sin() + (t * 1.3).sin() * 0.4)
            .collect();

        let a = TimeSeries::from_times(a_times).with_values(a_values);
        let b = TimeSeries::from_times(b_times).with_values(b_values);
        let alignment = align(
            &a,
            &b,
            AlignmentMethod::CrossCorrelation {
                search_half_width_seconds: 1.0,
                step_seconds: 0.005,
            },
        )
        .unwrap();
        // The comparison is later by 0.3 s, so the offset must be about -0.3.
        assert!(
            (alignment.offset_seconds + shift).abs() < 0.02,
            "offset {}",
            alignment.offset_seconds
        );
        assert!(alignment.quality.unwrap() > 0.9);
    }

    #[test]
    fn cross_correlation_needs_a_channel() {
        let a = TimeSeries::from_times(vec![0.0, 1.0, 2.0]);
        let b = ramp(0.0, 0.5, 10);
        let err = align(
            &a,
            &b,
            AlignmentMethod::CrossCorrelation {
                search_half_width_seconds: 1.0,
                step_seconds: 0.1,
            },
        )
        .unwrap_err();
        assert_eq!(err, AlignmentError::ChannelMissing);
    }

    #[test]
    fn cross_correlation_needs_variance() {
        // Two flat channels have no correlation structure, so the aligner must
        // say so rather than reporting a confident zero lag.
        let times: Vec<Real> = (0..50).map(|i| i as Real * 0.1).collect();
        let a = TimeSeries::from_times(times.clone()).with_values(vec![1.0; 50]);
        let b = TimeSeries::from_times(times).with_values(vec![2.0; 50]);
        let err = align(
            &a,
            &b,
            AlignmentMethod::CrossCorrelation {
                search_half_width_seconds: 1.0,
                step_seconds: 0.1,
            },
        )
        .unwrap_err();
        assert_eq!(err, AlignmentError::NoOverlap);
    }

    #[test]
    fn empty_series_are_rejected() {
        let a = TimeSeries::from_times(vec![]);
        let b = ramp(0.0, 0.1, 10);
        assert_eq!(
            align(&a, &b, AlignmentMethod::Absolute).unwrap_err(),
            AlignmentError::EmptyReference
        );
        assert_eq!(
            align(&b, &a, AlignmentMethod::Absolute).unwrap_err(),
            AlignmentError::EmptyComparison
        );
    }

    #[test]
    fn non_overlapping_series_report_zero_overlap_and_a_warning() {
        let a = ramp(0.0, 0.1, 10);
        let b = ramp(100.0, 0.1, 10);
        let alignment = align(&a, &b, AlignmentMethod::Absolute).unwrap();
        assert_eq!(alignment.overlap_seconds, 0.0);
        assert!(alignment.warning().is_some());
        assert!(!alignment.is_usable(0.1));
    }

    #[test]
    fn weak_alignment_quality_produces_a_warning() {
        let alignment = Alignment {
            method: AlignmentMethod::Absolute,
            offset_seconds: 0.0,
            quality: Some(0.2),
            overlap_seconds: 5.0,
            overlap_start: 0.0,
            overlap_end: 5.0,
        };
        let warning = alignment.warning().unwrap();
        assert!(warning.contains("0.20"));
    }

    #[test]
    fn sample_at_interpolates_and_refuses_to_extrapolate() {
        let times = vec![0.0, 1.0, 2.0];
        let values = vec![0.0, 10.0, 20.0];
        assert_eq!(sample_at(&times, &values, 0.5), Some(5.0));
        assert_eq!(sample_at(&times, &values, 1.5), Some(15.0));
        assert_eq!(sample_at(&times, &values, 0.0), Some(0.0));
        assert_eq!(sample_at(&times, &values, 2.0), Some(20.0));
        assert_eq!(sample_at(&times, &values, 3.0), None);
        assert_eq!(sample_at(&times, &values, -0.1), None);
        assert_eq!(sample_at(&[], &[], 0.0), None);
    }

    #[test]
    fn resample_marks_out_of_range_targets_as_missing() {
        let source_times = vec![0.0, 1.0, 2.0];
        let source_values = vec![1.0, 2.0, 3.0];
        let targets = vec![-1.0, 0.5, 1.5, 3.0];
        let out = resample(&source_times, &source_values, &targets, 0.0);
        assert_eq!(out[0], None);
        assert_eq!(out[1], Some(1.5));
        assert_eq!(out[2], Some(2.5));
        assert_eq!(out[3], None);
    }

    #[test]
    fn resample_applies_the_offset() {
        let source_times = vec![1.0, 2.0, 3.0];
        let source_values = vec![10.0, 20.0, 30.0];
        let targets = vec![0.5];
        // With a +0.5 offset, the target 0.5 corresponds to source time 0.0,
        // which is outside the range.
        assert_eq!(
            resample(&source_times, &source_values, &targets, 0.5)[0],
            None
        );
        // With a -0.5 offset, target 0.5 maps to source 1.0 exactly.
        assert_eq!(
            resample(&source_times, &source_values, &targets, -0.5)[0],
            Some(10.0)
        );
    }

    #[test]
    fn resample_vectors_interpolates_each_component() {
        let times = vec![0.0, 1.0];
        let values = vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 4.0, 6.0)];
        let out = resample_vectors(&times, &values, &[0.5], 0.0);
        let v = out[0].unwrap();
        assert!((v - Vec3::new(1.0, 2.0, 3.0)).norm() < 1e-12);
        let out = resample_vectors(&times, &values, &[5.0], 0.0);
        assert!(out[0].is_none());
    }

    #[test]
    fn time_to_event_difference_is_signed() {
        let a = ramp(0.0, 0.1, 50).with_event("apogee", 3.0);
        let b = ramp(0.0, 0.1, 50).with_event("apogee", 2.5);
        let d =
            time_to_event_difference(&a, &b, &EventRef::new("apogee"), &EventRef::new("apogee"))
                .unwrap();
        assert!((d - 0.5).abs() < 1e-12);
        assert!(time_to_event_difference(
            &a,
            &b,
            &EventRef::new("burnout"),
            &EventRef::new("apogee")
        )
        .is_none());
    }

    #[test]
    fn median_dt_of_a_regular_series() {
        let s = ramp(0.0, 0.02, 50);
        assert!((s.median_dt() - 0.02).abs() < 1e-12);
        assert_eq!(TimeSeries::from_times(vec![0.0]).median_dt(), 0.0);
    }

    #[test]
    fn available_methods_only_offer_what_can_work() {
        let a = ramp(0.0, 0.01, 100).with_event("lift_off", 1.0);
        let b = ramp(0.0, 0.01, 100);
        let methods = available_methods(&a, &b);
        assert!(methods.contains(&AlignmentMethod::Absolute));
        assert!(methods.contains(&AlignmentMethod::FirstSample));
        assert!(methods.contains(&AlignmentMethod::Manual {
            offset_seconds: 0.0
        }));
        // No common event, so event alignment must not be offered.
        assert!(!methods
            .iter()
            .any(|m| matches!(m, AlignmentMethod::Event { .. })));
        // Both series have values, so correlation is offered.
        assert!(methods
            .iter()
            .any(|m| matches!(m, AlignmentMethod::CrossCorrelation { .. })));
    }

    #[test]
    fn available_methods_offer_event_alignment_when_shared() {
        let a = ramp(0.0, 0.01, 100).with_event("apogee", 1.0);
        let b = ramp(0.0, 0.01, 100).with_event("apogee", 2.0);
        let methods = available_methods(&a, &b);
        assert!(methods
            .iter()
            .any(|m| matches!(m, AlignmentMethod::Event { .. })));
    }

    #[test]
    fn method_metadata() {
        assert!(AlignmentMethod::Absolute.requires_shared_clock());
        assert!(!AlignmentMethod::FirstSample.requires_shared_clock());
        assert!(!AlignmentMethod::Manual {
            offset_seconds: 0.0
        }
        .requires_shared_clock());
    }

    #[test]
    fn series_accessors() {
        let s = ramp(2.0, 0.5, 5);
        assert_eq!(s.start(), Some(2.0));
        assert_eq!(s.end(), Some(4.0));
        assert!((s.span() - 2.0).abs() < 1e-12);
        assert!(!s.is_empty());
        assert!(TimeSeries::from_times(vec![]).is_empty());
    }

    #[test]
    fn alignment_error_messages_are_actionable() {
        let errors = [
            AlignmentError::EmptyReference,
            AlignmentError::EmptyComparison,
            AlignmentError::NoOverlap,
            AlignmentError::ChannelMissing,
            AlignmentError::NonFinite,
            AlignmentError::MissingEvent {
                code: "apogee".to_string(),
                series: "comparison",
            },
        ];
        for e in errors {
            assert!(!e.user_message().is_empty());
            assert!(!e.suggested_action().is_empty());
        }
    }
}
