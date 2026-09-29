//! HexaDOF flight analysis: alignment, error metrics, event detection, and
//! simulation-versus-flight comparison.
//!
//! This crate turns two recorded histories into a defensible comparison. It does
//! not read files, does not render, and does not integrate dynamics: it consumes
//! time series and produces numbers with their assumptions attached.
//!
//! # Principles
//!
//! * Orientation is compared with a rotation distance, never by differencing
//!   Euler angles.
//! * Every metric states its sample population, because an RMSE without one is not
//!   actionable.
//! * A metric is never computed over interpolated or fabricated data. Samples
//!   outside a recorded range are reported as missing and counted.
//! * A mismatch between a simulation and a flight does not identify a single root
//!   cause, and no report from this crate says it does.

pub mod alignment;
pub mod comparison;
pub mod events;
pub mod metrics;

pub use alignment::{
    align, available_methods, resample, resample_vectors, sample_at, time_to_event_difference,
    Alignment, AlignmentError, AlignmentMethod, EventRef, TimeSeries,
};
pub use comparison::{
    attitude_metrics_for, compare, comparison_headline, metrics_for, row_score, suggested_methods,
    AttitudeChannel, Comparison, ComparisonChannel, ComparisonOptions, ComparisonSeries,
    ComparisonWarning, EventTimingDifference, Provenance,
};
pub use events::{
    alignment_anchors, detect_events, infer_apogee_from_acceleration, DetectedEvent,
    EventDetectionOptions, EventEvidence, EventTimeline, FlightSignals, Signal,
};
pub use metrics::{
    compute_attitude_metrics, compute_metrics, compute_vector_metrics, error_quaternion,
    rotation_distance, AttitudeMetrics, ErrorMetrics, MetricChannel, MetricRow, MetricsTable,
    PairedSeries, VectorErrorMetrics,
};

/// The number of alignment methods this crate offers.
pub const ALIGNMENT_METHOD_COUNT: usize = 5;

/// A one-line capability description for the diagnostics bundle.
pub fn capability_summary() -> String {
    format!(
        "Analysis: {} alignment methods, {} comparison channels, rotation-distance attitude metrics",
        ALIGNMENT_METHOD_COUNT,
        MetricChannel::default_set().len()
    )
}

/// Build a time series from parallel arrays, for callers that already hold
/// columns.
pub fn series_from_columns(
    times: Vec<hex_core::Real>,
    values: Option<Vec<hex_core::Real>>,
) -> TimeSeries {
    match values {
        Some(v) => TimeSeries::from_times(times).with_values(v),
        None => TimeSeries::from_times(times),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::Real;

    #[test]
    fn public_api_smoke_test() {
        let times: Vec<Real> = (0..100).map(|i| i as Real * 0.01).collect();
        let values: Vec<Real> = times.iter().map(|t| t * 3.0).collect();
        let a = series_from_columns(times.clone(), Some(values.clone()));
        let b = series_from_columns(times, Some(values));
        let alignment = align(&a, &b, AlignmentMethod::Absolute).unwrap();
        assert!(alignment.is_usable(0.1));
        assert!(!capability_summary().is_empty());
    }

    #[test]
    fn capability_summary_mentions_the_alignment_surface() {
        let s = capability_summary();
        assert!(s.contains("alignment methods"));
        assert!(s.contains("rotation-distance"));
        assert_eq!(ALIGNMENT_METHOD_COUNT, 5);
    }

    #[test]
    fn end_to_end_comparison_of_two_synthetic_flights() {
        // A simulated flight, and a flight of the same shape but 20 percent
        // larger and delayed by a tenth of a second.
        let n = 200;
        let dt = 0.01;
        let sim_times: Vec<Real> = (0..n).map(|i| i as Real * dt).collect();
        let sim_altitude: Vec<Real> = sim_times.iter().map(|t| 200.0 * t - 4.9 * t * t).collect();
        let delay = 0.1;
        let flight_times: Vec<Real> = sim_times.iter().map(|t| t + delay).collect();
        let flight_altitude: Vec<Real> = sim_times
            .iter()
            .map(|t| 1.2 * (200.0 * t - 4.9 * t * t))
            .collect();

        let sim = ComparisonSeries::new("run-001")
            .with_channel(ComparisonChannel {
                channel: MetricChannel::Altitude,
                times: sim_times.clone(),
                values: sim_altitude,
                unit: "m".to_string(),
                provenance: Provenance::Simulated,
                is_integrated: false,
            })
            .with_events(vec![DetectedEvent::new(
                "apogee",
                "Apogee",
                20.4,
                "altitude",
                2040.0,
                EventEvidence::Peak,
            )]);
        let flight = ComparisonSeries::new("flight-001")
            .with_channel(ComparisonChannel {
                channel: MetricChannel::Altitude,
                times: flight_times,
                values: flight_altitude,
                unit: "m".to_string(),
                provenance: Provenance::Measured,
                is_integrated: false,
            })
            .with_events(vec![DetectedEvent::new(
                "apogee",
                "Apogee",
                delay + 20.4,
                "altitude",
                2448.0,
                EventEvidence::Peak,
            )]);

        // Align on apogee, then compare.
        let comparison = compare(&sim, &flight, &ComparisonOptions::on_event("apogee")).unwrap();
        assert!(comparison.metrics.rows.iter().any(|r| r.has_data()));
        assert_eq!(comparison.event_differences.len(), 1);
        // The apogee difference is zero by construction of the alignment.
        // The event difference is the raw timing gap before alignment, which is
        // exactly what makes an event a good alignment anchor.
        assert!((comparison.event_differences[0].difference - delay).abs() < 1e-9);
        let text = comparison.to_text();
        assert!(text.contains("does not identify a single cause"));
    }
}
