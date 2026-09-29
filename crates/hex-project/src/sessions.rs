//! Telemetry session and flight session persistence.
//!
//! # Telemetry sessions
//!
//! ```text
//! telemetry/
//!   device-profile.json
//!   session-001.hlog
//!   session-001.json     metadata: profile reference, calibration, integrity
//! ```
//!
//! # Flight logs
//!
//! ```text
//! flights/flight-001/
//!   source.csv | source.hlog   the imported file, copied byte for byte
//!   mapping.json               channel mapping and calibration used
//!   validation.json            timestamp and data-quality findings
//!   session.json               the normalised session metadata
//!   derived.hlog               derived channels, in the log format
//!   events.json                user markers, when any were placed
//! ```
//!
//! The original file is copied, never modified, and its hash is recorded so the
//! imported session can prove which bytes it came from.

use std::path::{Path, PathBuf};

use hex_core::Real;
use hex_flight_data::{BinaryReader, BinaryWriter, ChannelDescriptor, ChannelRole, LogHeader};
use hex_telemetry::{CalibrationSet, DeviceProfile};

use crate::error::ProjectError;
use crate::io;
use crate::project::{dirs, Project};

/// The reusable device profile file name.
pub const DEVICE_PROFILE_FILE: &str = "device-profile.json";
/// The flight log mapping artifact name.
pub const MAPPING_FILE: &str = "mapping.json";
/// The flight log validation artifact name.
pub const VALIDATION_FILE: &str = "validation.json";
/// The flight session metadata artifact name.
pub const SESSION_FILE: &str = "session.json";
/// The derived channel artifact name inside a flight directory.
pub const DERIVED_FILE: &str = "derived.hlog";
/// User markers placed on a flight session.
pub const MARKERS_FILE: &str = "events.json";
/// The schema version of a flight session artifact set.
pub const FLIGHT_SCHEMA_VERSION: &str = "1.0";

/// Metadata stored beside a recorded telemetry session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TelemetrySessionMetadata {
    /// Schema version.
    pub schema_version: String,
    /// Directory-relative session name, for example `session-001`.
    pub name: String,
    /// Display label.
    pub label: String,
    /// Device profile identifier used to record.
    pub profile_id: String,
    /// Device profile name, so a session is readable without the profile file.
    pub profile_name: String,
    /// Serial port used.
    pub port: String,
    /// Baud rate used.
    pub baud_rate: u32,
    /// Packet format label.
    pub packet_format: String,
    /// ISO 8601 start time.
    pub started_at: String,
    /// Duration in seconds.
    pub duration_seconds: Real,
    /// Samples recorded.
    pub sample_count: u64,
    /// Observed average packet rate, hertz.
    pub average_rate_hz: Real,
    /// Packets rejected during recording.
    pub packets_rejected: u64,
    /// Bytes received during recording.
    pub bytes_received: u64,
    /// Whether the display was downsampled while the recording stayed full rate.
    pub display_downsampled: bool,
    /// Names of the channels stored in the log.
    pub channels: Vec<String>,
    /// The calibration applied, as a human-readable summary.
    pub calibration: Vec<String>,
    /// Warnings recorded during the session.
    pub warnings: Vec<String>,
}

impl TelemetrySessionMetadata {
    /// A summary line for the session list.
    pub fn label_with_counts(&self) -> String {
        format!(
            "{} ({} samples, {:.1} s, {:.1} Hz)",
            self.label, self.sample_count, self.duration_seconds, self.average_rate_hz
        )
    }

    /// Whether the recording had any integrity problems worth showing.
    pub fn has_integrity_issues(&self) -> bool {
        self.packets_rejected > 0 || !self.warnings.is_empty()
    }
}

/// A handle to a saved telemetry session.
#[derive(Debug, Clone, PartialEq)]
pub struct TelemetrySessionArtifact {
    /// The log file path.
    pub log_path: PathBuf,
    /// The metadata file path.
    pub metadata_path: PathBuf,
    /// Session name.
    pub name: String,
    /// The stored metadata.
    pub metadata: TelemetrySessionMetadata,
}

impl TelemetrySessionArtifact {
    /// Read the recorded samples back from the log.
    pub fn load_log(&self) -> Result<hex_flight_data::BinaryLog, ProjectError> {
        let bytes =
            std::fs::read(&self.log_path).map_err(|e| ProjectError::io(&self.log_path, e))?;
        BinaryReader::read_all(&bytes).map_err(|e| ProjectError::Parse {
            path: self.log_path.clone(),
            detail: e.to_string(),
        })
    }
}

/// Save a device profile so it can be reused.
///
/// A profile with the same identifier is replaced, because a profile is a
/// configuration rather than a result.
pub fn save_device_profile(
    project: &Project,
    profile: &DeviceProfile,
) -> Result<PathBuf, ProjectError> {
    let path = project.telemetry_dir().join(DEVICE_PROFILE_FILE);
    let existing: Vec<DeviceProfile> = if path.is_file() {
        io::read_json(&path).unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut profiles: Vec<DeviceProfile> = existing
        .into_iter()
        .filter(|p| p.id != profile.id)
        .collect();
    profiles.push(profile.clone());
    profiles.sort_by(|a, b| a.id.cmp(&b.id));
    io::write_json(&path, &profiles)?;
    Ok(path)
}

/// Load every stored device profile.
pub fn load_device_profiles(project: &Project) -> Vec<DeviceProfile> {
    let path = project.telemetry_dir().join(DEVICE_PROFILE_FILE);
    if !path.is_file() {
        return Vec::new();
    }
    io::read_json::<Vec<DeviceProfile>>(&path).unwrap_or_default()
}

/// Find a stored device profile by identifier.
pub fn find_device_profile(project: &Project, id: &str) -> Option<DeviceProfile> {
    load_device_profiles(project)
        .into_iter()
        .find(|p| p.id == id)
}

/// Delete a stored device profile.
pub fn delete_device_profile(project: &Project, id: &str) -> Result<bool, ProjectError> {
    let path = project.telemetry_dir().join(DEVICE_PROFILE_FILE);
    if !path.is_file() {
        return Ok(false);
    }
    let profiles: Vec<DeviceProfile> = io::read_json(&path)?;
    let before = profiles.len();
    let remaining: Vec<DeviceProfile> = profiles.into_iter().filter(|p| p.id != id).collect();
    let removed = remaining.len() != before;
    io::write_json(&path, &remaining)?;
    Ok(removed)
}

/// Save a recorded telemetry session.
///
/// `channel_names` and `columns` must be aligned; one column per channel. The log
/// is written with the HexaDOF binary codec, so it is the same format a flight log
/// uses and the same reader serves both.
#[allow(clippy::too_many_arguments)]
pub fn save_telemetry_session(
    project: &Project,
    label: &str,
    metadata: &TelemetrySessionMetadata,
    channel_names: &[String],
    columns: &[Vec<Real>],
    times: &[Real],
) -> Result<TelemetrySessionArtifact, ProjectError> {
    if channel_names.len() != columns.len() {
        return Err(ProjectError::Parse {
            path: project.telemetry_dir(),
            detail: format!(
                "{} channel names were supplied for {} columns",
                channel_names.len(),
                columns.len()
            ),
        });
    }
    let stem = io::slugify(label, "session");
    let log_path = io::unique_file(&project.telemetry_dir(), &stem, "hlog");
    let name = log_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| stem.clone());
    let metadata_path = project.telemetry_dir().join(format!("{}.json", name));

    let descriptors: Vec<ChannelDescriptor> = channel_names
        .iter()
        .map(|n| {
            let truncated: String = n.chars().take(32).collect();
            ChannelDescriptor::new(&truncated, ChannelRole::Raw, 1.0, 0.0)
        })
        .collect();
    let nominal_dt = if times.len() > 1 {
        let span = times[times.len() - 1] - times[0];
        if span > 0.0 {
            span / (times.len() - 1) as Real
        } else {
            0.0
        }
    } else {
        0.0
    };
    let header = LogHeader::new(nominal_dt)
        .with_device_id(&metadata.profile_name)
        .with_created(crate::clock::unix_seconds());
    let mut writer = BinaryWriter::new(header, descriptors);
    let mut row = vec![0.0; columns.len()];
    for (i, time) in times.iter().enumerate() {
        for (c, column) in columns.iter().enumerate() {
            row[c] = column.get(i).copied().unwrap_or(Real::NAN);
        }
        writer
            .push_sample(*time, &row)
            .map_err(|e| ProjectError::Parse {
                path: log_path.clone(),
                detail: e.to_string(),
            })?;
    }
    let bytes = writer.finish().map_err(|e| ProjectError::Parse {
        path: log_path.clone(),
        detail: e.to_string(),
    })?;
    io::write_atomic(&log_path, &bytes)?;

    let mut stored = metadata.clone();
    stored.name = name.clone();
    io::write_json(&metadata_path, &stored)?;

    Ok(TelemetrySessionArtifact {
        log_path,
        metadata_path,
        name,
        metadata: stored,
    })
}

/// List recorded telemetry sessions.
pub fn list_telemetry_sessions(project: &Project) -> Vec<TelemetrySessionArtifact> {
    let mut sessions = Vec::new();
    for file in io::list_files(&project.telemetry_dir()) {
        if !file.ends_with(".json") || file == DEVICE_PROFILE_FILE {
            continue;
        }
        let metadata_path = project.telemetry_dir().join(&file);
        let Ok(metadata) = io::read_json::<TelemetrySessionMetadata>(&metadata_path) else {
            continue;
        };
        let log_path = project
            .telemetry_dir()
            .join(format!("{}.hlog", metadata.name));
        sessions.push(TelemetrySessionArtifact {
            log_path,
            metadata_path,
            name: metadata.name.clone(),
            metadata,
        });
    }
    sessions.sort_by(|a, b| a.name.cmp(&b.name));
    sessions
}

/// Delete a telemetry session and its metadata.
pub fn delete_telemetry_session(project: &Project, name: &str) -> Result<(), ProjectError> {
    let metadata_path = project.telemetry_dir().join(format!("{}.json", name));
    let log_path = project.telemetry_dir().join(format!("{}.hlog", name));
    if !metadata_path.exists() && !log_path.exists() {
        return Err(ProjectError::SessionNotFound {
            name: name.to_string(),
        });
    }
    for path in [&metadata_path, &log_path] {
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| ProjectError::io(path, e))?;
        }
    }
    Ok(())
}

/// Metadata stored for an imported flight log.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FlightSessionMetadata {
    /// Schema version.
    pub schema_version: String,
    /// Session directory name, for example `flight-001`.
    pub name: String,
    /// Display label.
    pub label: String,
    /// The original file name.
    pub source_file: String,
    /// The original file size in bytes.
    pub source_bytes: u64,
    /// A content hash of the original file, so the session can prove its origin.
    pub source_hash: String,
    /// The import format that was detected.
    pub source_format: String,
    /// ISO 8601 import time.
    pub imported_at: String,
    /// Samples in the normalised session.
    pub sample_count: usize,
    /// Duration in seconds.
    pub duration_seconds: Real,
    /// Effective sample rate, hertz.
    pub sample_rate_hz: Real,
    /// Channels present in the normalised session.
    pub channels: Vec<String>,
    /// Derived channels generated.
    pub derived_channels: Vec<String>,
    /// Events detected or added.
    pub events: Vec<String>,
    /// Quality flags raised, as one-line summaries.
    pub quality_flags: Vec<String>,
    /// Whether the original file was copied into the project unchanged.
    pub original_preserved: bool,
}

impl FlightSessionMetadata {
    /// A summary line for the flight list.
    pub fn label_with_counts(&self) -> String {
        format!(
            "{} ({} samples, {:.2} s, {:.1} Hz)",
            self.label, self.sample_count, self.duration_seconds, self.sample_rate_hz
        )
    }

    /// Whether the import found any quality problem.
    pub fn has_quality_flags(&self) -> bool {
        !self.quality_flags.is_empty()
    }
}

/// A handle to an imported flight session.
#[derive(Debug, Clone, PartialEq)]
pub struct FlightSessionArtifact {
    /// The session directory.
    pub path: PathBuf,
    /// Directory name.
    pub name: String,
    /// The stored metadata.
    pub metadata: FlightSessionMetadata,
}

impl FlightSessionArtifact {
    /// The original source file path.
    pub fn source_path(&self) -> PathBuf {
        self.path.join(&self.metadata.source_file)
    }

    /// The mapping artifact path.
    pub fn mapping_path(&self) -> PathBuf {
        self.path.join(MAPPING_FILE)
    }

    /// The validation artifact path.
    pub fn validation_path(&self) -> PathBuf {
        self.path.join(VALIDATION_FILE)
    }

    /// The session metadata artifact path.
    pub fn session_path(&self) -> PathBuf {
        self.path.join(SESSION_FILE)
    }

    /// The derived channel artifact path.
    pub fn derived_path(&self) -> PathBuf {
        self.path.join(DERIVED_FILE)
    }

    /// The user marker artifact path.
    pub fn events_path(&self) -> PathBuf {
        self.path.join(MARKERS_FILE)
    }

    /// The user markers placed on this session, empty when none were.
    pub fn load_events(&self) -> Vec<hex_flight_data::SessionEvent> {
        io::read_json(&self.events_path()).unwrap_or_default()
    }

    /// Add a user marker to a saved session.
    ///
    /// Markers are additive, so they can be placed on an imported log without
    /// touching the copy of the source or any derived channel.
    pub fn add_marker(
        &self,
        time: Real,
        label: &str,
    ) -> Result<Vec<hex_flight_data::SessionEvent>, ProjectError> {
        let mut events = self.load_events();
        events.push(hex_flight_data::SessionEvent::new(
            "user.marker",
            time,
            label,
            format!("Placed by the user at {:.3} s.", time),
        ));
        events.sort_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        io::write_json(&self.events_path(), &events)?;
        Ok(events)
    }

    /// Whether the original source file is still present and matches its hash.
    pub fn source_is_intact(&self) -> bool {
        let path = self.source_path();
        if !path.is_file() {
            return false;
        }
        crate::clock::file_hash(&path)
            .map(|h| h == self.metadata.source_hash)
            .unwrap_or(false)
    }

    /// Load the import stage report, when one was stored.
    pub fn load_validation(&self) -> Option<serde_json::Value> {
        io::read_json(&self.validation_path()).ok()
    }
}

/// Import a flight log file into the project.
///
/// The source is copied byte for byte into the session directory, and its hash is
/// recorded. Nothing writes to the original path.
#[allow(clippy::too_many_arguments)]
pub fn import_flight_source(
    project: &Project,
    label: &str,
    source: &Path,
    metadata: &mut FlightSessionMetadata,
    mapping: &serde_json::Value,
    validation: &serde_json::Value,
    derived_channel_names: &[String],
    derived_columns: &[Vec<Real>],
    derived_times: &[Real],
) -> Result<FlightSessionArtifact, ProjectError> {
    if !source.is_file() {
        return Err(ProjectError::NotFound {
            path: source.to_path_buf(),
        });
    }
    let stem = io::slugify(label, "flight");
    let directory = io::unique_directory(&project.flights_dir(), &stem);
    std::fs::create_dir_all(&directory).map_err(|e| ProjectError::io(&directory, e))?;

    let file_name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "source.dat".to_string());
    let safe_name = format!("source.{}", extension_of(&file_name));
    let bytes = std::fs::read(source).map_err(|e| ProjectError::io(source, e))?;
    let destination = directory.join(&safe_name);
    io::write_atomic(&destination, &bytes)?;

    metadata.schema_version = FLIGHT_SCHEMA_VERSION.to_string();
    metadata.name = directory
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| stem.clone());
    metadata.source_file = safe_name;
    metadata.source_bytes = bytes.len() as u64;
    metadata.source_hash = crate::clock::content_hash(&bytes);
    metadata.original_preserved = true;
    if metadata.imported_at.is_empty() {
        metadata.imported_at = crate::clock::now_iso8601();
    }

    io::write_json(&directory.join(SESSION_FILE), metadata)?;
    io::write_json(&directory.join(MAPPING_FILE), mapping)?;
    io::write_json(&directory.join(VALIDATION_FILE), validation)?;

    if !derived_channel_names.is_empty() && derived_channel_names.len() == derived_columns.len() {
        write_derived(
            &directory.join(DERIVED_FILE),
            derived_channel_names,
            derived_columns,
            derived_times,
        )?;
    }

    Ok(FlightSessionArtifact {
        path: directory,
        name: metadata.name.clone(),
        metadata: metadata.clone(),
    })
}

fn write_derived(
    path: &Path,
    names: &[String],
    columns: &[Vec<Real>],
    times: &[Real],
) -> Result<(), ProjectError> {
    let descriptors: Vec<ChannelDescriptor> = names
        .iter()
        .map(|n| {
            let truncated: String = n.chars().take(32).collect();
            ChannelDescriptor::new(&truncated, ChannelRole::Raw, 1.0, 0.0)
        })
        .collect();
    let nominal_dt = if times.len() > 1 {
        let span = times[times.len() - 1] - times[0];
        if span > 0.0 {
            span / (times.len() - 1) as Real
        } else {
            0.0
        }
    } else {
        0.0
    };
    let header = LogHeader::new(nominal_dt)
        .with_device_id("hexadof-derived")
        .with_created(crate::clock::unix_seconds());
    let mut writer = BinaryWriter::new(header, descriptors);
    let mut row = vec![0.0; columns.len()];
    for (i, time) in times.iter().enumerate() {
        for (c, column) in columns.iter().enumerate() {
            row[c] = column.get(i).copied().unwrap_or(Real::NAN);
        }
        writer
            .push_sample(*time, &row)
            .map_err(|e| ProjectError::Parse {
                path: path.to_path_buf(),
                detail: e.to_string(),
            })?;
    }
    let bytes = writer.finish().map_err(|e| ProjectError::Parse {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })?;
    io::write_atomic(path, &bytes)
}

fn extension_of(file_name: &str) -> String {
    Path::new(file_name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 8)
        .unwrap_or_else(|| "dat".to_string())
}

/// List imported flight sessions.
pub fn list_flight_sessions(project: &Project) -> Vec<FlightSessionArtifact> {
    let mut sessions = Vec::new();
    for name in io::list_directories(&project.flights_dir()) {
        let path = project.flights_dir().join(&name);
        let Ok(metadata) = io::read_json::<FlightSessionMetadata>(&path.join(SESSION_FILE)) else {
            continue;
        };
        sessions.push(FlightSessionArtifact {
            path,
            name,
            metadata,
        });
    }
    sessions
}

/// Find a flight session by directory name.
pub fn find_flight_session(
    project: &Project,
    name: &str,
) -> Result<FlightSessionArtifact, ProjectError> {
    let path = project.flights_dir().join(name);
    if !path.is_dir() {
        return Err(ProjectError::FlightNotFound {
            name: name.to_string(),
        });
    }
    let metadata = io::read_json(&path.join(SESSION_FILE))?;
    Ok(FlightSessionArtifact {
        path,
        name: name.to_string(),
        metadata,
    })
}

/// Delete a flight session directory.
pub fn delete_flight_session(project: &Project, name: &str) -> Result<(), ProjectError> {
    let path = project.flights_dir().join(name);
    if !path.exists() {
        return Err(ProjectError::FlightNotFound {
            name: name.to_string(),
        });
    }
    io::remove_directory_within(project.root(), &path)
}

/// Write a comparison artifact into the project.
pub fn save_comparison(
    project: &Project,
    label: &str,
    comparison: &serde_json::Value,
) -> Result<PathBuf, ProjectError> {
    let directory = project.reports_dir().join("comparisons");
    std::fs::create_dir_all(&directory).map_err(|e| ProjectError::io(&directory, e))?;
    let path = io::unique_file(&directory, &io::slugify(label, "comparison"), "json");
    io::write_json(&path, comparison)?;
    Ok(path)
}

/// Write a text report into the project's report directory.
pub fn save_report(project: &Project, label: &str, body: &str) -> Result<PathBuf, ProjectError> {
    let path = io::unique_file(&project.reports_dir(), &io::slugify(label, "report"), "md");
    io::write_atomic(&path, body.as_bytes())?;
    Ok(path)
}

/// Store the calibration used for a session, so a session can be reinterpreted.
pub fn save_calibration(
    project: &Project,
    name: &str,
    calibration: &CalibrationSet,
) -> Result<PathBuf, ProjectError> {
    let directory = project.telemetry_dir().join("calibrations");
    std::fs::create_dir_all(&directory).map_err(|e| ProjectError::io(&directory, e))?;
    let path = directory.join(format!("{}.json", io::safe_file_name(name)?));
    io::write_json(&path, calibration)?;
    Ok(path)
}

/// Load a stored calibration.
pub fn load_calibration(project: &Project, name: &str) -> Result<CalibrationSet, ProjectError> {
    let path = project
        .telemetry_dir()
        .join("calibrations")
        .join(format!("{}.json", io::safe_file_name(name)?));
    io::read_json(&path)
}

/// A one-line description of the session layout, for the docs panel.
pub fn layout_description() -> String {
    format!(
        "{}/: {}, {{name}}.hlog, {{name}}.json | {}/{{name}}/: {}, {}, {}, {}, {}",
        dirs::TELEMETRY,
        DEVICE_PROFILE_FILE,
        dirs::FLIGHTS,
        SESSION_FILE,
        MAPPING_FILE,
        VALIDATION_FILE,
        DERIVED_FILE,
        "source"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::Vec3;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "hexadof-sessions-{}-{}-{}",
                label,
                std::process::id(),
                crate::clock::new_id()
            ));
            std::fs::create_dir_all(&base).expect("temp dir");
            Self(base)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn project(temp: &TempDir) -> Project {
        Project::create(temp.path().join("project"), "Sessions").unwrap()
    }

    fn session_metadata(label: &str) -> TelemetrySessionMetadata {
        TelemetrySessionMetadata {
            schema_version: "1.0".to_string(),
            name: String::new(),
            label: label.to_string(),
            profile_id: "profile.example.csv".to_string(),
            profile_name: "Example CSV device".to_string(),
            port: "COM4".to_string(),
            baud_rate: 115_200,
            packet_format: "CSV line".to_string(),
            started_at: crate::clock::now_iso8601(),
            duration_seconds: 2.0,
            sample_count: 200,
            average_rate_hz: 100.0,
            packets_rejected: 0,
            bytes_received: 8000,
            display_downsampled: false,
            channels: vec!["time".to_string(), "accel_x".to_string()],
            calibration: vec!["Gyroscope: not established".to_string()],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn device_profiles_round_trip_and_replace_by_id() {
        let temp = TempDir::new("profiles");
        let project = project(&temp);
        let mut profile = DeviceProfile::example_csv_profile();
        save_device_profile(&project, &profile).unwrap();
        assert_eq!(load_device_profiles(&project).len(), 1);

        profile.name = "Renamed".to_string();
        save_device_profile(&project, &profile).unwrap();
        let profiles = load_device_profiles(&project);
        assert_eq!(profiles.len(), 1, "the same id must replace, not duplicate");
        assert_eq!(profiles[0].name, "Renamed");
        assert!(find_device_profile(&project, "profile.example.csv").is_some());
        assert!(find_device_profile(&project, "nope").is_none());
    }

    #[test]
    fn a_second_profile_is_added() {
        let temp = TempDir::new("profiles2");
        let project = project(&temp);
        save_device_profile(&project, &DeviceProfile::example_csv_profile()).unwrap();
        save_device_profile(&project, &DeviceProfile::example_binary_profile(11)).unwrap();
        assert_eq!(load_device_profiles(&project).len(), 2);
    }

    #[test]
    fn deleting_a_device_profile_reports_whether_it_existed() {
        let temp = TempDir::new("profiledelete");
        let project = project(&temp);
        assert!(!delete_device_profile(&project, "x").unwrap());
        save_device_profile(&project, &DeviceProfile::example_csv_profile()).unwrap();
        assert!(delete_device_profile(&project, "profile.example.csv").unwrap());
        assert!(load_device_profiles(&project).is_empty());
    }

    #[test]
    fn telemetry_session_round_trips_through_the_log() {
        let temp = TempDir::new("telemetry");
        let project = project(&temp);
        let times: Vec<Real> = (0..200).map(|i| i as Real * 0.01).collect();
        let ax: Vec<Real> = times.iter().map(|t| t * 2.0).collect();
        let gz: Vec<Real> = times.iter().map(|_| 0.25).collect();
        let metadata = session_metadata("Bench run");
        let names = vec![
            "time".to_string(),
            "accel_x".to_string(),
            "gyro_z".to_string(),
        ];
        let columns = vec![times.clone(), ax.clone(), gz.clone()];

        let session =
            save_telemetry_session(&project, "Bench run", &metadata, &names, &columns, &times)
                .unwrap();
        assert_eq!(session.name, "bench-run");
        assert!(session.log_path.is_file());
        assert!(session.metadata_path.is_file());
        assert_eq!(session.metadata.name, "bench-run");

        let log = session.load_log().unwrap();
        assert_eq!(log.times.len(), 200);
        assert_eq!(log.channels.len(), 3);
        for (i, t) in times.iter().enumerate() {
            assert!((log.times[i] - t).abs() < 1e-12);
            assert!((log.columns[1][i] - ax[i]).abs() < 1e-12);
            assert!((log.columns[2][i] - 0.25).abs() < 1e-12);
        }
    }

    #[test]
    fn repeated_telemetry_sessions_never_overwrite() {
        let temp = TempDir::new("telemetry2");
        let project = project(&temp);
        let times: Vec<Real> = (0..10).map(|i| i as Real * 0.1).collect();
        let names = vec!["time".to_string()];
        let columns = vec![times.clone()];
        let a = save_telemetry_session(
            &project,
            "Run",
            &session_metadata("Run"),
            &names,
            &columns,
            &times,
        )
        .unwrap();
        let b = save_telemetry_session(
            &project,
            "Run",
            &session_metadata("Run"),
            &names,
            &columns,
            &times,
        )
        .unwrap();
        assert_ne!(a.name, b.name);
        assert_eq!(list_telemetry_sessions(&project).len(), 2);
    }

    #[test]
    fn mismatched_telemetry_columns_are_rejected() {
        let temp = TempDir::new("telemetrymismatch");
        let project = project(&temp);
        let times = vec![0.0, 1.0];
        let err = save_telemetry_session(
            &project,
            "Bad",
            &session_metadata("Bad"),
            &["a".to_string(), "b".to_string()],
            &[vec![0.0, 1.0]],
            &times,
        )
        .unwrap_err();
        assert!(err.user_message().contains("2 channel names"));
    }

    #[test]
    fn listing_and_deleting_telemetry_sessions() {
        let temp = TempDir::new("telemetrylist");
        let project = project(&temp);
        let times: Vec<Real> = (0..10).map(|i| i as Real * 0.1).collect();
        let session = save_telemetry_session(
            &project,
            "Session",
            &session_metadata("Session"),
            &["time".to_string()],
            std::slice::from_ref(&times),
            &times,
        )
        .unwrap();
        let listed = list_telemetry_sessions(&project);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, session.name);
        assert!(listed[0].metadata.label_with_counts().contains("samples"));

        delete_telemetry_session(&project, &session.name).unwrap();
        assert!(list_telemetry_sessions(&project).is_empty());
        assert!(delete_telemetry_session(&project, "nope").is_err());
    }

    #[test]
    fn session_metadata_integrity_predicate() {
        let mut metadata = session_metadata("x");
        assert!(!metadata.has_integrity_issues());
        metadata.packets_rejected = 3;
        assert!(metadata.has_integrity_issues());
    }

    #[test]
    fn flight_import_preserves_the_original_bytes() {
        let temp = TempDir::new("flight");
        let project = project(&temp);
        let source = temp.path().join("flight.csv");
        let content = b"timestamp,ax,ay,az\n0.0,0.1,0.2,9.81\n0.01,0.1,0.2,9.81\n";
        std::fs::write(&source, content).unwrap();

        let mut metadata = FlightSessionMetadata {
            schema_version: String::new(),
            name: String::new(),
            label: "Flight 001".to_string(),
            source_file: String::new(),
            source_bytes: 0,
            source_hash: String::new(),
            source_format: "Csv".to_string(),
            imported_at: crate::clock::now_iso8601(),
            sample_count: 2,
            duration_seconds: 0.01,
            sample_rate_hz: 100.0,
            channels: vec!["ax".to_string(), "ay".to_string(), "az".to_string()],
            derived_channels: Vec::new(),
            events: Vec::new(),
            quality_flags: Vec::new(),
            original_preserved: false,
        };
        let artifact = import_flight_source(
            &project,
            "Flight 001",
            &source,
            &mut metadata,
            &serde_json::json!({"delimiter": ","}),
            &serde_json::json!({"stages": []}),
            &[],
            &[],
            &[],
        )
        .unwrap();

        assert_eq!(artifact.name, "flight-001");
        assert_eq!(artifact.metadata.source_file, "source.csv");
        assert_eq!(
            std::fs::read(artifact.source_path()).unwrap(),
            content,
            "the copy must be byte identical"
        );
        assert!(artifact.source_is_intact());
        assert!(artifact.mapping_path().is_file());
        assert!(artifact.validation_path().is_file());
        assert!(artifact.session_path().is_file());
        assert!(!artifact.metadata.has_quality_flags());

        // The original file is untouched.
        assert_eq!(std::fs::read(&source).unwrap(), content);
    }

    #[test]
    fn a_modified_copy_is_detected_by_the_hash() {
        let temp = TempDir::new("flighthash");
        let project = project(&temp);
        let source = temp.path().join("flight.csv");
        std::fs::write(&source, b"a,b\n1,2\n").unwrap();
        let mut metadata = flight_metadata("Hash");
        let artifact = import_flight_source(
            &project,
            "Hash",
            &source,
            &mut metadata,
            &serde_json::json!({}),
            &serde_json::json!({}),
            &[],
            &[],
            &[],
        )
        .unwrap();
        assert!(artifact.source_is_intact());
        std::fs::write(artifact.source_path(), b"a,b\n9,9\n").unwrap();
        assert!(!artifact.source_is_intact());
    }

    #[test]
    fn a_missing_source_file_is_rejected() {
        let temp = TempDir::new("flightmissing");
        let project = project(&temp);
        let mut metadata = flight_metadata("Missing");
        let err = import_flight_source(
            &project,
            "Missing",
            &temp.path().join("nope.csv"),
            &mut metadata,
            &serde_json::json!({}),
            &serde_json::json!({}),
            &[],
            &[],
            &[],
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::NotFound { .. }));
    }

    #[test]
    fn derived_channels_are_written_when_supplied() {
        let temp = TempDir::new("derived");
        let project = project(&temp);
        let source = temp.path().join("flight.csv");
        std::fs::write(&source, b"a\n1\n").unwrap();
        let times: Vec<Real> = (0..50).map(|i| i as Real * 0.02).collect();
        let altitude: Vec<Real> = times.iter().map(|t| t * 100.0).collect();
        let mut metadata = flight_metadata("Derived");
        let artifact = import_flight_source(
            &project,
            "Derived",
            &source,
            &mut metadata,
            &serde_json::json!({}),
            &serde_json::json!({}),
            &["pressure_altitude".to_string()],
            std::slice::from_ref(&altitude),
            &times,
        )
        .unwrap();
        assert!(artifact.derived_path().is_file());

        let bytes = std::fs::read(artifact.derived_path()).unwrap();
        let log = BinaryReader::read_all(&bytes).unwrap();
        assert_eq!(log.channels.len(), 1);
        assert_eq!(log.times.len(), 50);
        assert!((log.columns[0][10] - altitude[10]).abs() < 1e-12);
    }

    #[test]
    fn user_markers_are_added_to_a_saved_flight_session() {
        let temp = TempDir::new("flightmarkers");
        let project = project(&temp);
        let source = temp.path().join("flight.csv");
        std::fs::write(&source, b"a\n1\n").unwrap();
        let mut metadata = flight_metadata("Marked");
        let artifact = import_flight_source(
            &project,
            "Marked",
            &source,
            &mut metadata,
            &serde_json::json!({}),
            &serde_json::json!({}),
            &[],
            &[],
            &[],
        )
        .unwrap();

        // No markers were placed at import, so there is nothing to read back.
        assert!(artifact.load_events().is_empty());
        assert!(!artifact.events_path().exists());

        let events = artifact.add_marker(1.25, "Camera start").unwrap();
        assert_eq!(events.len(), 1);
        artifact.add_marker(0.5, "Rail exit").unwrap();
        let stored = artifact.load_events();
        assert_eq!(stored.len(), 2);
        assert_eq!(stored[0].label, "Rail exit");
        assert_eq!(stored[0].kind, "user.marker");

        // A second artifact handle reads the same file, and marking never
        // touches the copy of the source log.
        let reopened = find_flight_session(&project, &artifact.name).unwrap();
        assert_eq!(reopened.load_events().len(), 2);
        assert!(reopened.source_is_intact());
        assert_eq!(
            std::fs::read(artifact.source_path()).unwrap(),
            b"a\n1\n".to_vec()
        );
    }

    #[test]
    fn listing_finding_and_deleting_flight_sessions() {
        let temp = TempDir::new("flightlist");
        let project = project(&temp);
        let source = temp.path().join("flight.csv");
        std::fs::write(&source, b"a\n1\n").unwrap();
        let mut metadata = flight_metadata("One");
        let artifact = import_flight_source(
            &project,
            "One",
            &source,
            &mut metadata,
            &serde_json::json!({}),
            &serde_json::json!({}),
            &[],
            &[],
            &[],
        )
        .unwrap();

        let listed = list_flight_sessions(&project);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, artifact.name);
        assert!(listed[0].metadata.label_with_counts().contains("samples"));

        let found = find_flight_session(&project, &artifact.name).unwrap();
        assert_eq!(found.name, artifact.name);
        assert!(find_flight_session(&project, "nope").is_err());

        delete_flight_session(&project, &artifact.name).unwrap();
        assert!(list_flight_sessions(&project).is_empty());
        assert!(delete_flight_session(&project, &artifact.name).is_err());
    }

    #[test]
    fn a_directory_without_session_metadata_is_not_listed() {
        let temp = TempDir::new("flightnodata");
        let project = project(&temp);
        std::fs::create_dir_all(project.flights_dir().join("flight-001")).unwrap();
        assert!(list_flight_sessions(&project).is_empty());
        assert!(project.flights_dir().join("flight-001").is_dir());
    }

    #[test]
    fn reports_and_comparisons_get_unique_names() {
        let temp = TempDir::new("reports");
        let project = project(&temp);
        let a = save_report(&project, "My Report", "# Report\n").unwrap();
        let b = save_report(&project, "My Report", "# Report\n").unwrap();
        assert_ne!(a, b);
        assert!(a.to_string_lossy().ends_with(".md"));
        assert!(a.to_string_lossy().contains("my-report"));

        let c =
            save_comparison(&project, "Run vs Flight", &serde_json::json!({"ok": true})).unwrap();
        assert!(c.to_string_lossy().contains("comparisons"));
        assert!(c.is_file());
    }

    #[test]
    fn calibrations_round_trip() {
        let temp = TempDir::new("calibration");
        let project = project(&temp);
        let calibration = CalibrationSet::gyro_bias_only(Vec3::new(0.01, -0.02, 0.003));
        save_calibration(&project, "bench", &calibration).unwrap();
        let loaded = load_calibration(&project, "bench").unwrap();
        assert!((loaded.gyroscope.offset.x - 0.01).abs() < 1e-15);
        assert!(loaded.is_usable_for_attitude());
        assert!(load_calibration(&project, "missing").is_err());
        // A name that could escape the directory is refused.
        assert!(save_calibration(&project, "../escape", &calibration).is_err());
    }

    #[test]
    fn layout_description_names_every_artifact() {
        let description = layout_description();
        for name in [
            DEVICE_PROFILE_FILE,
            MAPPING_FILE,
            VALIDATION_FILE,
            SESSION_FILE,
            DERIVED_FILE,
        ] {
            assert!(description.contains(name), "{} missing", name);
        }
    }

    #[test]
    fn extension_detection_is_conservative() {
        assert_eq!(extension_of("flight.csv"), "csv");
        assert_eq!(extension_of("LOG.HLOG"), "hlog");
        assert_eq!(extension_of("noextension"), "dat");
        assert_eq!(extension_of("weird.verylongextension"), "dat");
        assert_eq!(extension_of(""), "dat");
    }

    fn flight_metadata(label: &str) -> FlightSessionMetadata {
        FlightSessionMetadata {
            schema_version: String::new(),
            name: String::new(),
            label: label.to_string(),
            source_file: String::new(),
            source_bytes: 0,
            source_hash: String::new(),
            source_format: "Csv".to_string(),
            imported_at: crate::clock::now_iso8601(),
            sample_count: 1,
            duration_seconds: 0.0,
            sample_rate_hz: 0.0,
            channels: Vec::new(),
            derived_channels: Vec::new(),
            events: Vec::new(),
            quality_flags: Vec::new(),
            original_preserved: false,
        }
    }
}
