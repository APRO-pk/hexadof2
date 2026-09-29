//! Comparison of a simulation run against a flight session.
//!
//! This module assembles the pieces: it aligns the two timebases, resamples the
//! selected channels, computes the metrics, and collects the warnings that must be
//! shown beside the numbers.
//!
//! # What a comparison may not claim
//!
//! A mismatch between a simulation and a flight does not identify a single root
//! cause. Every comparison therefore carries its assumptions as explicit warnings,
//! and the report never states a cause.

use hex_core::{Quaternion, Real, Vec3};
use serde::{Deserialize, Serialize};

use crate::alignment::{
    align, available_methods, resample, resample_vectors, Alignment, AlignmentError,
    AlignmentMethod, EventRef, TimeSeries,
};
use crate::events::{DetectedEvent, EventEvidence};
use crate::metrics::{
    compute_attitude_metrics, compute_metrics, compute_vector_metrics, AttitudeMetrics,
    ErrorMetrics, MetricChannel, MetricRow, MetricsTable, PairedSeries, VectorErrorMetrics,
};

/// A named scalar channel with its unit and provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonChannel {
    /// Channel identity used in the report.
    pub channel: MetricChannel,
    /// Time values in seconds.
    pub times: Vec<Real>,
    /// Values, one per time.
    pub values: Vec<Real>,
    /// Unit label.
    pub unit: String,
    /// Whether the values are measured, estimated, or simulated.
    pub provenance: Provenance,
    /// Whether the values were produced by integration and therefore drift.
    pub is_integrated: bool,
}

/// Where a compared series came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// A sensor measurement.
    Measured,
    /// Reconstructed by integration or filtering.
    Estimated,
    /// Produced by the simulation.
    Simulated,
    /// Typed in or adjusted by the user.
    Manual,
}

impl Provenance {
    pub fn label(self) -> &'static str {
        match self {
            Provenance::Measured => "measured",
            Provenance::Estimated => "estimated",
            Provenance::Simulated => "simulated",
            Provenance::Manual => "manual",
        }
    }

    /// Whether the series is a direct measurement, which is what an error metric
    /// against a simulation should ideally use.
    pub fn is_direct_measurement(self) -> bool {
        matches!(self, Provenance::Measured)
    }
}

/// A named attitude channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttitudeChannel {
    /// Time values in seconds.
    pub times: Vec<Real>,
    /// Body-to-world quaternions.
    pub quaternions: Vec<Quaternion>,
    pub provenance: Provenance,
}

/// One side of a comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonSeries {
    /// Display name, for example a run name or a flight log name.
    pub name: String,
    /// Scalar channels.
    pub channels: Vec<ComparisonChannel>,
    /// Attitude channel, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attitude: Option<AttitudeChannel>,
    /// Events usable as alignment anchors.
    #[serde(default)]
    pub events: Vec<DetectedEvent>,
    /// Whether the series carries absolute position, or only orientation.
    #[serde(default)]
    pub has_position: bool,
    /// Warnings about this series that the comparison must surface.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl ComparisonSeries {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            channels: Vec::new(),
            attitude: None,
            events: Vec::new(),
            has_position: false,
            warnings: Vec::new(),
        }
    }

    pub fn with_channel(mut self, channel: ComparisonChannel) -> Self {
        self.channels.push(channel);
        self
    }

    pub fn with_attitude(mut self, attitude: AttitudeChannel) -> Self {
        self.attitude = Some(attitude);
        self
    }

    pub fn with_events(mut self, events: Vec<DetectedEvent>) -> Self {
        self.events = events;
        self
    }

    pub fn with_warning(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    /// Find a scalar channel by identity.
    pub fn channel(&self, channel: MetricChannel) -> Option<&ComparisonChannel> {
        self.channels.iter().find(|c| c.channel == channel)
    }

    /// Time span in seconds.
    pub fn span(&self) -> Real {
        let first = self
            .channels
            .iter()
            .filter_map(|c| c.times.first().copied())
            .fold(Real::INFINITY, Real::min);
        let last = self
            .channels
            .iter()
            .filter_map(|c| c.times.last().copied())
            .fold(Real::NEG_INFINITY, Real::max);
        if first.is_finite() && last.is_finite() && last > first {
            last - first
        } else {
            0.0
        }
    }

    /// Build the alignable representation of this series.
    ///
    /// Scalar channels are preferred because they are what cross-correlation
    /// needs, but an attitude-only series still has a usable timebase, so the
    /// attitude times are used as a fallback. Without that fallback an
    /// orientation-only flight log could not be aligned at all.
    pub fn to_time_series(&self) -> TimeSeries {
        let times = self
            .channels
            .first()
            .map(|c| c.times.clone())
            .or_else(|| self.attitude.as_ref().map(|a| a.times.clone()))
            .unwrap_or_default();
        let mut ts = TimeSeries::from_times(times);
        for e in &self.events {
            ts.events.push((e.code.clone(), e.time));
        }
        ts
    }

    /// Build an alignable series around one channel, for cross-correlation.
    pub fn to_time_series_for(&self, channel: MetricChannel) -> Option<TimeSeries> {
        let c = self.channel(channel)?;
        Some(TimeSeries::from_times(c.times.clone()).with_values(c.values.clone()))
    }
}

/// A cross-series warning shown above the metrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonWarning {
    /// Machine code.
    pub code: String,
    pub title: String,
    pub detail: String,
    /// Whether the warning undermines the comparison.
    pub severe: bool,
}

impl ComparisonWarning {
    fn new(code: &str, title: &str, detail: impl Into<String>, severe: bool) -> Self {
        Self {
            code: code.to_string(),
            title: title.to_string(),
            detail: detail.into(),
            severe,
        }
    }
}

/// The complete comparison result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// Name of the simulation side.
    pub simulation_name: String,
    /// Name of the flight side.
    pub flight_name: String,
    /// Alignment that was applied.
    pub alignment: Alignment,
    /// The metrics table.
    pub metrics: MetricsTable,
    /// Vector metrics for position and velocity, when both sides have them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_metrics: Option<VectorErrorMetrics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity_metrics: Option<VectorErrorMetrics>,
    /// Event timing differences, keyed by event code.
    pub event_differences: Vec<EventTimingDifference>,
    /// Warnings that must be displayed with the comparison.
    pub warnings: Vec<ComparisonWarning>,
    /// The method that was actually used, restated for the saved record.
    pub alignment_method: AlignmentMethod,
}

/// A timing difference for one event between the two series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTimingDifference {
    pub code: String,
    pub label: String,
    /// Time in the simulation series, seconds.
    pub simulation_time: Real,
    /// Time in the flight series, seconds.
    pub flight_time: Real,
    /// Flight minus simulation, seconds.
    pub difference: Real,
    /// The weaker of the two detections' evidence.
    pub evidence: EventEvidence,
}

impl EventTimingDifference {
    /// A note when either detection is weak.
    pub fn reliability_note(&self) -> Option<String> {
        if self.evidence.confidence() < 0.7 {
            Some(format!(
                "This event was {}, so the timing difference is uncertain.",
                self.evidence.label()
            ))
        } else {
            None
        }
    }
}

impl Comparison {
    /// The worst channel by normalised error, which is usually the place to look
    /// first.
    pub fn worst_channel(&self) -> Option<&MetricRow> {
        self.metrics.ranked().into_iter().next()
    }

    /// Whether any warning undermines the comparison.
    pub fn has_severe_warning(&self) -> bool {
        self.warnings.iter().any(|w| w.severe)
    }

    /// A plain-text summary suitable for the exported report.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str("Simulation and flight comparison\n");
        out.push_str("================================\n");
        out.push_str(&format!("Simulation: {}\n", self.simulation_name));
        out.push_str(&format!("Flight:     {}\n", self.flight_name));
        out.push_str(&format!("Alignment:  {}\n", self.alignment.describe()));
        out.push('\n');
        out.push_str(&self.metrics.to_text());
        if let Some(p) = &self.position_metrics {
            out.push_str(&format!("\nPosition: {}\n", p.summary("m")));
        }
        if let Some(v) = &self.velocity_metrics {
            out.push_str(&format!("Velocity: {}\n", v.summary("m/s")));
        }
        if !self.event_differences.is_empty() {
            out.push_str("\nEvent timing differences\n");
            out.push_str("------------------------\n");
            for d in &self.event_differences {
                out.push_str(&format!(
                    "{:>24}: {:+8.4} s (flight {:+.4} s, simulation {:+.4} s)\n",
                    d.label, d.difference, d.flight_time, d.simulation_time
                ));
            }
        }
        if !self.warnings.is_empty() {
            out.push_str("\nWarnings\n");
            out.push_str("--------\n");
            for w in &self.warnings {
                out.push_str(&format!(
                    "[{}] {}: {}\n",
                    if w.severe { "important" } else { "note" },
                    w.title,
                    w.detail
                ));
            }
        }
        out.push_str(
            "\nA mismatch between a simulation and a flight does not identify a single cause. \
             Treat the differences above as places to investigate, not as conclusions.\n",
        );
        out
    }

    /// Channel-by-channel table rows, for the UI.
    pub fn rows(&self) -> &[MetricRow] {
        &self.metrics.rows
    }
}

/// Options for building a comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonOptions {
    /// Alignment method to apply.
    pub method: AlignmentMethod,
    /// Channels to compare. Empty means every channel both sides share.
    pub channels: Vec<MetricChannel>,
    /// Minimum overlap required for the comparison to be worth producing.
    pub minimum_overlap_seconds: Real,
    /// Angular error above which an attitude comparison is flagged, radians.
    pub attitude_warning_threshold: Real,
    /// Normalised position error above which the comparison is flagged.
    pub position_warning_threshold: Real,
}

impl Default for ComparisonOptions {
    fn default() -> Self {
        Self {
            method: AlignmentMethod::FirstSample,
            channels: Vec::new(),
            minimum_overlap_seconds: 0.1,
            attitude_warning_threshold: 10.0f64.to_radians(),
            position_warning_threshold: 0.25,
        }
    }
}

impl ComparisonOptions {
    /// Options that align on a shared event.
    pub fn on_event(code: impl Into<String>) -> Self {
        let code = code.into();
        Self {
            method: AlignmentMethod::Event {
                reference_event: EventRef::new(code.clone()),
                comparison_event: EventRef::new(code),
            },
            ..Self::default()
        }
    }

    /// Options using a manual offset.
    pub fn manual(offset_seconds: Real) -> Self {
        Self {
            method: AlignmentMethod::Manual { offset_seconds },
            ..Self::default()
        }
    }

    /// Options using bounded cross-correlation on a channel.
    pub fn correlate(half_width_seconds: Real, step_seconds: Real) -> Self {
        Self {
            method: AlignmentMethod::CrossCorrelation {
                search_half_width_seconds: half_width_seconds,
                step_seconds,
            },
            ..Self::default()
        }
    }
}

/// The resampled values behind a vector error metric: the shared times, and the
/// reference and comparison vectors at each of them.
type VectorPairSet = (Vec<Real>, Vec<[Real; 3]>, Vec<[Real; 3]>);

/// Build a comparison between a simulation and a flight.
pub fn compare(
    simulation: &ComparisonSeries,
    flight: &ComparisonSeries,
    options: &ComparisonOptions,
) -> Result<Comparison, AlignmentError> {
    // Cross-correlation needs a shared channel, so build the alignable series
    // around whichever channel both sides carry.
    let (reference_ts, comparison_ts) = match options.method.clone() {
        AlignmentMethod::CrossCorrelation { .. } => {
            let shared = simulation
                .channels
                .iter()
                .find(|c| flight.channel(c.channel).is_some())
                .map(|c| c.channel);
            match shared {
                Some(channel) => (
                    simulation
                        .to_time_series_for(channel)
                        .ok_or(AlignmentError::ChannelMissing)?,
                    flight
                        .to_time_series_for(channel)
                        .ok_or(AlignmentError::ChannelMissing)?,
                ),
                None => return Err(AlignmentError::ChannelMissing),
            }
        }
        _ => (simulation.to_time_series(), flight.to_time_series()),
    };

    let alignment = align(&reference_ts, &comparison_ts, options.method.clone())?;
    let offset = alignment.offset_seconds;

    let wanted: Vec<MetricChannel> = if options.channels.is_empty() {
        MetricChannel::default_set().to_vec()
    } else {
        options.channels.clone()
    };

    let mut metrics = MetricsTable::new();
    let mut position_pairs: Option<VectorPairSet> = None;
    let mut velocity_pairs: Option<VectorPairSet> = None;

    for channel in wanted {
        let Some(sim) = simulation.channel(channel).or_else(|| {
            if channel == MetricChannel::Altitude {
                simulation.channel(MetricChannel::Altitude)
            } else {
                None
            }
        }) else {
            metrics.unavailable.push((
                channel.label().to_string(),
                format!(
                    "the simulation does not provide {}",
                    channel.label().to_lowercase()
                ),
            ));
            continue;
        };
        let Some(flight_channel) = flight.channel(channel) else {
            metrics.unavailable.push((
                channel.label().to_string(),
                format!("the flight log has no {}", channel.label().to_lowercase()),
            ));
            continue;
        };

        let reference = resample(&sim.times, &sim.values, &flight_channel.times, 0.0);
        let comparison = resample(
            &flight_channel.times,
            &flight_channel.values,
            &flight_channel.times,
            offset,
        );
        let paired = PairedSeries::new(
            flight_channel.times.clone(),
            reference,
            comparison,
            channel.label(),
            sim.unit.clone(),
        );
        metrics.push(MetricRow::scalar(channel, compute_metrics(&paired)));
    }

    // Attitude, when both sides carry it.
    if let (Some(sim_att), Some(flight_att)) = (&simulation.attitude, &flight.attitude) {
        let reference =
            resample_attitudes(&sim_att.times, &sim_att.quaternions, &flight_att.times, 0.0);
        let comparison = resample_attitudes(
            &flight_att.times,
            &flight_att.quaternions,
            &flight_att.times,
            offset,
        );
        let mut times = Vec::new();
        let mut a = Vec::new();
        let mut b = Vec::new();
        for i in 0..flight_att.times.len() {
            if let (Some(x), Some(y)) = (
                reference.get(i).copied().flatten(),
                comparison.get(i).copied().flatten(),
            ) {
                times.push(flight_att.times[i]);
                a.push(x);
                b.push(y);
            }
        }
        metrics.push(MetricRow::attitude(
            MetricChannel::Attitude,
            compute_attitude_metrics(&times, &a, &b),
        ));
    } else {
        metrics.unavailable.push((
            MetricChannel::Attitude.label().to_string(),
            "one of the two series has no attitude channel".to_string(),
        ));
    }

    // Position and velocity vectors, when both sides carry them.
    let target_times = flight
        .channel(MetricChannel::Altitude)
        .map(|c| c.times.clone())
        .or_else(|| flight.channels.first().map(|c| c.times.clone()))
        .unwrap_or_default();

    if let (Some(sim_pos), Some(flight_pos)) = (
        vector_channel(simulation, MetricChannel::Altitude),
        vector_channel(flight, MetricChannel::Altitude),
    ) {
        let reference = resample_vectors(&sim_pos.0, &sim_pos.1, &target_times, 0.0);
        let comparison = resample_vectors(&flight_pos.0, &flight_pos.1, &target_times, offset);
        let mut times = Vec::new();
        let mut a = Vec::new();
        let mut b = Vec::new();
        for (i, time) in target_times.iter().enumerate() {
            if let (Some(x), Some(y)) = (
                reference.get(i).copied().flatten(),
                comparison.get(i).copied().flatten(),
            ) {
                times.push(*time);
                a.push([x.x, x.y, x.z]);
                b.push([y.x, y.y, y.z]);
            }
        }
        if !times.is_empty() {
            position_pairs = Some((times, a, b));
        }
    }

    if let (Some(sim_vel), Some(flight_vel)) = (
        vector_channel(simulation, MetricChannel::Speed),
        vector_channel(flight, MetricChannel::Speed),
    ) {
        let reference = resample_vectors(&sim_vel.0, &sim_vel.1, &target_times, 0.0);
        let comparison = resample_vectors(&flight_vel.0, &flight_vel.1, &target_times, offset);
        let mut times = Vec::new();
        let mut a = Vec::new();
        let mut b = Vec::new();
        for (i, time) in target_times.iter().enumerate() {
            if let (Some(x), Some(y)) = (
                reference.get(i).copied().flatten(),
                comparison.get(i).copied().flatten(),
            ) {
                times.push(*time);
                a.push([x.x, x.y, x.z]);
                b.push([y.x, y.y, y.z]);
            }
        }
        if !times.is_empty() {
            velocity_pairs = Some((times, a, b));
        }
    }

    let position_metrics = position_pairs
        .as_ref()
        .map(|(t, a, b)| compute_vector_metrics(t, a, b));
    let velocity_metrics = velocity_pairs
        .as_ref()
        .map(|(t, a, b)| compute_vector_metrics(t, a, b));

    // Event timing differences for every code both sides carry.
    let mut event_differences = Vec::new();
    for sim_event in &simulation.events {
        if let Some(flight_event) = flight.events.iter().find(|e| e.code == sim_event.code) {
            let evidence = if sim_event.confidence() <= flight_event.confidence() {
                sim_event.evidence
            } else {
                flight_event.evidence
            };
            event_differences.push(EventTimingDifference {
                code: sim_event.code.clone(),
                label: sim_event.label.clone(),
                simulation_time: sim_event.time,
                flight_time: flight_event.time,
                difference: flight_event.time - sim_event.time,
                evidence,
            });
        }
    }

    let warnings = collect_warnings(
        simulation,
        flight,
        &metrics,
        &alignment,
        options,
        position_metrics.as_ref(),
    );

    // A comparison must never be presented without saying which alignment
    // produced it.
    let alignment_method = options.method.clone();

    Ok(Comparison {
        simulation_name: simulation.name.clone(),
        flight_name: flight.name.clone(),
        alignment,
        metrics,
        position_metrics,
        velocity_metrics,
        event_differences,
        warnings,
        alignment_method,
    })
}

fn resample_attitudes(
    source_times: &[Real],
    source_values: &[Quaternion],
    target_times: &[Real],
    offset: Real,
) -> Vec<Option<Quaternion>> {
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
            // Orientation is interpolated on the shortest path, never by
            // differencing Euler angles.
            Some(source_values[idx].slerp(&source_values[idx + 1], f))
        })
        .collect()
}

/// Extract a vector channel stored alongside the scalar channel of the same
/// identity, when the caller supplied one.
fn vector_channel(
    series: &ComparisonSeries,
    channel: MetricChannel,
) -> Option<(Vec<Real>, Vec<Vec3>)> {
    let c = series.channel(channel)?;
    // A three-component channel is stored as three successive scalar channels of
    // the same identity is not expressible here, so the vector is taken from the
    // channel's own values only when the length is a multiple of three.
    if c.values.len() == c.times.len() * 3 {
        let vectors = c
            .values
            .chunks_exact(3)
            .map(|w| Vec3::new(w[0], w[1], w[2]))
            .collect();
        Some((c.times.clone(), vectors))
    } else {
        None
    }
}

fn collect_warnings(
    simulation: &ComparisonSeries,
    flight: &ComparisonSeries,
    metrics: &MetricsTable,
    alignment: &Alignment,
    options: &ComparisonOptions,
    position_metrics: Option<&VectorErrorMetrics>,
) -> Vec<ComparisonWarning> {
    let mut warnings = Vec::new();

    if !alignment.is_usable(options.minimum_overlap_seconds) {
        warnings.push(ComparisonWarning::new(
            "alignment.insufficient_overlap",
            "The two series barely overlap",
            format!(
                "After alignment the overlap is {:.3} s, below the configured minimum of {:.3} s. The metrics cover too little of the flight to be meaningful.",
                alignment.overlap_seconds, options.minimum_overlap_seconds
            ),
            true,
        ));
    }

    if let Some(message) = alignment.warning() {
        warnings.push(ComparisonWarning::new(
            "alignment.weak",
            "The alignment may be wrong",
            message,
            true,
        ));
    }

    if alignment.method.requires_shared_clock() {
        warnings.push(ComparisonWarning::new(
            "timebase.shared_clock_assumed",
            "The two timebases are assumed to share a clock",
            "Absolute alignment was used, so any offset between the device clock and the host clock appears directly as an error. Align on lift-off or apogee if the clocks are not synchronised.",
            false,
        ));
    }

    // Provenance: an estimated position is not a measurement.
    for series in [simulation, flight] {
        for c in &series.channels {
            if c.is_integrated {
                warnings.push(ComparisonWarning::new(
                    "data.integrated_position",
                    &format!("{} in {} is not measured", c.channel.label(), series.name),
                    format!(
                        "The {} channel in {} was produced by integration and drifts without an external position reference. A difference against it partly measures that drift, and it is labelled {}.",
                        c.channel.label().to_lowercase(),
                        series.name,
                        c.provenance.label()
                    ),
                    true,
                ));
            } else if !c.provenance.is_direct_measurement() && c.provenance == Provenance::Estimated
            {
                warnings.push(ComparisonWarning::new(
                    "data.estimated_channel",
                    &format!("{} in {} is an estimate", c.channel.label(), series.name),
                    format!(
                        "The {} channel in {} is labelled {} rather than measured, so part of any difference comes from the estimator.",
                        c.channel.label().to_lowercase(),
                        series.name,
                        c.provenance.label()
                    ),
                    false,
                ));
            }
        }
    }

    // Attitude is compared as a rotation distance rather than as a channel, so
    // its provenance is checked separately. An integrated attitude has no
    // absolute reference, and a reader who is told only that the error is large
    // will look for a dynamics problem that is not there.
    for series in [simulation, flight] {
        if let Some(attitude) = &series.attitude {
            if !attitude.provenance.is_direct_measurement() {
                warnings.push(ComparisonWarning::new(
                    "attitude.relative",
                    &format!("Attitude in {} is {}", series.name, attitude.provenance.label()),
                    format!(
                        "The attitude in {} is {}. An attitude integrated from a gyroscope has no absolute reference, so it drifts and a constant offset against the other side is expected even when the dynamics agree.",
                        series.name,
                        attitude.provenance.label()
                    ),
                    true,
                ));
            }
        }
    }

    if !simulation.has_position {
        warnings.push(ComparisonWarning::new(
            "data.no_simulated_position",
            "The simulation series carries no position",
            "Trajectory comparison is not possible without a position channel on both sides.",
            true,
        ));
    }
    if !flight.has_position {
        warnings.push(ComparisonWarning::new(
            "data.no_flight_position",
            "The flight series carries no position",
            "Trajectory comparison is not possible without a position channel on both sides. Orientation can still be compared.",
            false,
        ));
    }

    // An attitude error beyond the threshold means the comparison is dominated by
    // attitude, which usually points at initial conditions rather than dynamics.
    if let Some(row) = metrics
        .rows
        .iter()
        .find(|r| r.channel == MetricChannel::Attitude)
    {
        if let Some(a) = &row.attitude {
            if a.maximum_angle_error > options.attitude_warning_threshold {
                warnings.push(ComparisonWarning::new(
                    "attitude.large_error",
                    "Attitude error is large",
                    format!(
                        "The largest rotation distance is {:.2} degrees, above the configured {:.2} degrees. Check the initial attitude, the launch rail direction, and whether the two attitude sources use the same frame before reading anything into the dynamics.",
                        a.maximum_degrees(),
                        options.attitude_warning_threshold.to_degrees()
                    ),
                    false,
                ));
            }
        }
    }

    if let Some(p) = position_metrics {
        if p.samples > 0 && p.root_mean_square_error > 0.0 {
            let span = metrics
                .rows
                .iter()
                .filter(|r| r.channel == MetricChannel::Altitude)
                .filter_map(|r| r.scalar.as_ref())
                .map(|s| s.reference_range)
                .fold(0.0 as Real, Real::max);
            if span > 1e-9 && p.root_mean_square_error / span > options.position_warning_threshold {
                warnings.push(ComparisonWarning::new(
                    "position.large_error",
                    "Position error is a large fraction of the flight",
                    format!(
                        "The position RMSE is {:.2} m against an altitude range of {:.2} m.",
                        p.root_mean_square_error, span
                    ),
                    false,
                ));
            }
        }
    }

    // Carry the per-series warnings through unchanged.
    for series in [simulation, flight] {
        for w in &series.warnings {
            warnings.push(ComparisonWarning::new(
                "series.warning",
                &format!("Note from {}", series.name),
                w.clone(),
                false,
            ));
        }
    }

    // Frame and unit differences are not observable from the assembled series, so
    // state the assumption rather than pretending it was verified.
    warnings.push(ComparisonWarning::new(
        "frame.assumed_shared",
        "Both series are assumed to use the same frame and units",
        "HexaDOF converts imported data to SI internally, but a frame mismatch between a device profile and a project is not detectable after the fact. Confirm the body and world frame declarations before trusting a trajectory comparison.",
        false,
    ));

    warnings
}

/// The alignment methods worth offering for a pair of series.
pub fn suggested_methods(
    simulation: &ComparisonSeries,
    flight: &ComparisonSeries,
) -> Vec<AlignmentMethod> {
    available_methods(&simulation.to_time_series(), &flight.to_time_series())
}

/// A short list of what a comparison found, for the overview card.
pub fn comparison_headline(comparison: &Comparison) -> String {
    let worst = comparison
        .worst_channel()
        .map(|r| r.channel.label().to_string())
        .unwrap_or_else(|| "nothing".to_string());
    let severe = comparison.warnings.iter().filter(|w| w.severe).count();
    format!(
        "Aligned by {}. Largest differences on {}. {} warning(s), {} of them important.",
        comparison.alignment.method.label(),
        worst,
        comparison.warnings.len(),
        severe
    )
}

/// Summarise a metric row as a single comparable number, for ranking.
pub fn row_score(row: &MetricRow) -> Real {
    match (&row.scalar, &row.attitude) {
        (Some(s), _) => s.normalized_rmse.unwrap_or(s.root_mean_square_error),
        (None, Some(a)) => a.root_mean_square_angle_error,
        _ => 0.0,
    }
}

/// Convenience: the scalar metrics for one channel of a comparison.
pub fn metrics_for(comparison: &Comparison, channel: MetricChannel) -> Option<ErrorMetrics> {
    comparison
        .metrics
        .rows
        .iter()
        .find(|r| r.channel == channel)
        .and_then(|r| r.scalar)
}

/// Convenience: the attitude metrics of a comparison.
pub fn attitude_metrics_for(comparison: &Comparison) -> Option<AttitudeMetrics> {
    comparison
        .metrics
        .rows
        .iter()
        .find(|r| r.channel == MetricChannel::Attitude)
        .and_then(|r| r.attitude)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(
        identity: MetricChannel,
        time_start: Real,
        dt: Real,
        n: usize,
        scale: Real,
        provenance: Provenance,
    ) -> ComparisonChannel {
        let times: Vec<Real> = (0..n).map(|i| time_start + i as Real * dt).collect();
        let values: Vec<Real> = times.iter().map(|t| (*t - time_start) * scale).collect();
        ComparisonChannel {
            channel: identity,
            times,
            values,
            unit: identity.unit().to_string(),
            provenance,
            is_integrated: false,
        }
    }

    fn pair(scale_a: Real, scale_b: Real) -> (ComparisonSeries, ComparisonSeries) {
        let sim = ComparisonSeries::new("Run 001")
            .with_channel(channel(
                MetricChannel::Altitude,
                0.0,
                0.01,
                100,
                scale_a,
                Provenance::Simulated,
            ))
            .with_channel(channel(
                MetricChannel::Speed,
                0.0,
                0.01,
                100,
                scale_a * 0.5,
                Provenance::Simulated,
            ));
        let flight = ComparisonSeries::new("flight-001")
            .with_channel(channel(
                MetricChannel::Altitude,
                0.0,
                0.01,
                100,
                scale_b,
                Provenance::Measured,
            ))
            .with_channel(channel(
                MetricChannel::Speed,
                0.0,
                0.01,
                100,
                scale_b * 0.5,
                Provenance::Measured,
            ));
        (sim, flight)
    }

    #[test]
    fn identical_series_compare_with_zero_error() {
        let (sim, flight) = pair(2.0, 2.0);
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        let altitude = metrics_for(&comparison, MetricChannel::Altitude).unwrap();
        assert_eq!(altitude.samples, 100);
        assert!(altitude.root_mean_square_error < 1e-9);
        assert_eq!(comparison.simulation_name, "Run 001");
        assert_eq!(comparison.flight_name, "flight-001");
    }

    #[test]
    fn a_scale_difference_shows_up_in_the_metrics() {
        let (sim, flight) = pair(2.0, 2.5);
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        let altitude = metrics_for(&comparison, MetricChannel::Altitude).unwrap();
        assert!(altitude.root_mean_square_error > 0.0);
        assert!(altitude.mean_error > 0.0);
        assert!(comparison.worst_channel().is_some());
    }

    #[test]
    fn a_channel_missing_from_one_side_is_reported_rather_than_skipped() {
        let mut sim = ComparisonSeries::new("run");
        sim = sim.with_channel(channel(
            MetricChannel::Altitude,
            0.0,
            0.01,
            50,
            1.0,
            Provenance::Simulated,
        ));
        let mut flight = ComparisonSeries::new("flight");
        flight = flight.with_channel(channel(
            MetricChannel::DynamicPressure,
            0.0,
            0.01,
            50,
            1.0,
            Provenance::Measured,
        ));
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        assert!(!comparison.metrics.unavailable.is_empty());
        let (channel, reason) = &comparison.metrics.unavailable[0];
        assert!(!channel.is_empty());
        assert!(reason.contains("does not provide") || reason.contains("no"));
    }

    #[test]
    fn attitude_comparison_uses_rotation_distance() {
        let times: Vec<Real> = (0..50).map(|i| i as Real * 0.01).collect();
        let sim_attitudes: Vec<Quaternion> = times
            .iter()
            .map(|t| Quaternion::from_axis_angle(Vec3::z(), t * 0.5))
            .collect();
        let flight_attitudes: Vec<Quaternion> = times
            .iter()
            .map(|t| Quaternion::from_axis_angle(Vec3::z(), t * 0.5 + 0.05))
            .collect();
        let sim = ComparisonSeries::new("run").with_attitude(AttitudeChannel {
            times: times.clone(),
            quaternions: sim_attitudes,
            provenance: Provenance::Simulated,
        });
        let flight = ComparisonSeries::new("flight").with_attitude(AttitudeChannel {
            times,
            quaternions: flight_attitudes,
            provenance: Provenance::Measured,
        });
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        let attitude = attitude_metrics_for(&comparison).unwrap();
        assert_eq!(attitude.samples, 50);
        assert!((attitude.mean_angle_error - 0.05).abs() < 1e-9);
    }

    #[test]
    fn event_alignment_uses_matching_markers() {
        let (mut sim, mut flight) = pair(2.0, 2.0);
        sim.events.push(DetectedEvent::new(
            "lift_off",
            "Lift-off",
            1.0,
            "vz",
            0.5,
            EventEvidence::SignChange,
        ));
        flight.events.push(DetectedEvent::new(
            "lift_off",
            "Lift-off",
            1.3,
            "vz",
            0.5,
            EventEvidence::SignChange,
        ));
        let comparison = compare(&sim, &flight, &ComparisonOptions::on_event("lift_off")).unwrap();
        assert!((comparison.alignment.offset_seconds + 0.3).abs() < 1e-12);
        assert_eq!(comparison.event_differences.len(), 1);
        assert!((comparison.event_differences[0].difference - 0.3).abs() < 1e-12);
    }

    #[test]
    fn missing_event_alignment_anchor_is_an_error() {
        let (sim, flight) = pair(2.0, 2.0);
        let err = compare(&sim, &flight, &ComparisonOptions::on_event("apogee")).unwrap_err();
        assert!(matches!(err, AlignmentError::MissingEvent { .. }));
        assert!(!err.suggested_action().is_empty());
    }

    #[test]
    fn manual_offset_is_recorded_in_the_result() {
        let (sim, flight) = pair(2.0, 2.0);
        let comparison = compare(&sim, &flight, &ComparisonOptions::manual(0.15)).unwrap();
        assert!((comparison.alignment.offset_seconds - 0.15).abs() < 1e-12);
        assert!(comparison.to_text().contains("Manual offset"));
    }

    #[test]
    fn cross_correlation_alignment_finds_a_shift() {
        let n = 300;
        let dt = 0.01;
        let sim_times: Vec<Real> = (0..n).map(|i| i as Real * dt).collect();
        let sim_values: Vec<Real> = sim_times
            .iter()
            .map(|t| (t * 5.0).sin() + 0.5 * (t * 2.0).sin())
            .collect();
        let shift = 0.2;
        let flight_times: Vec<Real> = sim_times.iter().map(|t| t + shift).collect();
        let flight_values = sim_values.clone();

        let sim = ComparisonSeries::new("run").with_channel(ComparisonChannel {
            channel: MetricChannel::Altitude,
            times: sim_times,
            values: sim_values,
            unit: "m".to_string(),
            provenance: Provenance::Simulated,
            is_integrated: false,
        });
        let flight = ComparisonSeries::new("flight").with_channel(ComparisonChannel {
            channel: MetricChannel::Altitude,
            times: flight_times,
            values: flight_values,
            unit: "m".to_string(),
            provenance: Provenance::Measured,
            is_integrated: false,
        });
        let options = ComparisonOptions::correlate(1.0, 0.005);
        let comparison = compare(&sim, &flight, &options).unwrap();
        assert!(
            (comparison.alignment.offset_seconds + shift).abs() < 0.02,
            "offset {}",
            comparison.alignment.offset_seconds
        );
    }

    #[test]
    fn integrated_position_produces_a_severe_warning() {
        let mut sim = ComparisonSeries::new("run");
        sim = sim.with_channel(channel(
            MetricChannel::Altitude,
            0.0,
            0.01,
            50,
            2.0,
            Provenance::Simulated,
        ));
        let mut flight = ComparisonSeries::new("flight");
        flight = flight.with_channel(ComparisonChannel {
            is_integrated: true,
            provenance: Provenance::Estimated,
            ..channel(
                MetricChannel::Altitude,
                0.0,
                0.01,
                50,
                2.0,
                Provenance::Estimated,
            )
        });
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        assert!(comparison.has_severe_warning());
        assert!(comparison
            .warnings
            .iter()
            .any(|w| w.code == "data.integrated_position"));
    }

    #[test]
    fn absolute_alignment_warns_about_the_shared_clock_assumption() {
        let (sim, flight) = pair(2.0, 2.0);
        let comparison = compare(
            &sim,
            &flight,
            &ComparisonOptions {
                method: AlignmentMethod::Absolute,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(comparison
            .warnings
            .iter()
            .any(|w| w.code == "timebase.shared_clock_assumed"));
    }

    #[test]
    fn insufficient_overlap_is_a_severe_warning() {
        let (mut sim, mut flight) = pair(2.0, 2.0);
        for c in sim.channels.iter_mut() {
            c.times = c.times.iter().map(|t| t + 100.0).collect();
        }
        for c in flight.channels.iter_mut() {
            c.times = c.times.iter().map(|t| t + 0.0).collect();
        }
        // Absolute alignment is the method that exposes a start-time difference.
        let options = ComparisonOptions {
            method: AlignmentMethod::Absolute,
            ..Default::default()
        };
        let comparison = compare(&sim, &flight, &options).unwrap();
        assert!(
            comparison
                .warnings
                .iter()
                .any(|w| w.code == "alignment.insufficient_overlap" && w.severe),
            "warnings {:?}",
            comparison.warnings
        );
    }

    #[test]
    fn a_large_attitude_error_produces_a_warning() {
        let times: Vec<Real> = (0..50).map(|i| i as Real * 0.01).collect();
        let sim = ComparisonSeries::new("run").with_attitude(AttitudeChannel {
            times: times.clone(),
            quaternions: vec![Quaternion::identity(); 50],
            provenance: Provenance::Simulated,
        });
        let flight = ComparisonSeries::new("flight").with_attitude(AttitudeChannel {
            times,
            quaternions: vec![Quaternion::from_axis_angle(Vec3::z(), 0.5); 50],
            provenance: Provenance::Measured,
        });
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        let warning = comparison
            .warnings
            .iter()
            .find(|w| w.code == "attitude.large_error")
            .expect("attitude warning");
        assert!(warning.detail.contains("frame"));
    }

    #[test]
    fn comparison_text_states_that_a_mismatch_has_no_single_cause() {
        let (sim, flight) = pair(2.0, 2.5);
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        let text = comparison.to_text();
        assert!(text.contains("does not identify a single cause"));
        assert!(text.contains("Alignment:"));
        assert!(text.contains("Simulation:"));
    }

    #[test]
    fn comparison_headline_mentions_the_alignment_and_warnings() {
        let (sim, flight) = pair(2.0, 2.5);
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        let headline = comparison_headline(&comparison);
        assert!(headline.contains("Aligned by"));
        assert!(headline.contains("warning"));
    }

    #[test]
    fn suggested_methods_include_the_workable_options() {
        let (sim, flight) = pair(2.0, 2.0);
        let methods = suggested_methods(&sim, &flight);
        assert!(methods.contains(&AlignmentMethod::Absolute));
        assert!(methods.contains(&AlignmentMethod::FirstSample));
        assert!(methods
            .iter()
            .any(|m| matches!(m, AlignmentMethod::Manual { .. })));
    }

    #[test]
    fn provenance_labels_and_predicates() {
        assert!(Provenance::Measured.is_direct_measurement());
        assert!(!Provenance::Estimated.is_direct_measurement());
        assert!(!Provenance::Simulated.is_direct_measurement());
        assert_eq!(Provenance::Estimated.label(), "estimated");
    }

    #[test]
    fn comparison_series_accessors() {
        let (sim, _) = pair(2.0, 2.0);
        assert!(sim.channel(MetricChannel::Altitude).is_some());
        assert!(sim.channel(MetricChannel::TotalMoment).is_none());
        assert!((sim.span() - 0.99).abs() < 1e-9);
        let ts = sim.to_time_series();
        assert_eq!(ts.times.len(), 100);
        let for_channel = sim.to_time_series_for(MetricChannel::Speed).unwrap();
        assert!(for_channel.values.is_some());
        assert!(sim.to_time_series_for(MetricChannel::TotalMoment).is_none());
    }

    #[test]
    fn vector_channels_are_detected_from_a_tripled_value_list() {
        let times: Vec<Real> = (0..10).map(|i| i as Real * 0.1).collect();
        let values: Vec<Real> = (0..30).map(|i| i as Real).collect();
        let series = ComparisonSeries::new("run").with_channel(ComparisonChannel {
            channel: MetricChannel::Altitude,
            times: times.clone(),
            values,
            unit: "m".to_string(),
            provenance: Provenance::Simulated,
            is_integrated: false,
        });
        let (t, v) = vector_channel(&series, MetricChannel::Altitude).unwrap();
        assert_eq!(t.len(), 10);
        assert_eq!(v.len(), 10);
        assert!((v[1] - Vec3::new(3.0, 4.0, 5.0)).norm() < 1e-12);
    }

    #[test]
    fn a_scalar_channel_is_not_mistaken_for_a_vector() {
        let (sim, _) = pair(2.0, 2.0);
        assert!(vector_channel(&sim, MetricChannel::Altitude).is_none());
    }

    #[test]
    fn row_score_prefers_normalised_error() {
        let row = MetricRow::scalar(
            MetricChannel::Altitude,
            compute_metrics(&PairedSeries::new(
                vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0],
                vec![Some(0.0); 8],
                vec![Some(0.0); 8],
                "alt",
                "m",
            )),
        );
        assert_eq!(row_score(&row), 0.0);
    }

    #[test]
    fn empty_series_cannot_be_compared() {
        let sim = ComparisonSeries::new("run");
        let flight = ComparisonSeries::new("flight");
        let err = compare(&sim, &flight, &ComparisonOptions::default()).unwrap_err();
        assert_eq!(err, AlignmentError::EmptyReference);
    }

    #[test]
    fn event_difference_carries_the_weaker_evidence() {
        let (mut sim, mut flight) = pair(2.0, 2.0);
        sim.events.push(DetectedEvent::new(
            "apogee",
            "Apogee",
            5.0,
            "altitude",
            100.0,
            EventEvidence::Peak,
        ));
        flight.events.push(DetectedEvent::new(
            "apogee",
            "Apogee",
            5.4,
            "accel",
            0.0,
            EventEvidence::Integrated,
        ));
        let comparison = compare(&sim, &flight, &ComparisonOptions::default()).unwrap();
        let d = &comparison.event_differences[0];
        assert_eq!(d.evidence, EventEvidence::Integrated);
        assert!(d.reliability_note().unwrap().contains("uncertain"));
    }
}
