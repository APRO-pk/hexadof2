//! HexaDOF project persistence: project directories, run artifacts, telemetry and
//! flight sessions, reports, and application settings.
//!
//! # What this crate guarantees
//!
//! * A project is a directory with explicit, human-readable metadata.
//! * Every write goes through a temporary file and a rename, so an interrupted
//!   save never leaves a truncated project file.
//! * Saved runs are immutable. Saving again produces a new directory.
//! * An imported flight log is copied byte for byte, its hash is recorded, and the
//!   original file is never modified.
//! * A project-relative reference that would escape the project directory is
//!   refused, so a crafted project file cannot read or write outside its folder.
//! * Real-time histories use the `.hlog` codec from `hex-flight-data`, so a run
//!   history and a flight log are read by the same verified reader.

pub mod artifacts;
pub mod clock;
pub mod error;
pub mod io;
pub mod project;
pub mod sessions;
pub mod settings;

pub use artifacts::{
    add_marker, delete_run, export_history_csv, find_run, latest_run, list_runs,
    next_run_directory, save_run, RunArtifact, RunHistory, CONFIG_FILE, EVENTS_FILE, RESULTS_FILE,
    RUN_CHANNELS, RUN_SCHEMA_VERSION, SUMMARY_FILE,
};
pub use clock::{
    content_hash, file_hash, new_id, new_uuid_like, now_file_stamp, now_iso8601, now_unix_millis,
    unix_seconds, UtcDateTime,
};
pub use error::ProjectError;
pub use project::{
    dirs, discover_projects, read_metadata, Project, ProjectInventory, ProjectMetadata,
    PROJECT_FILE_NAME, PROJECT_SCHEMA_VERSION, REQUIRED_DIRECTORIES,
};
pub use sessions::{
    delete_device_profile, delete_flight_session, delete_telemetry_session, find_device_profile,
    find_flight_session, import_flight_source, list_flight_sessions, list_telemetry_sessions,
    load_calibration, load_device_profiles, save_calibration, save_comparison, save_device_profile,
    save_report, save_telemetry_session, FlightSessionArtifact, FlightSessionMetadata,
    TelemetrySessionArtifact, TelemetrySessionMetadata, DERIVED_FILE, DEVICE_PROFILE_FILE,
    FLIGHT_SCHEMA_VERSION, MAPPING_FILE, MARKERS_FILE, SESSION_FILE, VALIDATION_FILE,
};
pub use settings::{
    config_root, AppearanceSettings, Density, DiagnosticsSettings, DynamicsSettings, ExportFormat,
    GeneralSettings, LogLevel, Settings, StorageSettings, TelemetrySettings, Theme,
    SETTINGS_FILE_NAME, SETTINGS_SCHEMA_VERSION,
};

/// The layout of a run artifact directory.
pub fn run_layout_description() -> String {
    artifacts::layout_description()
}

/// The layout of the telemetry and flight artifact directories.
pub fn session_layout_description() -> String {
    sessions::layout_description()
}

/// A one-line capability description for the diagnostics bundle.
pub fn capability_summary() -> String {
    format!(
        "Projects: {} required directories, immutable runs, {} stored run channels, settings schema {}",
        REQUIRED_DIRECTORIES.len(),
        RUN_CHANNELS.len(),
        SETTINGS_SCHEMA_VERSION
    )
}

/// A description of what a project contains and where the application keeps its
/// settings, for the diagnostics bundle.
pub fn describe_environment(project: Option<&Project>) -> Vec<String> {
    let mut lines = vec![format!(
        "Settings file: {}",
        Settings::default_path().display()
    )];
    match project {
        Some(p) => {
            lines.push(format!("Project: {}", p.root().display()));
            lines.push(format!("Project status: {}", p.status_line()));
            lines.push(format!("Inventory: {}", p.inventory().summary()));
            for w in &p.open_warnings {
                lines.push(format!("Open warning: {}", w));
            }
        }
        None => lines.push("No project is open".to_string()),
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "hexadof-root-{}-{}-{}",
                label,
                std::process::id(),
                new_id()
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
    fn capability_summary_is_informative() {
        let s = capability_summary();
        assert!(s.contains("directories"));
        assert!(s.contains("immutable runs"));
        assert!(s.contains(&RUN_CHANNELS.len().to_string()));
    }

    #[test]
    fn layout_descriptions_name_their_artifacts() {
        let run = run_layout_description();
        for name in [CONFIG_FILE, SUMMARY_FILE, EVENTS_FILE, RESULTS_FILE] {
            assert!(run.contains(name), "{} missing from {}", name, run);
        }
        let session = session_layout_description();
        for name in [
            DEVICE_PROFILE_FILE,
            MAPPING_FILE,
            VALIDATION_FILE,
            SESSION_FILE,
        ] {
            assert!(session.contains(name), "{} missing from {}", name, session);
        }
    }

    #[test]
    fn environment_description_without_a_project() {
        let lines = describe_environment(None);
        assert!(lines.iter().any(|l| l.contains("Settings file")));
        assert!(lines.iter().any(|l| l.contains("No project is open")));
    }

    #[test]
    fn environment_description_includes_open_warnings() {
        let temp = TempDir::new("env");
        let project = Project::create(temp.path().join("p"), "Env").unwrap();
        std::fs::remove_dir_all(project.reports_dir()).unwrap();
        let reopened = Project::open(temp.path().join("p")).unwrap();
        assert!(!reopened.open_warnings.is_empty());
        let lines = describe_environment(Some(&reopened));
        assert!(lines.iter().any(|l| l.contains("Open warning")));
        assert!(lines.iter().any(|l| l.contains("Inventory")));
    }

    #[test]
    fn the_data_crates_compose_end_to_end() {
        // A project is created, a model is imported into it, a run is simulated and
        // saved, a flight log is copied in, a comparison is stored, and a report is
        // written. This is the whole persistence path in one test.
        let temp = TempDir::new("endtoend");
        let mut project = Project::create(temp.path().join("project"), "End to end").unwrap();

        // Import a dynamics model that the model crate validates.
        let model = hex_model::ModelDocument::minimal(
            "builtin.example",
            "Example",
            2.0,
            [0.05, 0.05, 0.01],
        );
        let model_json = model.to_json_pretty().unwrap();
        let model_source = temp.path().join("rocket.dynamic.json");
        std::fs::write(&model_source, &model_json).unwrap();
        let reference = project.import_model_file(&model_source, None).unwrap();
        assert_eq!(reference, "models/rocket.dynamic.json");

        let imported =
            hex_model::import_json(&model_json, &hex_model::ImportOptions::default()).unwrap();
        assert!(
            !imported.validation.has_errors(),
            "{:?}",
            imported.validation
        );
        assert!(project.model_path().unwrap().unwrap().is_file());

        // Configure and run a simulation.
        let config = hex_dynamics::RunConfig {
            name: "Vertical".to_string(),
            end_time: 0.5,
            output_interval: 0.01,
            solver: hex_dynamics::SolverKind::fixed(0.0005),
            initial: hex_dynamics::InitialConditions::at_rest(2.0),
            mass: hex_dynamics::MassSpec::point(2.0, 0.05),
            environment: hex_dynamics::Environment::vacuum_uniform_gravity(9.81),
            forces: hex_dynamics::ForceConfig {
                gravity: true,
                thrust: Some(hex_dynamics::ThrustProfile::constant(60.0, 0.3)),
                aero: Some(hex_dynamics::AeroModel::drag_only_estimate(0.4)),
                ..hex_dynamics::ForceConfig::default()
            },
            ..hex_dynamics::RunConfig::default()
        };
        let outcome = hex_dynamics::run_simulation(config.clone());
        assert!(outcome.status.is_usable(), "{:?}", outcome.status);
        let run = save_run(
            &project,
            "Vertical",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        assert!(run.is_complete());
        let history = run.load_history().unwrap();
        assert_eq!(history.len(), outcome.samples.len());
        assert!(history.is_intact());

        // Copy a flight log in through the flight-data importer.
        let csv = "timestamp,ax,ay,az,gx,gy,gz,baro\n0.00,0.0,0.0,9.81,0.0,0.0,0.0,101325\n0.01,0.1,0.0,9.81,0.0,0.0,0.0,101320\n";
        let csv_source = temp.path().join("flight.csv");
        std::fs::write(&csv_source, csv).unwrap();
        let (session, report) =
            hex_flight_data::import_csv(csv, &hex_flight_data::CsvImportOptions::default(), &[])
                .unwrap();
        assert!(report.succeeded, "{:?}", report.primary_error);
        assert_eq!(session.times.len(), 2);

        let mut metadata = FlightSessionMetadata {
            schema_version: String::new(),
            name: String::new(),
            label: "Flight 001".to_string(),
            source_file: String::new(),
            source_bytes: 0,
            source_hash: String::new(),
            source_format: "Csv".to_string(),
            imported_at: now_iso8601(),
            sample_count: session.times.len(),
            duration_seconds: session.duration(),
            sample_rate_hz: session.sample_rate_hz(),
            channels: session.channels.iter().map(|c| c.name.clone()).collect(),
            derived_channels: Vec::new(),
            events: Vec::new(),
            quality_flags: Vec::new(),
            original_preserved: false,
        };
        let flight = import_flight_source(
            &project,
            "Flight 001",
            &csv_source,
            &mut metadata,
            &serde_json::to_value(&session.mapping_summary).unwrap_or_default(),
            &serde_json::json!({
                "succeeded": report.succeeded,
                "primary_error": report.primary_error,
                "stages": report.stages.iter().map(|s| format!("{:?}: {}", s.stage, s.message)).collect::<Vec<_>>(),
            }),
            &[],
            &[],
            &[],
        )
        .unwrap();
        assert!(flight.source_is_intact());
        assert_eq!(std::fs::read(flight.source_path()).unwrap(), csv.as_bytes());

        // Compare the two and store the comparison, then write a report.
        let simulation_series = comparison_series_from_run(&run);
        let flight_series = comparison_series_from_flight(&flight, csv);
        let comparison = hex_analysis::compare(
            &simulation_series,
            &flight_series,
            &hex_analysis::ComparisonOptions::default(),
        )
        .unwrap();
        let stored = save_comparison(
            &project,
            "Vertical vs Flight 001",
            &serde_json::to_value(&comparison).unwrap(),
        )
        .unwrap();
        assert!(stored.is_file());

        let report_path = save_report(&project, "End to end", &comparison.to_text()).unwrap();
        assert!(report_path.is_file());
        let text = std::fs::read_to_string(&report_path).unwrap();
        assert!(text.contains("does not identify a single cause"));

        // The project inventory reflects everything that was written.
        let inventory = project.inventory();
        assert_eq!(inventory.models, 1);
        assert_eq!(inventory.simulations, 1);
        assert_eq!(inventory.flight_logs, 1);
        assert!(inventory.reports >= 2);
    }

    fn comparison_series_from_run(run: &RunArtifact) -> hex_analysis::ComparisonSeries {
        let history = run.load_history().unwrap();
        let altitude = history
            .channel(hex_dynamics::ChannelSelector::Altitude)
            .unwrap()
            .to_vec();
        hex_analysis::ComparisonSeries::new(run.name.clone()).with_channel(
            hex_analysis::ComparisonChannel {
                channel: hex_analysis::MetricChannel::Altitude,
                times: history.times.clone(),
                values: altitude,
                unit: "m".to_string(),
                provenance: hex_analysis::Provenance::Simulated,
                is_integrated: false,
            },
        )
    }

    fn comparison_series_from_flight(
        flight: &FlightSessionArtifact,
        csv: &str,
    ) -> hex_analysis::ComparisonSeries {
        // The flight log carries pressure, so derive a comparable altitude column
        // through the flight-data derivation helper rather than inventing one.
        let (session, _) =
            hex_flight_data::import_csv(csv, &hex_flight_data::CsvImportOptions::default(), &[])
                .unwrap();
        let pressure = session
            .channel(hex_flight_data::ChannelRole::BaroPressure)
            .map(|c| c.corrected_values())
            .unwrap_or_default();
        let altitude = hex_flight_data::derive_barometric_altitude(&pressure, 101_325.0, 288.15);
        hex_analysis::ComparisonSeries::new(flight.name.clone()).with_channel(
            hex_analysis::ComparisonChannel {
                channel: hex_analysis::MetricChannel::Altitude,
                times: session.times.clone(),
                values: altitude,
                unit: "m".to_string(),
                provenance: hex_analysis::Provenance::Measured,
                is_integrated: false,
            },
        )
    }
}
