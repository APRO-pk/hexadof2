//! The import pipeline: one recorded stage per step, and a session at the end.
//!
//! Every import walks the same ordered stages, and each stage appends an
//! [`StageResult`] describing what it did, how long it took, and whether it
//! warned. A failing stage stops the pipeline, records the failure, and sets the
//! report's `primary_error`, so the import dialog can show the user exactly how
//! far the file got and why it stopped.
//!
//! Reading is strictly one-way. The source file is opened for reading only, and
//! nothing in this module ever writes to it; the session keeps a hash of the
//! bytes so a later reader can prove the original was not touched.

use std::path::Path;
use std::time::Instant;

use hex_core::units::UnitConfidence;
use hex_core::{AxisMapping, Frame, Real};

use crate::binary::BinaryReader;
use crate::channel::{snap_sign, Channel, ChannelFamily, ChannelRole};
use crate::csv::{parse_csv, CsvImportOptions, RawTable};
use crate::derived::{
    derive_barometric_altitude, derive_vector_channels, DerivedChannel, Provenance,
    ISA_SEA_LEVEL_PRESSURE, ISA_SEA_LEVEL_TEMPERATURE,
};
use crate::error::FlightDataError;
use crate::session::{FlightSession, SourceFormat, SourceInfo};
use crate::validation::{
    assess_quality, sequence_gap_flag, validate_timestamps, QualityOptions,
    TimestampValidationOptions,
};

/// Stages of an import, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportStage {
    /// The caller chose a file.
    Selected,
    /// The bytes were tokenised or framed.
    Parsed,
    /// Columns were turned into channels.
    ChannelsDiscovered,
    /// Source columns were assigned physical roles.
    Mapped,
    /// Declared units were converted to SI.
    UnitsConverted,
    /// Sensor axes were permuted into body axes.
    FramesConverted,
    /// The time axis was checked and rebased.
    TimestampsValidated,
    /// Per-channel calibration was applied.
    Calibrated,
    /// Derived series were computed.
    DerivedGenerated,
    /// The session was assembled and returned.
    SessionCreated,
    /// A stage failed and the pipeline stopped.
    Failed,
}

impl ImportStage {
    /// The ordered list of stages a successful import runs.
    pub fn all() -> [ImportStage; 10] {
        [
            ImportStage::Selected,
            ImportStage::Parsed,
            ImportStage::ChannelsDiscovered,
            ImportStage::Mapped,
            ImportStage::UnitsConverted,
            ImportStage::FramesConverted,
            ImportStage::TimestampsValidated,
            ImportStage::Calibrated,
            ImportStage::DerivedGenerated,
            ImportStage::SessionCreated,
        ]
    }

    /// Human-readable label shown in the import progress list.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Selected => "File selected",
            Self::Parsed => "Parsed",
            Self::ChannelsDiscovered => "Channels discovered",
            Self::Mapped => "Channels mapped",
            Self::UnitsConverted => "Units converted",
            Self::FramesConverted => "Frames converted",
            Self::TimestampsValidated => "Timestamps validated",
            Self::Calibrated => "Calibrated",
            Self::DerivedGenerated => "Derived channels generated",
            Self::SessionCreated => "Session created",
            Self::Failed => "Failed",
        }
    }

    /// Stable machine code, for example `units_converted`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Selected => "selected",
            Self::Parsed => "parsed",
            Self::ChannelsDiscovered => "channels_discovered",
            Self::Mapped => "mapped",
            Self::UnitsConverted => "units_converted",
            Self::FramesConverted => "frames_converted",
            Self::TimestampsValidated => "timestamps_validated",
            Self::Calibrated => "calibrated",
            Self::DerivedGenerated => "derived_generated",
            Self::SessionCreated => "session_created",
            Self::Failed => "failed",
        }
    }
}

/// Outcome of one stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    /// The stage completed and changed something.
    Ok,
    /// The stage completed but found something the user should see.
    Warning,
    /// The stage failed and the pipeline stopped.
    Failed,
    /// The stage had nothing to do.
    Skipped,
}

impl StageStatus {
    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Ok => "Ok",
            Self::Warning => "Warning",
            Self::Failed => "Failed",
            Self::Skipped => "Skipped",
        }
    }

    /// True when the stage stopped the pipeline.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// What one stage did.
#[derive(Debug, Clone, PartialEq)]
pub struct StageResult {
    /// Stage that ran.
    pub stage: ImportStage,
    /// How the stage ended.
    pub status: StageStatus,
    /// One-line description shown in the progress list.
    pub message: String,
    /// Optional expanded detail shown when the user asks.
    pub detail: Option<String>,
    /// Wall-clock duration of the stage.
    pub duration_ms: u128,
}

/// The ordered record of an import attempt.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportReport {
    /// One entry per stage that ran, in order.
    pub stages: Vec<StageResult>,
    /// True when no stage failed.
    pub succeeded: bool,
    /// The error that stopped the pipeline, when one did.
    pub primary_error: Option<String>,
}

impl ImportReport {
    /// An empty report.
    pub fn new() -> Self {
        Self {
            stages: Vec::new(),
            succeeded: true,
            primary_error: None,
        }
    }

    /// The result of a stage, when it ran.
    pub fn stage(&self, stage: ImportStage) -> Option<&StageResult> {
        self.stages.iter().find(|result| result.stage == stage)
    }

    /// Number of stages with the given status.
    pub fn count(&self, status: StageStatus) -> usize {
        self.stages
            .iter()
            .filter(|result| result.status == status)
            .count()
    }

    /// True when no stage failed and no stage warned.
    pub fn is_clean(&self) -> bool {
        self.succeeded && self.count(StageStatus::Warning) == 0
    }

    /// One-line summary for a status strip.
    pub fn summary(&self) -> String {
        match &self.primary_error {
            Some(error) => format!(
                "Import failed after {} stage(s): {error}",
                self.stages.len()
            ),
            None => format!(
                "Import finished: {} stage(s), {} warning(s)",
                self.stages.len(),
                self.count(StageStatus::Warning)
            ),
        }
    }
}

impl Default for ImportReport {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-channel calibration applied by the [`ImportStage::Calibrated`] stage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelCalibration {
    /// Value added after scaling.
    pub offset: Real,
    /// Multiplier applied to the raw value.
    pub scale: Real,
    /// Whether the calibration is applied at all.
    pub enabled: bool,
}

impl Default for ChannelCalibration {
    fn default() -> Self {
        Self {
            offset: 0.0,
            scale: 1.0,
            enabled: false,
        }
    }
}

impl ChannelCalibration {
    /// An enabled affine calibration.
    pub fn new(scale: Real, offset: Real) -> Self {
        Self {
            offset,
            scale,
            enabled: true,
        }
    }

    /// Apply the calibration, or pass the value through when disabled.
    pub fn apply(&self, raw: Real) -> Real {
        if self.enabled {
            raw * self.scale + self.offset
        } else {
            raw
        }
    }

    /// True when the calibration would change a value.
    pub fn is_identity(&self) -> bool {
        !self.enabled || (self.scale == 1.0 && self.offset == 0.0)
    }
}

/// One source column's assignment into the normalised channel model.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelMapping {
    /// Header name in the source file.
    pub source_name: String,
    /// Role the column fills.
    pub role: ChannelRole,
    /// Unit declared for the column, when the source or user supplied one.
    pub unit: String,
    /// Axis sign applied to the channel, always exactly `1.0` or `-1.0`.
    pub sign: Real,
    /// Signed axis permutation mapping the sensor axes onto body axes.
    pub axis_mapping: AxisMapping,
    /// Per-channel calibration.
    pub calibration: ChannelCalibration,
}

impl ChannelMapping {
    /// Map a source column to a role with no unit override or calibration.
    pub fn new(source_name: impl Into<String>, role: ChannelRole) -> Self {
        Self {
            source_name: source_name.into(),
            role,
            unit: role.unit().to_string(),
            sign: 1.0,
            axis_mapping: AxisMapping::identity(),
            calibration: ChannelCalibration::default(),
        }
    }

    /// Set the declared unit of the source column.
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    /// Set the axis sign, snapped to exactly `+1.0` or `-1.0`.
    pub fn with_sign(mut self, sign: Real) -> Self {
        self.sign = snap_sign(sign);
        self
    }

    /// Attach a calibration.
    pub fn with_calibration(mut self, calibration: ChannelCalibration) -> Self {
        self.calibration = calibration;
        self
    }

    /// Attach an axis permutation.
    pub fn with_axis_mapping(mut self, axis_mapping: AxisMapping) -> Self {
        self.axis_mapping = axis_mapping;
        self
    }
}

/// Everything one import attempt produced, including a failed attempt.
#[derive(Debug)]
pub struct ImportOutcome {
    /// The session, when the pipeline completed.
    pub session: Option<FlightSession>,
    /// The stage-by-stage record.
    pub report: ImportReport,
    /// The error that stopped the pipeline, when one did.
    pub error: Option<FlightDataError>,
}

impl ImportOutcome {
    /// Collapse the outcome into the shape the documented entry points return.
    pub fn into_result(self) -> Result<(FlightSession, ImportReport), FlightDataError> {
        match (self.session, self.error) {
            (Some(session), _) => Ok((session, self.report)),
            (None, Some(error)) => Err(error),
            (None, None) => Err(FlightDataError::EmptyFile),
        }
    }

    /// True when a session was produced.
    pub fn succeeded(&self) -> bool {
        self.session.is_some()
    }
}

/// Accumulates stage results while the pipeline runs.
struct StageLog {
    stages: Vec<StageResult>,
    primary_error: Option<String>,
}

impl StageLog {
    fn new() -> Self {
        Self {
            stages: Vec::new(),
            primary_error: None,
        }
    }

    fn record(
        &mut self,
        stage: ImportStage,
        status: StageStatus,
        message: impl Into<String>,
        detail: Option<String>,
        duration_ms: u128,
    ) {
        let message = message.into();
        self.stages.push(StageResult {
            stage,
            status,
            message: message.clone(),
            detail,
            duration_ms,
        });
        if status.is_failure() {
            self.primary_error = Some(message);
        }
    }

    fn ok(
        &mut self,
        stage: ImportStage,
        message: impl Into<String>,
        detail: Option<String>,
        ms: u128,
    ) {
        self.record(stage, StageStatus::Ok, message, detail, ms);
    }

    fn warning(
        &mut self,
        stage: ImportStage,
        message: impl Into<String>,
        detail: Option<String>,
        ms: u128,
    ) {
        self.record(stage, StageStatus::Warning, message, detail, ms);
    }

    fn skipped(&mut self, stage: ImportStage, message: impl Into<String>, ms: u128) {
        self.record(stage, StageStatus::Skipped, message, None, ms);
    }

    fn failed(
        &mut self,
        stage: ImportStage,
        message: impl Into<String>,
        detail: Option<String>,
        ms: u128,
    ) {
        self.record(stage, StageStatus::Failed, message, detail, ms);
    }

    fn finish(self) -> ImportReport {
        ImportReport {
            succeeded: self.stages.iter().all(|result| !result.status.is_failure()),
            stages: self.stages,
            primary_error: self.primary_error,
        }
    }
}

/// Normalise a header to bare lowercase alphanumerics.
fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Split a trailing unit annotation from a column name.
///
/// Loggers commonly write `accel_x [g]` or `altitude (m)`. Splitting the unit
/// out means the unit converter has something real to work with instead of
/// guessing.
pub fn split_unit_from_name(name: &str) -> (String, Option<String>) {
    let trimmed = name.trim();
    for (open, close) in [('[', ']'), ('(', ')')] {
        if let Some(start) = trimmed.rfind(open) {
            if trimmed.ends_with(close) && start + 1 < trimmed.len() - 1 {
                let unit = trimmed[start + 1..trimmed.len() - 1].trim().to_string();
                let base = trimmed[..start].trim().to_string();
                if !base.is_empty() && !unit.is_empty() {
                    return (base, Some(unit));
                }
            }
        }
    }
    (trimmed.to_string(), None)
}

/// Guess the role of a column from its name.
///
/// The guess is only a starting point: the user's mapping always wins, and an
/// unrecognised name stays [`ChannelRole::Raw`] rather than being forced into a
/// plausible-looking role.
pub fn infer_role_from_name(name: &str) -> ChannelRole {
    let n = normalize_name(name);
    if n.is_empty() {
        return ChannelRole::Raw;
    }
    let axis = if n.ends_with('x') {
        Some(0usize)
    } else if n.ends_with('y') {
        Some(1)
    } else if n.ends_with('z') {
        Some(2)
    } else {
        None
    };
    let axis_role =
        |triple: [ChannelRole; 3]| axis.map(|index| triple[index]).unwrap_or(ChannelRole::Raw);

    if n == "t" || n.contains("timestamp") || n.contains("time") {
        return ChannelRole::Time;
    }
    if n.contains("quat") || matches!(n.as_str(), "qw" | "qx" | "qy" | "qz") {
        return match n.chars().last() {
            Some('x') => ChannelRole::QuaternionX,
            Some('y') => ChannelRole::QuaternionY,
            Some('z') => ChannelRole::QuaternionZ,
            _ => ChannelRole::QuaternionW,
        };
    }
    if n.contains("baro") && n.contains("temp") {
        return ChannelRole::BaroTemperature;
    }
    if n.contains("baro") || n.contains("press") {
        return ChannelRole::BaroPressure;
    }
    if n.contains("acc") || matches!(n.as_str(), "ax" | "ay" | "az") {
        return axis_role([
            ChannelRole::AccelX,
            ChannelRole::AccelY,
            ChannelRole::AccelZ,
        ]);
    }
    if n.contains("gyr") || matches!(n.as_str(), "gx" | "gy" | "gz" | "wx" | "wy" | "wz") {
        // `ax`, `ay`, `az` are recognised on the accelerometer, so the matching
        // short names for the rate axes are recognised here too. A log that uses
        // them would otherwise import its gyroscope as unlabelled raw channels,
        // which silently removes any attitude reconstruction.
        return axis_role([ChannelRole::GyroX, ChannelRole::GyroY, ChannelRole::GyroZ]);
    }
    if n.contains("mag") && !n.contains("magnitude") {
        return axis_role([ChannelRole::MagX, ChannelRole::MagY, ChannelRole::MagZ]);
    }
    if n.contains("lat") {
        return ChannelRole::GnssLatitude;
    }
    if n.contains("lon") || n.contains("lng") {
        return ChannelRole::GnssLongitude;
    }
    if n.contains("alt") {
        return ChannelRole::GnssAltitude;
    }
    if n.contains("speed") || n.contains("groundspeed") || n == "gs" {
        return ChannelRole::GnssSpeed;
    }
    if n.contains("temp") {
        return ChannelRole::Temperature;
    }
    if n.contains("throt") || n.contains("thr") {
        return ChannelRole::MotorThrottle;
    }
    if n.contains("servo") || n.contains("surf") || n.contains("aileron") || n.contains("elev") {
        return ChannelRole::ControlSurface;
    }
    if n.contains("volt") || n.contains("vbat") || n == "v" {
        return ChannelRole::Voltage;
    }
    if n.contains("curr") || n.contains("amp") {
        return ChannelRole::Current;
    }
    ChannelRole::Raw
}

fn find_mapping<'a>(mapping: &'a [ChannelMapping], name: &str) -> Option<&'a ChannelMapping> {
    mapping
        .iter()
        .find(|entry| entry.source_name.eq_ignore_ascii_case(name.trim()))
}

/// Build channels from a numeric table, honouring the supplied mapping.
fn channels_from_table(
    table: &RawTable,
    mapping: &[ChannelMapping],
) -> (Vec<Channel>, Vec<String>) {
    let mut channels = Vec::with_capacity(table.headers.len());
    let mut summary = Vec::new();

    for (index, header) in table.headers.iter().enumerate() {
        let values = table.columns.get(index).cloned().unwrap_or_default();
        let (base_name, declared_unit) = split_unit_from_name(header);
        let explicit = find_mapping(mapping, header).or_else(|| find_mapping(mapping, &base_name));

        let role = explicit
            .map(|entry| entry.role)
            .unwrap_or_else(|| infer_role_from_name(&base_name));
        let sign = explicit.map(|entry| snap_sign(entry.sign)).unwrap_or(1.0);
        let unit = explicit
            .map(|entry| entry.unit.clone())
            .or_else(|| declared_unit.clone())
            .unwrap_or_else(|| role.unit().to_string());
        let confidence = if explicit.is_some() || declared_unit.is_some() {
            UnitConfidence::Declared
        } else if role == ChannelRole::Raw {
            UnitConfidence::Unknown
        } else {
            UnitConfidence::Assumed
        };

        let mut channel = Channel::new(base_name.clone(), role, values);
        channel.unit = unit.clone();
        channel.sign = sign;
        channel.unit_confidence = confidence;
        if explicit.is_some() {
            summary.push(format!(
                "{} <- '{}' (from the supplied mapping)",
                role.code(),
                header
            ));
        } else {
            summary.push(format!(
                "{} <- '{}' (role inferred from the column name)",
                role.code(),
                header
            ));
        }
        channels.push(channel);
    }

    (channels, summary)
}

/// Resolve which channel carries the time axis.
fn resolve_time_channel(
    table: &RawTable,
    options: &CsvImportOptions,
    mapping: &[ChannelMapping],
) -> Option<usize> {
    if let Some(name) = &options.timestamp_column {
        if let Some(index) = table.column_index(name) {
            return Some(index);
        }
    }
    if let Some(entry) = mapping.iter().find(|entry| entry.role == ChannelRole::Time) {
        if let Some(index) = table.column_index(&entry.source_name) {
            return Some(index);
        }
    }
    table
        .headers
        .iter()
        .position(|header| infer_role_from_name(header) == ChannelRole::Time)
}

/// Convert every channel's declared unit into SI.
fn convert_units(channels: &mut [Channel]) -> (usize, Vec<String>) {
    let mut converted = 0usize;
    let mut unrecognised = Vec::new();
    for channel in channels.iter_mut() {
        if channel.role == ChannelRole::Time || channel.unit.is_empty() {
            continue;
        }
        match channel.role.quantity_kind().scale_for(&channel.unit) {
            Some(scale) => {
                if scale.factor != 1.0 || scale.offset != 0.0 {
                    channel.scale = scale;
                    converted += 1;
                }
            }
            None => unrecognised.push(format!("{} [{}]", channel.name, channel.unit)),
        }
    }
    (converted, unrecognised)
}

/// Permute one sensor triple from sensor axes into body axes.
fn apply_axis_mapping(
    channels: &mut [Channel],
    family: ChannelFamily,
    mapping: &AxisMapping,
) -> bool {
    if mapping == &AxisMapping::identity() {
        return false;
    }
    let Some(triple) = ChannelRole::axis_triple(family) else {
        return false;
    };
    let mut indices = [0usize; 3];
    for (slot, role) in triple.iter().enumerate() {
        match channels.iter().position(|channel| channel.role == *role) {
            Some(index) => indices[slot] = index,
            None => return false,
        }
    }
    let count = indices
        .iter()
        .map(|index| channels[*index].values.len())
        .min()
        .unwrap_or(0);
    if count == 0 {
        return false;
    }

    let source = [
        channels[indices[0]].values.clone(),
        channels[indices[1]].values.clone(),
        channels[indices[2]].values.clone(),
    ];
    let mut rotated: [Vec<Real>; 3] = [
        Vec::with_capacity(count),
        Vec::with_capacity(count),
        Vec::with_capacity(count),
    ];
    for ((&x, &y), &z) in source[0].iter().zip(source[1].iter()).zip(source[2].iter()) {
        let body = mapping.sensor_to_body(hex_core::Vec3::new(x, y, z));
        rotated[0].push(body.x);
        rotated[1].push(body.y);
        rotated[2].push(body.z);
    }
    channels[indices[0]].values = std::mem::take(&mut rotated[0]);
    channels[indices[1]].values = std::mem::take(&mut rotated[1]);
    channels[indices[2]].values = std::mem::take(&mut rotated[2]);
    true
}

/// Compute the derived series for a set of channels.
fn derive_channels(channels: &[Channel]) -> Vec<DerivedChannel> {
    let mut derived = Vec::new();

    for (family, magnitude_name, magnitude_unit) in [
        (ChannelFamily::Accelerometer, "accel_magnitude", "m/s^2"),
        (ChannelFamily::Gyroscope, "gyro_magnitude", "rad/s"),
    ] {
        let Some(triple) = ChannelRole::axis_triple(family) else {
            continue;
        };
        let axes: Vec<&Channel> = triple
            .iter()
            .filter_map(|role| channels.iter().find(|channel| channel.role == *role))
            .collect();
        if axes.len() != 3 {
            continue;
        }
        let corrected: Vec<Vec<Real>> = axes.iter().map(|c| c.corrected_values()).collect();
        let vectors = derive_vector_channels(&corrected[0], &corrected[1], &corrected[2]);
        let values: Vec<Real> = vectors.iter().map(|v| v.norm()).collect();
        let mut channel = DerivedChannel::new(
            magnitude_name,
            ChannelRole::Raw,
            values,
            Provenance::Estimated,
        );
        channel.unit = magnitude_unit.to_string();
        channel.notes = format!("Euclidean magnitude of the three {} axes.", family.label());
        derived.push(channel);
    }

    if let Some(pressure) = channels
        .iter()
        .find(|channel| channel.role == ChannelRole::BaroPressure)
    {
        let values = derive_barometric_altitude(
            &pressure.corrected_values(),
            ISA_SEA_LEVEL_PRESSURE,
            ISA_SEA_LEVEL_TEMPERATURE,
        );
        let mut channel = DerivedChannel::new(
            "barometric_altitude",
            ChannelRole::GnssAltitude,
            values,
            Provenance::Estimated,
        );
        channel.unit = "m".to_string();
        channel.notes =
            "ISA pressure altitude from the barometer, using a standard sea-level pressure."
                .to_string();
        derived.push(channel);
    }

    derived
}

/// Run the CSV stages from `Parsed` through `SessionCreated`.
fn run_csv_pipeline(
    log: &mut StageLog,
    text: &str,
    options: &CsvImportOptions,
    mapping: &[ChannelMapping],
    source: SourceInfo,
) -> Result<FlightSession, FlightDataError> {
    let started = Instant::now();
    let table = match parse_csv(text, options) {
        Ok(table) => table,
        Err(error) => {
            log.failed(
                ImportStage::Parsed,
                error.user_message(),
                Some(error.to_string()),
                started.elapsed().as_millis(),
            );
            return Err(error);
        }
    };
    if table.columns.is_empty() {
        let error = FlightDataError::EmptyFile;
        log.failed(
            ImportStage::Parsed,
            error.user_message(),
            None,
            started.elapsed().as_millis(),
        );
        return Err(error);
    }
    let parsed_detail = format!(
        "{} column(s), {} row(s); {} missing cell(s), {} unparsable cell(s)",
        table.headers.len(),
        table.row_count(),
        table.missing_cells,
        table.unparsable_cells
    );
    log.ok(
        ImportStage::Parsed,
        format!("Parsed {} row(s)", table.row_count()),
        Some(parsed_detail),
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    let (mut channels, mapping_summary) = channels_from_table(&table, mapping);
    let time_index = resolve_time_channel(&table, options, mapping);
    let Some(time_index) = time_index else {
        let error = FlightDataError::NoTimestampColumn;
        log.failed(
            ImportStage::ChannelsDiscovered,
            error.user_message(),
            Some(format!("Headers: {}", table.headers.join(", "))),
            started.elapsed().as_millis(),
        );
        return Err(error);
    };
    log.ok(
        ImportStage::ChannelsDiscovered,
        format!("Discovered {} channel(s)", channels.len()),
        Some(format!("Time column is '{}'", table.headers[time_index])),
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    // The time column keeps its role so the UI can show it, and its scale is the
    // timestamp unit so corrected values are seconds.
    let timestamp_factor = options.timestamp_unit.to_seconds_factor().ok_or_else(|| {
        FlightDataError::MappingIncomplete {
            missing: vec!["timestamp tick rate".to_string()],
        }
    });
    let timestamp_factor = match timestamp_factor {
        Ok(factor) => factor,
        Err(error) => {
            log.failed(
                ImportStage::Mapped,
                error.user_message(),
                None,
                started.elapsed().as_millis(),
            );
            return Err(error);
        }
    };
    channels[time_index].role = ChannelRole::Time;
    channels[time_index].unit = "s".to_string();
    channels[time_index].scale = hex_core::UnitScale::new(timestamp_factor, 0.0);
    channels[time_index].unit_confidence = UnitConfidence::Declared;

    let mapped_roles = channels
        .iter()
        .filter(|channel| channel.role != ChannelRole::Raw)
        .count();
    let mut summary = mapping_summary;
    summary.push(format!(
        "{} of {} column(s) carry a recognised role",
        mapped_roles,
        channels.len()
    ));
    log.ok(
        ImportStage::Mapped,
        format!("{} channel(s) mapped", channels.len()),
        Some(summary.join("; ")),
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    let (converted, unrecognised) = convert_units(&mut channels);
    let unit_detail = if unrecognised.is_empty() {
        None
    } else {
        Some(format!("No unit rule for: {}", unrecognised.join(", ")))
    };
    if unrecognised.is_empty() {
        log.ok(
            ImportStage::UnitsConverted,
            format!("{converted} unit conversion(s) applied"),
            unit_detail,
            started.elapsed().as_millis(),
        );
    } else {
        log.warning(
            ImportStage::UnitsConverted,
            format!(
                "{converted} converted, {} unit(s) unrecognised",
                unrecognised.len()
            ),
            unit_detail,
            started.elapsed().as_millis(),
        );
    }

    let started = Instant::now();
    let mut frames_converted = Vec::new();
    for family in [
        ChannelFamily::Accelerometer,
        ChannelFamily::Gyroscope,
        ChannelFamily::Magnetometer,
    ] {
        let axis_mapping = ChannelRole::axis_triple(family)
            .iter()
            .flat_map(|triple| triple.iter())
            .find_map(|role| {
                channels
                    .iter()
                    .find(|channel| channel.role == *role)
                    .and_then(|channel| find_mapping(mapping, &channel.name))
                    .map(|entry| entry.axis_mapping)
            })
            .unwrap_or_else(AxisMapping::identity);
        if apply_axis_mapping(&mut channels, family, &axis_mapping) {
            frames_converted.push(family.label().to_string());
        }
    }
    if frames_converted.is_empty() {
        log.skipped(
            ImportStage::FramesConverted,
            "Sensor axes already match body axes",
            started.elapsed().as_millis(),
        );
    } else {
        log.ok(
            ImportStage::FramesConverted,
            format!("Rotated {}", frames_converted.join(", ")),
            None,
            started.elapsed().as_millis(),
        );
    }

    let started = Instant::now();
    let raw_times = channels[time_index].corrected_values();
    let timestamps = validate_timestamps(
        &raw_times,
        &TimestampValidationOptions {
            expected_rate_hz: None,
            ..Default::default()
        },
    );
    let time_status = if timestamps.has_errors() {
        StageStatus::Warning
    } else {
        StageStatus::Ok
    };
    log.record(
        ImportStage::TimestampsValidated,
        time_status,
        format!(
            "{:.3} Hz effective, {} finding(s)",
            timestamps.analysis.effective_rate_hz,
            timestamps.warnings.len()
        ),
        timestamps
            .warnings
            .first()
            .map(|warning| warning.one_line()),
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    let mut calibration_summary = Vec::new();
    for channel in channels.iter_mut() {
        if channel.role == ChannelRole::Time {
            continue;
        }
        if let Some(entry) = find_mapping(mapping, &channel.name) {
            if !entry.calibration.is_identity() {
                let calibration = entry.calibration;
                for value in channel.values.iter_mut() {
                    *value = calibration.apply(*value);
                }
                calibration_summary.push(format!(
                    "{}: scale {:.6}, offset {:.6}",
                    channel.name, calibration.scale, calibration.offset
                ));
            }
        }
    }
    if calibration_summary.is_empty() {
        log.skipped(
            ImportStage::Calibrated,
            "No channel calibration was supplied",
            started.elapsed().as_millis(),
        );
    } else {
        log.ok(
            ImportStage::Calibrated,
            format!("{} channel(s) calibrated", calibration_summary.len()),
            Some(calibration_summary.join("; ")),
            started.elapsed().as_millis(),
        );
    }

    let started = Instant::now();
    let times = timestamps.corrected_times.clone();
    let derived = derive_channels(&channels);
    if derived.is_empty() {
        log.skipped(
            ImportStage::DerivedGenerated,
            "No derived channel could be computed from the mapped roles",
            started.elapsed().as_millis(),
        );
    } else {
        log.ok(
            ImportStage::DerivedGenerated,
            format!("{} derived channel(s)", derived.len()),
            Some(
                derived
                    .iter()
                    .map(|channel| channel.name.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            started.elapsed().as_millis(),
        );
    }

    let started = Instant::now();
    let quality = assess_quality(&channels, &times, &QualityOptions::default());
    let mut session = FlightSession::new(
        source,
        Frame::default(),
        timestamps.timebase.clone(),
        times,
        channels,
        quality,
        timestamps,
    );
    session.mapping_summary = summary;
    session.calibration_summary = calibration_summary;
    session.derived = derived;
    log.ok(
        ImportStage::SessionCreated,
        format!("Session {} created", session.id),
        Some(format!(
            "{} sample(s) over {:.3} s",
            session.sample_count(),
            session.duration()
        )),
        started.elapsed().as_millis(),
    );

    Ok(session)
}

/// Run the binary stages from `Parsed` through `SessionCreated`.
fn run_binary_pipeline(
    log: &mut StageLog,
    bytes: &[u8],
    mapping: &[ChannelMapping],
    source: SourceInfo,
) -> Result<FlightSession, FlightDataError> {
    let started = Instant::now();
    let decoded = match BinaryReader::read_all(bytes) {
        Ok(decoded) => decoded,
        Err(error) => {
            log.failed(
                ImportStage::Parsed,
                error.user_message(),
                Some(error.to_string()),
                started.elapsed().as_millis(),
            );
            return Err(FlightDataError::Binary(error));
        }
    };
    let integrity = decoded.integrity;
    if integrity.is_clean() {
        log.ok(
            ImportStage::Parsed,
            format!("Decoded {} record(s)", integrity.records_read),
            Some(integrity.summary()),
            started.elapsed().as_millis(),
        );
    } else {
        log.warning(
            ImportStage::Parsed,
            format!("Decoded {} of the log's records", integrity.records_read),
            Some(integrity.summary()),
            started.elapsed().as_millis(),
        );
    }

    let started = Instant::now();
    let mut channels: Vec<Channel> = decoded
        .channels
        .iter()
        .enumerate()
        .map(|(index, descriptor)| {
            let values = decoded.columns.get(index).cloned().unwrap_or_default();
            descriptor.to_channel(values)
        })
        .collect();
    log.ok(
        ImportStage::ChannelsDiscovered,
        format!("Discovered {} channel(s)", channels.len()),
        Some(
            channels
                .iter()
                .map(|channel| format!("{} [{}]", channel.name, channel.unit))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    let mut summary = Vec::new();
    for channel in channels.iter_mut() {
        let role = find_mapping(mapping, &channel.name)
            .map(|entry| entry.role)
            .unwrap_or(channel.role);
        let sign = find_mapping(mapping, &channel.name)
            .map(|entry| snap_sign(entry.sign))
            .unwrap_or(1.0);
        channel.role = role;
        channel.sign = sign;
        summary.push(format!("{} <- '{}'", role.code(), channel.name));
    }
    log.ok(
        ImportStage::Mapped,
        format!("{} channel(s) mapped", channels.len()),
        Some(summary.join("; ")),
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    let scaled = channels
        .iter()
        .filter(|channel| channel.scale != hex_core::UnitScale::IDENTITY)
        .count();
    log.ok(
        ImportStage::UnitsConverted,
        format!("{scaled} stored scale(s) applied"),
        Some("Binary descriptors carry their own scale factor and offset.".to_string()),
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    log.skipped(
        ImportStage::FramesConverted,
        "Binary records store body axes directly",
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    let timestamps = validate_timestamps(&decoded.times, &TimestampValidationOptions::default());
    let status = if timestamps.has_errors() {
        StageStatus::Warning
    } else {
        StageStatus::Ok
    };
    log.record(
        ImportStage::TimestampsValidated,
        status,
        format!(
            "{:.3} Hz effective, {} finding(s)",
            timestamps.analysis.effective_rate_hz,
            timestamps.warnings.len()
        ),
        timestamps
            .warnings
            .first()
            .map(|warning| warning.one_line()),
        started.elapsed().as_millis(),
    );

    let started = Instant::now();
    let mut calibration_summary = Vec::new();
    for channel in channels.iter_mut() {
        if let Some(entry) = find_mapping(mapping, &channel.name) {
            if !entry.calibration.is_identity() {
                let calibration = entry.calibration;
                for value in channel.values.iter_mut() {
                    *value = calibration.apply(*value);
                }
                calibration_summary.push(format!(
                    "{}: scale {:.6}, offset {:.6}",
                    channel.name, calibration.scale, calibration.offset
                ));
            }
        }
    }
    if calibration_summary.is_empty() {
        log.skipped(
            ImportStage::Calibrated,
            "No channel calibration was supplied",
            started.elapsed().as_millis(),
        );
    } else {
        log.ok(
            ImportStage::Calibrated,
            format!("{} channel(s) calibrated", calibration_summary.len()),
            Some(calibration_summary.join("; ")),
            started.elapsed().as_millis(),
        );
    }

    let started = Instant::now();
    let times = timestamps.corrected_times.clone();
    let derived = derive_channels(&channels);
    if derived.is_empty() {
        log.skipped(
            ImportStage::DerivedGenerated,
            "No derived channel could be computed from the mapped roles",
            started.elapsed().as_millis(),
        );
    } else {
        log.ok(
            ImportStage::DerivedGenerated,
            format!("{} derived channel(s)", derived.len()),
            None,
            started.elapsed().as_millis(),
        );
    }

    let started = Instant::now();
    let mut quality = assess_quality(&channels, &times, &QualityOptions::default());
    if integrity.sequence_gaps > 0 || integrity.crc_failures > 0 {
        let start = times.first().copied().unwrap_or(0.0);
        let end = times.last().copied().unwrap_or(0.0);
        quality.flags.push(sequence_gap_flag(
            integrity.sequence_gaps + integrity.crc_failures,
            start,
            end,
        ));
        quality.recompute();
    }
    let mut session = FlightSession::new(
        source,
        Frame::default(),
        timestamps.timebase.clone(),
        times,
        channels,
        quality,
        timestamps,
    );
    session.mapping_summary = summary;
    session.calibration_summary = calibration_summary;
    session.derived = derived;
    log.ok(
        ImportStage::SessionCreated,
        format!("Session {} created", session.id),
        Some(format!(
            "{} sample(s) over {:.3} s",
            session.sample_count(),
            session.duration()
        )),
        started.elapsed().as_millis(),
    );

    Ok(session)
}

fn finish(log: StageLog, session: Result<FlightSession, FlightDataError>) -> ImportOutcome {
    match session {
        Ok(session) => ImportOutcome {
            session: Some(session),
            report: log.finish(),
            error: None,
        },
        Err(error) => {
            let report = log.finish();
            ImportOutcome {
                session: None,
                report,
                error: Some(error),
            }
        }
    }
}

/// Import CSV text, returning the session and the stage-by-stage report.
pub fn import_csv(
    text: &str,
    options: &CsvImportOptions,
    mapping: &[ChannelMapping],
) -> Result<(FlightSession, ImportReport), FlightDataError> {
    import_csv_outcome(text, options, mapping, "in-memory CSV").into_result()
}

/// Import CSV text and keep the report even when the pipeline fails.
pub fn import_csv_outcome(
    text: &str,
    options: &CsvImportOptions,
    mapping: &[ChannelMapping],
    source_label: &str,
) -> ImportOutcome {
    let mut log = StageLog::new();
    let started = Instant::now();
    let source = SourceInfo::new(source_label, SourceFormat::Csv, text.len() as u64)
        .with_content(text.as_bytes());
    log.ok(
        ImportStage::Selected,
        format!("Selected '{}'", source.file_name),
        Some(format!(
            "{} byte(s), hash {}",
            source.byte_length, source.content_hash
        )),
        started.elapsed().as_millis(),
    );
    let session = run_csv_pipeline(&mut log, text, options, mapping, source);
    finish(log, session)
}

/// Import a `.hlog` binary log, returning the session and the report.
pub fn import_binary(
    bytes: &[u8],
    mapping: &[ChannelMapping],
) -> Result<(FlightSession, ImportReport), FlightDataError> {
    import_binary_outcome(bytes, mapping, "in-memory binary log").into_result()
}

/// Import a binary log and keep the report even when the pipeline fails.
pub fn import_binary_outcome(
    bytes: &[u8],
    mapping: &[ChannelMapping],
    source_label: &str,
) -> ImportOutcome {
    let mut log = StageLog::new();
    let started = Instant::now();
    let source = SourceInfo::new(
        source_label,
        SourceFormat::HexaDofBinary,
        bytes.len() as u64,
    )
    .with_content(bytes);
    log.ok(
        ImportStage::Selected,
        format!("Selected '{}'", source.file_name),
        Some(format!(
            "{} byte(s), hash {}",
            source.byte_length, source.content_hash
        )),
        started.elapsed().as_millis(),
    );
    let session = run_binary_pipeline(&mut log, bytes, mapping, source);
    finish(log, session)
}

/// Import any supported file, choosing the codec from its extension.
pub fn import_path(
    path: impl AsRef<Path>,
    options: &CsvImportOptions,
    mapping: &[ChannelMapping],
) -> Result<(FlightSession, ImportReport), FlightDataError> {
    import_path_outcome(path, options, mapping).into_result()
}

/// Import a file and keep the report even when the pipeline fails.
pub fn import_path_outcome(
    path: impl AsRef<Path>,
    options: &CsvImportOptions,
    mapping: &[ChannelMapping],
) -> ImportOutcome {
    let path = path.as_ref();
    let label = path.to_string_lossy().to_string();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string();
    let mut log = StageLog::new();
    let started = Instant::now();

    let Some(format) = SourceFormat::from_extension(&extension) else {
        let error = FlightDataError::UnsupportedFormat { extension };
        log.failed(
            ImportStage::Selected,
            error.user_message(),
            Some(error.to_string()),
            started.elapsed().as_millis(),
        );
        return finish(log, Err(error));
    };

    match format {
        SourceFormat::Csv => {
            let text = match std::fs::read_to_string(path) {
                Ok(text) => text,
                Err(error) => {
                    let error = FlightDataError::from(error);
                    log.failed(
                        ImportStage::Selected,
                        error.user_message(),
                        Some(error.to_string()),
                        started.elapsed().as_millis(),
                    );
                    return finish(log, Err(error));
                }
            };
            let source = SourceInfo::new(label, SourceFormat::Csv, text.len() as u64)
                .with_content(text.as_bytes());
            log.ok(
                ImportStage::Selected,
                format!("Selected '{}'", source.file_name),
                Some(format!("{} byte(s)", source.byte_length)),
                started.elapsed().as_millis(),
            );
            let session = run_csv_pipeline(&mut log, &text, options, mapping, source);
            finish(log, session)
        }
        SourceFormat::HexaDofBinary => {
            let bytes = match std::fs::read(path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    let error = FlightDataError::from(error);
                    log.failed(
                        ImportStage::Selected,
                        error.user_message(),
                        Some(error.to_string()),
                        started.elapsed().as_millis(),
                    );
                    return finish(log, Err(error));
                }
            };
            let source = SourceInfo::new(label, SourceFormat::HexaDofBinary, bytes.len() as u64)
                .with_content(&bytes);
            log.ok(
                ImportStage::Selected,
                format!("Selected '{}'", source.file_name),
                Some(format!("{} byte(s)", source.byte_length)),
                started.elapsed().as_millis(),
            );
            let session = run_binary_pipeline(&mut log, &bytes, mapping, source);
            finish(log, session)
        }
        other => {
            let error = FlightDataError::UnsupportedFormat {
                extension: extension.clone(),
            };
            log.failed(
                ImportStage::Selected,
                format!(
                    "{} files are not supported by this build ({})",
                    other.label(),
                    error
                ),
                None,
                started.elapsed().as_millis(),
            );
            finish(log, Err(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binary::{BinaryWriter, ChannelDescriptor, LogHeader};

    fn csv_text(rows: usize) -> String {
        let mut text = String::from("time,accel_x [m/s^2],accel_y,accel_z\n");
        for i in 0..rows {
            let t = i as Real * 0.01;
            text.push_str(&format!("{t},{t},{t},{t}\n"));
        }
        text
    }

    fn binary_log(samples: usize, per_record: usize) -> Vec<u8> {
        let channels = vec![
            ChannelDescriptor::new("accel_x", ChannelRole::AccelX, 1.0, 0.0),
            ChannelDescriptor::new("accel_y", ChannelRole::AccelY, 1.0, 0.0),
            ChannelDescriptor::new("accel_z", ChannelRole::AccelZ, 1.0, 0.0),
        ];
        let mut writer = BinaryWriter::new(LogHeader::new(0.01), channels);
        writer.max_samples_per_record = per_record;
        for i in 0..samples {
            let t = i as Real * 0.01;
            writer.push_sample(t, &[t, t, t]).unwrap();
        }
        writer.finish().unwrap()
    }

    #[test]
    fn full_csv_import_runs_every_stage_without_failure() {
        let text = csv_text(50);
        let (session, report) = import_csv(&text, &CsvImportOptions::default(), &[]).unwrap();
        assert_eq!(report.stages.len(), ImportStage::all().len());
        assert!(report.succeeded);
        assert_eq!(report.count(StageStatus::Failed), 0);
        assert!(report.primary_error.is_none());
        assert_eq!(session.sample_count(), 50);
        assert_eq!(session.times[0], 0.0);
        assert!(session.channel(ChannelRole::Time).is_some());
        assert!(session.channel(ChannelRole::AccelX).is_some());
        assert!(session
            .derived
            .iter()
            .any(|channel| channel.name == "accel_magnitude"));
        assert_eq!(
            report.stage(ImportStage::FramesConverted).unwrap().status,
            StageStatus::Skipped
        );
        assert!(report.summary().contains("Import finished"));
    }

    #[test]
    fn a_csv_without_a_time_column_fails_with_the_documented_error() {
        let text = "accel_x,accel_y\n1.0,2.0\n3.0,4.0\n";
        let error = import_csv(text, &CsvImportOptions::default(), &[]).unwrap_err();
        assert!(matches!(error, FlightDataError::NoTimestampColumn));
        assert!(error.user_message().contains("looks like a timestamp"));

        let outcome = import_csv_outcome(text, &CsvImportOptions::default(), &[], "memory.csv");
        assert!(outcome.error.is_some());
        assert!(!outcome.report.succeeded);
        assert!(outcome.report.primary_error.is_some());
        assert_eq!(
            outcome.report.stages.last().unwrap().stage,
            ImportStage::ChannelsDiscovered
        );
        assert_eq!(
            outcome.report.stages.last().unwrap().status,
            StageStatus::Failed
        );
    }

    #[test]
    fn declared_units_are_converted_to_si() {
        let mut text = String::from("timestamp,accel_x [g],accel_y [g],accel_z [g]\n");
        for i in 0..10 {
            text.push_str(&format!("{i},1.0,1.0,1.0\n"));
        }
        let (session, report) = import_csv(&text, &CsvImportOptions::default(), &[]).unwrap();
        let accel = session.channel(ChannelRole::AccelX).unwrap();
        assert_eq!(accel.unit, "g");
        assert!((accel.corrected_values()[0] - hex_core::STANDARD_GRAVITY).abs() < 1e-9);
        let stage = report.stage(ImportStage::UnitsConverted).unwrap();
        assert_eq!(stage.status, StageStatus::Ok);
        assert!(stage.message.contains("3 unit conversion"));
    }

    #[test]
    fn unrecognised_units_produce_a_warning_stage() {
        let mut text = String::from("time,accel_x [furlongs]\n");
        for i in 0..10 {
            text.push_str(&format!("{i},1.0\n"));
        }
        let (_session, report) = import_csv(&text, &CsvImportOptions::default(), &[]).unwrap();
        let stage = report.stage(ImportStage::UnitsConverted).unwrap();
        assert_eq!(stage.status, StageStatus::Warning);
        assert!(stage.detail.as_deref().unwrap().contains("furlongs"));
        assert!(!report.is_clean());
    }

    #[test]
    fn mapping_overrides_role_unit_and_sign() {
        let text = csv_text(20);
        let mapping = vec![
            ChannelMapping::new("accel_x", ChannelRole::AccelY).with_sign(-1.0),
            ChannelMapping::new("time", ChannelRole::Time),
        ];
        let (session, _report) = import_csv(&text, &CsvImportOptions::default(), &mapping).unwrap();
        let channel = session.channel(ChannelRole::AccelY).unwrap();
        assert_eq!(channel.name, "accel_x");
        assert_eq!(channel.sign, -1.0);
        assert!(channel.corrected_values()[1] < 0.0);
        assert!(session
            .mapping_summary
            .iter()
            .any(|line| line.contains("from the supplied mapping")));
    }

    #[test]
    fn calibration_is_applied_and_summarised() {
        let text = csv_text(20);
        let mapping = vec![ChannelMapping::new("accel_z", ChannelRole::AccelZ)
            .with_calibration(ChannelCalibration::new(2.0, 1.0))];
        let (session, report) = import_csv(&text, &CsvImportOptions::default(), &mapping).unwrap();
        let channel = session.channel(ChannelRole::AccelZ).unwrap();
        // Original raw value at sample 1 is 0.01; calibrated it becomes 1.02.
        assert!((channel.values[1] - 1.02).abs() < 1e-12);
        assert_eq!(
            report.stage(ImportStage::Calibrated).unwrap().status,
            StageStatus::Ok
        );
        assert_eq!(session.calibration_summary.len(), 1);
    }

    #[test]
    fn calibration_defaults_to_disabled_and_passes_values_through() {
        let calibration = ChannelCalibration::new(3.0, 4.0);
        assert!((calibration.apply(2.0) - 10.0).abs() < 1e-12);
        let default = ChannelCalibration::default();
        assert!((default.apply(2.0) - 2.0).abs() < 1e-12);
        assert!(default.is_identity());
        assert!(ChannelCalibration {
            enabled: true,
            scale: 1.0,
            offset: 0.0,
        }
        .is_identity());
    }

    #[test]
    fn binary_import_produces_a_session_with_the_same_stage_count() {
        let bytes = binary_log(100, 25);
        let (session, report) = import_binary(&bytes, &[]).unwrap();
        assert_eq!(report.stages.len(), ImportStage::all().len());
        assert!(report.succeeded);
        assert_eq!(session.sample_count(), 100);
        assert_eq!(session.channels.len(), 3);
        assert!(session.channel(ChannelRole::AccelZ).is_some());
        assert_eq!(session.source.format, SourceFormat::HexaDofBinary);
        assert!(session.source.original_preserved);
    }

    #[test]
    fn binary_import_reports_integrity_problems_as_flags() {
        let bytes = binary_log(6, 2);
        let record_len = 20 + 2 * 3 * 8 + 4;
        let first = 66 + 3 * 52;
        let mut damaged = Vec::new();
        damaged.extend_from_slice(&bytes[..first + record_len]);
        damaged.extend_from_slice(&bytes[first + 2 * record_len..]);

        let (session, report) = import_binary(&damaged, &[]).unwrap();
        assert_eq!(
            report.stage(ImportStage::Parsed).unwrap().status,
            StageStatus::Warning
        );
        assert!(session.quality.has_code("packet.sequence_gap"));
    }

    #[test]
    fn a_damaged_binary_log_fails_at_the_parsed_stage() {
        let mut bytes = binary_log(4, 2);
        bytes[0] = 0;
        let outcome = import_binary_outcome(&bytes, &[], "damaged.hlog");
        assert!(matches!(outcome.error, Some(FlightDataError::Binary(_))));
        assert!(!outcome.report.succeeded);
        assert_eq!(
            outcome.report.stages.last().unwrap().stage,
            ImportStage::Parsed
        );
    }

    #[test]
    fn import_path_dispatches_on_the_extension() {
        let directory = std::env::temp_dir().join("hex-flight-data-tests");
        std::fs::create_dir_all(&directory).unwrap();

        let csv_path = directory.join("dispatch.csv");
        std::fs::write(&csv_path, csv_text(12)).unwrap();
        let (session, report) = import_path(&csv_path, &CsvImportOptions::default(), &[]).unwrap();
        assert_eq!(session.source.format, SourceFormat::Csv);
        assert_eq!(session.source.file_name, "dispatch.csv");
        assert_eq!(report.stages.len(), ImportStage::all().len());

        let hlog_path = directory.join("dispatch.hlog");
        std::fs::write(&hlog_path, binary_log(30, 10)).unwrap();
        let (session, _report) =
            import_path(&hlog_path, &CsvImportOptions::default(), &[]).unwrap();
        assert_eq!(session.source.format, SourceFormat::HexaDofBinary);
        assert_eq!(session.sample_count(), 30);

        let bad_path = directory.join("dispatch.exe");
        std::fs::write(&bad_path, b"nope").unwrap();
        let error = import_path(&bad_path, &CsvImportOptions::default(), &[]).unwrap_err();
        assert!(matches!(error, FlightDataError::UnsupportedFormat { .. }));

        let missing = directory.join("does-not-exist.csv");
        let outcome = import_path_outcome(&missing, &CsvImportOptions::default(), &[]);
        assert!(matches!(outcome.error, Some(FlightDataError::Io { .. })));
        assert_eq!(
            outcome.report.stages.last().unwrap().status,
            StageStatus::Failed
        );
    }

    #[test]
    fn role_inference_covers_the_common_logger_names() {
        assert_eq!(infer_role_from_name("time_s"), ChannelRole::Time);
        assert_eq!(infer_role_from_name("t"), ChannelRole::Time);
        assert_eq!(infer_role_from_name("accel_x"), ChannelRole::AccelX);
        assert_eq!(infer_role_from_name("Ax"), ChannelRole::AccelX);
        assert_eq!(infer_role_from_name("gyroY"), ChannelRole::GyroY);
        // The short axis names the accelerometer already accepted, and the
        // angular rate names an aerospace log uses for the same axes.
        assert_eq!(infer_role_from_name("gx"), ChannelRole::GyroX);
        assert_eq!(infer_role_from_name("Gy"), ChannelRole::GyroY);
        assert_eq!(infer_role_from_name("gz"), ChannelRole::GyroZ);
        assert_eq!(infer_role_from_name("wx"), ChannelRole::GyroX);
        assert_eq!(infer_role_from_name("wy"), ChannelRole::GyroY);
        assert_eq!(infer_role_from_name("wz"), ChannelRole::GyroZ);
        assert_eq!(infer_role_from_name("magnitude"), ChannelRole::Raw);
        assert_eq!(infer_role_from_name("mag_z"), ChannelRole::MagZ);
        assert_eq!(
            infer_role_from_name("baro_pressure"),
            ChannelRole::BaroPressure
        );
        assert_eq!(
            infer_role_from_name("baro_temp"),
            ChannelRole::BaroTemperature
        );
        assert_eq!(infer_role_from_name("latitude"), ChannelRole::GnssLatitude);
        assert_eq!(
            infer_role_from_name("longitude"),
            ChannelRole::GnssLongitude
        );
        assert_eq!(infer_role_from_name("altitude"), ChannelRole::GnssAltitude);
        assert_eq!(infer_role_from_name("ground_speed"), ChannelRole::GnssSpeed);
        assert_eq!(infer_role_from_name("q_z"), ChannelRole::QuaternionZ);
        assert_eq!(infer_role_from_name("throttle"), ChannelRole::MotorThrottle);
        assert_eq!(infer_role_from_name("vbat"), ChannelRole::Voltage);
        assert_eq!(infer_role_from_name("current_a"), ChannelRole::Current);
        assert_eq!(infer_role_from_name("acceleration"), ChannelRole::Raw);
        assert_eq!(infer_role_from_name("accel_magnitude"), ChannelRole::Raw);
        assert_eq!(infer_role_from_name("wibble"), ChannelRole::Raw);
        assert_eq!(infer_role_from_name(""), ChannelRole::Raw);
    }

    #[test]
    fn unit_annotations_are_split_from_column_names() {
        assert_eq!(
            split_unit_from_name("accel_x [g]"),
            ("accel_x".to_string(), Some("g".to_string()))
        );
        assert_eq!(
            split_unit_from_name("altitude (m)"),
            ("altitude".to_string(), Some("m".to_string()))
        );
        assert_eq!(split_unit_from_name("plain"), ("plain".to_string(), None));
        assert_eq!(split_unit_from_name("[]"), ("[]".to_string(), None));
    }

    #[test]
    fn axis_mapping_rotates_a_sensor_triple() {
        let mut channels = vec![
            Channel::new("ax", ChannelRole::AccelX, vec![1.0, 2.0]),
            Channel::new("ay", ChannelRole::AccelY, vec![3.0, 4.0]),
            Channel::new("az", ChannelRole::AccelZ, vec![5.0, 6.0]),
        ];
        let mapping = AxisMapping::from_sensor_axes(hex_core::SensorAxes::XyzNegZ);
        assert!(apply_axis_mapping(
            &mut channels,
            ChannelFamily::Accelerometer,
            &mapping
        ));
        assert_eq!(channels[2].values, vec![-5.0, -6.0]);
        assert_eq!(channels[0].values, vec![1.0, 2.0]);
        assert!(!apply_axis_mapping(
            &mut channels,
            ChannelFamily::Magnetometer,
            &mapping
        ));
    }

    #[test]
    fn a_failed_parse_stops_the_pipeline_at_the_first_stage() {
        let outcome = import_csv_outcome("", &CsvImportOptions::default(), &[], "empty.csv");
        assert!(matches!(outcome.error, Some(FlightDataError::EmptyFile)));
        assert_eq!(outcome.report.stages.len(), 2);
        assert_eq!(outcome.report.stages[0].stage, ImportStage::Selected);
        assert_eq!(outcome.report.stages[1].stage, ImportStage::Parsed);
        assert!(!outcome.report.succeeded);
        assert!(outcome
            .report
            .primary_error
            .as_deref()
            .unwrap()
            .contains("no data"));
    }

    #[test]
    fn stage_metadata_is_stable() {
        assert_eq!(ImportStage::all().len(), 10);
        assert_eq!(ImportStage::Selected.code(), "selected");
        assert_eq!(ImportStage::SessionCreated.label(), "Session created");
        assert_eq!(StageStatus::Failed.label(), "Failed");
        assert!(StageStatus::Failed.is_failure());
        assert!(!StageStatus::Warning.is_failure());

        let report = ImportReport::new();
        assert!(report.succeeded);
        assert_eq!(report.count(StageStatus::Ok), 0);
        assert!(report.stage(ImportStage::Parsed).is_none());
        assert!(report.is_clean());
    }
}
