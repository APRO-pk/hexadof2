//! Simulation run artifacts.
//!
//! A saved run is immutable. It is a directory under `simulations/` holding:
//!
//! ```text
//! simulations/run-001/
//!   config.json     the complete run configuration, so the run is reproducible
//!   summary.json    the recorded summary, statistics, and warnings
//!   events.json     the event list, including user markers
//!   results.hlog    the state history, in the HexaDOF binary log format
//! ```
//!
//! The history uses the `.hlog` codec from `hex-flight-data` rather than a format
//! invented here, so a run and a flight log are read by the same verified reader
//! and a comparison never has to convert between two ad-hoc layouts.
//!
//! Nothing overwrites an existing run directory. Saving repeatedly produces
//! `run-002`, `run-003`, and so on.

use std::path::{Path, PathBuf};

use hex_core::Real;
use hex_dynamics::{ChannelSelector, FlightEvent, RunConfig, RunSummary, StateSample};
use hex_flight_data::{
    BinaryDecodeError, BinaryReader, BinaryWriter, ChannelDescriptor, ChannelRole, LogHeader,
};

use crate::error::ProjectError;
use crate::io;
use crate::project::{dirs, Project};

/// The configuration artifact name.
pub const CONFIG_FILE: &str = "config.json";
/// The summary artifact name.
pub const SUMMARY_FILE: &str = "summary.json";
/// The events artifact name.
pub const EVENTS_FILE: &str = "events.json";
/// The state history artifact name.
pub const RESULTS_FILE: &str = "results.hlog";
/// The schema version of a run artifact set.
pub const RUN_SCHEMA_VERSION: &str = "1.0";

/// The channels a run history is stored with.
///
/// Fixed and ordered, so a reader can map a column index to a channel without
/// consulting the file. The names appear in the `.hlog` channel table for a human
/// reading the file with other tools.
pub const RUN_CHANNELS: &[(ChannelSelector, &str, ChannelRole)] = &[
    (ChannelSelector::Time, "time", ChannelRole::Time),
    (ChannelSelector::PositionX, "position_x", ChannelRole::Raw),
    (ChannelSelector::PositionY, "position_y", ChannelRole::Raw),
    (ChannelSelector::PositionZ, "position_z", ChannelRole::Raw),
    (ChannelSelector::VelocityX, "velocity_x", ChannelRole::Raw),
    (ChannelSelector::VelocityY, "velocity_y", ChannelRole::Raw),
    (ChannelSelector::VelocityZ, "velocity_z", ChannelRole::Raw),
    (
        ChannelSelector::QuaternionW,
        "quaternion_w",
        ChannelRole::QuaternionW,
    ),
    (
        ChannelSelector::QuaternionX,
        "quaternion_x",
        ChannelRole::QuaternionX,
    ),
    (
        ChannelSelector::QuaternionY,
        "quaternion_y",
        ChannelRole::QuaternionY,
    ),
    (
        ChannelSelector::QuaternionZ,
        "quaternion_z",
        ChannelRole::QuaternionZ,
    ),
    (
        ChannelSelector::AngularRateX,
        "angular_rate_x",
        ChannelRole::GyroX,
    ),
    (
        ChannelSelector::AngularRateY,
        "angular_rate_y",
        ChannelRole::GyroY,
    ),
    (
        ChannelSelector::AngularRateZ,
        "angular_rate_z",
        ChannelRole::GyroZ,
    ),
    (ChannelSelector::Mass, "mass", ChannelRole::Raw),
    (
        ChannelSelector::ForceBodyX,
        "force_body_x",
        ChannelRole::Raw,
    ),
    (
        ChannelSelector::ForceBodyY,
        "force_body_y",
        ChannelRole::Raw,
    ),
    (
        ChannelSelector::ForceBodyZ,
        "force_body_z",
        ChannelRole::Raw,
    ),
    (
        ChannelSelector::MomentBodyX,
        "moment_body_x",
        ChannelRole::Raw,
    ),
    (
        ChannelSelector::MomentBodyY,
        "moment_body_y",
        ChannelRole::Raw,
    ),
    (
        ChannelSelector::MomentBodyZ,
        "moment_body_z",
        ChannelRole::Raw,
    ),
    (
        ChannelSelector::AccelerationWorldX,
        "acceleration_x",
        ChannelRole::AccelX,
    ),
    (
        ChannelSelector::AccelerationWorldY,
        "acceleration_y",
        ChannelRole::AccelY,
    ),
    (
        ChannelSelector::AccelerationWorldZ,
        "acceleration_z",
        ChannelRole::AccelZ,
    ),
    (
        ChannelSelector::DynamicPressure,
        "dynamic_pressure",
        ChannelRole::BaroPressure,
    ),
    (
        ChannelSelector::AngleOfAttack,
        "angle_of_attack",
        ChannelRole::Raw,
    ),
    (ChannelSelector::Sideslip, "sideslip", ChannelRole::Raw),
    (ChannelSelector::AirDensity, "air_density", ChannelRole::Raw),
    (ChannelSelector::Mach, "mach", ChannelRole::Raw),
    (ChannelSelector::Roll, "roll", ChannelRole::Raw),
    (ChannelSelector::Pitch, "pitch", ChannelRole::Raw),
    (ChannelSelector::Yaw, "yaw", ChannelRole::Raw),
];

/// A handle to a saved run directory.
#[derive(Debug, Clone, PartialEq)]
pub struct RunArtifact {
    /// The run directory.
    pub path: PathBuf,
    /// Directory name, for example `run-001`.
    pub name: String,
    /// The summary, when it has been read.
    pub summary: RunSummary,
}

impl RunArtifact {
    /// The configuration file path.
    pub fn config_path(&self) -> PathBuf {
        self.path.join(CONFIG_FILE)
    }

    /// The summary file path.
    pub fn summary_path(&self) -> PathBuf {
        self.path.join(SUMMARY_FILE)
    }

    /// The events file path.
    pub fn events_path(&self) -> PathBuf {
        self.path.join(EVENTS_FILE)
    }

    /// The history file path.
    pub fn results_path(&self) -> PathBuf {
        self.path.join(RESULTS_FILE)
    }

    /// Whether every expected artifact is present.
    pub fn is_complete(&self) -> bool {
        self.config_path().is_file()
            && self.summary_path().is_file()
            && self.results_path().is_file()
    }

    /// Total size of the artifact directory in bytes.
    pub fn size_bytes(&self) -> u64 {
        io::directory_size(&self.path)
    }

    /// The stored configuration.
    pub fn load_config(&self) -> Result<RunConfig, ProjectError> {
        io::read_json(&self.config_path())
    }

    /// The stored events, including user markers.
    pub fn load_events(&self) -> Result<Vec<FlightEvent>, ProjectError> {
        let path = self.events_path();
        if !path.is_file() {
            return Ok(Vec::new());
        }
        io::read_json(&path)
    }

    /// The stored state history.
    pub fn load_history(&self) -> Result<RunHistory, ProjectError> {
        let path = self.results_path();
        let bytes = std::fs::read(&path).map_err(|e| ProjectError::io(&path, e))?;
        let log = BinaryReader::read_all(&bytes).map_err(|e| history_error(&path, e))?;
        RunHistory::from_log(path, log)
    }

    /// A display label for the run list.
    pub fn label(&self) -> String {
        format!(
            "{} ({}, {} samples, {:.3} s)",
            self.summary.name, self.name, self.summary.sample_count, self.summary.duration
        )
    }
}

/// A run's state history read back from disk.
#[derive(Debug, Clone, PartialEq)]
pub struct RunHistory {
    /// The file the history came from.
    pub path: PathBuf,
    /// Times in seconds.
    pub times: Vec<Real>,
    /// One column per entry in [`RUN_CHANNELS`].
    pub columns: Vec<Vec<Real>>,
    /// Integrity counters from the reader.
    pub integrity: hex_flight_data::DecodeIntegrity,
}

impl RunHistory {
    fn from_log(path: PathBuf, log: hex_flight_data::BinaryLog) -> Result<Self, ProjectError> {
        if log.channels.len() != RUN_CHANNELS.len() {
            return Err(ProjectError::Parse {
                path,
                detail: format!(
                    "the history declares {} channels but this build expects {}",
                    log.channels.len(),
                    RUN_CHANNELS.len()
                ),
            });
        }
        Ok(Self {
            path,
            times: log.times,
            columns: log.columns,
            integrity: log.integrity,
        })
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// Column data for a channel.
    ///
    /// The stored channel set is the minimum needed to reconstruct a state, so a
    /// derived selector that is an exact alias of a stored column resolves to it
    /// rather than being reported as absent. In the ENU world frame altitude is the
    /// world Z position and vertical velocity is the world Z velocity, so those two
    /// aliases are exact.
    pub fn channel(&self, selector: ChannelSelector) -> Option<&[Real]> {
        let resolved = match selector {
            ChannelSelector::Altitude => ChannelSelector::PositionZ,
            ChannelSelector::VerticalVelocity => ChannelSelector::VelocityZ,
            other => other,
        };
        RUN_CHANNELS
            .iter()
            .position(|(c, _, _)| *c == resolved)
            .and_then(|i| self.columns.get(i))
            .map(|c| c.as_slice())
    }

    /// Whether the history contains any record-level integrity failure.
    ///
    /// A run with a damaged history must say so rather than presenting the
    /// surviving samples as complete.
    pub fn is_intact(&self) -> bool {
        self.integrity.crc_failures == 0 && self.integrity.truncated_records == 0
    }

    /// A caveat when the history is damaged.
    pub fn integrity_warning(&self) -> Option<String> {
        if self.is_intact() {
            return None;
        }
        Some(format!(
            "The saved history has {} checksum failure(s) and {} truncated record(s). {} of {} samples were recovered.",
            self.integrity.crc_failures,
            self.integrity.truncated_records,
            self.integrity.samples_read,
            self.times.len().max(self.integrity.samples_read)
        ))
    }

    /// Rebuild samples, for the replay and comparison paths.
    pub fn to_samples(&self) -> Vec<StateSample> {
        let mut samples = Vec::with_capacity(self.times.len());
        for (i, time) in self.times.iter().enumerate() {
            let mut sample = StateSample {
                time: *time,
                ..StateSample::default()
            };
            for (selector, _, _) in RUN_CHANNELS.iter() {
                let value = self
                    .channel(*selector)
                    .and_then(|c| c.get(i).copied())
                    .unwrap_or(0.0);
                assign_sample_field(&mut sample, *selector, value);
            }
            samples.push(sample);
        }
        samples
    }
}

/// Write one channel value into the matching field of a sample.
fn assign_sample_field(sample: &mut StateSample, selector: ChannelSelector, value: Real) {
    match selector {
        ChannelSelector::Time => sample.time = value,
        ChannelSelector::PositionX => sample.position[0] = value,
        ChannelSelector::PositionY => sample.position[1] = value,
        ChannelSelector::PositionZ => sample.position[2] = value,
        ChannelSelector::VelocityX => sample.velocity[0] = value,
        ChannelSelector::VelocityY => sample.velocity[1] = value,
        ChannelSelector::VelocityZ => sample.velocity[2] = value,
        ChannelSelector::QuaternionW => sample.attitude[0] = value,
        ChannelSelector::QuaternionX => sample.attitude[1] = value,
        ChannelSelector::QuaternionY => sample.attitude[2] = value,
        ChannelSelector::QuaternionZ => sample.attitude[3] = value,
        ChannelSelector::AngularRateX => sample.angular_velocity[0] = value,
        ChannelSelector::AngularRateY => sample.angular_velocity[1] = value,
        ChannelSelector::AngularRateZ => sample.angular_velocity[2] = value,
        ChannelSelector::Mass => sample.mass = value,
        ChannelSelector::ForceBodyX => sample.force_body[0] = value,
        ChannelSelector::ForceBodyY => sample.force_body[1] = value,
        ChannelSelector::ForceBodyZ => sample.force_body[2] = value,
        ChannelSelector::MomentBodyX => sample.moment_body[0] = value,
        ChannelSelector::MomentBodyY => sample.moment_body[1] = value,
        ChannelSelector::MomentBodyZ => sample.moment_body[2] = value,
        ChannelSelector::AccelerationWorldX => sample.acceleration_world[0] = value,
        ChannelSelector::AccelerationWorldY => sample.acceleration_world[1] = value,
        ChannelSelector::AccelerationWorldZ => sample.acceleration_world[2] = value,
        ChannelSelector::DynamicPressure => sample.dynamic_pressure = value,
        ChannelSelector::AngleOfAttack => sample.angle_of_attack = value,
        ChannelSelector::Sideslip => sample.sideslip = value,
        ChannelSelector::AirDensity => sample.air_density = value,
        ChannelSelector::Mach => sample.mach = value,
        ChannelSelector::Roll => sample.euler_321[0] = value,
        ChannelSelector::Pitch => sample.euler_321[1] = value,
        ChannelSelector::Yaw => sample.euler_321[2] = value,
        // The magnitude selectors are derived from columns that are themselves
        // stored, so there is nothing separate to assign here.
        ChannelSelector::Altitude
        | ChannelSelector::Speed
        | ChannelSelector::VerticalVelocity
        | ChannelSelector::AccelerationMagnitude
        | ChannelSelector::AngularRateMagnitude
        | ChannelSelector::ForceMagnitude
        | ChannelSelector::MomentMagnitude => {}
    }
}

fn history_error(path: &Path, error: BinaryDecodeError) -> ProjectError {
    ProjectError::Parse {
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

/// Save a completed run into the project.
///
/// Returns the created artifact. An existing directory is never reused, so a run
/// is written once and then read-only.
pub fn save_run(
    project: &Project,
    name: &str,
    summary: &RunSummary,
    samples: &[StateSample],
    events: &[FlightEvent],
    config: &RunConfig,
) -> Result<RunArtifact, ProjectError> {
    let stem = io::slugify(name, "run");
    let directory = io::unique_directory(&project.simulations_dir(), &stem);
    if directory.exists() {
        return Err(ProjectError::ArtifactExists { path: directory });
    }
    std::fs::create_dir_all(&directory).map_err(|e| ProjectError::io(&directory, e))?;

    // Write the history first, so a partial save is missing its summary rather
    // than claiming success with no data.
    write_history(&directory.join(RESULTS_FILE), samples)?;
    io::write_json(&directory.join(CONFIG_FILE), config)?;
    io::write_json(&directory.join(SUMMARY_FILE), summary)?;
    io::write_json(&directory.join(EVENTS_FILE), events)?;

    let run_name = directory
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| stem.clone());

    Ok(RunArtifact {
        path: directory,
        name: run_name,
        summary: summary.clone(),
    })
}

/// Serialise a state history with the HexaDOF binary log codec.
fn write_history(path: &Path, samples: &[StateSample]) -> Result<(), ProjectError> {
    let channels: Vec<ChannelDescriptor> = RUN_CHANNELS
        .iter()
        .map(|(_, name, role)| ChannelDescriptor::new(name, *role, 1.0, 0.0))
        .collect();
    let nominal_dt = if samples.len() > 1 {
        let span = samples[samples.len() - 1].time - samples[0].time;
        if span > 0.0 {
            span / (samples.len() - 1) as Real
        } else {
            0.0
        }
    } else {
        0.0
    };
    let header = LogHeader::new(nominal_dt)
        .with_device_id("hexadof-simulation")
        .with_created(crate::clock::unix_seconds());
    let mut writer = BinaryWriter::new(header, channels);
    let mut row = vec![0.0; RUN_CHANNELS.len()];
    for sample in samples {
        for (i, (selector, _, _)) in RUN_CHANNELS.iter().enumerate() {
            row[i] = selector.value(sample);
        }
        writer
            .push_sample(sample.time, &row)
            .map_err(|e| history_error(path, e))?;
    }
    let bytes = writer.finish().map_err(|e| history_error(path, e))?;
    io::write_atomic(path, &bytes)
}

/// List the runs in a project, newest directory name last.
pub fn list_runs(project: &Project) -> Vec<RunArtifact> {
    let mut runs = Vec::new();
    for name in io::list_directories(&project.simulations_dir()) {
        let path = project.simulations_dir().join(&name);
        match io::read_json::<RunSummary>(&path.join(SUMMARY_FILE)) {
            Ok(summary) => runs.push(RunArtifact {
                path,
                name,
                summary,
            }),
            Err(_) => {
                // A directory with no readable summary is not a run. It is left
                // alone rather than deleted, because a user may be recovering it.
                continue;
            }
        }
    }
    runs
}

/// Find a run by directory name.
pub fn find_run(project: &Project, name: &str) -> Result<RunArtifact, ProjectError> {
    let path = project.simulations_dir().join(name);
    if !path.is_dir() {
        return Err(ProjectError::RunNotFound {
            name: name.to_string(),
        });
    }
    let summary = io::read_json::<RunSummary>(&path.join(SUMMARY_FILE))?;
    Ok(RunArtifact {
        path,
        name: name.to_string(),
        summary,
    })
}

/// Delete a run directory, refusing to follow a symbolic link.
pub fn delete_run(project: &Project, name: &str) -> Result<(), ProjectError> {
    let path = project.simulations_dir().join(name);
    if !path.exists() {
        return Err(ProjectError::RunNotFound {
            name: name.to_string(),
        });
    }
    io::remove_directory_within(project.root(), &path)
}

/// Read the most recent run, for the overview screen.
pub fn latest_run(project: &Project) -> Option<RunArtifact> {
    let mut runs = list_runs(project);
    // Directory names carry a user label, so they do not sort into save order.
    // The modification time of the summary file is what actually reflects when a
    // run was written, and the name breaks a tie within the same filesystem tick.
    runs.sort_by(|a, b| {
        let ta = modified_time(&a.summary_path());
        let tb = modified_time(&b.summary_path());
        ta.cmp(&tb).then_with(|| a.name.cmp(&b.name))
    });
    runs.pop()
}

fn modified_time(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Add a user event marker to a saved run.
///
/// Markers are additive, so they can be added to an immutable run without
/// invalidating its results.
pub fn add_marker(run: &RunArtifact, time: Real, label: &str) -> Result<usize, ProjectError> {
    let mut events = run.load_events()?;
    events.push(FlightEvent::marker(time, label));
    hex_dynamics::sort_events(&mut events);
    io::write_json(&run.events_path(), &events)?;
    Ok(events.len())
}

/// Export a run history as CSV, without modifying the run.
pub fn export_history_csv(run: &RunArtifact, destination: &Path) -> Result<usize, ProjectError> {
    let history = run.load_history()?;
    let mut text = String::new();
    for (i, (_, name, _)) in RUN_CHANNELS.iter().enumerate() {
        if i > 0 {
            text.push(',');
        }
        text.push_str(name);
    }
    text.push('\n');
    for row in 0..history.len() {
        for (i, _) in RUN_CHANNELS.iter().enumerate() {
            if i > 0 {
                text.push(',');
            }
            let value = history
                .columns
                .get(i)
                .and_then(|c| c.get(row).copied())
                .unwrap_or(Real::NAN);
            if value.is_finite() {
                text.push_str(&format!("{:.9}", value));
            }
        }
        text.push('\n');
    }
    io::write_atomic(destination, text.as_bytes())?;
    Ok(history.len())
}

/// The directory a run would be saved into, without creating it.
pub fn next_run_directory(project: &Project, name: &str) -> PathBuf {
    io::unique_directory(&project.simulations_dir(), &io::slugify(name, "run"))
}

/// A one-line description of the run artifact layout, for the docs panel.
pub fn layout_description() -> String {
    format!(
        "{}/{}, {}, {}, {}",
        dirs::SIMULATIONS,
        CONFIG_FILE,
        SUMMARY_FILE,
        EVENTS_FILE,
        RESULTS_FILE
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_dynamics::{
        run_simulation, Environment, ForceConfig, InitialConditions, MassSpec, RunConfig,
        SimulationMode, SolverKind,
    };

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "hexadof-artifacts-{}-{}-{}",
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

    fn scenario() -> RunConfig {
        RunConfig {
            name: "Artifact test".to_string(),
            end_time: 0.5,
            output_interval: 0.01,
            solver: SolverKind::fixed(0.0005),
            initial: InitialConditions::at_rest(2.0)
                .with_velocity(hex_core::Vec3::new(1.0, 0.0, 10.0)),
            mass: MassSpec::point(2.0, 0.05),
            environment: Environment::vacuum_uniform_gravity(9.81),
            forces: ForceConfig::default(),
            mode: SimulationMode::SixDof,
            ..RunConfig::default()
        }
    }

    fn project(temp: &TempDir) -> Project {
        Project::create(temp.path().join("project"), "Artifacts").unwrap()
    }

    #[test]
    fn save_and_load_a_run_round_trips_every_channel() {
        let temp = TempDir::new("roundtrip");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        assert!(outcome.status.is_usable(), "{:?}", outcome.status);

        let run = save_run(
            &project,
            "Artifact test",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();

        assert!(run.is_complete());
        assert_eq!(run.name, "artifact-test");
        assert!(run.config_path().is_file());
        assert!(run.summary_path().is_file());
        assert!(run.events_path().is_file());
        assert!(run.results_path().is_file());

        let history = run.load_history().unwrap();
        assert_eq!(history.len(), outcome.samples.len());
        assert!(history.is_intact());

        // Every channel must survive exactly, because the codec stores binary64.
        for (selector, _, _) in RUN_CHANNELS {
            let column = history.channel(*selector).expect("channel present");
            for (i, sample) in outcome.samples.iter().enumerate() {
                let expected = selector.value(sample);
                assert!(
                    (column[i] - expected).abs() < 1e-12,
                    "channel {:?} at sample {}: {} vs {}",
                    selector,
                    i,
                    column[i],
                    expected
                );
            }
        }
    }

    #[test]
    fn reloaded_config_matches_the_original() {
        let temp = TempDir::new("config");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let run = save_run(
            &project,
            "config",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        let reloaded = run.load_config().unwrap();
        assert_eq!(reloaded.name, config.name);
        assert_eq!(reloaded.end_time, config.end_time);
        assert_eq!(reloaded.solver, config.solver);
        assert_eq!(reloaded.initial.mass, config.initial.mass);
        assert!(reloaded.validate().is_empty());
    }

    #[test]
    fn repeated_saves_never_overwrite() {
        let temp = TempDir::new("immutable");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let first = save_run(
            &project,
            "Same name",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        let second = save_run(
            &project,
            "Same name",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        assert_ne!(first.path, second.path);
        assert_eq!(first.name, "same-name");
        assert_eq!(second.name, "same-name-002");
        assert!(first.path.is_dir() && second.path.is_dir());
    }

    #[test]
    fn listing_and_finding_runs() {
        let temp = TempDir::new("list");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let a = save_run(
            &project,
            "Alpha",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        let b = save_run(
            &project,
            "Beta",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();

        let runs = list_runs(&project);
        assert_eq!(runs.len(), 2);
        assert!(runs.iter().any(|r| r.name == a.name));
        assert!(runs.iter().any(|r| r.name == b.name));
        assert!(runs[0].label().contains("samples"));

        let found = find_run(&project, &a.name).unwrap();
        assert_eq!(found.name, a.name);
        assert!(find_run(&project, "nope").is_err());
    }

    #[test]
    fn a_directory_without_a_summary_is_not_listed() {
        let temp = TempDir::new("nodata");
        let project = project(&temp);
        std::fs::create_dir_all(project.simulations_dir().join("run-001")).unwrap();
        assert!(list_runs(&project).is_empty());
        // And it is not deleted, because a user may be recovering it.
        assert!(project.simulations_dir().join("run-001").is_dir());
    }

    #[test]
    fn latest_run_picks_the_highest_numbered() {
        let temp = TempDir::new("latest");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        for name in ["one", "two", "three"] {
            save_run(
                &project,
                name,
                &outcome.summary,
                &outcome.samples,
                &outcome.events,
                &config,
            )
            .unwrap();
        }
        assert_eq!(latest_run(&project).unwrap().name, "three");
    }

    #[test]
    fn no_runs_gives_no_latest() {
        let temp = TempDir::new("empty");
        let project = project(&temp);
        assert!(latest_run(&project).is_none());
        assert!(list_runs(&project).is_empty());
    }

    #[test]
    fn markers_are_added_without_touching_the_history() {
        let temp = TempDir::new("markers");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let run = save_run(
            &project,
            "markers",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        let before = std::fs::read(run.results_path()).unwrap();

        let count = add_marker(&run, 0.25, "Camera start").unwrap();
        assert!(count >= 1);
        let events = run.load_events().unwrap();
        assert!(events
            .iter()
            .any(|e| e.user_added && e.label == "Camera start"));

        // The history is byte-identical, which is what immutability means here.
        let after = std::fs::read(run.results_path()).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn deleting_a_run_removes_only_that_run() {
        let temp = TempDir::new("delete");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let a = save_run(
            &project,
            "Keep",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        let b = save_run(
            &project,
            "Remove",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();

        delete_run(&project, &b.name).unwrap();
        assert!(!b.path.exists());
        assert!(a.path.exists());
        assert_eq!(list_runs(&project).len(), 1);
        assert!(delete_run(&project, &b.name).is_err());
    }

    #[test]
    fn csv_export_has_a_header_and_one_row_per_sample() {
        let temp = TempDir::new("csv");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let run = save_run(
            &project,
            "csv",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();

        let destination = temp.path().join("export.csv");
        let rows = export_history_csv(&run, &destination).unwrap();
        assert_eq!(rows, outcome.samples.len());
        let text = std::fs::read_to_string(&destination).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), rows + 1);
        assert!(lines[0].starts_with("time,position_x"));
        let columns = lines[0].split(',').count();
        assert_eq!(columns, RUN_CHANNELS.len());
        assert_eq!(lines[1].split(',').count(), columns);
    }

    #[test]
    fn to_samples_reconstructs_the_history() {
        let temp = TempDir::new("samples");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let run = save_run(
            &project,
            "samples",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();

        let history = run.load_history().unwrap();
        let rebuilt = history.to_samples();
        assert_eq!(rebuilt.len(), outcome.samples.len());
        for (a, b) in rebuilt.iter().zip(outcome.samples.iter()) {
            assert!((a.time - b.time).abs() < 1e-12);
            assert!((a.position[2] - b.position[2]).abs() < 1e-12);
            assert!((a.attitude[0] - b.attitude[0]).abs() < 1e-12);
            assert!((a.mass - b.mass).abs() < 1e-12);
            assert!((a.moment_body[1] - b.moment_body[1]).abs() < 1e-12);
            assert!((a.dynamic_pressure - b.dynamic_pressure).abs() < 1e-12);
        }
    }

    #[test]
    fn a_truncated_history_reports_integrity_failure() {
        let temp = TempDir::new("truncated");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let run = save_run(
            &project,
            "truncated",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();

        // Chop the last few bytes, as an interrupted write would.
        let bytes = std::fs::read(run.results_path()).unwrap();
        let shortened = &bytes[..bytes.len() - 40];
        std::fs::write(run.results_path(), shortened).unwrap();

        let history = run.load_history().unwrap();
        assert!(!history.is_intact());
        assert!(history.integrity_warning().is_some());
    }

    #[test]
    fn loading_a_corrupt_history_is_an_error_not_a_panic() {
        let temp = TempDir::new("corrupt");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let run = save_run(
            &project,
            "corrupt",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        std::fs::write(run.results_path(), b"not a log at all").unwrap();
        let err = run.load_history().unwrap_err();
        assert!(matches!(err, ProjectError::Parse { .. }));
        assert!(!err.suggested_action().is_empty());
    }

    #[test]
    fn a_missing_history_file_is_an_io_error() {
        let temp = TempDir::new("nohistory");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let run = save_run(
            &project,
            "nohistory",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        std::fs::remove_file(run.results_path()).unwrap();
        assert!(matches!(
            run.load_history().unwrap_err(),
            ProjectError::Io { .. }
        ));
        assert!(!run.is_complete());
    }

    #[test]
    fn run_channel_table_is_unique_and_complete() {
        let mut names: Vec<&str> = RUN_CHANNELS.iter().map(|(_, n, _)| *n).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate channel name");

        let mut selectors: Vec<String> = RUN_CHANNELS
            .iter()
            .map(|(c, _, _)| format!("{:?}", c))
            .collect();
        selectors.sort();
        selectors.dedup();
        assert_eq!(selectors.len(), count, "duplicate selector");
        assert!(count >= 30);
    }

    #[test]
    fn next_run_directory_does_not_create_anything() {
        let temp = TempDir::new("next");
        let project = project(&temp);
        let path = next_run_directory(&project, "My Run");
        assert!(!path.exists());
        assert!(path.to_string_lossy().ends_with("my-run"));
    }

    #[test]
    fn layout_description_names_every_artifact() {
        let description = layout_description();
        for name in [CONFIG_FILE, SUMMARY_FILE, EVENTS_FILE, RESULTS_FILE] {
            assert!(description.contains(name), "{} missing", name);
        }
    }

    #[test]
    fn save_reports_a_size_and_a_label() {
        let temp = TempDir::new("size");
        let project = project(&temp);
        let config = scenario();
        let outcome = run_simulation(config.clone());
        let run = save_run(
            &project,
            "size",
            &outcome.summary,
            &outcome.samples,
            &outcome.events,
            &config,
        )
        .unwrap();
        assert!(run.size_bytes() > 100);
        assert!(run.label().contains("samples"));
    }
}
