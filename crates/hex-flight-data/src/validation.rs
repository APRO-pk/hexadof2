//! Timestamp validation and data-quality assessment for an imported table.
//!
//! Validation runs on the corrected time axis and on the normalised channels, so
//! the findings are expressed in the same terms the rest of the application
//! uses. Every finding carries a stable code, a severity, a time range, the
//! affected channel, a suggested action, and a resolved flag, because the
//! product requires each warning to be actionable or explicitly dismissed.

use serde::{Deserialize, Serialize};

use hex_core::{
    analyze_clock, ClockAnalysis, Real, RolloverCorrection, Severity, TimeSource, Timebase,
    TimestampUnit, ValidationIssue,
};

use crate::channel::{Channel, ChannelRole};

/// Options controlling timestamp validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimestampValidationOptions {
    /// Multiple of the median interval above which an interval counts as a gap.
    pub gap_factor: Real,
    /// Absolute gap duration above which a gap is always reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_gap_seconds: Option<Real>,
    /// Rate the user expects. A mismatch is reported but not fatal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_rate_hz: Option<Real>,
    /// Relative change in the local interval that starts a new rate segment.
    pub rate_change_tolerance: Real,
    /// Width of a wrapping device counter, when the source uses one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollover_bits: Option<u32>,
    /// Two timestamps closer than this are treated as duplicates.
    pub duplicate_epsilon: Real,
}

impl Default for TimestampValidationOptions {
    fn default() -> Self {
        Self {
            gap_factor: 3.0,
            maximum_gap_seconds: None,
            expected_rate_hz: None,
            rate_change_tolerance: 0.1,
            rollover_bits: None,
            duplicate_epsilon: 1e-9,
        }
    }
}

/// A run of samples captured at one sampling rate.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RateSegment {
    /// First sample index of the run, inclusive.
    pub start_index: usize,
    /// Last sample index of the run, inclusive.
    pub end_index: usize,
    /// Sampling rate across the run.
    pub rate_hz: Real,
}

/// Everything the importer learned about a time axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimestampValidation {
    /// Gap, duplicate, and reversal analysis from the core clock analyser.
    pub analysis: ClockAnalysis,
    /// Actionable findings, in the order they were produced.
    pub warnings: Vec<ValidationIssue>,
    /// Sampling-rate runs detected across the series.
    pub rate_segments: Vec<RateSegment>,
    /// Rollover correction applied, when the source used a wrapping counter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollover: Option<RolloverCorrection>,
    /// Time axis rebased so the first sample sits at zero.
    pub corrected_times: Vec<Real>,
    /// Record of how the corrected timebase was established.
    pub timebase: Timebase,
}

impl TimestampValidation {
    /// Number of findings at or above the given severity.
    pub fn count_at_least(&self, severity: Severity) -> usize {
        self.warnings
            .iter()
            .filter(|w| w.severity >= severity)
            .count()
    }

    /// True when nothing blocked the import.
    pub fn has_errors(&self) -> bool {
        self.warnings.iter().any(|w| w.severity.is_blocking())
    }

    /// Sampling rate of the first detected segment, when there is one.
    pub fn primary_rate_hz(&self) -> Option<Real> {
        self.rate_segments.first().map(|segment| segment.rate_hz)
    }
}

/// Detect runs of constant sampling rate.
///
/// A non-positive interval always starts a new run, because a reversal or a
/// duplicate makes the local rate meaningless.
fn detect_rate_segments(times: &[Real], tolerance: Real) -> Vec<RateSegment> {
    if times.len() < 2 {
        return Vec::new();
    }
    let tolerance = tolerance.max(0.0);
    let mut segments = Vec::new();
    let mut start = 0usize;
    let mut reference = times[1] - times[0];
    let mut sum = 0.0;
    let mut count = 0usize;

    for index in 0..times.len() - 1 {
        let dt = times[index + 1] - times[index];
        let breaks =
            dt <= 0.0 || reference <= 0.0 || (dt - reference).abs() > tolerance * reference;
        if index > start && breaks {
            let rate = if count > 0 && sum > 0.0 {
                count as Real / sum
            } else {
                0.0
            };
            segments.push(RateSegment {
                start_index: start,
                end_index: index,
                rate_hz: rate,
            });
            start = index;
            sum = 0.0;
            count = 0;
        }
        if index == start {
            reference = dt;
        }
        sum += dt;
        count += 1;
    }

    let rate = if count > 0 && sum > 0.0 {
        count as Real / sum
    } else {
        0.0
    };
    segments.push(RateSegment {
        start_index: start,
        end_index: times.len() - 1,
        rate_hz: rate,
    });
    segments
}

/// Validate a raw time column and produce the corrected timebase.
pub fn validate_timestamps(
    times: &[Real],
    options: &TimestampValidationOptions,
) -> TimestampValidation {
    let mut warnings = Vec::new();

    let (working, rollover) = match options.rollover_bits {
        Some(bits) => {
            let mut correction = RolloverCorrection::for_bits(bits);
            let unwrapped = correction.apply(times);
            warnings.push(
                ValidationIssue::info(
                    "time.rollover",
                    "Wrapping counter corrected",
                    format!(
                        "The timestamp counter wrapped {} time(s) and was unwrapped using a {}-bit modulus.",
                        correction.wraps_detected, bits
                    ),
                )
                .with_suggestion("No action needed; the corrected time axis is monotonic where the log is."),
            );
            (unwrapped, Some(correction))
        }
        None => (times.to_vec(), None),
    };

    let start_offset = working
        .iter()
        .copied()
        .find(|t| t.is_finite())
        .unwrap_or(0.0);
    let corrected_times: Vec<Real> = working.iter().map(|t| t - start_offset).collect();

    let non_finite = corrected_times.iter().filter(|t| !t.is_finite()).count();
    if non_finite > 0 {
        warnings.push(
            ValidationIssue::error(
                "time.non_finite",
                "Timestamp is not a number",
                format!("{non_finite} timestamp(s) are NaN or infinite, so those samples have no place on the time axis."),
            )
            .with_suggestion("Remove the affected rows or repair the logger output."),
        );
    }

    if corrected_times.is_empty() {
        warnings.push(
            ValidationIssue::error(
                "time.empty",
                "No samples",
                "The table contains no rows, so there is no time axis to validate.",
            )
            .with_suggestion("Check the row limit and the delimiter, then import again."),
        );
    }

    let gap_factor = options.gap_factor.max(1.0);
    let analysis = analyze_clock(&corrected_times, gap_factor);
    let rate_segments = detect_rate_segments(&corrected_times, options.rate_change_tolerance);

    for gap in &analysis.gaps {
        let beyond_limit = options
            .maximum_gap_seconds
            .map(|limit| gap.duration > limit)
            .unwrap_or(false);
        warnings.push(
            ValidationIssue::warning(
                "time.gap",
                "Gap in the time axis",
                format!(
                    "A gap of {:.6} s starts at sample {} (about {} missing sample(s)).",
                    gap.duration, gap.after_index, gap.estimated_missing
                ),
            )
            .with_context(format!(
                "samples {} to {}",
                gap.after_index,
                gap.after_index + 1
            ))
            .with_suggestion(if beyond_limit {
                "The gap exceeds the configured maximum; check the logger's buffer settings."
            } else {
                "Confirm that the logger really did stop recording for this interval."
            }),
        );
    }

    if !analysis.duplicate_indices.is_empty() {
        warnings.push(
            ValidationIssue::warning(
                "time.duplicate",
                "Duplicate timestamps",
                format!(
                    "{} sample(s) share a timestamp with the previous sample, for example sample {}.",
                    analysis.duplicate_indices.len(),
                    analysis.duplicate_indices[0]
                ),
            )
            .with_suggestion("Deduplicate the log, or keep the duplicates and treat the extra samples as a burst."),
        );
    }

    for index in &analysis.reversal_indices {
        warnings.push(
            ValidationIssue::error(
                "time.reversal",
                "Time runs backwards",
                format!(
                    "Sample {index} is earlier than sample {} by {:.6} s.",
                    index.saturating_sub(1),
                    corrected_times[*index - 1] - corrected_times[*index]
                ),
            )
            .with_context(format!("sample {index}"))
            .with_suggestion(
                "Split the log at the reversal, or re-export it with a monotonic clock.",
            ),
        );
    }

    if rate_segments.len() > 1 {
        let rates = rate_segments
            .iter()
            .map(|segment| format!("{:.3} Hz", segment.rate_hz))
            .collect::<Vec<_>>()
            .join(", ");
        warnings.push(
            ValidationIssue::warning(
                "time.rate_change",
                "Sampling rate changes",
                format!(
                    "The series was captured at {} different rates: {rates}.",
                    rate_segments.len()
                ),
            )
            .with_suggestion("Resample to a single rate before running system identification."),
        );
    }

    if let Some(expected) = options.expected_rate_hz {
        if expected > 0.0 && analysis.effective_rate_hz > 0.0 {
            let error = (analysis.effective_rate_hz - expected).abs() / expected;
            if error > options.rate_change_tolerance {
                warnings.push(
                    ValidationIssue::warning(
                        "time.rate_mismatch",
                        "Sampling rate does not match the expectation",
                        format!(
                            "The log runs at {:.3} Hz but {:.3} Hz was expected ({:.1}% difference).",
                            analysis.effective_rate_hz,
                            expected,
                            error * 100.0
                        ),
                    )
                    .with_suggestion("Confirm the configured sample rate on the flight controller."),
                );
            }
        }
    }

    let timebase = Timebase {
        source: TimeSource::Device,
        unit: TimestampUnit::Seconds,
        start_offset,
        nominal_dt: if analysis.median_dt > 0.0 {
            Some(analysis.median_dt)
        } else {
            None
        },
        rollover_corrected: rollover
            .map(|correction| correction.wraps_detected > 0)
            .unwrap_or(false),
        note: None,
    };

    TimestampValidation {
        analysis,
        warnings,
        rate_segments,
        rollover,
        corrected_times,
        timebase,
    }
}

/// Options controlling the data-quality pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityOptions {
    /// Plausible acceleration range in m/s^2.
    pub accel_plausible_range: (Real, Real),
    /// Plausible angular rate range in rad/s.
    pub gyro_plausible_range: (Real, Real),
    /// Pressure change between neighbouring samples that counts as a jump, in Pa.
    pub pressure_jump_threshold_pa: Real,
    /// Sample count below which a channel is reported as sparse.
    pub minimum_samples: usize,
    /// Spread below which a channel is reported as constant.
    pub constant_channel_tolerance: Real,
}

impl Default for QualityOptions {
    fn default() -> Self {
        Self {
            accel_plausible_range: (-160.0, 160.0),
            gyro_plausible_range: (-35.0, 35.0),
            pressure_jump_threshold_pa: 2000.0,
            minimum_samples: 8,
            constant_channel_tolerance: 1e-12,
        }
    }
}

/// One actionable data-quality finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataQualityFlag {
    /// Stable machine code, for example `channel.nan`.
    pub code: String,
    /// How much the finding matters.
    pub severity: Severity,
    /// Channel the finding applies to, or an axis label such as `time`.
    pub channel: String,
    /// First affected sample index.
    pub start_index: usize,
    /// Last affected sample index.
    pub end_index: usize,
    /// Time at `start_index`.
    pub start_time: Real,
    /// Time at `end_index`.
    pub end_time: Real,
    /// Plain-language explanation.
    pub detail: String,
    /// What the user can do about it.
    pub suggested_action: String,
    /// True once the user dismissed the finding.
    pub resolved: bool,
}

impl DataQualityFlag {
    /// Mark the finding as reviewed, so it no longer drives the overall severity.
    pub fn resolve(&mut self) {
        self.resolved = true;
    }
}

/// Aggregate result of a data-quality pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QualityReport {
    /// Every finding, including resolved ones.
    pub flags: Vec<DataQualityFlag>,
    /// Samples missing, estimated from the gaps in the time axis.
    pub missing_samples: usize,
    /// NaN values across all channels.
    pub nan_values: usize,
    /// Infinite values across all channels.
    pub infinite_values: usize,
    /// Channels whose values never change.
    pub constant_channels: Vec<String>,
    /// Channels that reached the edge of their plausible range.
    pub clipped_channels: Vec<String>,
    /// Worst severity among the unresolved findings.
    pub overall_severity: Severity,
    /// One-line summary for a status strip.
    pub summary: String,
}

impl QualityReport {
    /// An empty report, used when a pass finds nothing.
    pub fn clean() -> Self {
        let mut report = Self {
            flags: Vec::new(),
            missing_samples: 0,
            nan_values: 0,
            infinite_values: 0,
            constant_channels: Vec::new(),
            clipped_channels: Vec::new(),
            overall_severity: Severity::Info,
            summary: String::new(),
        };
        report.recompute();
        report
    }

    /// Findings the user has not dismissed.
    pub fn unresolved(&self) -> impl Iterator<Item = &DataQualityFlag> {
        self.flags.iter().filter(|flag| !flag.resolved)
    }

    /// Number of unresolved findings with the given code.
    pub fn count_code(&self, code: &str) -> usize {
        self.unresolved().filter(|flag| flag.code == code).count()
    }

    /// True when any unresolved finding carries the given code.
    pub fn has_code(&self, code: &str) -> bool {
        self.count_code(code) > 0
    }

    /// Dismiss every unresolved finding with the given code.
    pub fn resolve_code(&mut self, code: &str) {
        for flag in &mut self.flags {
            if flag.code == code {
                flag.resolve();
            }
        }
        self.recompute();
    }

    /// Recompute the worst severity and the summary line.
    pub fn recompute(&mut self) {
        self.overall_severity = self
            .unresolved()
            .map(|flag| flag.severity)
            .max()
            .unwrap_or(Severity::Info);
        self.summary = format!(
            "{} finding(s): {} error(s), {} warning(s); {} missing sample(s), {} NaN value(s)",
            self.flags.len(),
            self.unresolved()
                .filter(|f| f.severity == Severity::Error)
                .count(),
            self.unresolved()
                .filter(|f| f.severity == Severity::Warning)
                .count(),
            self.missing_samples,
            self.nan_values
        );
    }
}

fn push_flag(report: &mut QualityReport, flag: DataQualityFlag) {
    report.flags.push(flag);
}

#[allow(clippy::too_many_arguments)]
fn make_flag(
    code: &str,
    severity: Severity,
    channel: &str,
    start_index: usize,
    end_index: usize,
    times: &[Real],
    detail: String,
    suggested_action: &str,
) -> DataQualityFlag {
    DataQualityFlag {
        code: code.to_string(),
        severity,
        channel: channel.to_string(),
        start_index,
        end_index,
        start_time: times.get(start_index).copied().unwrap_or(0.0),
        end_time: times.get(end_index).copied().unwrap_or(0.0),
        detail,
        suggested_action: suggested_action.to_string(),
        resolved: false,
    }
}

/// Build a `packet.sequence_gap` finding for a binary log.
///
/// Sequence continuity is a property of the transport, not of the numeric
/// columns, so the binary importer calls this with the counters it decoded.
pub fn sequence_gap_flag(count: usize, start_time: Real, end_time: Real) -> DataQualityFlag {
    DataQualityFlag {
        code: "packet.sequence_gap".to_string(),
        severity: Severity::Warning,
        channel: "protocol".to_string(),
        start_index: 0,
        end_index: 0,
        start_time,
        end_time,
        detail: format!(
            "{count} record sequence gap(s) were detected; samples may be missing between the surviving records."
        ),
        suggested_action:
            "Check the link quality and the logger buffer size, then re-record if the gap matters."
                .to_string(),
        resolved: false,
    }
}

/// The plausible range used for a channel's clipping check.
fn range_for(role: ChannelRole, options: &QualityOptions) -> Option<(Real, Real)> {
    match role.family() {
        Some(crate::channel::ChannelFamily::Accelerometer) => Some(options.accel_plausible_range),
        Some(crate::channel::ChannelFamily::Gyroscope) => Some(options.gyro_plausible_range),
        _ => role.plausible_range(),
    }
}

/// Assess channel and time-axis quality for an imported table.
pub fn assess_quality(
    channels: &[Channel],
    times: &[Real],
    options: &QualityOptions,
) -> QualityReport {
    let mut report = QualityReport::clean();
    let analysis = analyze_clock(times, 3.0);
    report.missing_samples = analysis.gaps.iter().map(|gap| gap.estimated_missing).sum();

    let last_index = times.len().saturating_sub(1);

    for channel in channels {
        let values = channel.corrected_values();
        let stats = channel.statistics_within(range_for(channel.role, options));

        report.nan_values += stats.nan_count;
        report.infinite_values += stats.infinite_count;

        if values.len() < options.minimum_samples {
            push_flag(
                &mut report,
                make_flag(
                    "channel.missing",
                    Severity::Warning,
                    &channel.name,
                    0,
                    values.len().saturating_sub(1),
                    times,
                    format!(
                        "The channel has {} sample(s); at least {} are needed for analysis.",
                        values.len(),
                        options.minimum_samples
                    ),
                    "Check the delimiter and the row limit, or drop the channel from the mapping.",
                ),
            );
        }

        if stats.nan_count > 0 {
            let first = values.iter().position(|v| v.is_nan()).unwrap_or(0);
            push_flag(
                &mut report,
                make_flag(
                    "channel.nan",
                    Severity::Warning,
                    &channel.name,
                    first,
                    first,
                    times,
                    format!(
                        "{} sample(s) could not be parsed or were blank and were stored as NaN.",
                        stats.nan_count
                    ),
                    "Repair the source rows, or accept the NaN run and gap-fill later.",
                ),
            );
        }

        if stats.infinite_count > 0 {
            let first = values.iter().position(|v| v.is_infinite()).unwrap_or(0);
            push_flag(
                &mut report,
                make_flag(
                    "channel.infinite",
                    Severity::Error,
                    &channel.name,
                    first,
                    first,
                    times,
                    format!(
                        "{} sample(s) are infinite, which no filter or estimator can use.",
                        stats.infinite_count
                    ),
                    "Treat the affected samples as invalid and remove them before analysis.",
                ),
            );
        }

        if stats.finite_count >= 2 && stats.range() <= options.constant_channel_tolerance {
            report.constant_channels.push(channel.name.clone());
            push_flag(
                &mut report,
                make_flag(
                    "channel.constant",
                    Severity::Warning,
                    &channel.name,
                    0,
                    values.len().saturating_sub(1),
                    times,
                    format!(
                        "The channel is stuck at {:.9}; the sensor may be disconnected.",
                        stats.mean
                    ),
                    "Check the wiring and the sensor configuration, then re-record.",
                ),
            );
        }

        if stats.saturated_low {
            report.clipped_channels.push(channel.name.clone());
            push_flag(
                &mut report,
                make_flag(
                    "channel.clipped_low",
                    Severity::Warning,
                    &channel.name,
                    0,
                    last_index,
                    times,
                    format!(
                        "The channel reached {:.6}, the bottom of its plausible range.",
                        stats.min
                    ),
                    "Verify the sensor range and the unit, then lower the expected signal.",
                ),
            );
        }

        if stats.saturated_high {
            report.clipped_channels.push(channel.name.clone());
            push_flag(
                &mut report,
                make_flag(
                    "channel.clipped_high",
                    Severity::Warning,
                    &channel.name,
                    0,
                    last_index,
                    times,
                    format!(
                        "The channel reached {:.6}, the top of its plausible range.",
                        stats.max
                    ),
                    "Verify the sensor range and the unit, then raise the expected signal.",
                ),
            );
        }

        if channel.role == ChannelRole::BaroPressure {
            let limit = options.pressure_jump_threshold_pa.abs();
            for (index, pair) in values.windows(2).enumerate() {
                let previous = pair[0];
                let current = pair[1];
                if !previous.is_finite() || !current.is_finite() {
                    continue;
                }
                if (current - previous).abs() > limit {
                    push_flag(
                        &mut report,
                        make_flag(
                            "pressure.jump",
                            Severity::Warning,
                            &channel.name,
                            index,
                            index + 1,
                            times,
                            format!(
                                "Pressure changed by {:.3} Pa in one sample, above the {:.3} Pa limit.",
                                (current - previous).abs(),
                                limit
                            ),
                            "Check for a tubing blockage or an I2C glitch at the affected sample.",
                        ),
                    );
                    break;
                }
            }
        }
    }

    report.constant_channels.sort();
    report.constant_channels.dedup();
    report.clipped_channels.sort();
    report.clipped_channels.dedup();

    // A sustained body rate beyond the plausible range is more than clipping:
    // it means the gyro or its scale is wrong.
    let gyro_roles = [ChannelRole::GyroX, ChannelRole::GyroY, ChannelRole::GyroZ];
    let gyro_axes: Vec<&Channel> = gyro_roles
        .iter()
        .filter_map(|role| channels.iter().find(|c| c.role == *role))
        .collect();
    if gyro_axes.len() == 3 {
        let limit = options
            .gyro_plausible_range
            .0
            .abs()
            .max(options.gyro_plausible_range.1.abs());
        let corrected: Vec<Vec<Real>> = gyro_axes.iter().map(|c| c.corrected_values()).collect();
        for (index, ((&x, &y), &z)) in corrected[0]
            .iter()
            .zip(corrected[1].iter())
            .zip(corrected[2].iter())
            .enumerate()
        {
            let magnitude = (x * x + y * y + z * z).sqrt();
            if magnitude.is_finite() && magnitude > limit {
                push_flag(
                    &mut report,
                    make_flag(
                        "gyro.unrealistic_rate",
                        Severity::Warning,
                        "gyro",
                        index,
                        index,
                        times,
                        format!(
                            "Body rate magnitude reached {:.3} rad/s at this sample, above the {:.3} rad/s limit.",
                            magnitude, limit
                        ),
                        "Check the gyro full-scale range and the deg/s to rad/s conversion.",
                    ),
                );
                break;
            }
        }
    }

    for index in analysis.reversal_indices.iter().copied() {
        push_flag(
            &mut report,
            make_flag(
                "time.reversal",
                Severity::Error,
                "time",
                index.saturating_sub(1),
                index,
                times,
                format!(
                    "Time at sample {index} is earlier than the previous sample by {:.9} s.",
                    times[index - 1] - times[index]
                ),
                "Split the log at the reversal or re-export it with a monotonic clock.",
            ),
        );
    }

    for index in analysis.duplicate_indices.iter().copied() {
        push_flag(
            &mut report,
            make_flag(
                "time.duplicate",
                Severity::Warning,
                "time",
                index.saturating_sub(1),
                index,
                times,
                format!("Sample {index} repeats the previous timestamp exactly."),
                "Deduplicate the log, or keep the burst and mark it as intentional.",
            ),
        );
    }

    for gap in &analysis.gaps {
        push_flag(
            &mut report,
            make_flag(
                "time.gap",
                Severity::Warning,
                "time",
                gap.after_index,
                gap.after_index + 1,
                times,
                format!(
                    "The time axis jumps by {:.6} s here, about {} missing sample(s).",
                    gap.duration, gap.estimated_missing
                ),
                "Confirm the logger really stopped, then decide whether to resample across the gap.",
            ),
        );
    }

    report.recompute();
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelRole;

    fn ramp(count: usize, dt: Real) -> Vec<Real> {
        (0..count).map(|i| i as Real * dt).collect()
    }

    fn channel(role: ChannelRole, values: Vec<Real>) -> Channel {
        Channel::new(role.code(), role, values)
    }

    #[test]
    fn default_options_match_the_documented_values() {
        let o = TimestampValidationOptions::default();
        assert_eq!(o.gap_factor, 3.0);
        assert_eq!(o.rate_change_tolerance, 0.1);
        assert_eq!(o.duplicate_epsilon, 1e-9);
        assert!(o.maximum_gap_seconds.is_none());
        assert!(o.rollover_bits.is_none());

        let q = QualityOptions::default();
        assert_eq!(q.accel_plausible_range, (-160.0, 160.0));
        assert_eq!(q.gyro_plausible_range, (-35.0, 35.0));
        assert_eq!(q.pressure_jump_threshold_pa, 2000.0);
        assert_eq!(q.minimum_samples, 8);
        assert_eq!(q.constant_channel_tolerance, 1e-12);
    }

    #[test]
    fn clean_time_series_has_no_warnings_and_one_rate_segment() {
        let times = ramp(200, 0.005);
        let result = validate_timestamps(&times, &TimestampValidationOptions::default());
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
        assert_eq!(result.rate_segments.len(), 1);
        assert!((result.rate_segments[0].rate_hz - 200.0).abs() < 1e-6);
        assert!(result.analysis.strictly_increasing);
        assert!(!result.has_errors());
    }

    #[test]
    fn start_offset_rebases_the_first_sample_to_zero() {
        let times = vec![100.0, 100.01, 100.02, 100.03];
        let result = validate_timestamps(&times, &TimestampValidationOptions::default());
        assert_eq!(result.corrected_times[0], 0.0);
        assert!((result.corrected_times[3] - 0.03).abs() < 1e-12);
        assert!((result.timebase.start_offset - 100.0).abs() < 1e-12);
        assert!((result.timebase.correct(100.02) - 0.02).abs() < 1e-12);
        assert!(result.timebase.nominal_rate_hz().unwrap() > 99.0);
    }

    #[test]
    fn gaps_are_reported_with_a_time_gap_code() {
        let mut times = ramp(50, 0.01);
        times.extend(ramp(50, 0.01).into_iter().map(|t| t + 5.0));
        let result = validate_timestamps(&times, &TimestampValidationOptions::default());
        assert_eq!(result.analysis.gaps.len(), 1);
        let gap = result
            .warnings
            .iter()
            .find(|w| w.code == "time.gap")
            .expect("gap warning");
        assert_eq!(gap.severity, Severity::Warning);
        assert!(gap.suggestion.is_some());
        assert!(gap.detail.contains("missing sample"));
    }

    #[test]
    fn maximum_gap_seconds_changes_the_suggestion() {
        let mut times = ramp(20, 0.01);
        times.extend(ramp(20, 0.01).into_iter().map(|t| t + 2.0));
        let options = TimestampValidationOptions {
            maximum_gap_seconds: Some(1.0),
            ..Default::default()
        };
        let result = validate_timestamps(&times, &options);
        let gap = result
            .warnings
            .iter()
            .find(|w| w.code == "time.gap")
            .unwrap();
        assert!(gap.suggestion.as_deref().unwrap().contains("exceeds"));
    }

    #[test]
    fn duplicate_timestamps_are_reported() {
        let times = vec![0.0, 0.01, 0.01, 0.02, 0.03];
        let result = validate_timestamps(&times, &TimestampValidationOptions::default());
        assert_eq!(result.analysis.duplicate_indices, vec![2]);
        assert!(result.warnings.iter().any(|w| w.code == "time.duplicate"));
    }

    #[test]
    fn reversals_are_reported_as_errors() {
        let times = vec![0.0, 0.1, 0.05, 0.15];
        let result = validate_timestamps(&times, &TimestampValidationOptions::default());
        assert_eq!(result.analysis.reversal_indices, vec![2]);
        assert!(result.has_errors());
        let warning = result
            .warnings
            .iter()
            .find(|w| w.code == "time.reversal")
            .unwrap();
        assert_eq!(warning.severity, Severity::Error);
        assert!(warning.suggestion.is_some());
    }

    #[test]
    fn rollover_is_unwrapped_and_recorded() {
        let times = vec![250.0, 254.0, 2.0, 6.0, 10.0];
        let options = TimestampValidationOptions {
            rollover_bits: Some(8),
            ..Default::default()
        };
        let result = validate_timestamps(&times, &options);
        let correction = result.rollover.expect("rollover recorded");
        assert_eq!(correction.wraps_detected, 1);
        assert!(result.timebase.rollover_corrected);
        assert!(result.analysis.strictly_increasing);
        assert!(result.warnings.iter().any(|w| w.code == "time.rollover"));
        // 250, 254, then 258 after unwrapping; the first sample is the offset.
        assert!((result.corrected_times[2] - 8.0).abs() < 1e-9);
    }

    #[test]
    fn rate_changes_split_the_series_into_segments() {
        // 100 samples at 200 Hz, then 99 more at 100 Hz.
        let mut times = ramp(100, 0.005);
        let last = *times.last().unwrap();
        for i in 1..100 {
            times.push(last + i as Real * 0.01);
        }
        assert_eq!(times.len(), 199);
        let result = validate_timestamps(&times, &TimestampValidationOptions::default());
        assert_eq!(result.rate_segments.len(), 2);
        assert!((result.rate_segments[0].rate_hz - 200.0).abs() < 1e-6);
        assert!((result.rate_segments[1].rate_hz - 100.0).abs() < 1e-6);
        assert_eq!(result.rate_segments[0].start_index, 0);
        assert_eq!(result.rate_segments[0].end_index, 99);
        assert_eq!(result.rate_segments[1].start_index, 99);
        assert_eq!(result.rate_segments[1].end_index, 198);
        assert!(result.warnings.iter().any(|w| w.code == "time.rate_change"));
        assert!((result.primary_rate_hz().unwrap() - 200.0).abs() < 1e-6);
    }

    #[test]
    fn expected_rate_mismatch_is_reported() {
        let times = ramp(100, 0.005);
        let options = TimestampValidationOptions {
            expected_rate_hz: Some(400.0),
            ..Default::default()
        };
        let result = validate_timestamps(&times, &options);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.code == "time.rate_mismatch"));

        let options = TimestampValidationOptions {
            expected_rate_hz: Some(200.0),
            ..Default::default()
        };
        let result = validate_timestamps(&times, &options);
        assert!(result
            .warnings
            .iter()
            .all(|w| w.code != "time.rate_mismatch"));
    }

    #[test]
    fn empty_and_non_finite_time_axes_are_rejected_loudly() {
        let result = validate_timestamps(&[], &TimestampValidationOptions::default());
        assert!(result.has_errors());
        assert!(result.warnings.iter().any(|w| w.code == "time.empty"));

        let result = validate_timestamps(
            &[0.0, Real::NAN, 0.02],
            &TimestampValidationOptions::default(),
        );
        assert!(result.warnings.iter().any(|w| w.code == "time.non_finite"));
        assert!(result.has_errors());
    }

    #[test]
    fn clean_channels_produce_an_informational_report() {
        let channels = vec![
            channel(ChannelRole::AccelX, ramp(100, 0.01)),
            channel(ChannelRole::AccelY, ramp(100, -0.01)),
        ];
        let times = ramp(100, 0.01);
        let report = assess_quality(&channels, &times, &QualityOptions::default());
        assert_eq!(report.flags.len(), 0, "{:?}", report.flags);
        assert_eq!(report.overall_severity, Severity::Info);
        assert_eq!(report.nan_values, 0);
        assert_eq!(report.infinite_values, 0);
        assert!(report.constant_channels.is_empty());
        assert!(report.clipped_channels.is_empty());
        assert!(report.summary.contains("0 finding(s)"));
        assert!(!report.has_code("channel.nan"));
    }

    #[test]
    fn nan_and_infinite_values_are_flagged_separately() {
        let mut values = ramp(20, 0.01);
        values[3] = Real::NAN;
        values[7] = Real::INFINITY;
        let channels = vec![channel(ChannelRole::AccelX, values)];
        let report = assess_quality(&channels, &ramp(20, 0.01), &QualityOptions::default());
        assert!(report.has_code("channel.nan"));
        assert!(report.has_code("channel.infinite"));
        assert_eq!(report.nan_values, 1);
        assert_eq!(report.infinite_values, 1);
        assert_eq!(report.overall_severity, Severity::Error);

        let nan = report
            .flags
            .iter()
            .find(|f| f.code == "channel.nan")
            .unwrap();
        assert_eq!(nan.channel, "accel.x");
        assert_eq!(nan.start_index, 3);
        assert!(!nan.resolved);
        assert!(!nan.suggested_action.is_empty());
    }

    #[test]
    fn constant_channels_are_flagged() {
        let channels = vec![channel(ChannelRole::GyroX, vec![0.25; 40])];
        let report = assess_quality(&channels, &ramp(40, 0.01), &QualityOptions::default());
        assert!(report.has_code("channel.constant"));
        assert_eq!(report.constant_channels, vec!["gyro.x".to_string()]);
        assert_eq!(report.overall_severity, Severity::Warning);
    }

    #[test]
    fn sparse_channels_are_flagged_as_missing() {
        let channels = vec![channel(ChannelRole::AccelZ, vec![1.0, 2.0, 3.0])];
        let report = assess_quality(&channels, &ramp(3, 0.01), &QualityOptions::default());
        assert!(report.has_code("channel.missing"));
        let flag = report
            .flags
            .iter()
            .find(|f| f.code == "channel.missing")
            .unwrap();
        assert!(flag.detail.contains("at least 8"));
    }

    #[test]
    fn clipping_is_flagged_on_both_ends() {
        let low = vec![-160.0; 20];
        let high = vec![160.0; 20];
        let channels = vec![
            channel(ChannelRole::AccelX, low),
            channel(ChannelRole::AccelY, high),
        ];
        let report = assess_quality(&channels, &ramp(20, 0.01), &QualityOptions::default());
        assert!(report.has_code("channel.clipped_low"));
        assert!(report.has_code("channel.clipped_high"));
        assert_eq!(report.clipped_channels.len(), 2);
    }

    #[test]
    fn pressure_jumps_are_flagged_once() {
        let mut values = vec![101_325.0; 20];
        values[10] = 101_325.0 + 5_000.0;
        let channels = vec![channel(ChannelRole::BaroPressure, values)];
        let report = assess_quality(&channels, &ramp(20, 0.01), &QualityOptions::default());
        assert_eq!(report.count_code("pressure.jump"), 1);
        let flag = report
            .flags
            .iter()
            .find(|f| f.code == "pressure.jump")
            .unwrap();
        assert_eq!(flag.start_index, 9);
        assert_eq!(flag.end_index, 10);
    }

    #[test]
    fn unrealistic_gyro_rates_are_flagged_from_the_vector_magnitude() {
        let channels = vec![
            channel(ChannelRole::GyroX, vec![40.0; 20]),
            channel(ChannelRole::GyroY, vec![40.0; 20]),
            channel(ChannelRole::GyroZ, vec![0.0; 20]),
        ];
        let report = assess_quality(&channels, &ramp(20, 0.01), &QualityOptions::default());
        assert!(report.has_code("gyro.unrealistic_rate"));
        let flag = report
            .flags
            .iter()
            .find(|f| f.code == "gyro.unrealistic_rate")
            .unwrap();
        assert!(flag.detail.contains("rad/s"));
    }

    #[test]
    fn time_axis_problems_are_flagged_by_the_quality_pass() {
        // The median interval stays at 0.01 s, so the 0.975 s jump is a gap.
        let times = vec![0.0, 0.01, 0.01, 0.005, 0.015, 0.025, 1.0, 1.01, 1.02, 1.03];
        let channels = vec![channel(ChannelRole::AccelX, ramp(10, 0.1))];
        let report = assess_quality(&channels, &times, &QualityOptions::default());
        assert!(report.has_code("time.duplicate"));
        assert!(report.has_code("time.reversal"));
        assert!(report.has_code("time.gap"));
        assert_eq!(report.overall_severity, Severity::Error);
        assert!(report.missing_samples > 0);
    }

    #[test]
    fn resolving_a_code_drops_it_from_the_overall_severity() {
        let channels = vec![channel(ChannelRole::AccelX, vec![1.0; 20])];
        let mut report = assess_quality(&channels, &ramp(20, 0.01), &QualityOptions::default());
        assert_eq!(report.overall_severity, Severity::Warning);
        report.resolve_code("channel.constant");
        assert_eq!(report.overall_severity, Severity::Info);
        assert_eq!(report.count_code("channel.constant"), 0);
        assert!(report
            .flags
            .iter()
            .any(|f| f.code == "channel.constant" && f.resolved));
    }

    #[test]
    fn sequence_gap_flag_describes_the_transport() {
        let flag = sequence_gap_flag(2, 1.0, 5.0);
        assert_eq!(flag.code, "packet.sequence_gap");
        assert_eq!(flag.severity, Severity::Warning);
        assert!(flag.detail.contains("2 record sequence gap"));
        assert!(!flag.resolved);
        let mut report = QualityReport::clean();
        report.flags.push(flag);
        report.recompute();
        assert!(report.has_code("packet.sequence_gap"));
    }
}
