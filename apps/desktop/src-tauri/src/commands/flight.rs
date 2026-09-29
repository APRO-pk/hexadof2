//! Flight-log import, validation, session creation, and replay commands.

use hex_analysis::EventTimeline;
use hex_core::TimestampUnit;
use hex_flight_data::{
    assess_quality, import_binary, import_csv, import_path, infer_role_from_name, parse_csv_strict,
    validate_timestamps, Channel, ChannelMapping as FlightChannelMapping, ChannelRole,
    CsvImportOptions, FlightDataError, FlightSession, ImportReport, QualityOptions,
    TimestampValidationOptions,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::commands::analysis::flight_timeline;
use crate::commands::simulation::ReplayData;
use crate::error::{missing, CommandError};
use crate::events;
use crate::state::{emit, emit_notice, AppState};

/// Options for the flight import dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightImportRequest {
    /// Absolute path to the log file.
    pub path: String,
    /// CSV delimiter, as a single character.
    pub delimiter: Option<String>,
    /// Whether the first row is a header. `None` lets the importer decide.
    pub has_header: Option<bool>,
    /// Name of the timestamp column.
    pub timestamp_column: Option<String>,
    /// Timestamp unit: `seconds`, `milliseconds`, `microseconds`, or `ticks`.
    pub timestamp_unit: Option<String>,
    /// Tick rate, when the unit is `ticks`.
    pub tick_rate: Option<f64>,
    /// Comment prefix to ignore.
    pub comment_prefix: Option<String>,
    /// Maximum rows to read.
    pub max_rows: Option<usize>,
    /// Whether decimal commas are used.
    pub decimal_comma: Option<bool>,
    /// Explicit channel mappings, when the user has configured them.
    pub mappings: Vec<FlightMappingRequest>,
    /// Expected sample rate, hertz.
    pub expected_rate_hz: Option<f64>,
    /// Gap factor for the timestamp check.
    pub gap_factor: Option<f64>,
    /// Timestamp counter width in bits, for a wrapping clock.
    pub rollover_bits: Option<u32>,
    /// Accel plausible range in metres per second squared, as `[low, high]`.
    pub accel_range: Option<[f64; 2]>,
    /// Gyro plausible range in radians per second, as `[low, high]`.
    pub gyro_range: Option<[f64; 2]>,
    /// Label for the new flight session.
    pub label: Option<String>,
    /// Whether to copy the source into the project and create a session.
    pub create_session: Option<bool>,
}

/// One channel mapping row supplied by the import dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightMappingRequest {
    /// Field name in the source file.
    pub source_name: String,
    /// Destination role code, for example `accel.x`.
    pub role: String,
    /// Unit as the device reports it.
    pub unit: String,
    /// Sign, which must be `1` or `-1`.
    pub sign: Option<f64>,
    /// Offset subtracted before scaling.
    pub calibration_offset: Option<f64>,
    /// Scale applied after the offset.
    pub calibration_scale: Option<f64>,
}

fn timestamp_unit(request: &FlightImportRequest) -> TimestampUnit {
    match request.timestamp_unit.as_deref() {
        Some("milliseconds") | Some("ms") => TimestampUnit::Milliseconds,
        Some("microseconds") | Some("us") => TimestampUnit::Microseconds,
        Some("ticks") => TimestampUnit::Ticks(request.tick_rate.unwrap_or(1.0)),
        _ => TimestampUnit::Seconds,
    }
}

/// Build the flight-data CSV options this request describes.
pub fn csv_options(request: &FlightImportRequest) -> CsvImportOptions {
    let delimiter = request
        .delimiter
        .as_deref()
        .and_then(|d| d.chars().next())
        .map(|c| c as u8)
        .unwrap_or(b',');
    CsvImportOptions {
        delimiter,
        has_header: request.has_header.unwrap_or(true),
        timestamp_column: request.timestamp_column.clone(),
        timestamp_unit: timestamp_unit(request),
        comment_prefix: request
            .comment_prefix
            .as_deref()
            .and_then(|p| p.chars().next()),
        max_rows: request.max_rows,
        decimal_comma: request.decimal_comma.unwrap_or(false),
    }
}

fn role_from_code(code: &str) -> Option<ChannelRole> {
    use ChannelRole::*;
    Some(match code {
        "time" => Time,
        "accel.x" | "accel_x" => AccelX,
        "accel.y" | "accel_y" => AccelY,
        "accel.z" | "accel_z" => AccelZ,
        "gyro.x" | "gyro_x" => GyroX,
        "gyro.y" | "gyro_y" => GyroY,
        "gyro.z" | "gyro_z" => GyroZ,
        "mag.x" | "mag_x" => MagX,
        "mag.y" | "mag_y" => MagY,
        "mag.z" | "mag_z" => MagZ,
        "baro.pressure" | "barometer" | "pressure" => BaroPressure,
        "baro.temperature" | "temperature" => BaroTemperature,
        "gnss.latitude" | "latitude" => GnssLatitude,
        "gnss.longitude" | "longitude" => GnssLongitude,
        "gnss.altitude" | "altitude" => GnssAltitude,
        "gnss.speed" | "speed" => GnssSpeed,
        "quaternion.w" => QuaternionW,
        "quaternion.x" => QuaternionX,
        "quaternion.y" => QuaternionY,
        "quaternion.z" => QuaternionZ,
        "motor.throttle" | "motor" => MotorThrottle,
        "control.surface" | "control_surface" => ControlSurface,
        "power.voltage" | "voltage" => Voltage,
        "power.current" | "current" => Current,
        "raw" => Raw,
        "ignored" | "unused" => Ignored,
        _ => return None,
    })
}

/// Build explicit mappings from the request, falling back to name inference.
pub fn flight_mappings(request: &FlightImportRequest) -> Vec<FlightChannelMapping> {
    request
        .mappings
        .iter()
        .filter_map(|m| {
            let role = role_from_code(&m.role)?;
            Some(FlightChannelMapping {
                source_name: m.source_name.clone(),
                role,
                unit: m.unit.clone(),
                sign: m.sign.unwrap_or(1.0),
                axis_mapping: hex_core::AxisMapping::identity(),
                calibration: hex_flight_data::ChannelCalibration {
                    offset: m.calibration_offset.unwrap_or(0.0),
                    scale: m.calibration_scale.unwrap_or(1.0),
                    enabled: m.calibration_offset.unwrap_or(0.0) != 0.0
                        || m.calibration_scale.map(|s| s != 1.0).unwrap_or(false),
                },
            })
        })
        .collect()
}

/// A preview of a file before it is imported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightPreview {
    /// Detected delimiter.
    pub delimiter: String,
    /// Whether a header row was detected.
    pub has_header: bool,
    /// Header names, or generated names when there is no header.
    pub headers: Vec<String>,
    /// The role each column is inferred to carry.
    pub inferred_roles: Vec<String>,
    /// Whether the inferred role is a physical channel.
    pub inferred_physical: Vec<bool>,
    /// A few sample rows of the first columns.
    pub sample_rows: Vec<Vec<String>>,
    /// Warnings from the preview pass.
    pub warnings: Vec<String>,
    /// Total rows counted in the preview.
    pub row_count: usize,
    /// Timestamp samples that would be used.
    pub timestamp_preview: Vec<f64>,
}

/// Preview a flight log without creating a session.
#[tauri::command]
pub fn flight_preview(request: FlightImportRequest) -> Result<FlightPreview, CommandError> {
    let options = csv_options(&request);
    let bytes = std::fs::read(&request.path).map_err(|e| {
        CommandError::new(
            "flight.io",
            "The flight log could not be read",
            format!("{} could not be opened: {}", request.path, e),
            "Check that the file exists and that it is readable.",
        )
    })?;
    let text = decode_text(&bytes);
    let preview = hex_flight_data::preview_csv(&text, &options)?;

    let headers = if preview.headers.is_empty() {
        (0..preview.sample_rows.first().map(|r| r.len()).unwrap_or(0))
            .map(|i| format!("column_{}", i + 1))
            .collect()
    } else {
        preview.headers.clone()
    };
    let inferred: Vec<ChannelRole> = headers.iter().map(|h| infer_role_from_name(h)).collect();
    let timestamp_preview: Vec<f64> = preview
        .sample_rows
        .iter()
        .filter_map(|row| row.first())
        .filter_map(|v| v.trim().parse::<f64>().ok())
        .collect();

    Ok(FlightPreview {
        delimiter: (preview.delimiter as char).to_string(),
        has_header: preview.detected_has_header,
        inferred_roles: inferred.iter().map(|r| r.code().to_string()).collect(),
        inferred_physical: inferred
            .iter()
            .map(|r| !matches!(r, ChannelRole::Raw | ChannelRole::Ignored))
            .collect(),
        sample_rows: preview.sample_rows,
        warnings: preview.warnings,
        row_count: preview.row_count_preview,
        headers,
        timestamp_preview,
    })
}

/// Decode a file as UTF-8, falling back to a lossy decode so a stray byte does not
/// block an otherwise readable log.
fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_string(),
        Err(_) => String::from_utf8_lossy(bytes).to_string(),
    }
}

/// A validation summary for a timestamp column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimestampReport {
    /// Samples examined.
    pub sample_count: usize,
    /// Median interval in seconds.
    pub median_dt_seconds: f64,
    /// Effective sample rate, hertz.
    pub effective_rate_hz: f64,
    /// Whether time is strictly increasing.
    pub strictly_increasing: bool,
    /// Indices where time did not increase.
    pub non_monotonic_indices: Vec<usize>,
    /// Number of duplicate timestamps.
    pub duplicate_count: usize,
    /// Number of negative time steps.
    pub reversal_count: usize,
    /// Gaps found: start time, duration, and estimated missing samples.
    pub gaps: Vec<(f64, f64, usize)>,
    /// Detected sampling-rate segments.
    pub rate_segments: Vec<(f64, f64)>,
    /// Whether a wrap was corrected.
    pub rollover_corrected: bool,
    /// Warnings raised.
    pub warnings: Vec<String>,
}

/// Validate the timestamps of a file without importing it.
#[tauri::command]
pub fn flight_validate(request: FlightImportRequest) -> Result<TimestampReport, CommandError> {
    let options = csv_options(&request);
    let bytes = std::fs::read(&request.path).map_err(|e| {
        CommandError::new(
            "flight.io",
            "The flight log could not be read",
            format!("{} could not be opened: {}", request.path, e),
            "Check that the file exists and that it is readable.",
        )
    })?;
    let text = decode_text(&bytes);
    let table = parse_csv_strict(&text, &options)?;

    let timestamp_index = options
        .timestamp_column
        .as_ref()
        .and_then(|name| table.headers.iter().position(|h| h == name))
        .unwrap_or(0);
    let raw_times: Vec<f64> = table
        .columns
        .get(timestamp_index)
        .cloned()
        .unwrap_or_default();

    let validation_options = TimestampValidationOptions {
        gap_factor: request.gap_factor.unwrap_or(3.0),
        expected_rate_hz: request.expected_rate_hz,
        rollover_bits: request.rollover_bits,
        ..TimestampValidationOptions::default()
    };
    let validation = validate_timestamps(&raw_times, &validation_options);

    Ok(TimestampReport {
        sample_count: validation.analysis.sample_count,
        median_dt_seconds: validation.analysis.median_dt,
        effective_rate_hz: validation.analysis.effective_rate_hz,
        strictly_increasing: validation.analysis.strictly_increasing,
        non_monotonic_indices: validation.analysis.non_monotonic_indices.clone(),
        duplicate_count: validation.analysis.duplicate_indices.len(),
        reversal_count: validation.analysis.reversal_indices.len(),
        gaps: validation
            .analysis
            .gaps
            .iter()
            .map(|g| (g.start_time, g.duration, g.estimated_missing))
            .collect(),
        rate_segments: validation
            .rate_segments
            .iter()
            .map(|s| (s.rate_hz, s.start_index as f64))
            .collect(),
        rollover_corrected: validation.rollover.is_some(),
        warnings: validation.warnings.iter().map(|w| w.one_line()).collect(),
    })
}

/// The report the import pipeline produced, plus the session summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightImportResult {
    /// The stage-by-stage report.
    pub report: ImportReportView,
    /// A summary of the created session.
    pub session: Option<FlightSessionSummary>,
    /// The artifact directory name, when a session was written.
    pub artifact_name: Option<String>,
}

/// A serialisable view of the import report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportReportView {
    /// Whether every stage succeeded.
    pub succeeded: bool,
    /// The error that stopped the pipeline.
    pub primary_error: Option<String>,
    /// One entry per stage.
    pub stages: Vec<ImportStageView>,
}

/// One import stage as the pipeline panel presents it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportStageView {
    /// Stage name.
    pub stage: String,
    /// Stage status.
    pub status: String,
    /// Message.
    pub message: String,
    /// Longer detail, when the stage produced one.
    pub detail: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

fn report_view(report: &ImportReport) -> ImportReportView {
    ImportReportView {
        succeeded: report.succeeded,
        primary_error: report.primary_error.clone(),
        stages: report
            .stages
            .iter()
            .map(|s| ImportStageView {
                stage: format!("{:?}", s.stage),
                status: format!("{:?}", s.status),
                message: s.message.clone(),
                detail: s.detail.clone(),
                duration_ms: s.duration_ms as u64,
            })
            .collect(),
    }
}

/// A summary of an imported flight session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightSessionSummary {
    /// Session identifier.
    pub id: String,
    /// Source file name.
    pub source_file: String,
    /// Source format label.
    pub source_format: String,
    /// Samples.
    pub sample_count: usize,
    /// Duration in seconds.
    pub duration_seconds: f64,
    /// Effective sample rate.
    pub sample_rate_hz: f64,
    /// Channel names with their roles.
    pub channels: Vec<(String, String, String)>,
    /// Derived channel names.
    pub derived_channels: Vec<String>,
    /// Quality flags, one line each.
    pub quality_flags: Vec<String>,
    /// Overall quality severity.
    pub quality_severity: String,
    /// Whether the source carried absolute position.
    pub has_position: bool,
    /// The timebase note.
    pub timebase_note: String,
}

fn session_summary(session: &FlightSession) -> FlightSessionSummary {
    FlightSessionSummary {
        id: session.id.clone(),
        source_file: session.source.file_name.clone(),
        source_format: format!("{:?}", session.source.format),
        sample_count: session.times.len(),
        duration_seconds: session.duration(),
        sample_rate_hz: session.sample_rate_hz(),
        channels: session
            .channels
            .iter()
            .map(|c| (c.name.clone(), c.role.code().to_string(), c.unit.clone()))
            .collect(),
        derived_channels: session.derived.iter().map(|d| d.name.clone()).collect(),
        quality_flags: session
            .quality
            .flags
            .iter()
            .map(|f| format!("{}: {} ({})", f.code, f.detail, f.suggested_action))
            .collect(),
        quality_severity: format!("{:?}", session.quality.overall_severity),
        has_position: session
            .channel(hex_flight_data::ChannelRole::GnssAltitude)
            .is_some(),
        timebase_note: session.timebase.note.clone().unwrap_or_else(|| {
            format!(
                "{} at {}",
                session.timebase.source.label(),
                session.timebase.unit.label()
            )
        }),
    }
}

/// Import a flight log, optionally creating a session in the open project.
#[tauri::command]
pub fn flight_import(
    app: AppHandle,
    state: State<'_, AppState>,
    request: FlightImportRequest,
) -> Result<FlightImportResult, CommandError> {
    let options = csv_options(&request);
    let mappings = flight_mappings(&request);
    let path = std::path::PathBuf::from(&request.path);
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let (session, report) = if extension == "hlog" {
        let bytes = std::fs::read(&path).map_err(|e| {
            CommandError::new(
                "flight.io",
                "The flight log could not be read",
                format!("{} could not be opened: {}", request.path, e),
                "Check that the file exists and that it is readable.",
            )
        })?;
        import_binary(&bytes, &mappings)?
    } else {
        let bytes = std::fs::read(&path).map_err(|e| {
            CommandError::new(
                "flight.io",
                "The flight log could not be read",
                format!("{} could not be opened: {}", request.path, e),
                "Check that the file exists and that it is readable.",
            )
        })?;
        let text = decode_text(&bytes);
        import_csv(&text, &options, &mappings)?
    };

    for stage in &report.stages {
        emit(
            &app,
            events::channel::IMPORT_PROGRESS,
            events::ImportProgressEvent {
                stage: format!("{:?}", stage.stage),
                status: format!("{:?}", stage.status),
                message: stage.message.clone(),
                duration_ms: stage.duration_ms as u64,
            },
        );
    }

    let summary = session_summary(&session);
    let mut artifact_name = None;

    if request.create_session.unwrap_or(false) {
        let project = state.with(|s| s.project.clone())?;
        match project {
            Some(project) => {
                let label = request
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("Flight {}", summary.source_file));
                let mut metadata = hex_project::FlightSessionMetadata {
                    schema_version: String::new(),
                    name: String::new(),
                    label: label.clone(),
                    source_file: String::new(),
                    source_bytes: 0,
                    source_hash: String::new(),
                    source_format: summary.source_format.clone(),
                    imported_at: hex_project::now_iso8601(),
                    sample_count: summary.sample_count,
                    duration_seconds: summary.duration_seconds,
                    sample_rate_hz: summary.sample_rate_hz,
                    channels: summary.channels.iter().map(|(n, _, _)| n.clone()).collect(),
                    derived_channels: summary.derived_channels.clone(),
                    events: Vec::new(),
                    quality_flags: summary.quality_flags.clone(),
                    original_preserved: false,
                };
                let mapping_json = serde_json::to_value(&request.mappings).unwrap_or_default();
                let validation_json =
                    serde_json::to_value(report_view(&report)).unwrap_or_default();
                let derived_names: Vec<String> =
                    session.derived.iter().map(|d| d.name.clone()).collect();
                let derived_columns: Vec<Vec<f64>> =
                    session.derived.iter().map(|d| d.values.clone()).collect();
                match hex_project::import_flight_source(
                    &project,
                    &label,
                    &path,
                    &mut metadata,
                    &mapping_json,
                    &validation_json,
                    &derived_names,
                    &derived_columns,
                    &session.times,
                ) {
                    Ok(artifact) => {
                        artifact_name = Some(artifact.name.clone());
                        emit_notice(
                            &app,
                            events::NoticeEvent::info(
                                "Flight log imported",
                                format!("Saved as {}.", artifact.name),
                            ),
                        );
                    }
                    Err(e) => emit_notice(
                        &app,
                        events::NoticeEvent::warning(
                            "The flight session could not be saved",
                            e.user_message(),
                        ),
                    ),
                }
            }
            None => emit_notice(
                &app,
                events::NoticeEvent::warning(
                    "The flight session was not saved",
                    "No project is open, so the session exists only in this session.",
                ),
            ),
        }
    }

    if !summary.quality_flags.is_empty() {
        emit_notice(
            &app,
            events::NoticeEvent::warning(
                "Data quality flags",
                format!(
                    "{} finding(s) were reported. Open the quality panel for the detail.",
                    summary.quality_flags.len()
                ),
            ),
        );
    }

    state.with(|s| {
        s.flight = Some(session);
        s.flight_name = artifact_name.clone();
        Ok::<(), CommandError>(())
    })??;

    Ok(FlightImportResult {
        report: report_view(&report),
        session: Some(summary),
        artifact_name,
    })
}

/// Re-run the timestamp and quality checks on the imported session.
#[tauri::command]
pub fn flight_validate_session(
    state: State<'_, AppState>,
) -> Result<TimestampReport, CommandError> {
    state.with(|s| {
        let session = s.flight.as_ref().ok_or_else(|| {
            missing(
                "flight.none_imported",
                "No flight log is imported",
                "There is nothing to validate.",
                "Import a flight log on the Flight Analysis screen.",
            )
        })?;
        let validation = &session.timestamps;
        Ok(TimestampReport {
            sample_count: validation.analysis.sample_count,
            median_dt_seconds: validation.analysis.median_dt,
            effective_rate_hz: validation.analysis.effective_rate_hz,
            strictly_increasing: validation.analysis.strictly_increasing,
            non_monotonic_indices: validation.analysis.non_monotonic_indices.clone(),
            duplicate_count: validation.analysis.duplicate_indices.len(),
            reversal_count: validation.analysis.reversal_indices.len(),
            gaps: validation
                .analysis
                .gaps
                .iter()
                .map(|g| (g.start_time, g.duration, g.estimated_missing))
                .collect(),
            rate_segments: validation
                .rate_segments
                .iter()
                .map(|s| (s.rate_hz, s.start_index as f64))
                .collect(),
            rollover_corrected: validation.rollover.is_some(),
            warnings: validation.warnings.iter().map(|w| w.one_line()).collect(),
        })
    })?
}

/// Re-run the data quality assessment on the imported session.
#[tauri::command]
pub fn flight_assess_quality(
    state: State<'_, AppState>,
) -> Result<hex_flight_data::QualityReport, CommandError> {
    state.with(|s| {
        let session = s.flight.as_ref().ok_or_else(|| {
            missing(
                "flight.none_imported",
                "No flight log is imported",
                "There is nothing to assess.",
                "Import a flight log on the Flight Analysis screen.",
            )
        })?;
        Ok(assess_quality(
            &session.channels,
            &session.times,
            &QualityOptions::default(),
        ))
    })?
}

/// The channels of the imported session.
#[tauri::command]
pub fn flight_channels(
    state: State<'_, AppState>,
) -> Result<Vec<FlightChannelSummary>, CommandError> {
    state.with(|s| {
        let session = s.flight.as_ref().ok_or_else(|| {
            missing(
                "flight.none_imported",
                "No flight log is imported",
                "There are no channels to list.",
                "Import a flight log on the Flight Analysis screen.",
            )
        })?;
        Ok(session.channels.iter().map(channel_summary).collect())
    })?
}

/// One channel as the mapping table presents it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightChannelSummary {
    /// Column name in the source.
    pub name: String,
    /// Role code.
    pub role: String,
    /// Role label.
    pub role_label: String,
    /// Unit as recorded.
    pub unit: String,
    /// Whether the unit is known.
    pub unit_known: bool,
    /// Samples.
    pub count: usize,
    /// Minimum of the corrected values.
    pub min: f64,
    /// Maximum of the corrected values.
    pub max: f64,
    /// Mean of the corrected values.
    pub mean: f64,
    /// Whether the channel never changes.
    pub constant: bool,
    /// Whether the channel is usable at all.
    pub usable: bool,
}

fn channel_summary(channel: &Channel) -> FlightChannelSummary {
    let stats = channel.statistics();
    FlightChannelSummary {
        name: channel.name.clone(),
        role: channel.role.code().to_string(),
        role_label: channel.role.label().to_string(),
        unit: channel.unit.clone(),
        unit_known: channel.unit_confidence == hex_core::units::UnitConfidence::Declared
            || channel.unit.eq_ignore_ascii_case(channel.role.unit()),
        count: stats.count,
        min: stats.min,
        max: stats.max,
        mean: stats.mean,
        constant: stats.constant,
        usable: channel.is_usable(),
    }
}

/// A time series of one channel of the imported session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightSeries {
    /// Channel name.
    pub name: String,
    /// Unit.
    pub unit: String,
    /// Times in seconds.
    pub times: Vec<f64>,
    /// Corrected values.
    pub values: Vec<f64>,
}

/// Read one channel of the imported session as a plottable series.
#[tauri::command]
pub fn flight_series(
    state: State<'_, AppState>,
    name: Option<String>,
    role: Option<String>,
    maximum_samples: Option<usize>,
) -> Result<FlightSeries, CommandError> {
    state.with(|s| {
        let session = s.flight.as_ref().ok_or_else(|| {
            missing(
                "flight.none_imported",
                "No flight log is imported",
                "There is nothing to read.",
                "Import a flight log on the Flight Analysis screen.",
            )
        })?;
        let channel = match (&name, &role) {
            (Some(n), _) => session.channel_by_name(n),
            (None, Some(r)) => role_from_code(r).and_then(|role| session.channel(role)),
            (None, None) => session.channels.first(),
        }
        .ok_or_else(|| {
            missing(
                "flight.channel_missing",
                "That channel is not present",
                "The flight log does not carry the requested channel.",
                "Choose a channel from the list on the Flight Analysis screen.",
            )
        })?;

        let corrected = channel.corrected_values();
        let limit = maximum_samples.unwrap_or(4000).max(2);
        let stride = (session.times.len() / limit).max(1);
        let mut times = Vec::new();
        let mut values = Vec::new();
        for (i, t) in session.times.iter().enumerate() {
            if i % stride != 0 && i + 1 != session.times.len() {
                continue;
            }
            times.push(*t);
            values.push(corrected.get(i).copied().unwrap_or(f64::NAN));
        }
        Ok(FlightSeries {
            name: channel.name.clone(),
            unit: channel.unit.clone(),
            times,
            values,
        })
    })?
}

/// Build replay data from the imported flight session.
#[tauri::command]
pub fn flight_replay_data(
    state: State<'_, AppState>,
    maximum_samples: Option<usize>,
    initial_attitude: Option<[f64; 4]>,
) -> Result<ReplayData, CommandError> {
    state.with(|s| {
        let session = s.flight.as_ref().ok_or_else(|| {
            missing(
                "flight.none_imported",
                "No flight log is imported",
                "There is nothing to replay.",
                "Import a flight log on the Flight Analysis screen.",
            )
        })?;

        let limit = maximum_samples.unwrap_or(2000).max(2);
        let stride = (session.times.len() / limit).max(1);

        // Orientation is propagated from the gyroscope when the log carries one.
        // The result is labelled as an integration so the viewport can say so.
        let gyro_channels = [ChannelRole::GyroX, ChannelRole::GyroY, ChannelRole::GyroZ];
        let rates: Option<Vec<hex_core::Vec3>> =
            if gyro_channels.iter().all(|r| session.channel(*r).is_some()) {
                let x = session
                    .channel(ChannelRole::GyroX)
                    .unwrap()
                    .corrected_values();
                let y = session
                    .channel(ChannelRole::GyroY)
                    .unwrap()
                    .corrected_values();
                let z = session
                    .channel(ChannelRole::GyroZ)
                    .unwrap()
                    .corrected_values();
                Some(
                    (0..session.times.len())
                        .map(|i| {
                            hex_core::Vec3::new(
                                x.get(i).copied().unwrap_or(0.0),
                                y.get(i).copied().unwrap_or(0.0),
                                z.get(i).copied().unwrap_or(0.0),
                            )
                        })
                        .collect(),
                )
            } else {
                None
            };

        let initial = initial_attitude
            .map(|q| hex_core::Quaternion::new(q[0], q[1], q[2], q[3]))
            .unwrap_or_else(hex_core::Quaternion::identity);
        let quaternions = match &rates {
            Some(rates) => hex_flight_data::propagate_gyro_attitude(
                &session.times,
                rates,
                initial,
                hex_core::Vec3::zeros(),
            ),
            None => vec![initial; session.times.len()],
        };

        // Position is only replayed when the log actually measured or estimated
        // it. A fabricated trajectory would be indistinguishable from data.
        let position_channels = [
            ChannelRole::GnssLatitude,
            ChannelRole::GnssLongitude,
            ChannelRole::GnssAltitude,
        ];
        let has_position = position_channels
            .iter()
            .all(|r| session.channel(*r).is_some());
        let positions: Vec<[f64; 3]> = if has_position {
            let lat = session
                .channel(ChannelRole::GnssLatitude)
                .unwrap()
                .corrected_values();
            let lon = session
                .channel(ChannelRole::GnssLongitude)
                .unwrap()
                .corrected_values();
            let alt = session
                .channel(ChannelRole::GnssAltitude)
                .unwrap()
                .corrected_values();
            (0..session.times.len())
                .map(|i| {
                    [
                        lon.get(i).copied().unwrap_or(0.0),
                        lat.get(i).copied().unwrap_or(0.0),
                        alt.get(i).copied().unwrap_or(0.0),
                    ]
                })
                .collect()
        } else {
            vec![[0.0; 3]; session.times.len()]
        };

        let events: Vec<hex_dynamics::FlightEvent> = session
            .events
            .iter()
            .map(|e| hex_dynamics::FlightEvent {
                id: format!("{}-{:.6}", e.kind, e.time),
                kind: hex_dynamics::EventKind::Custom,
                time: e.time,
                label: e.label.clone(),
                description: e.description.clone(),
                trigger_value: None,
                trigger_unit: None,
                // A marker the user placed is marked as such, so the replay
                // strip can draw it differently from a detection.
                user_added: e.kind == "user.marker",
            })
            .collect();

        let mut times = Vec::new();
        let mut out_positions = Vec::new();
        let mut out_quaternions = Vec::new();
        for (i, t) in session.times.iter().enumerate() {
            if i % stride != 0 && i + 1 != session.times.len() {
                continue;
            }
            times.push(*t);
            out_positions.push(positions.get(i).copied().unwrap_or([0.0; 3]));
            let q = quaternions.get(i).copied().unwrap_or(initial);
            out_quaternions.push([q.w, q.x, q.y, q.z]);
        }

        Ok(ReplayData {
            times,
            positions: out_positions,
            quaternions: out_quaternions,
            velocities: Vec::new(),
            angular_rates: Vec::new(),
            events,
            source: if rates.is_some() {
                "gyro integration".to_string()
            } else {
                "orientation only".to_string()
            },
            has_position,
        })
    })?
}

/// The imported flight sessions in the open project.
#[tauri::command]
pub fn flight_session_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::state::SavedSessionSummary>, CommandError> {
    let project = state.require_project()?;
    Ok(hex_project::list_flight_sessions(&project)
        .into_iter()
        .map(|s| crate::state::SavedSessionSummary {
            name: s.name,
            label: s.metadata.label.clone(),
            samples: s.metadata.sample_count,
            duration_seconds: s.metadata.duration_seconds,
            has_issues: s.metadata.has_quality_flags(),
        })
        .collect())
}

/// Add a user marker to the imported flight log.
///
/// The marker is placed on the corrected session time axis, kept in memory so
/// the timeline updates immediately, and written into the project session when
/// the log was imported into one. The original file is never touched.
#[tauri::command]
pub fn flight_add_marker(
    app: AppHandle,
    state: State<'_, AppState>,
    time: f64,
    label: String,
) -> Result<EventTimeline, CommandError> {
    if !time.is_finite() {
        return Err(crate::error::invalid(
            "The marker time must be a finite number.",
            "Type a time in seconds.",
        ));
    }
    let label = label.trim().to_string();
    if label.is_empty() {
        return Err(crate::error::invalid(
            "The marker needs a label.",
            "Type a short label, for example Burnout.",
        ));
    }

    let (timeline, artifact_name) = state.with(|s| {
        let session = s.flight.as_mut().ok_or_else(|| {
            missing(
                "flight.none_imported",
                "No flight log is imported",
                "There is nothing to mark.",
                "Import a flight log on the Flight Analysis screen.",
            )
        })?;
        // A marker outside the recorded span cannot be shown on the timeline, so
        // it is refused rather than silently clamped to the nearest sample.
        if let Some((first, last)) = session.time_span() {
            let tolerance = (last - first).abs() * 1e-9 + 1e-9;
            if time < first - tolerance || time > last + tolerance {
                return Err(crate::error::invalid(
                    "The marker time is outside the recorded flight.",
                    format!(
                        "This log covers {:.3} s to {:.3} s. Choose a time inside that range.",
                        first, last
                    ),
                ));
            }
        }
        session.add_marker(time, &label);
        Ok::<_, CommandError>((flight_timeline(session, None), s.flight_name.clone()))
    })??;

    if let Some(name) = artifact_name {
        let project = state.with(|s| s.project.clone())?;
        if let Some(project) = project {
            match hex_project::find_flight_session(&project, &name) {
                Ok(artifact) => {
                    if let Err(e) = artifact.add_marker(time, &label) {
                        emit_notice(
                            &app,
                            events::NoticeEvent::warning(
                                "The marker was not saved with the session",
                                e.user_message(),
                            ),
                        );
                    }
                }
                Err(e) => emit_notice(
                    &app,
                    events::NoticeEvent::warning(
                        "The marker was not saved with the session",
                        e.user_message(),
                    ),
                ),
            }
        } else {
            emit_notice(
                &app,
                events::NoticeEvent::warning(
                    "The marker is only in this session",
                    "No project is open, so the marker will not survive a restart.",
                ),
            );
        }
    }

    emit_notice(
        &app,
        events::NoticeEvent::info("Marker added", format!("{} at {:.3} s.", label, time)),
    );
    Ok(timeline)
}

/// The roles a mapping row can target.
#[tauri::command]
pub fn flight_role_options() -> Vec<(String, String, String)> {
    [
        ChannelRole::Time,
        ChannelRole::AccelX,
        ChannelRole::AccelY,
        ChannelRole::AccelZ,
        ChannelRole::GyroX,
        ChannelRole::GyroY,
        ChannelRole::GyroZ,
        ChannelRole::MagX,
        ChannelRole::MagY,
        ChannelRole::MagZ,
        ChannelRole::BaroPressure,
        ChannelRole::BaroTemperature,
        ChannelRole::GnssLatitude,
        ChannelRole::GnssLongitude,
        ChannelRole::GnssAltitude,
        ChannelRole::GnssSpeed,
        ChannelRole::QuaternionW,
        ChannelRole::QuaternionX,
        ChannelRole::QuaternionY,
        ChannelRole::QuaternionZ,
        ChannelRole::MotorThrottle,
        ChannelRole::ControlSurface,
        ChannelRole::Voltage,
        ChannelRole::Current,
        ChannelRole::Raw,
        ChannelRole::Ignored,
    ]
    .iter()
    .map(|r| {
        (
            r.code().to_string(),
            r.label().to_string(),
            r.unit().to_string(),
        )
    })
    .collect()
}

/// The import error a caller should show for a path that does not exist.
pub fn missing_file_error(path: &str) -> CommandError {
    CommandError::new(
        "flight.io",
        "The flight log could not be opened",
        format!("{} does not exist.", path),
        "Choose an existing CSV or HexaDOF binary log.",
    )
}

/// Convert a flight-data error into a command error with the file attached.
pub fn flight_error(path: &str, error: FlightDataError) -> CommandError {
    let mut mapped: CommandError = error.into();
    mapped.detail = format!("{} ({})", mapped.detail, path);
    mapped
}

/// Import via the extension-detecting entry point, for callers that do not know
/// the format.
#[tauri::command]
pub fn flight_import_path(
    app: AppHandle,
    state: State<'_, AppState>,
    request: FlightImportRequest,
) -> Result<FlightImportResult, CommandError> {
    let options = csv_options(&request);
    let mappings = flight_mappings(&request);
    let path = std::path::PathBuf::from(&request.path);
    let (session, report) =
        import_path(&path, &options, &mappings).map_err(|e| flight_error(&request.path, e))?;

    for stage in &report.stages {
        emit(
            &app,
            events::channel::IMPORT_PROGRESS,
            events::ImportProgressEvent {
                stage: format!("{:?}", stage.stage),
                status: format!("{:?}", stage.status),
                message: stage.message.clone(),
                duration_ms: stage.duration_ms as u64,
            },
        );
    }
    let summary = session_summary(&session);
    state.with(|s| {
        s.flight = Some(session);
        Ok::<(), CommandError>(())
    })??;
    Ok(FlightImportResult {
        report: report_view(&report),
        session: Some(summary),
        artifact_name: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "hexadof-flight-{}-{}-{}",
                label,
                std::process::id(),
                hex_project::new_id()
            ));
            std::fs::create_dir_all(&base).expect("temp dir");
            Self(base)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const SAMPLE: &str = "timestamp,ax,ay,az,gx,gy,gz,baro\n0.00,0.10,0.00,9.81,0.01,0.00,0.00,101325\n0.01,0.10,0.00,9.80,0.01,0.00,0.00,101320\n0.02,0.11,0.01,9.79,0.01,0.00,0.00,101315\n0.03,0.11,0.01,9.78,0.01,0.00,0.00,101310\n";

    fn request_for(path: &str) -> FlightImportRequest {
        FlightImportRequest {
            path: path.to_string(),
            delimiter: None,
            has_header: Some(true),
            timestamp_column: Some("timestamp".to_string()),
            timestamp_unit: Some("seconds".to_string()),
            tick_rate: None,
            comment_prefix: None,
            max_rows: None,
            decimal_comma: None,
            mappings: Vec::new(),
            expected_rate_hz: Some(100.0),
            gap_factor: Some(3.0),
            rollover_bits: None,
            accel_range: None,
            gyro_range: None,
            label: Some("Test flight".to_string()),
            create_session: Some(false),
        }
    }

    #[test]
    fn csv_options_reflect_the_request() {
        let request = FlightImportRequest {
            delimiter: Some(";".to_string()),
            has_header: Some(false),
            timestamp_unit: Some("milliseconds".to_string()),
            decimal_comma: Some(true),
            ..request_for("x.csv")
        };
        let options = csv_options(&request);
        assert_eq!(options.delimiter, b';');
        assert!(!options.has_header);
        assert_eq!(options.timestamp_unit, TimestampUnit::Milliseconds);
        assert!(options.decimal_comma);
    }

    #[test]
    fn tick_timestamps_need_a_rate() {
        let request = FlightImportRequest {
            timestamp_unit: Some("ticks".to_string()),
            tick_rate: Some(1000.0),
            ..request_for("x.csv")
        };
        match csv_options(&request).timestamp_unit {
            TimestampUnit::Ticks(rate) => assert!((rate - 1000.0).abs() < 1e-12),
            other => panic!("expected ticks, got {:?}", other),
        }
    }

    #[test]
    fn role_codes_round_trip_through_the_mapping() {
        for (code, _, _) in flight_role_options() {
            assert!(role_from_code(&code).is_some(), "{} does not resolve", code);
        }
        assert!(role_from_code("nonsense").is_none());
    }

    #[test]
    fn explicit_mappings_are_built_with_calibration() {
        let request = FlightImportRequest {
            mappings: vec![FlightMappingRequest {
                source_name: "ax".to_string(),
                role: "accel.x".to_string(),
                unit: "g".to_string(),
                sign: Some(-1.0),
                calibration_offset: Some(0.05),
                calibration_scale: Some(1.01),
            }],
            ..request_for("x.csv")
        };
        let mappings = flight_mappings(&request);
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].role, ChannelRole::AccelX);
        assert!((mappings[0].sign + 1.0).abs() < 1e-12);
        assert!(mappings[0].calibration.enabled);
        assert!((mappings[0].calibration.offset - 0.05).abs() < 1e-12);
    }

    #[test]
    fn an_unknown_role_code_is_dropped() {
        let request = FlightImportRequest {
            mappings: vec![FlightMappingRequest {
                source_name: "x".to_string(),
                role: "nonsense".to_string(),
                unit: "1".to_string(),
                sign: None,
                calibration_offset: None,
                calibration_scale: None,
            }],
            ..request_for("x.csv")
        };
        assert!(flight_mappings(&request).is_empty());
    }

    #[test]
    fn preview_reports_headers_roles_and_rows() {
        let temp = TempDir::new("preview");
        let path = temp.path().join("flight.csv");
        std::fs::write(&path, SAMPLE).unwrap();
        let preview = flight_preview(request_for(&path.to_string_lossy())).unwrap();
        assert_eq!(preview.delimiter, ",");
        assert!(preview.has_header);
        assert_eq!(preview.headers.len(), 8);
        assert_eq!(preview.headers[0], "timestamp");
        assert_eq!(preview.inferred_roles.len(), 8);
        assert!(preview.inferred_roles[0].contains("time"));
        assert!(preview.inferred_roles[1].contains("accel"));
        assert_eq!(preview.row_count, 4);
        assert!((preview.timestamp_preview[0]).abs() < 1e-12);
    }

    #[test]
    fn preview_of_a_missing_file_is_an_actionable_error() {
        let error = flight_preview(request_for("C:/definitely/not/here.csv")).unwrap_err();
        assert_eq!(error.code, "flight.io");
        assert!(!error.suggestion.is_empty());
    }

    #[test]
    fn validate_reports_a_clean_timestamp_column() {
        let temp = TempDir::new("validate");
        let path = temp.path().join("flight.csv");
        std::fs::write(&path, SAMPLE).unwrap();
        let report = flight_validate(request_for(&path.to_string_lossy())).unwrap();
        assert_eq!(report.sample_count, 4);
        assert!(report.strictly_increasing);
        assert!((report.median_dt_seconds - 0.01).abs() < 1e-9);
        assert!((report.effective_rate_hz - 100.0).abs() < 1e-6);
        assert!(report.gaps.is_empty());
        assert_eq!(report.duplicate_count, 0);
        assert_eq!(report.reversal_count, 0);
    }

    #[test]
    fn validate_reports_gaps_and_duplicates() {
        let temp = TempDir::new("gaps");
        let path = temp.path().join("flight.csv");
        std::fs::write(
            &path,
            "timestamp,ax\n0.00,0.1\n0.01,0.1\n0.01,0.2\n0.51,0.3\n0.52,0.3\n",
        )
        .unwrap();
        let report = flight_validate(request_for(&path.to_string_lossy())).unwrap();
        assert!(!report.strictly_increasing);
        assert_eq!(report.duplicate_count, 1);
        assert_eq!(report.gaps.len(), 1);
        assert!(report.gaps[0].2 >= 40);
    }

    #[test]
    fn an_import_creates_a_session_and_a_report() {
        let temp = TempDir::new("import");
        let path = temp.path().join("flight.csv");
        std::fs::write(&path, SAMPLE).unwrap();
        let (session, report) = import_csv(
            SAMPLE,
            &csv_options(&request_for(&path.to_string_lossy())),
            &[],
        )
        .unwrap();
        assert!(report.succeeded, "{:?}", report.primary_error);
        let summary = session_summary(&session);
        assert_eq!(summary.sample_count, 4);
        assert!(!summary.channels.is_empty());
        assert!(!summary.source_file.is_empty());
        assert!(summary.sample_rate_hz > 0.0);

        let view = report_view(&report);
        assert!(view.succeeded);
        assert!(!view.stages.is_empty());
        assert!(view.stages.iter().any(|s| s.stage.contains("Parsed")));
    }

    #[test]
    fn a_broken_csv_stops_the_pipeline_with_a_reason() {
        let error = import_csv(
            "no numbers here at all\n",
            &csv_options(&request_for("x.csv")),
            &[],
        )
        .unwrap_err();
        let mapped: CommandError = error.into();
        assert_eq!(mapped.code, "flight.error");
        assert!(!mapped.suggestion.is_empty());
    }

    #[test]
    fn channel_summaries_carry_statistics() {
        let (session, _) = import_csv(SAMPLE, &csv_options(&request_for("x.csv")), &[]).unwrap();
        let summaries: Vec<FlightChannelSummary> =
            session.channels.iter().map(channel_summary).collect();
        assert!(!summaries.is_empty());
        let baro = summaries
            .iter()
            .find(|s| s.role.contains("baro"))
            .expect("pressure channel");
        assert!(baro.max > baro.min);
        assert!(!baro.constant);
        assert!(baro.usable);
    }

    #[test]
    fn missing_file_error_names_the_path() {
        let error = missing_file_error("C:/nope.csv");
        assert!(error.detail.contains("C:/nope.csv"));
        assert!(!error.suggestion.is_empty());
    }

    #[test]
    fn flight_error_keeps_the_path_in_the_detail() {
        let base = FlightDataError::EmptyFile;
        let mapped = flight_error("C:/x.csv", base);
        assert!(mapped.detail.contains("C:/x.csv"));
    }
}
