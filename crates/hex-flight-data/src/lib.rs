//! Flight-log reading and normalisation for HexaDOF.
//!
//! This crate owns everything between a recorded file and the in-memory session
//! the rest of the application reasons about:
//!
//! * [`csv`] tokenises delimited text, detects the delimiter and the header, and
//!   counts every cell it could not parse instead of silently dropping it.
//! * [`binary`] defines and implements the framed, checksummed `.hlog` format.
//! * [`channel`] is the normalised channel model: raw samples plus the unit
//!   scale, axis sign, and frame needed to turn them into SI body-axis values.
//! * [`validation`] checks the time axis and the data quality of an import.
//! * [`derived`] computes barometric altitude, vector channels, integrated
//!   position and velocity, and gyro-propagated attitude, each labelled with its
//!   provenance.
//! * [`session`] assembles the result, and [`import`] runs the staged pipeline
//!   that produces it.
//!
//! # The original file is never modified
//!
//! Every entry point opens the source for reading only. Nothing here writes to,
//! rewrites, or converts a flight log in place; the session records a hash of the
//! bytes it read so that claim stays checkable. Derived data lives in the
//! session and in separate artifacts such as `mapping.json`.
//!
//! # Every stage reports its own status
//!
//! An import is a sequence of named stages, and each one appends an
//! [`import::StageResult`] with a status, a message, and its duration. A stage
//! that fails stops the pipeline and becomes the report's `primary_error`, so the
//! UI can always show how far the file got and why it stopped.
//!
//! # Conventions
//!
//! * Values are stored raw. [`channel::Channel::corrected_values`] applies the
//!   unit scale and the axis sign to produce SI values.
//! * Timestamps are corrected so the first sample sits at zero seconds, and the
//!   applied offset is recorded in [`hex_core::Timebase::start_offset`].
//! * Quaternions follow the core convention: scalar first, body-to-world,
//!   `q_dot = 0.5 * q * [0, w_body]`.

#![deny(missing_docs)]

pub mod binary;
pub mod channel;
pub mod csv;
pub mod derived;
pub mod error;
pub mod import;
pub mod session;
pub mod validation;

pub use binary::{
    BinaryDecodeError, BinaryLog, BinaryReader, BinaryWriter, ChannelDescriptor,
    ChannelDescriptorDecoded, DecodeIntegrity, LogHeader, LogHeaderDecoded, LogStreamReader,
    StreamRecord, CURRENT_FORMAT_VERSION, DEFAULT_MAX_SAMPLES_PER_RECORD, MAGIC, RECORD_MAGIC,
};
pub use channel::{snap_sign, Channel, ChannelFamily, ChannelId, ChannelRole, ChannelStatistics};
pub use csv::{parse_csv, parse_csv_strict, preview_csv, CsvImportOptions, CsvPreview, RawTable};
pub use derived::{
    derive_barometric_altitude, derive_vector_channels, estimate_gyro_bias, integrate_position,
    integrate_position_values, integrate_velocity, integrate_velocity_values,
    propagate_gyro_attitude, DerivedChannel, DerivedVectorChannel, Provenance,
    INTEGRATION_DRIFT_WARNING,
};
pub use error::FlightDataError;
pub use import::{
    import_binary, import_binary_outcome, import_csv, import_csv_outcome, import_path,
    import_path_outcome, infer_role_from_name, ChannelCalibration, ChannelMapping, ImportOutcome,
    ImportReport, ImportStage, StageResult, StageStatus,
};
pub use session::{
    deterministic_id, format_unix_seconds, now_iso8601, EstimatorConfigSummary, FlightSession,
    SessionEvent, SourceFormat, SourceInfo,
};
pub use validation::{
    assess_quality, sequence_gap_flag, validate_timestamps, DataQualityFlag, QualityOptions,
    QualityReport, RateSegment, TimestampValidation, TimestampValidationOptions,
};
