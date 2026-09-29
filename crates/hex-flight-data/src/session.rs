//! The assembled flight session: everything one import produced.
//!
//! A [`FlightSession`] is the artifact the rest of the application opens. It is
//! serialisable to `mapping.json` so a session can be reopened without the
//! original log, and it records how it was built (mapping, calibration, timebase,
//! quality) so the UI can explain every number it shows.
//!
//! # The original file is never modified
//!
//! Import reads the source file and copies what it needs into the session. The
//! file on disk is never opened for writing, never rewritten, and never
//! converted in place; [`SourceInfo::original_preserved`] is always true and
//! exists so a serialised session states that guarantee explicitly.

use serde::{Deserialize, Serialize};

use hex_core::{Frame, Real, Timebase};

use crate::channel::{Channel, ChannelRole};
use crate::derived::DerivedChannel;
use crate::error::FlightDataError;
use crate::validation::{QualityReport, TimestampValidation};

/// The format a session was imported from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    /// Delimited text.
    Csv,
    /// The HexaDOF `.hlog` binary log.
    HexaDofBinary,
    /// A raw telemetry transport capture.
    TelemetryLog,
    /// The extension did not select a codec.
    Unknown,
}

impl SourceFormat {
    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Csv => "CSV",
            Self::HexaDofBinary => "HexaDOF binary log",
            Self::TelemetryLog => "Telemetry log",
            Self::Unknown => "Unknown",
        }
    }

    /// Choose a codec from a file extension, without the leading dot.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.trim().to_ascii_lowercase().as_str() {
            "csv" | "txt" | "tsv" => Some(Self::Csv),
            "hlog" | "hdl" => Some(Self::HexaDofBinary),
            "tlog" | "ulog" | "bin" => Some(Self::TelemetryLog),
            _ => None,
        }
    }
}

/// Where a session came from, and proof that the source is untouched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceInfo {
    /// Path as supplied by the caller, which may be relative or a label.
    pub path: String,
    /// Final path component, shown in the session list.
    pub file_name: String,
    /// Codec the session was built with.
    pub format: SourceFormat,
    /// Size of the source in bytes.
    pub byte_length: u64,
    /// CRC-32 of the source bytes, in hexadecimal, used to detect edits.
    pub content_hash: String,
    /// ISO-8601 UTC time at which the import ran.
    pub imported_at: String,
    /// Always true: HexaDOF never writes to the original log.
    #[serde(default = "default_true")]
    pub original_preserved: bool,
}

fn default_true() -> bool {
    true
}

impl SourceInfo {
    /// Describe a source without inspecting it.
    pub fn new(path: impl Into<String>, format: SourceFormat, byte_length: u64) -> Self {
        let path = path.into();
        let file_name = path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(path.as_str())
            .to_string();
        Self {
            path,
            file_name,
            format,
            byte_length,
            content_hash: String::new(),
            imported_at: now_iso8601(),
            original_preserved: true,
        }
    }

    /// Describe a source and hash the bytes it was read from.
    pub fn with_content(mut self, content: &[u8]) -> Self {
        self.content_hash = format!("{:08x}", crc32fast::hash(content));
        self.byte_length = content.len() as u64;
        self
    }
}

/// Something that happened during the flight, placed on the session time axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    /// Machine kind, for example `takeoff` or `user.marker`.
    pub kind: String,
    /// Time on the corrected session axis, in seconds.
    pub time: Real,
    /// Short label shown on the timeline.
    pub label: String,
    /// Longer explanation shown in the event list.
    pub description: String,
}

impl SessionEvent {
    /// Build an event.
    pub fn new(
        kind: impl Into<String>,
        time: Real,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            time,
            label: label.into(),
            description: description.into(),
        }
    }
}

/// The estimator settings a session was configured with, copied for display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstimatorConfigSummary {
    /// Estimator mode, for example `complementary`.
    pub mode: String,
    /// Whether accelerometer correction was active.
    pub correction_enabled: bool,
    /// Accelerometer correction threshold in m/s^2.
    pub accel_correction_threshold: Real,
    /// Note shown next to the settings.
    pub note: String,
}

impl Default for EstimatorConfigSummary {
    fn default() -> Self {
        Self {
            mode: "complementary".to_string(),
            correction_enabled: true,
            accel_correction_threshold: 2.0,
            note: "Attitude propagated from the gyro, corrected toward the accelerometer when the measured acceleration is close to gravity.".to_string(),
        }
    }
}

/// One imported flight.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightSession {
    /// Deterministic identifier derived from the source path and import time.
    pub id: String,
    /// Where the data came from.
    pub source: SourceInfo,
    /// One line per mapping decision the importer made.
    pub mapping_summary: Vec<String>,
    /// One line per calibration applied.
    pub calibration_summary: Vec<String>,
    /// World and body frame the session is expressed in.
    pub frame: Frame,
    /// How the corrected time axis was established.
    pub timebase: Timebase,
    /// Corrected sample times in seconds, starting at zero.
    pub times: Vec<Real>,
    /// Normalised channels, in mapping order.
    pub channels: Vec<Channel>,
    /// Derived series computed during import.
    pub derived: Vec<DerivedChannel>,
    /// Events placed on the time axis.
    pub events: Vec<SessionEvent>,
    /// Data-quality findings.
    pub quality: QualityReport,
    /// Timestamp analysis and warnings.
    pub timestamps: TimestampValidation,
    /// Estimator settings, when the caller supplied them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimator_config: Option<EstimatorConfigSummary>,
}

impl FlightSession {
    /// Assemble a session from an import.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: SourceInfo,
        frame: Frame,
        timebase: Timebase,
        times: Vec<Real>,
        channels: Vec<Channel>,
        quality: QualityReport,
        timestamps: TimestampValidation,
    ) -> Self {
        let id = deterministic_id(&source.path, &source.imported_at);
        Self {
            id,
            source,
            mapping_summary: Vec::new(),
            calibration_summary: Vec::new(),
            frame,
            timebase,
            times,
            channels,
            derived: Vec::new(),
            events: Vec::new(),
            quality,
            timestamps,
            estimator_config: None,
        }
    }

    /// Place an event on the time axis.
    pub fn add_event(
        &mut self,
        kind: impl Into<String>,
        time: Real,
        label: impl Into<String>,
        description: impl Into<String>,
    ) {
        self.events
            .push(SessionEvent::new(kind, time, label, description));
    }

    /// Add a marker the user placed, and keep the events in time order.
    ///
    /// A marker is additive and never touches the recorded source, so it can be
    /// placed on an imported log without changing what the vehicle measured. The
    /// event carries the `user.marker` kind, which is what separates a human
    /// observation from something the detector inferred.
    pub fn add_marker(&mut self, time: Real, label: &str) -> SessionEvent {
        let event = SessionEvent::new(
            "user.marker",
            time,
            label,
            format!(
                "Placed by the user at {:.3} s on the corrected session time axis.",
                time
            ),
        );
        self.events.push(event.clone());
        self.events.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        event
    }

    /// The markers the user placed, in time order.
    pub fn markers(&self) -> Vec<&SessionEvent> {
        self.events
            .iter()
            .filter(|e| e.kind == "user.marker")
            .collect()
    }

    /// The first and last sample time, when the session has any samples.
    pub fn time_span(&self) -> Option<(Real, Real)> {
        match (self.times.first(), self.times.last()) {
            (Some(first), Some(last)) => Some((*first, *last)),
            _ => None,
        }
    }

    /// The channel filling a role, when the mapping produced one.
    pub fn channel(&self, role: ChannelRole) -> Option<&Channel> {
        self.channels.iter().find(|channel| channel.role == role)
    }

    /// A channel by source name, case-insensitively.
    pub fn channel_by_name(&self, name: &str) -> Option<&Channel> {
        self.channels
            .iter()
            .find(|channel| channel.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Number of samples on the time axis.
    pub fn sample_count(&self) -> usize {
        self.times.len()
    }

    /// The corrected time axis in seconds.
    pub fn times(&self) -> &[Real] {
        &self.times
    }

    /// Length of the recording in seconds, zero for a single sample.
    pub fn duration(&self) -> Real {
        match (self.times.first(), self.times.last()) {
            (Some(first), Some(last)) if self.times.len() > 1 => last - first,
            _ => 0.0,
        }
    }

    /// Effective sampling rate in hertz, zero when it cannot be established.
    pub fn sample_rate_hz(&self) -> Real {
        if self.timestamps.analysis.effective_rate_hz > 0.0 {
            return self.timestamps.analysis.effective_rate_hz;
        }
        self.timebase.nominal_rate_hz().unwrap_or(0.0)
    }

    /// Index of the sample closest to `time`.
    ///
    /// Returns `None` only when the session has no samples.
    pub fn nearest_index(&self, time: Real) -> Option<usize> {
        if self.times.is_empty() {
            return None;
        }
        if !time.is_finite() {
            return Some(0);
        }
        match self.times.binary_search_by(|probe| {
            probe
                .partial_cmp(&time)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(index) => Some(index),
            Err(insert) => {
                if insert == 0 {
                    Some(0)
                } else if insert >= self.times.len() {
                    Some(self.times.len() - 1)
                } else {
                    let before = self.times[insert - 1];
                    let after = self.times[insert];
                    if (time - before).abs() <= (after - time).abs() {
                        Some(insert - 1)
                    } else {
                        Some(insert)
                    }
                }
            }
        }
    }

    /// Serialise the session to the metadata artifact written next to the log.
    pub fn to_json(&self) -> Result<String, FlightDataError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Rebuild a session from its metadata artifact.
    pub fn from_json(text: &str) -> Result<Self, FlightDataError> {
        Ok(serde_json::from_str(text)?)
    }
}

/// Build a stable, UUID-shaped identifier from the source path and import time.
///
/// The identifier must be reproducible: reopening the same file with the same
/// import timestamp has to produce the same session id, so the UI can cache
/// analysis results against it.
pub fn deterministic_id(source_path: &str, imported_at: &str) -> String {
    let mut first = crc32fast::Hasher::new();
    first.update(source_path.as_bytes());
    first.update(b"|");
    first.update(imported_at.as_bytes());
    let a = first.finalize();

    let mut second = crc32fast::Hasher::new_with_initial(a);
    second.update(b"hexadof-session");
    let b = second.finalize();

    let mut third = crc32fast::Hasher::new_with_initial(b);
    third.update(source_path.as_bytes());
    let c = third.finalize();

    let mut fourth = crc32fast::Hasher::new_with_initial(c);
    fourth.update(imported_at.as_bytes());
    let d = fourth.finalize();

    format!(
        "{a:08x}-{:04x}-4{:03x}-{:04x}-{d:08x}{:04x}",
        (b >> 16) & 0xffff,
        b & 0x0fff,
        ((c >> 16) & 0x3fff) | 0x8000,
        c & 0xffff
    )
}

/// Civil date from a count of days since 1970-01-01, by Howard Hinnant's method.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (year + i64::from(month <= 2), month, day)
}

/// Format Unix seconds as an ISO-8601 UTC timestamp.
///
/// The crate has no calendar dependency, and this is the only date formatting
/// it needs, so the conversion is written out rather than pulled in.
pub fn format_unix_seconds(unix_seconds: i64) -> String {
    let days = unix_seconds.div_euclid(86_400);
    let seconds_of_day = unix_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// The current time as an ISO-8601 UTC timestamp.
pub fn now_iso8601() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_unix_seconds(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelRole;
    use crate::validation::{
        assess_quality, validate_timestamps, QualityOptions, TimestampValidationOptions,
    };

    fn sample_session() -> FlightSession {
        let times: Vec<Real> = (0..100).map(|i| i as Real * 0.01).collect();
        let channels = vec![
            Channel::new("time", ChannelRole::Time, times.clone()),
            Channel::new(
                "ax",
                ChannelRole::AccelX,
                (0..100).map(|i| i as Real * 0.1).collect(),
            ),
            Channel::new("gyro_x", ChannelRole::GyroX, vec![0.01; 100]),
        ];
        let timestamps = validate_timestamps(&times, &TimestampValidationOptions::default());
        let quality = assess_quality(&channels, &times, &QualityOptions::default());
        FlightSession::new(
            SourceInfo::new("logs/flight-01.csv", SourceFormat::Csv, 2048),
            Frame::default(),
            timestamps.timebase.clone(),
            times,
            channels,
            quality,
            timestamps,
        )
    }

    #[test]
    fn channel_lookup_is_by_role_and_name() {
        let session = sample_session();
        assert_eq!(session.channel(ChannelRole::AccelX).unwrap().name, "ax");
        assert!(session.channel(ChannelRole::BaroPressure).is_none());
        assert_eq!(
            session.channel_by_name("GYRO_X").unwrap().role,
            ChannelRole::GyroX
        );
        assert!(session.channel_by_name("missing").is_none());
    }

    #[test]
    fn duration_and_sample_rate_describe_the_axis() {
        let session = sample_session();
        assert_eq!(session.sample_count(), 100);
        assert!((session.duration() - 0.99).abs() < 1e-9);
        assert!((session.sample_rate_hz() - 100.0).abs() < 1e-6);
        assert_eq!(session.times().len(), 100);
    }

    #[test]
    fn a_user_marker_is_kept_in_time_order_and_never_looks_detected() {
        let mut session = sample_session();
        assert!(session.markers().is_empty());
        assert_eq!(session.time_span().map(|(a, _)| a), Some(0.0));

        session.add_marker(0.75, "Camera start");
        session.add_marker(0.20, "Rail exit");
        let markers = session.markers();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].label, "Rail exit");
        assert_eq!(markers[1].label, "Camera start");
        assert_eq!(markers[0].kind, "user.marker");
        assert!(markers[0].description.contains("Placed by the user"));

        // The marker is an event on the session axis, not a channel change: the
        // recorded samples are untouched.
        assert_eq!(session.sample_count(), 100);
        let span = session.time_span().expect("a time span");
        assert!((span.1 - 0.99).abs() < 1e-9);
    }

    #[test]
    fn nearest_index_snaps_to_the_closest_sample() {
        let session = sample_session();
        assert_eq!(session.nearest_index(0.0), Some(0));
        assert_eq!(session.nearest_index(0.014), Some(1));
        assert_eq!(session.nearest_index(0.016), Some(2));
        assert_eq!(session.nearest_index(-5.0), Some(0));
        assert_eq!(session.nearest_index(99.0), Some(99));
        assert_eq!(session.nearest_index(Real::NAN), Some(0));

        let mut empty = sample_session();
        empty.times.clear();
        assert_eq!(empty.nearest_index(1.0), None);
        assert_eq!(empty.duration(), 0.0);
    }

    #[test]
    fn events_are_appended_in_order() {
        let mut session = sample_session();
        session.add_event("takeoff", 0.5, "Takeoff", "Throttle up detected");
        session.add_event("user.marker", 2.0, "Marker", "Observer note");
        assert_eq!(session.events.len(), 2);
        assert_eq!(session.events[0].kind, "takeoff");
        assert!(session.events[1].time > session.events[0].time);
    }

    #[test]
    fn json_round_trip_preserves_everything() {
        let mut session = sample_session();
        session.add_event("takeoff", 0.5, "Takeoff", "Throttle up");
        session.derived.push(DerivedChannel::new(
            "barometric_altitude",
            ChannelRole::GnssAltitude,
            vec![0.0, 1.0, 2.0],
            crate::derived::Provenance::Estimated,
        ));
        session.estimator_config = Some(EstimatorConfigSummary::default());

        let json = session.to_json().unwrap();
        assert!(json.contains("\"accel_x\""));
        let restored = FlightSession::from_json(&json).unwrap();
        assert_eq!(restored, session);
        assert!(restored.source.original_preserved);
    }

    #[test]
    fn json_parsing_reports_a_serialization_error() {
        let err = FlightSession::from_json("{ not json").unwrap_err();
        assert!(matches!(err, FlightDataError::Serialization { .. }));
        assert!(err.user_message().contains("could not be processed"));
    }

    #[test]
    fn identifiers_are_deterministic_and_path_sensitive() {
        let a = deterministic_id("logs/flight-01.csv", "2024-01-01T00:00:00Z");
        let b = deterministic_id("logs/flight-01.csv", "2024-01-01T00:00:00Z");
        let c = deterministic_id("logs/flight-02.csv", "2024-01-01T00:00:00Z");
        assert_eq!(a, b);
        assert_ne!(a, c);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert_eq!(parts[0].len(), 8);
        assert_eq!(parts[1].len(), 4);
        assert_eq!(parts[2].len(), 4);
        assert!(parts[2].starts_with('4'));
        assert_eq!(parts[3].len(), 4);
        assert_eq!(parts[4].len(), 12);
    }

    #[test]
    fn source_format_follows_the_extension() {
        assert_eq!(SourceFormat::from_extension("CSV"), Some(SourceFormat::Csv));
        assert_eq!(
            SourceFormat::from_extension("hlog"),
            Some(SourceFormat::HexaDofBinary)
        );
        assert_eq!(
            SourceFormat::from_extension("tlog"),
            Some(SourceFormat::TelemetryLog)
        );
        assert_eq!(SourceFormat::from_extension("exe"), None);
        assert_eq!(SourceFormat::Csv.label(), "CSV");
    }

    #[test]
    fn source_info_records_name_hash_and_preservation() {
        let info = SourceInfo::new("logs/flight-01.csv", SourceFormat::Csv, 0)
            .with_content(b"time,ax\n0,1\n");
        assert_eq!(info.file_name, "flight-01.csv");
        assert_eq!(info.byte_length, 12);
        assert_eq!(info.content_hash.len(), 8);
        assert!(info.original_preserved);
        assert!(!info.imported_at.is_empty());
    }

    #[test]
    fn timestamps_format_as_iso_utc() {
        assert_eq!(format_unix_seconds(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_seconds(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(format_unix_seconds(-1), "1969-12-31T23:59:59Z");
        assert!(now_iso8601().ends_with('Z'));
    }
}
