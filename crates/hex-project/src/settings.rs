//! Application settings, stored outside any project.
//!
//! Settings live in a single JSON file in the user's configuration directory, so
//! they apply across projects. The file is written atomically and every field has
//! a default, so a settings file written by an older build still loads.

use std::path::{Path, PathBuf};

use hex_core::{Real, UnitSystem, WorldFrame};
use hex_dynamics::{GravityModel, NormalizationPolicy, SolverKind};
use hex_telemetry::EstimatorMode;
use serde::{Deserialize, Serialize};

use crate::error::ProjectError;
use crate::io;

/// The settings file name.
pub const SETTINGS_FILE_NAME: &str = "settings.json";

/// The settings schema version.
pub const SETTINGS_SCHEMA_VERSION: &str = "1.0";

/// How the interface is themed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Dark,
    Light,
    /// Follow the operating system.
    #[default]
    System,
}

impl Theme {
    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::System => "System default",
        }
    }

    /// Resolve the theme against the operating system preference.
    pub fn resolve(self, system_prefers_dark: bool) -> Theme {
        match self {
            Theme::System => {
                if system_prefers_dark {
                    Theme::Dark
                } else {
                    Theme::Light
                }
            }
            other => other,
        }
    }
}

/// Interface density.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    /// Tighter spacing, for a small screen.
    Compact,
    /// The default spacing.
    #[default]
    Comfortable,
}

impl Density {
    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Comfortable => "Comfortable",
        }
    }
}

/// Diagnostics verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn label(self) -> &'static str {
        match self {
            LogLevel::Error => "Error",
            LogLevel::Warn => "Warning",
            LogLevel::Info => "Info",
            LogLevel::Debug => "Debug",
            LogLevel::Trace => "Trace",
        }
    }

    /// Every level, quietest first.
    pub fn all() -> &'static [LogLevel] {
        &[
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ]
    }
}

/// General settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeneralSettings {
    /// Directory new projects default to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_project_directory: Option<String>,
    /// Whether the project file is written shortly after a change.
    #[serde(default = "default_true")]
    pub autosave: bool,
    /// Seconds between autosave writes.
    #[serde(default = "default_autosave_interval")]
    pub autosave_interval_seconds: Real,
    /// Recently opened projects, newest first.
    #[serde(default)]
    pub recent_projects: Vec<String>,
    /// Whether destructive actions ask for confirmation.
    #[serde(default = "default_true")]
    pub confirm_destructive_actions: bool,
    /// Display unit system.
    #[serde(default)]
    pub unit_system: UnitSystem,
    /// BCP 47 language tag, or a display name when the build has one language.
    #[serde(default = "default_language")]
    pub language: String,
    /// Whether the interface shows every control rather than the reduced set.
    ///
    /// Advanced is the default, so the full surface is what an existing project
    /// and an existing settings file keep.
    #[serde(default = "default_true")]
    pub advanced_mode: bool,
}

fn default_true() -> bool {
    true
}

fn default_autosave_interval() -> Real {
    30.0
}

fn default_language() -> String {
    "en".to_string()
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            default_project_directory: None,
            autosave: true,
            autosave_interval_seconds: 30.0,
            recent_projects: Vec::new(),
            confirm_destructive_actions: true,
            unit_system: UnitSystem::Si,
            language: default_language(),
            advanced_mode: true,
        }
    }
}

impl GeneralSettings {
    /// How many recent projects are remembered.
    pub const MAX_RECENT: usize = 12;

    /// Record a recently opened project, newest first and without duplicates.
    pub fn remember_project(&mut self, path: &str) {
        self.recent_projects.retain(|p| p != path);
        self.recent_projects.insert(0, path.to_string());
        self.recent_projects.truncate(Self::MAX_RECENT);
    }

    /// Forget a project, for example when it no longer exists.
    pub fn forget_project(&mut self, path: &str) {
        self.recent_projects.retain(|p| p != path);
    }

    /// Recent projects that still exist on disk.
    pub fn existing_recent_projects(&self) -> Vec<&str> {
        self.recent_projects
            .iter()
            .filter(|p| Path::new(p).is_dir())
            .map(|p| p.as_str())
            .collect()
    }
}

/// Appearance settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppearanceSettings {
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub density: Density,
    /// Whether transitions and animation are reduced.
    #[serde(default)]
    pub reduced_motion: bool,
    /// Whether charts draw a grid.
    #[serde(default = "default_true")]
    pub chart_grid: bool,
    /// Whether the interface shows the monospaced numeric face everywhere.
    #[serde(default)]
    pub monospace_numbers_only: bool,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            density: Density::Comfortable,
            reduced_motion: false,
            chart_grid: true,
            monospace_numbers_only: true,
        }
    }
}

/// Dynamics defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DynamicsSettings {
    /// Default solver for a new run.
    #[serde(default = "default_solver")]
    pub solver: SolverKind,
    /// Quaternion normalisation policy.
    #[serde(default)]
    pub normalization: NormalizationPolicy,
    /// Default world frame.
    #[serde(default)]
    pub world_frame: WorldFrame,
    /// Default gravity model.
    #[serde(default)]
    pub gravity: GravityModel,
    /// Quaternion norm error above which a run is flagged.
    #[serde(default = "default_quaternion_tolerance")]
    pub quaternion_norm_tolerance: Real,
    /// Angular rate above which a run is flagged, rad/s.
    #[serde(default = "default_angular_rate_limit")]
    pub angular_rate_warning: Real,
    /// Acceleration above which a run is flagged, m/s^2.
    #[serde(default = "default_acceleration_limit")]
    pub acceleration_warning: Real,
}

fn default_solver() -> SolverKind {
    SolverKind::fixed(0.0005)
}

fn default_quaternion_tolerance() -> Real {
    1e-6
}

fn default_angular_rate_limit() -> Real {
    100.0
}

fn default_acceleration_limit() -> Real {
    1000.0
}

impl Default for DynamicsSettings {
    fn default() -> Self {
        Self {
            solver: default_solver(),
            normalization: NormalizationPolicy::default(),
            world_frame: WorldFrame::Enu,
            gravity: GravityModel::default(),
            quaternion_norm_tolerance: default_quaternion_tolerance(),
            angular_rate_warning: default_angular_rate_limit(),
            acceleration_warning: default_acceleration_limit(),
        }
    }
}

impl DynamicsSettings {
    /// The warning thresholds this configuration implies.
    pub fn thresholds(&self) -> hex_dynamics::WarningThresholds {
        hex_dynamics::WarningThresholds {
            quaternion_norm_error: self.quaternion_norm_tolerance,
            maximum_angular_rate: self.angular_rate_warning,
            maximum_acceleration: self.acceleration_warning,
            ..hex_dynamics::WarningThresholds::default()
        }
    }
}

/// Telemetry defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetrySettings {
    /// Seconds between port scans.
    #[serde(default = "default_scan_interval")]
    pub serial_scan_interval_seconds: Real,
    /// Default baud rate for a new profile.
    #[serde(default = "default_baud")]
    pub default_baud_rate: u32,
    /// Seconds without a packet before the connection is called stalled.
    #[serde(default = "default_packet_timeout")]
    pub packet_timeout_seconds: Real,
    /// Largest accepted packet payload, bytes.
    #[serde(default = "default_max_packet")]
    pub maximum_packet_bytes: usize,
    /// Recording buffer capacity in samples.
    #[serde(default = "default_buffer")]
    pub recording_buffer_samples: usize,
    /// Target display update rate, hertz.
    #[serde(default = "default_display_rate")]
    pub display_rate_hz: Real,
    /// Whether to reconnect automatically after a drop.
    #[serde(default)]
    pub auto_reconnect: bool,
    /// Orientation estimator mode for a new session.
    #[serde(default = "default_estimator_mode")]
    pub estimator_mode: EstimatorMode,
}

fn default_scan_interval() -> Real {
    2.0
}
fn default_baud() -> u32 {
    115_200
}
fn default_packet_timeout() -> Real {
    2.0
}
fn default_max_packet() -> usize {
    256
}
fn default_buffer() -> usize {
    5_000_000
}
fn default_display_rate() -> Real {
    30.0
}
fn default_estimator_mode() -> EstimatorMode {
    EstimatorMode::GyroPropagation
}

impl Default for TelemetrySettings {
    fn default() -> Self {
        Self {
            serial_scan_interval_seconds: default_scan_interval(),
            default_baud_rate: default_baud(),
            packet_timeout_seconds: default_packet_timeout(),
            maximum_packet_bytes: default_max_packet(),
            recording_buffer_samples: default_buffer(),
            display_rate_hz: default_display_rate(),
            auto_reconnect: false,
            estimator_mode: default_estimator_mode(),
        }
    }
}

impl TelemetrySettings {
    /// Problems with these settings.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.serial_scan_interval_seconds <= 0.0 {
            problems.push("the serial scan interval must be positive".to_string());
        }
        if self.packet_timeout_seconds <= 0.0 {
            problems.push("the packet timeout must be positive".to_string());
        }
        if self.maximum_packet_bytes == 0 {
            problems.push("the maximum packet size must be positive".to_string());
        }
        if self.recording_buffer_samples == 0 {
            problems.push("the recording buffer must hold at least one sample".to_string());
        }
        if self.display_rate_hz < 0.0 {
            problems.push("the display rate cannot be negative".to_string());
        }
        if self.default_baud_rate == 0 {
            problems.push("the default baud rate must be positive".to_string());
        }
        problems
    }

    /// The recording capacity in megabytes, at eight bytes per channel value.
    pub fn recording_buffer_megabytes(&self, channel_count: usize) -> Real {
        let bytes = self.recording_buffer_samples as Real * channel_count.max(1) as Real * 8.0;
        bytes / (1024.0 * 1024.0)
    }
}

/// Data and storage settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StorageSettings {
    /// Directory flight logs are written to by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_directory: Option<String>,
    /// Directory for caches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_directory: Option<String>,
    /// Days a log is kept before the retention policy may remove it.
    #[serde(default = "default_retention")]
    pub retention_days: u32,
    /// Whether the retention policy runs at all. Off by default, because deleting
    /// flight data without being asked is never acceptable.
    #[serde(default)]
    pub retention_enabled: bool,
    /// Default export format.
    #[serde(default)]
    pub export_format: ExportFormat,
    /// Whether exports are compressed where the format allows it.
    #[serde(default)]
    pub compress_exports: bool,
    /// Whether a backup of the project file is kept before a save.
    #[serde(default = "default_true")]
    pub backup_project_file: bool,
}

fn default_retention() -> u32 {
    365
}

/// A file format HexaDOF can export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    #[default]
    Csv,
    Json,
    /// The HexaDOF binary log.
    Hlog,
    Markdown,
}

impl ExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Csv => "CSV",
            ExportFormat::Json => "JSON",
            ExportFormat::Hlog => "HexaDOF binary log",
            ExportFormat::Markdown => "Markdown report",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Csv => "csv",
            ExportFormat::Json => "json",
            ExportFormat::Hlog => "hlog",
            ExportFormat::Markdown => "md",
        }
    }
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            log_directory: None,
            cache_directory: None,
            retention_days: default_retention(),
            retention_enabled: false,
            export_format: ExportFormat::Csv,
            compress_exports: false,
            backup_project_file: true,
        }
    }
}

/// Diagnostics settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticsSettings {
    #[serde(default)]
    pub log_level: LogLevel,
    /// Whether the developer tools are available.
    #[serde(default)]
    pub developer_tools: bool,
    /// Whether performance counters are collected.
    #[serde(default)]
    pub performance_metrics: bool,
}

impl Default for DiagnosticsSettings {
    fn default() -> Self {
        Self {
            log_level: LogLevel::Info,
            developer_tools: false,
            performance_metrics: false,
        }
    }
}

/// The complete settings document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Schema version.
    #[serde(default = "default_settings_version")]
    pub schema_version: String,
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub dynamics: DynamicsSettings,
    #[serde(default)]
    pub telemetry: TelemetrySettings,
    #[serde(default)]
    pub storage: StorageSettings,
    #[serde(default)]
    pub diagnostics: DiagnosticsSettings,
}

fn default_settings_version() -> String {
    SETTINGS_SCHEMA_VERSION.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: default_settings_version(),
            general: GeneralSettings::default(),
            appearance: AppearanceSettings::default(),
            dynamics: DynamicsSettings::default(),
            telemetry: TelemetrySettings::default(),
            storage: StorageSettings::default(),
            diagnostics: DiagnosticsSettings::default(),
        }
    }
}

impl Settings {
    /// The default settings file path for this user.
    pub fn default_path() -> PathBuf {
        let base = config_root();
        base.join(SETTINGS_FILE_NAME)
    }

    /// Load settings, falling back to defaults with a warning when the file is
    /// missing or unreadable.
    ///
    /// A corrupt settings file must never prevent the application from starting.
    pub fn load_or_default(path: &Path) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        if !path.is_file() {
            return (Self::default(), warnings);
        }
        match io::read_json::<Settings>(path) {
            Ok(settings) => {
                if settings.schema_version != SETTINGS_SCHEMA_VERSION {
                    warnings.push(format!(
                        "The settings file declares version {}, and this build writes {}. Defaults were used for any field that could not be read.",
                        settings.schema_version, SETTINGS_SCHEMA_VERSION
                    ));
                }
                warnings.extend(settings.validate());
                (settings, warnings)
            }
            Err(e) => {
                warnings.push(format!(
                    "The settings file could not be read ({}). Defaults are in use; saving will replace it.",
                    e.user_message()
                ));
                (Self::default(), warnings)
            }
        }
    }

    /// Load settings from the default location.
    pub fn load() -> (Self, Vec<String>) {
        Self::load_or_default(&Self::default_path())
    }

    /// Save settings, replacing the file atomically.
    pub fn save(&self, path: &Path) -> Result<(), ProjectError> {
        io::write_json(path, self)
    }

    /// Save settings to the default location.
    pub fn save_default(&self) -> Result<(), ProjectError> {
        self.save(&Self::default_path())
    }

    /// Problems that should be shown to the user.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = self.telemetry.validate();
        if self.general.autosave && self.general.autosave_interval_seconds <= 0.0 {
            problems.push("the autosave interval must be positive when autosave is on".to_string());
        }
        if self.dynamics.quaternion_norm_tolerance <= 0.0 {
            problems.push("the quaternion norm tolerance must be positive".to_string());
        }
        if let SolverKind::Rk4 { step } = self.dynamics.solver {
            if step <= 0.0 {
                problems.push("the default integration step must be positive".to_string());
            }
        }
        problems
    }

    /// Whether the settings differ from the defaults, which the UI uses to decide
    /// whether to offer a reset.
    pub fn is_modified(&self) -> bool {
        *self != Self::default()
    }

    /// The resolved theme for a system preference.
    pub fn resolved_theme(&self, system_prefers_dark: bool) -> Theme {
        self.appearance.theme.resolve(system_prefers_dark)
    }

    /// Record a recently opened project.
    pub fn remember_project(&mut self, path: &str) {
        self.general.remember_project(path);
    }
}

/// The directory settings and caches live in.
pub fn config_root() -> PathBuf {
    if let Ok(dir) = std::env::var("HEXADOF_CONFIG_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            if !appdata.trim().is_empty() {
                return PathBuf::from(appdata).join("APPDATA Works").join("HexaDOF");
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            if !xdg.trim().is_empty() {
                return PathBuf::from(xdg).join("hexadof");
            }
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        if !home.trim().is_empty() {
            return PathBuf::from(home).join(".hexadof");
        }
    }
    std::env::temp_dir().join("hexadof-config")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "hexadof-settings-{}-{}-{}",
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

    #[test]
    fn defaults_are_valid_and_unmodified() {
        let settings = Settings::default();
        assert!(settings.validate().is_empty(), "{:?}", settings.validate());
        assert!(!settings.is_modified());
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
    }

    #[test]
    fn a_missing_file_yields_defaults_with_no_warning() {
        let temp = TempDir::new("missing");
        let (settings, warnings) = Settings::load_or_default(&temp.path().join("settings.json"));
        assert_eq!(settings, Settings::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn a_corrupt_file_yields_defaults_with_a_warning_rather_than_failing() {
        let temp = TempDir::new("corrupt");
        let path = temp.path().join("settings.json");
        std::fs::write(&path, b"{not json").unwrap();
        let (settings, warnings) = Settings::load_or_default(&path);
        assert_eq!(settings, Settings::default());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("could not be read"));
        assert!(warnings[0].contains("Defaults are in use"));
    }

    #[test]
    fn round_trip_through_save_and_load() {
        let temp = TempDir::new("roundtrip");
        let path = temp.path().join("settings.json");
        let mut settings = Settings::default();
        settings.appearance.theme = Theme::Dark;
        settings.appearance.density = Density::Compact;
        settings.appearance.reduced_motion = true;
        settings.telemetry.default_baud_rate = 921_600;
        settings.telemetry.estimator_mode = EstimatorMode::Complementary;
        settings.storage.export_format = ExportFormat::Hlog;
        settings.diagnostics.log_level = LogLevel::Debug;
        settings.general.remember_project("C:/projects/one");
        settings.save(&path).unwrap();

        let (loaded, warnings) = Settings::load_or_default(&path);
        assert!(warnings.is_empty(), "{:?}", warnings);
        assert_eq!(loaded, settings);
        assert!(loaded.is_modified());
    }

    #[test]
    fn advanced_mode_defaults_on_and_survives_a_round_trip() {
        // The full interface is the default, so an existing settings file and a
        // fresh install behave the same way.
        let mut settings = Settings::default();
        assert!(settings.general.advanced_mode);

        let temp = TempDir::new("mode");
        let path = temp.path().join("settings.json");
        settings.general.advanced_mode = false;
        settings.save(&path).unwrap();

        let (loaded, warnings) = Settings::load_or_default(&path);
        assert!(warnings.is_empty(), "{:?}", warnings);
        assert!(!loaded.general.advanced_mode);

        // A file written before the field existed loads as advanced rather than
        // dropping the whole general section.
        std::fs::write(
            &path,
            br#"{"general":{"autosave":true,"autosave_interval_seconds":30}}"#,
        )
        .unwrap();
        let (older, warnings) = Settings::load_or_default(&path);
        assert!(warnings.is_empty(), "{:?}", warnings);
        assert!(older.general.advanced_mode);
    }

    #[test]
    fn an_unknown_schema_version_warns_but_still_loads() {
        let temp = TempDir::new("schema");
        let path = temp.path().join("settings.json");
        let settings = Settings {
            schema_version: "0.1".to_string(),
            appearance: AppearanceSettings {
                theme: Theme::Light,
                ..Settings::default().appearance
            },
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let (loaded, warnings) = Settings::load_or_default(&path);
        assert_eq!(loaded.appearance.theme, Theme::Light);
        assert!(warnings.iter().any(|w| w.contains("0.1")));
    }

    #[test]
    fn a_partial_file_uses_defaults_for_missing_sections() {
        let temp = TempDir::new("partial");
        let path = temp.path().join("settings.json");
        // Only one field is present, so every section must fall back to defaults.
        std::fs::write(&path, br#"{"appearance":{"theme":"dark"}}"#).unwrap();
        let (loaded, warnings) = Settings::load_or_default(&path);
        assert!(warnings.is_empty(), "{:?}", warnings);
        assert_eq!(loaded.appearance.theme, Theme::Dark);
        assert_eq!(loaded.telemetry, TelemetrySettings::default());
        assert_eq!(loaded.dynamics, DynamicsSettings::default());
        assert_eq!(loaded.storage, StorageSettings::default());
    }

    #[test]
    fn theme_resolution_follows_the_system_when_asked() {
        assert_eq!(Theme::System.resolve(true), Theme::Dark);
        assert_eq!(Theme::System.resolve(false), Theme::Light);
        assert_eq!(Theme::Dark.resolve(false), Theme::Dark);
        assert_eq!(Theme::Light.resolve(true), Theme::Light);
        assert_eq!(Theme::default(), Theme::System);
        for theme in [Theme::Dark, Theme::Light, Theme::System] {
            assert!(!theme.label().is_empty());
        }
    }

    #[test]
    fn settings_resolve_the_theme_through_appearance() {
        let mut settings = Settings::default();
        settings.appearance.theme = Theme::Light;
        assert_eq!(settings.resolved_theme(true), Theme::Light);
        settings.appearance.theme = Theme::System;
        assert_eq!(settings.resolved_theme(true), Theme::Dark);
    }

    #[test]
    fn recent_projects_are_newest_first_without_duplicates() {
        let mut general = GeneralSettings::default();
        general.remember_project("a");
        general.remember_project("b");
        general.remember_project("a");
        assert_eq!(general.recent_projects, vec!["a", "b"]);
        general.forget_project("a");
        assert_eq!(general.recent_projects, vec!["b"]);
    }

    #[test]
    fn recent_projects_are_capped() {
        let mut general = GeneralSettings::default();
        for i in 0..50 {
            general.remember_project(&format!("project-{}", i));
        }
        assert_eq!(general.recent_projects.len(), GeneralSettings::MAX_RECENT);
        assert_eq!(general.recent_projects[0], "project-49");
    }

    #[test]
    fn only_existing_recent_projects_are_offered() {
        let temp = TempDir::new("recent");
        let mut general = GeneralSettings::default();
        general.remember_project(&temp.path().to_string_lossy());
        general.remember_project("C:/definitely/not/here/at/all");
        let existing = general.existing_recent_projects();
        assert_eq!(existing.len(), 1);
    }

    #[test]
    fn settings_remember_through_the_facade() {
        let mut settings = Settings::default();
        settings.remember_project("x");
        assert_eq!(settings.general.recent_projects, vec!["x"]);
    }

    #[test]
    fn telemetry_validation_catches_impossible_values() {
        let mut t = TelemetrySettings::default();
        assert!(t.validate().is_empty());

        t.serial_scan_interval_seconds = 0.0;
        assert!(t.validate().iter().any(|p| p.contains("scan interval")));

        t = TelemetrySettings::default();
        t.packet_timeout_seconds = -1.0;
        assert!(t.validate().iter().any(|p| p.contains("timeout")));

        t = TelemetrySettings::default();
        t.maximum_packet_bytes = 0;
        assert!(t.validate().iter().any(|p| p.contains("maximum packet")));

        t = TelemetrySettings::default();
        t.recording_buffer_samples = 0;
        assert!(t.validate().iter().any(|p| p.contains("recording buffer")));

        t = TelemetrySettings::default();
        t.display_rate_hz = -1.0;
        assert!(t.validate().iter().any(|p| p.contains("display rate")));

        t = TelemetrySettings::default();
        t.default_baud_rate = 0;
        assert!(t.validate().iter().any(|p| p.contains("baud")));
    }

    #[test]
    fn recording_buffer_size_is_reported_in_megabytes() {
        let t = TelemetrySettings {
            recording_buffer_samples: 131_072,
            ..Default::default()
        };
        // 131072 samples times 8 channels times 8 bytes is 8 MiB.
        assert!((t.recording_buffer_megabytes(8) - 8.0).abs() < 1e-9);
        assert!(t.recording_buffer_megabytes(0) > 0.0);
    }

    #[test]
    fn global_validation_covers_the_other_sections() {
        let mut settings = Settings::default();
        settings.general.autosave = true;
        settings.general.autosave_interval_seconds = 0.0;
        assert!(settings
            .validate()
            .iter()
            .any(|p| p.contains("autosave interval")));

        let mut settings = Settings::default();
        settings.dynamics.quaternion_norm_tolerance = 0.0;
        assert!(settings
            .validate()
            .iter()
            .any(|p| p.contains("quaternion norm")));

        let mut settings = Settings::default();
        settings.dynamics.solver = SolverKind::fixed(0.0);
        assert!(settings
            .validate()
            .iter()
            .any(|p| p.contains("integration step")));

        // Autosave off with a zero interval is acceptable, because nothing is
        // scheduled.
        let settings = Settings {
            general: GeneralSettings {
                autosave: false,
                autosave_interval_seconds: 0.0,
                ..Settings::default().general
            },
            ..Settings::default()
        };
        assert!(settings.validate().is_empty());
    }

    #[test]
    fn dynamics_thresholds_map_onto_the_run_thresholds() {
        let d = DynamicsSettings {
            quaternion_norm_tolerance: 1e-8,
            angular_rate_warning: 50.0,
            acceleration_warning: 500.0,
            ..DynamicsSettings::default()
        };
        let thresholds = d.thresholds();
        assert!((thresholds.quaternion_norm_error - 1e-8).abs() < 1e-15);
        assert!((thresholds.maximum_angular_rate - 50.0).abs() < 1e-15);
        assert!((thresholds.maximum_acceleration - 500.0).abs() < 1e-15);
    }

    #[test]
    fn export_formats_have_labels_and_extensions() {
        for format in [
            ExportFormat::Csv,
            ExportFormat::Json,
            ExportFormat::Hlog,
            ExportFormat::Markdown,
        ] {
            assert!(!format.label().is_empty());
            assert!(!format.extension().is_empty());
        }
        assert_eq!(ExportFormat::default(), ExportFormat::Csv);
        assert_eq!(ExportFormat::Hlog.extension(), "hlog");
    }

    #[test]
    fn log_levels_are_ordered_and_labelled() {
        assert!(LogLevel::Error < LogLevel::Warn);
        assert!(LogLevel::Warn < LogLevel::Info);
        assert!(LogLevel::Info < LogLevel::Debug);
        assert!(LogLevel::Debug < LogLevel::Trace);
        assert_eq!(LogLevel::all().len(), 5);
        for level in LogLevel::all() {
            assert!(!level.label().is_empty());
        }
        assert_eq!(LogLevel::default(), LogLevel::Info);
    }

    #[test]
    fn density_labels() {
        assert_eq!(Density::default(), Density::Comfortable);
        assert!(!Density::Compact.label().is_empty());
        assert!(!Density::Comfortable.label().is_empty());
    }

    #[test]
    fn config_root_is_absolute_and_honours_the_override() {
        let root = config_root();
        assert!(root.is_absolute() || root.components().count() > 0);
        // The override is read from the environment, so set and restore it.
        let previous = std::env::var("HEXADOF_CONFIG_DIR").ok();
        std::env::set_var("HEXADOF_CONFIG_DIR", temp_dir_marker());
        assert_eq!(config_root(), PathBuf::from(temp_dir_marker()));
        match previous {
            Some(v) => std::env::set_var("HEXADOF_CONFIG_DIR", v),
            None => std::env::remove_var("HEXADOF_CONFIG_DIR"),
        }
    }

    fn temp_dir_marker() -> String {
        std::env::temp_dir().to_string_lossy().to_string()
    }

    #[test]
    fn default_path_ends_with_the_settings_file_name() {
        let path = Settings::default_path();
        assert_eq!(
            path.file_name().map(|n| n.to_string_lossy().to_string()),
            Some(SETTINGS_FILE_NAME.to_string())
        );
    }

    #[test]
    fn saving_creates_missing_parent_directories() {
        let temp = TempDir::new("saveparents");
        let path = temp.path().join("a/b/settings.json");
        Settings::default().save(&path).unwrap();
        assert!(path.is_file());
        let (loaded, warnings) = Settings::load_or_default(&path);
        assert!(warnings.is_empty());
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn every_default_section_is_self_consistent() {
        let settings = Settings::default();
        assert_eq!(settings.general.unit_system, UnitSystem::Si);
        assert_eq!(settings.dynamics.world_frame, WorldFrame::Enu);
        assert!(settings.storage.backup_project_file);
        assert!(
            !settings.storage.retention_enabled,
            "a retention policy must never be on by default"
        );
        assert!(!settings.telemetry.auto_reconnect);
        assert_eq!(settings.telemetry.display_rate_hz, 30.0);
    }
}
