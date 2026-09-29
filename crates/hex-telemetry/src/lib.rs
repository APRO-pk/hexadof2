//! HexaDOF telemetry: USB serial transport, packet protocols, channel mapping,
//! sensor calibration, lossless recording, and orientation estimation.
//!
//! This crate never renders anything and never touches the project file format.
//! It converts a stream of bytes into timestamped SI samples, records every one
//! of them, and reports how much it trusts each derived value.
//!
//! # Safety default
//!
//! The MVP is read-only. Nothing in this crate writes a command to a device. The
//! only encoder present exists for loopback testing and for generating firmware
//! examples, and is documented as such.
//!
//! # Data flow
//!
//! ```text
//! byte source
//!   -> PacketDecoder    framing and integrity
//!   -> map_packet       units, sign, axis mapping, calibration
//!   -> AttitudeEstimator
//!   -> Recorder         lossless, full rate
//!   -> DisplayStream    throttled, may downsample
//! ```
//!
//! The recorder is ahead of the display stream on purpose. A slow interface must
//! never cost a recorded sample, and when the display is downsampled the UI says
//! so rather than pretending it is showing everything.

pub mod calibration;
pub mod estimation;
pub mod mapping;
pub mod protocol;
pub mod serial;
pub mod transport;

pub use calibration::{
    average, calibrated_destinations, calibration_impact, frame_matches, run_validation,
    CalibrationSet, ChannelCalibration, CheckStatus, PhysicalOrientation, SensorValidation,
    ThreeAxisCalibration, ValidationInput, ValidationStep,
};
pub use estimation::{
    attitude_error_series, estimate_attitudes, estimate_gyro_bias, estimate_series,
    rms_attitude_error, AttitudeEstimator, AttitudeSource, EstimatorHealth, EstimatorInput,
    EstimatorState,
};
pub use mapping::{
    apply_axis_mapping, convert_value, describe_mapping, ChannelDestination, ChannelMapping,
    DeviceProfile, EstimatorMode, EstimatorSettings, MappedValue, MappingProblem, MappingRow,
    SensorFamily,
};
pub use protocol::{
    encode_frame, packets_to_columns, timebase_for, BinaryFrameSpec, ChecksumSpec, DecodeOutcome,
    DecodedPacket, Endianness, FieldEncoding, FrameRejection, PacketDecoder, PacketFormat,
    PayloadField,
};
pub use serial::{
    list_ports, ByteSource, ConnectionHealth, ConnectionState, FlowControl, Parity, PortInfo,
    ScriptedSource, SerialSettings, TransportError, COMMON_BAUD_RATES,
};
pub use transport::{
    map_packet, recorded_time, recorder_columns, ByteInbox, DisplayPolicy, DisplayStream,
    HostClock, IngestionPipeline, PipelineCounters, ProcessedPacket, Recorder, RecorderStatus,
    SessionGuard, TelemetryChannel, TelemetrySample,
};

/// A one-line description of what this crate provides, used in the diagnostics
/// bundle.
pub fn capability_summary() -> String {
    format!(
        "Telemetry: {} baud rates, CSV line and binary framed protocols, {} channel destinations, {} selectable channels",
        COMMON_BAUD_RATES.len(),
        ChannelDestination::all().len(),
        TelemetryChannel::all().len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::Real;

    #[test]
    fn public_api_is_reachable() {
        let profile = DeviceProfile::example_csv_profile();
        assert!(profile.is_usable());
        let pipeline = IngestionPipeline::new(profile, CalibrationSet::default(), 1024);
        assert_eq!(pipeline.counters().packets_decoded, 0);
        assert!(!capability_summary().is_empty());

        let settings = SerialSettings::default();
        assert!(settings.maximum_bytes_per_second() > 0.0);
        let health = ConnectionHealth::default();
        assert_eq!(health.state(0.0, 1.0), ConnectionState::Connected);
    }

    #[test]
    fn capability_summary_counts_the_protocol_surface() {
        let s = capability_summary();
        assert!(s.contains("CSV line"));
        assert!(s.contains("binary framed"));
    }

    #[test]
    fn a_scripted_session_records_and_estimates() {
        use hex_core::STANDARD_GRAVITY;
        let profile = DeviceProfile::example_csv_profile();
        let mut pipeline = IngestionPipeline::new(profile, CalibrationSet::default(), 4096);
        pipeline.start_recording();

        let mut text = String::new();
        for i in 0..200 {
            let t = i as Real * 0.01;
            text.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                t, 0.0, 0.0, -STANDARD_GRAVITY, 0.0, 0.0, 0.1, 101_325.0
            ));
        }
        let packets = pipeline.ingest(text.as_bytes());
        assert_eq!(packets, 200);
        assert_eq!(pipeline.recorder().len(), 200);

        let (names, columns) = recorder_columns(pipeline.recorder(), &[TelemetryChannel::GyroZ]);
        assert_eq!(names.len(), 1);
        assert_eq!(columns[0].len(), 200);
        // A constant 0.1 rad/s over 2 s is about a 0.2 radian rotation.
        let attitude = pipeline.estimator().attitude();
        assert!(attitude.angular_distance(&hex_core::Quaternion::identity()) > 0.1);
    }
}
