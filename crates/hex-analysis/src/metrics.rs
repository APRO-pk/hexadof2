//! Error metrics for comparing two series.
//!
//! Every metric here is defined on the same length-resampled pair, so the numbers
//! are comparable. The definitions are stated in the docs because "RMSE" with an
//! unstated sample population is not a number anyone can act on.
//!
//! For attitude the comparison uses a rotation distance, never a difference of
//! Euler angles. Differencing angles is wrong near a wrap and misleading near
//! gimbal lock, and it is the most common mistake in this kind of tool.

use hex_core::{Quaternion, Real};
use serde::{Deserialize, Serialize};

/// A paired set of samples ready for metric computation.
///
/// Pairs where either side is missing are excluded from every metric and counted
/// separately, so a metric is never computed over interpolated or fabricated data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedSeries {
    /// Time on the reference timebase, seconds.
    pub times: Vec<Real>,
    /// Reference values, one per time.
    pub reference: Vec<Real>,
    /// Comparison values, one per time.
    pub comparison: Vec<Real>,
    /// Number of target times where the reference had no sample.
    pub reference_missing: usize,
    /// Number of target times where the comparison had no sample.
    pub comparison_missing: usize,
    /// Unit of the values, for the report.
    pub unit: String,
    /// Name of the channel being compared.
    pub channel: String,
}

impl PairedSeries {
    /// Build from two optional-value vectors produced by resampling.
    pub fn new(
        times: Vec<Real>,
        reference: Vec<Option<Real>>,
        comparison: Vec<Option<Real>>,
        channel: impl Into<String>,
        unit: impl Into<String>,
    ) -> Self {
        let mut t = Vec::new();
        let mut a = Vec::new();
        let mut b = Vec::new();
        let mut reference_missing = 0usize;
        let mut comparison_missing = 0usize;
        for (i, time) in times.iter().enumerate() {
            let ra = reference.get(i).copied().flatten();
            let rb = comparison.get(i).copied().flatten();
            if ra.is_none() {
                reference_missing += 1;
            }
            if rb.is_none() {
                comparison_missing += 1;
            }
            if let (Some(x), Some(y)) = (ra, rb) {
                if x.is_finite() && y.is_finite() {
                    t.push(*time);
                    a.push(x);
                    b.push(y);
                }
            }
        }
        Self {
            times: t,
            reference: a,
            comparison: b,
            reference_missing,
            comparison_missing,
            unit: unit.into(),
            channel: channel.into(),
        }
    }

    /// Number of usable pairs.
    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// Fraction of target times that produced a usable pair.
    pub fn coverage(&self) -> Real {
        let total = self.times.len() + self.reference_missing.max(self.comparison_missing);
        if total == 0 {
            0.0
        } else {
            self.times.len() as Real / total as Real
        }
    }

    /// Difference at each pair, comparison minus reference.
    pub fn differences(&self) -> Vec<Real> {
        self.reference
            .iter()
            .zip(self.comparison.iter())
            .map(|(r, c)| c - r)
            .collect()
    }
}

/// The full metric set for one channel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ErrorMetrics {
    /// Number of paired samples the metrics used.
    pub samples: usize,
    /// Mean of the signed difference, comparison minus reference.
    pub mean_error: Real,
    /// Mean of the absolute difference.
    pub mean_absolute_error: Real,
    /// Root mean square of the difference.
    pub root_mean_square_error: Real,
    /// Largest absolute difference.
    pub maximum_absolute_error: Real,
    /// Time at which the largest absolute error occurred, seconds.
    pub maximum_error_time: Real,
    /// Difference at the final paired sample.
    pub final_error: Real,
    /// Standard deviation of the signed difference, which separates bias from
    /// scatter.
    pub error_standard_deviation: Real,
    /// Reference value range over the paired samples.
    pub reference_range: Real,
    /// Root mean square error as a fraction of the reference range, when the range
    /// is meaningful. This is the figure that says whether an error matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalized_rmse: Option<Real>,
}

impl ErrorMetrics {
    /// Whether the metrics describe any data.
    pub fn is_defined(&self) -> bool {
        self.samples > 0
    }

    /// A one-line summary for the comparison table.
    pub fn summary(&self, unit: &str) -> String {
        if self.samples == 0 {
            return "no paired samples".to_string();
        }
        format!(
            "n = {}, MAE {:.4} {}, RMSE {:.4} {}, max {:.4} {}",
            self.samples,
            self.mean_absolute_error,
            unit,
            self.root_mean_square_error,
            unit,
            self.maximum_absolute_error,
            unit
        )
    }

    /// A caveat the UI must show alongside the numbers.
    pub fn caveat(&self) -> Option<String> {
        if self.samples < 8 {
            return Some(format!(
                "Only {} paired samples. These metrics are not statistically meaningful.",
                self.samples
            ));
        }
        None
    }
}

/// Compute the metric set for a paired series.
///
/// Returns all-zero metrics with `samples = 0` for an empty pair, rather than
/// NaN, so a table can render it without special cases.
pub fn compute_metrics(paired: &PairedSeries) -> ErrorMetrics {
    if paired.is_empty() {
        return ErrorMetrics {
            samples: 0,
            mean_error: 0.0,
            mean_absolute_error: 0.0,
            root_mean_square_error: 0.0,
            maximum_absolute_error: 0.0,
            maximum_error_time: 0.0,
            final_error: 0.0,
            error_standard_deviation: 0.0,
            reference_range: 0.0,
            normalized_rmse: None,
        };
    }

    let n = paired.len() as Real;
    let differences = paired.differences();
    let mean_error = differences.iter().sum::<Real>() / n;
    let mean_absolute_error = differences.iter().map(|d| d.abs()).sum::<Real>() / n;
    let root_mean_square_error = (differences.iter().map(|d| d * d).sum::<Real>() / n).sqrt();

    let mut maximum_absolute_error: Real = 0.0;
    let mut maximum_error_time = paired.times[0];
    for (d, t) in differences.iter().zip(paired.times.iter()) {
        if d.abs() > maximum_absolute_error {
            maximum_absolute_error = d.abs();
            maximum_error_time = *t;
        }
    }

    let variance = differences
        .iter()
        .map(|d| (d - mean_error) * (d - mean_error))
        .sum::<Real>()
        / n;

    let ref_min = paired
        .reference
        .iter()
        .copied()
        .fold(Real::INFINITY, Real::min);
    let ref_max = paired
        .reference
        .iter()
        .copied()
        .fold(Real::NEG_INFINITY, Real::max);
    let reference_range = ref_max - ref_min;

    let normalized_rmse = if reference_range > 1e-12 {
        Some(root_mean_square_error / reference_range)
    } else {
        None
    };

    ErrorMetrics {
        samples: paired.len(),
        mean_error,
        mean_absolute_error,
        root_mean_square_error,
        maximum_absolute_error,
        maximum_error_time,
        final_error: *differences.last().unwrap_or(&0.0),
        error_standard_deviation: variance.sqrt(),
        reference_range,
        normalized_rmse,
    }
}

/// Metrics for an attitude comparison, in radians and degrees.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AttitudeMetrics {
    /// Number of paired quaternions.
    pub samples: usize,
    /// Mean rotation distance.
    pub mean_angle_error: Real,
    /// Root mean square rotation distance.
    pub root_mean_square_angle_error: Real,
    /// Largest rotation distance.
    pub maximum_angle_error: Real,
    /// Time of the largest rotation distance, seconds.
    pub maximum_error_time: Real,
    /// Final rotation distance.
    pub final_angle_error: Real,
}

impl AttitudeMetrics {
    pub fn is_defined(&self) -> bool {
        self.samples > 0
    }

    /// Mean error in degrees, which is the figure users reason about.
    pub fn mean_degrees(&self) -> Real {
        self.mean_angle_error.to_degrees()
    }

    pub fn maximum_degrees(&self) -> Real {
        self.maximum_angle_error.to_degrees()
    }

    pub fn summary(&self) -> String {
        if self.samples == 0 {
            return "no paired attitudes".to_string();
        }
        format!(
            "n = {}, mean {:.2} deg, RMS {:.2} deg, max {:.2} deg",
            self.samples,
            self.mean_degrees(),
            self.root_mean_square_angle_error.to_degrees(),
            self.maximum_degrees()
        )
    }

    pub fn caveat(&self) -> Option<String> {
        if self.samples < 8 {
            return Some(format!(
                "Only {} paired attitudes. Treat these figures as indicative.",
                self.samples
            ));
        }
        None
    }
}

/// Compute attitude metrics from paired quaternions.
pub fn compute_attitude_metrics(
    times: &[Real],
    reference: &[Quaternion],
    comparison: &[Quaternion],
) -> AttitudeMetrics {
    let n = times.len().min(reference.len()).min(comparison.len());
    if n == 0 {
        return AttitudeMetrics {
            samples: 0,
            mean_angle_error: 0.0,
            root_mean_square_angle_error: 0.0,
            maximum_angle_error: 0.0,
            maximum_error_time: 0.0,
            final_angle_error: 0.0,
        };
    }

    let mut sum = 0.0;
    let mut sum_sq = 0.0;
    let mut maximum: Real = 0.0;
    let mut maximum_time = times[0];
    let mut last = 0.0;

    for i in 0..n {
        let angle = reference[i].angular_distance(&comparison[i]);
        sum += angle;
        sum_sq += angle * angle;
        if angle > maximum {
            maximum = angle;
            maximum_time = times[i];
        }
        if i == n - 1 {
            last = angle;
        }
    }

    AttitudeMetrics {
        samples: n,
        mean_angle_error: sum / n as Real,
        root_mean_square_angle_error: (sum_sq / n as Real).sqrt(),
        maximum_angle_error: maximum,
        maximum_error_time: maximum_time,
        final_angle_error: last,
    }
}

/// The rotation distance between two attitudes, in radians.
///
/// Uses `q_error = inverse(reference) * estimate` and
/// `angle = 2 * acos(clamp(|q_error.w|, -1, 1))`. The absolute value is taken so
/// that `q` and `-q`, which describe the same rotation, compare as equal.
pub fn rotation_distance(reference: &Quaternion, estimate: &Quaternion) -> Real {
    let error = reference.error_from(estimate);
    2.0 * error.w.abs().clamp(0.0, 1.0).acos()
}

/// The error quaternion between a reference and an estimate.
pub fn error_quaternion(reference: &Quaternion, estimate: &Quaternion) -> Quaternion {
    reference.error_from(estimate)
}

/// A vector-valued metric set, for trajectory comparison.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VectorErrorMetrics {
    /// Components of the mean vector error.
    pub mean_error: [Real; 3],
    /// Mean magnitude of the vector error.
    pub mean_absolute_error: Real,
    /// Root mean square of the vector error magnitude.
    pub root_mean_square_error: Real,
    /// Largest vector error magnitude.
    pub maximum_absolute_error: Real,
    /// Time of the largest error.
    pub maximum_error_time: Real,
    /// Final vector error magnitude.
    pub final_error: Real,
    /// Samples used.
    pub samples: usize,
}

impl VectorErrorMetrics {
    pub fn summary(&self, unit: &str) -> String {
        if self.samples == 0 {
            return "no paired samples".to_string();
        }
        format!(
            "n = {}, mean {:.4} {}, RMS {:.4} {}, max {:.4} {}",
            self.samples,
            self.mean_absolute_error,
            unit,
            self.root_mean_square_error,
            unit,
            self.maximum_absolute_error,
            unit
        )
    }
}

/// Compute vector metrics from paired vector series.
pub fn compute_vector_metrics(
    times: &[Real],
    reference: &[[Real; 3]],
    comparison: &[[Real; 3]],
) -> VectorErrorMetrics {
    let n = times.len().min(reference.len()).min(comparison.len());
    if n == 0 {
        return VectorErrorMetrics {
            mean_error: [0.0; 3],
            mean_absolute_error: 0.0,
            root_mean_square_error: 0.0,
            maximum_absolute_error: 0.0,
            maximum_error_time: 0.0,
            final_error: 0.0,
            samples: 0,
        };
    }

    let mut sum = [0.0; 3];
    let mut magnitude_sum = 0.0;
    let mut square_sum = 0.0;
    let mut maximum: Real = 0.0;
    let mut maximum_time = times[0];
    let mut last = 0.0;

    for i in 0..n {
        let d = [
            comparison[i][0] - reference[i][0],
            comparison[i][1] - reference[i][1],
            comparison[i][2] - reference[i][2],
        ];
        for k in 0..3 {
            sum[k] += d[k];
        }
        let magnitude = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        magnitude_sum += magnitude;
        square_sum += magnitude * magnitude;
        if magnitude > maximum {
            maximum = magnitude;
            maximum_time = times[i];
        }
        if i == n - 1 {
            last = magnitude;
        }
    }

    VectorErrorMetrics {
        mean_error: [sum[0] / n as Real, sum[1] / n as Real, sum[2] / n as Real],
        mean_absolute_error: magnitude_sum / n as Real,
        root_mean_square_error: (square_sum / n as Real).sqrt(),
        maximum_absolute_error: maximum,
        maximum_error_time: maximum_time,
        final_error: last,
        samples: n,
    }
}

/// Which derived quantity a report table row describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricChannel {
    Altitude,
    VerticalVelocity,
    Speed,
    Acceleration,
    DynamicPressure,
    AngleOfAttack,
    Sideslip,
    PressureAltitude,
    AngularRate,
    Attitude,
    TotalForce,
    TotalMoment,
}

impl MetricChannel {
    pub fn label(self) -> &'static str {
        match self {
            MetricChannel::Altitude => "Altitude",
            MetricChannel::VerticalVelocity => "Vertical velocity",
            MetricChannel::Speed => "Speed",
            MetricChannel::Acceleration => "Acceleration",
            MetricChannel::DynamicPressure => "Dynamic pressure",
            MetricChannel::AngleOfAttack => "Angle of attack",
            MetricChannel::Sideslip => "Sideslip",
            MetricChannel::PressureAltitude => "Pressure altitude",
            MetricChannel::AngularRate => "Angular rate",
            MetricChannel::Attitude => "Attitude",
            MetricChannel::TotalForce => "Total force",
            MetricChannel::TotalMoment => "Total moment",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            MetricChannel::Altitude | MetricChannel::PressureAltitude => "m",
            MetricChannel::VerticalVelocity | MetricChannel::Speed => "m/s",
            MetricChannel::Acceleration => "m/s^2",
            MetricChannel::DynamicPressure => "Pa",
            MetricChannel::AngleOfAttack | MetricChannel::Sideslip => "rad",
            MetricChannel::AngularRate => "rad/s",
            MetricChannel::Attitude => "deg",
            MetricChannel::TotalForce => "N",
            MetricChannel::TotalMoment => "N m",
        }
    }

    /// The channels the specification requires in a comparison.
    pub fn default_set() -> &'static [MetricChannel] {
        use MetricChannel::*;
        &[
            Altitude,
            VerticalVelocity,
            Speed,
            Acceleration,
            AngularRate,
            Attitude,
            PressureAltitude,
            DynamicPressure,
            TotalForce,
            TotalMoment,
        ]
    }

    /// Whether this channel is compared as an attitude rather than a scalar.
    pub fn is_attitude(self) -> bool {
        matches!(self, MetricChannel::Attitude)
    }
}

/// One row of the comparison metrics table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricRow {
    pub channel: MetricChannel,
    pub unit: String,
    /// Scalar metrics, absent for an attitude channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scalar: Option<ErrorMetrics>,
    /// Attitude metrics, present only for an attitude channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attitude: Option<AttitudeMetrics>,
    /// Caveat shown beside the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caveat: Option<String>,
}

impl MetricRow {
    pub fn scalar(channel: MetricChannel, metrics: ErrorMetrics) -> Self {
        Self {
            channel,
            unit: channel.unit().to_string(),
            caveat: metrics.caveat(),
            scalar: Some(metrics),
            attitude: None,
        }
    }

    pub fn attitude(channel: MetricChannel, metrics: AttitudeMetrics) -> Self {
        Self {
            channel,
            unit: channel.unit().to_string(),
            caveat: metrics.caveat(),
            scalar: None,
            attitude: Some(metrics),
        }
    }

    /// A one-line rendering for the report.
    pub fn one_line(&self) -> String {
        let body = match (&self.scalar, &self.attitude) {
            (Some(s), _) => s.summary(&self.unit),
            (None, Some(a)) => a.summary(),
            _ => "no data".to_string(),
        };
        format!("{}: {}", self.channel.label(), body)
    }

    /// The primary number for sorting, so the worst channel floats to the top.
    pub fn primary_error(&self) -> Real {
        match (&self.scalar, &self.attitude) {
            (Some(s), _) => s.root_mean_square_error,
            (None, Some(a)) => a.root_mean_square_angle_error,
            _ => 0.0,
        }
    }

    /// Whether the row has any data behind it.
    pub fn has_data(&self) -> bool {
        match (&self.scalar, &self.attitude) {
            (Some(s), _) => s.is_defined(),
            (None, Some(a)) => a.is_defined(),
            _ => false,
        }
    }
}

/// The complete metrics table for a comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsTable {
    pub rows: Vec<MetricRow>,
    /// Time-to-event differences keyed by event code.
    pub event_time_differences: Vec<(String, Real)>,
    /// Channels that could not be compared at all, with the reason.
    pub unavailable: Vec<(String, String)>,
}

impl MetricsTable {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            event_time_differences: Vec::new(),
            unavailable: Vec::new(),
        }
    }

    pub fn push(&mut self, row: MetricRow) {
        self.rows.push(row);
    }

    /// Rows with data, worst first.
    pub fn ranked(&self) -> Vec<&MetricRow> {
        let mut rows: Vec<&MetricRow> = self.rows.iter().filter(|r| r.has_data()).collect();
        rows.sort_by(|a, b| {
            b.primary_error()
                .partial_cmp(&a.primary_error())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        rows
    }

    /// A plain-text rendering for the exported report.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("Channel comparison\n");
        out.push_str("==================\n");
        if self.rows.is_empty() {
            out.push_str("No channels were compared.\n");
        }
        for row in &self.rows {
            out.push_str(&row.one_line());
            out.push('\n');
            if let Some(caveat) = &row.caveat {
                out.push_str(&format!("  note: {}\n", caveat));
            }
        }
        if !self.event_time_differences.is_empty() {
            out.push_str("\nEvent timing\n");
            out.push_str("------------\n");
            for (code, difference) in &self.event_time_differences {
                out.push_str(&format!("{:>24}: {:+.4} s\n", code, difference));
            }
        }
        if !self.unavailable.is_empty() {
            out.push_str("\nNot compared\n");
            out.push_str("------------\n");
            for (channel, reason) in &self.unavailable {
                out.push_str(&format!("{}: {}\n", channel, reason));
            }
        }
        out
    }
}

impl Default for MetricsTable {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::Vec3;

    #[test]
    fn metrics_of_an_identical_series_are_zero() {
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let paired = PairedSeries::new(
            times,
            vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)],
            vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)],
            "altitude",
            "m",
        );
        let m = compute_metrics(&paired);
        assert_eq!(m.samples, 4);
        assert_eq!(m.mean_error, 0.0);
        assert_eq!(m.mean_absolute_error, 0.0);
        assert_eq!(m.root_mean_square_error, 0.0);
        assert_eq!(m.maximum_absolute_error, 0.0);
        assert_eq!(m.final_error, 0.0);
        assert!(m.is_defined());
    }

    #[test]
    fn metrics_are_computed_against_a_hand_worked_example() {
        // Differences are 1, -1, 2, -2.
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let paired = PairedSeries::new(
            times,
            vec![Some(0.0), Some(0.0), Some(0.0), Some(0.0)],
            vec![Some(1.0), Some(-1.0), Some(2.0), Some(-2.0)],
            "x",
            "m",
        );
        let m = compute_metrics(&paired);
        assert!((m.mean_error - 0.0).abs() < 1e-12);
        assert!((m.mean_absolute_error - 1.5).abs() < 1e-12);
        // RMS of (1, 1, 4, 4) is sqrt(2.5).
        assert!((m.root_mean_square_error - 2.5f64.sqrt()).abs() < 1e-12);
        assert!((m.maximum_absolute_error - 2.0).abs() < 1e-12);
        assert!((m.maximum_error_time - 2.0).abs() < 1e-12);
        assert!((m.final_error + 2.0).abs() < 1e-12);
        // Standard deviation of (1,-1,2,-2) about zero is sqrt(2.5).
        assert!((m.error_standard_deviation - 2.5f64.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn metrics_separate_bias_from_scatter() {
        // A constant offset gives a mean error but no scatter.
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let paired = PairedSeries::new(times, vec![Some(0.0); 4], vec![Some(5.0); 4], "x", "m");
        let m = compute_metrics(&paired);
        assert!((m.mean_error - 5.0).abs() < 1e-12);
        assert!(m.error_standard_deviation < 1e-12);
        // A reference with no range has no meaningful normalised error, so the
        // field is absent rather than reported as zero.
        assert!(m.normalized_rmse.is_none());
    }

    #[test]
    fn normalized_rmse_is_absent_when_the_reference_has_no_range() {
        let times = vec![0.0, 1.0];
        let paired = PairedSeries::new(
            times,
            vec![Some(3.0), Some(3.0)],
            vec![Some(3.5), Some(3.5)],
            "x",
            "m",
        );
        let m = compute_metrics(&paired);
        assert!(m.normalized_rmse.is_none());
    }

    #[test]
    fn empty_pair_produces_zeroed_metrics_not_nan() {
        let paired = PairedSeries::new(vec![], vec![], vec![], "x", "m");
        let m = compute_metrics(&paired);
        assert_eq!(m.samples, 0);
        assert_eq!(m.mean_error, 0.0);
        assert!(m.normalized_rmse.is_none());
        assert!(!m.is_defined());
        assert!(m.summary("m").contains("no paired"));
    }

    #[test]
    fn missing_samples_are_excluded_and_counted() {
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let paired = PairedSeries::new(
            times,
            vec![Some(1.0), None, Some(3.0), Some(4.0)],
            vec![Some(1.0), Some(2.0), Some(3.0), None],
            "x",
            "m",
        );
        assert_eq!(paired.len(), 2);
        assert_eq!(paired.reference_missing, 1);
        assert_eq!(paired.comparison_missing, 1);
        // Both endpoints were excluded, so the remaining pairs are 0 and 2.
        assert_eq!(paired.times, vec![0.0, 2.0]);
        let m = compute_metrics(&paired);
        assert_eq!(m.samples, 2);
    }

    #[test]
    fn non_finite_samples_are_excluded() {
        let times = vec![0.0, 1.0, 2.0];
        let paired = PairedSeries::new(
            times,
            vec![Some(1.0), Some(Real::NAN), Some(3.0)],
            vec![Some(1.0), Some(2.0), Some(Real::INFINITY)],
            "x",
            "m",
        );
        assert_eq!(paired.len(), 1);
        assert_eq!(paired.times, vec![0.0]);
    }

    #[test]
    fn short_series_produce_a_caveat() {
        let times = vec![0.0, 1.0];
        let paired = PairedSeries::new(
            times,
            vec![Some(0.0), Some(1.0)],
            vec![Some(0.1), Some(1.1)],
            "x",
            "m",
        );
        let m = compute_metrics(&paired);
        assert!(m.caveat().is_some());
        assert!(m.caveat().unwrap().contains("not statistically meaningful"));
    }

    #[test]
    fn metrics_summary_text_is_readable() {
        let times: Vec<Real> = (0..20).map(|i| i as Real).collect();
        let paired = PairedSeries::new(times, vec![Some(0.0); 20], vec![Some(0.5); 20], "x", "m");
        let m = compute_metrics(&paired);
        let text = m.summary("m");
        assert!(text.contains("n = 20"));
        assert!(text.contains("MAE"));
        assert!(m.caveat().is_none());
    }

    #[test]
    fn rotation_distance_matches_the_analytic_value() {
        let a = Quaternion::identity();
        let b = Quaternion::from_axis_angle(Vec3::z(), 0.7);
        assert!((rotation_distance(&a, &b) - 0.7).abs() < 1e-12);
        assert!((rotation_distance(&b, &a) - 0.7).abs() < 1e-12);
    }

    #[test]
    fn rotation_distance_is_zero_for_the_same_rotation_either_sign() {
        let q = Quaternion::from_axis_angle(Vec3::new(1.0, 1.0, 0.0), 1.2);
        let negated = Quaternion::new(-q.w, -q.x, -q.y, -q.z);
        assert!(rotation_distance(&q, &q) < 1e-12);
        assert!(rotation_distance(&q, &negated) < 1e-12);
    }

    #[test]
    fn attitude_metrics_use_rotation_distance_not_euler_differences() {
        // 350 degrees and 10 degrees are 20 degrees apart on the short path, but
        // subtracting the angles directly would give 340.
        let reference = Quaternion::from_euler_321(0.0, 0.0, 350.0f64.to_radians());
        let estimate = Quaternion::from_euler_321(0.0, 0.0, 10.0f64.to_radians());
        let distance = rotation_distance(&reference, &estimate);
        assert!(
            (distance.to_degrees() - 20.0).abs() < 1e-6,
            "distance {} deg",
            distance.to_degrees()
        );
    }

    #[test]
    fn attitude_metrics_are_computed_over_pairs() {
        let times = vec![0.0, 1.0, 2.0];
        let reference = vec![Quaternion::identity(); 3];
        let comparison = vec![
            Quaternion::identity(),
            Quaternion::from_axis_angle(Vec3::z(), 0.1),
            Quaternion::from_axis_angle(Vec3::z(), 0.2),
        ];
        let m = compute_attitude_metrics(&times, &reference, &comparison);
        assert_eq!(m.samples, 3);
        assert!((m.mean_angle_error - 0.1).abs() < 1e-9);
        assert!((m.maximum_angle_error - 0.2).abs() < 1e-9);
        assert!((m.maximum_error_time - 2.0).abs() < 1e-12);
        assert!((m.final_angle_error - 0.2).abs() < 1e-9);
        assert!(m.summary().contains("deg"));
    }

    #[test]
    fn attitude_metrics_of_no_pairs_are_defined_as_zero() {
        let m = compute_attitude_metrics(&[], &[], &[]);
        assert_eq!(m.samples, 0);
        assert!(!m.is_defined());
        assert!(m.summary().contains("no paired"));
        assert!(m.mean_degrees().abs() < 1e-15);
    }

    #[test]
    fn attitude_metrics_caveat_on_a_short_series() {
        let times = vec![0.0, 1.0];
        let reference = vec![Quaternion::identity(); 2];
        let comparison = vec![Quaternion::identity(); 2];
        let m = compute_attitude_metrics(&times, &reference, &comparison);
        assert!(m.caveat().is_some());
    }

    #[test]
    fn error_quaternion_is_the_relative_rotation() {
        let reference = Quaternion::from_axis_angle(Vec3::z(), 0.4);
        let estimate = Quaternion::from_axis_angle(Vec3::z(), 0.9);
        let e = error_quaternion(&reference, &estimate);
        assert!((e.angular_distance(&Quaternion::identity()) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn vector_metrics_are_computed_from_components() {
        let times = vec![0.0, 1.0];
        let reference = [[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        let comparison = [[3.0, 4.0, 0.0], [0.0, 0.0, 12.0]];
        let m = compute_vector_metrics(&times, &reference, &comparison);
        assert_eq!(m.samples, 2);
        // Magnitudes are 5 and 12.
        assert!((m.mean_absolute_error - 8.5).abs() < 1e-12);
        assert!((m.maximum_absolute_error - 12.0).abs() < 1e-12);
        assert!((m.maximum_error_time - 1.0).abs() < 1e-12);
        assert!((m.final_error - 12.0).abs() < 1e-12);
        assert!((m.mean_error[0] - 1.5).abs() < 1e-12);
    }

    #[test]
    fn vector_metrics_of_no_pairs() {
        let m = compute_vector_metrics(&[], &[], &[]);
        assert_eq!(m.samples, 0);
        assert!(m.summary("m").contains("no paired"));
    }

    #[test]
    fn vector_series_of_different_lengths_uses_the_shortest() {
        let times = vec![0.0, 1.0, 2.0];
        let reference = [[0.0; 3]; 3];
        let comparison = [[1.0, 0.0, 0.0]];
        let m = compute_vector_metrics(&times, &reference, &comparison);
        assert_eq!(m.samples, 1);
    }

    #[test]
    fn metric_channel_metadata_is_complete() {
        for c in MetricChannel::default_set() {
            assert!(!c.label().is_empty());
            assert!(!c.unit().is_empty());
        }
        assert!(MetricChannel::Attitude.is_attitude());
        assert!(!MetricChannel::Altitude.is_attitude());
        assert_eq!(MetricChannel::default_set().len(), 10);
    }

    #[test]
    fn metrics_table_ranks_by_error() {
        let mut table = MetricsTable::new();
        let small = compute_metrics(&PairedSeries::new(
            vec![0.0, 1.0],
            vec![Some(0.0), Some(1.0)],
            vec![Some(0.01), Some(1.01)],
            "a",
            "m",
        ));
        let large = compute_metrics(&PairedSeries::new(
            vec![0.0, 1.0],
            vec![Some(0.0), Some(1.0)],
            vec![Some(0.5), Some(1.5)],
            "b",
            "m",
        ));
        table.push(MetricRow::scalar(MetricChannel::Altitude, small));
        table.push(MetricRow::scalar(MetricChannel::Speed, large));
        let ranked = table.ranked();
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].channel, MetricChannel::Speed);
    }

    #[test]
    fn metrics_table_renders_as_text() {
        let mut table = MetricsTable::new();
        table.push(MetricRow::scalar(
            MetricChannel::Altitude,
            compute_metrics(&PairedSeries::new(
                vec![0.0, 1.0],
                vec![Some(0.0), Some(1.0)],
                vec![Some(0.1), Some(1.1)],
                "alt",
                "m",
            )),
        ));
        table.push(MetricRow::attitude(
            MetricChannel::Attitude,
            compute_attitude_metrics(
                &[0.0, 1.0],
                &[Quaternion::identity(), Quaternion::identity()],
                &[
                    Quaternion::identity(),
                    Quaternion::from_axis_angle(Vec3::z(), 0.05),
                ],
            ),
        ));
        table
            .event_time_differences
            .push(("apogee".to_string(), 0.25));
        table
            .unavailable
            .push(("Dynamic pressure".to_string(), "not recorded".to_string()));
        let text = table.to_text();
        assert!(text.contains("Channel comparison"));
        assert!(text.contains("Altitude"));
        assert!(text.contains("Attitude"));
        assert!(text.contains("apogee"));
        assert!(text.contains("not recorded"));
    }

    #[test]
    fn metric_row_primary_error_and_data_predicate() {
        let row = MetricRow::scalar(
            MetricChannel::Altitude,
            compute_metrics(&PairedSeries::new(
                vec![0.0, 1.0],
                vec![Some(0.0), Some(1.0)],
                vec![Some(0.5), Some(1.5)],
                "alt",
                "m",
            )),
        );
        assert!(row.has_data());
        assert!(row.primary_error() > 0.4);
        assert!(row.one_line().contains("Altitude"));

        let empty_attitude = MetricRow::attitude(
            MetricChannel::Attitude,
            compute_attitude_metrics(&[], &[], &[]),
        );
        assert!(!empty_attitude.has_data());
        assert_eq!(empty_attitude.primary_error(), 0.0);
    }

    #[test]
    fn coverage_reports_the_fraction_of_usable_pairs() {
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let paired = PairedSeries::new(
            times,
            vec![Some(1.0), None, Some(3.0), Some(4.0)],
            vec![Some(1.0), Some(2.0), Some(3.0), Some(4.0)],
            "x",
            "m",
        );
        // Three of four target times pair up.
        assert!((paired.coverage() - 0.75).abs() < 1e-12);
        let empty = PairedSeries::new(vec![], vec![], vec![], "x", "m");
        assert_eq!(empty.coverage(), 0.0);
    }
}
