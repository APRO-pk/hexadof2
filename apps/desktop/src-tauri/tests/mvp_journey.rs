//! The MVP journey, end to end, on disk.
//!
//! The Tauri commands are thin wrappers around the functions this test calls, so
//! walking them in the order the definition of done lists proves the workflow
//! composes: a project is created, a model is imported and validated, a six
//! degree of freedom run is executed and saved, the saved run is reopened, a
//! telemetry session is recorded from a device stream, a flight log is imported
//! and audited, both sides are aligned and compared, and a report is exported.
//!
//! The assertions are as much about what the software refuses to claim as about
//! what it computes. A comparison never names a single cause, a simulated flight
//! never certifies safety, and a derived channel is never presented as a
//! measurement.

use std::path::{Path, PathBuf};

use hexadof_desktop_lib::commands::analysis::{
    derived_dynamic_pressure, flight_series, metric_channel_from_code, simulation_series,
};
use hexadof_desktop_lib::commands::flight::{csv_options, flight_mappings, FlightImportRequest};
use hexadof_desktop_lib::commands::model::model_example;
use hexadof_desktop_lib::commands::simulation::{
    build_config, replay_from_samples, selector_from_name, view_from_outcome, ChannelRequest,
    ScenarioRequest,
};
use hexadof_desktop_lib::state::StoredRun;

use hex_analysis::{
    compare, detect_events, AlignmentMethod, ComparisonOptions, EventDetectionOptions,
    EventTimeline, FlightSignals, MetricChannel, Signal,
};
use hex_core::{Quaternion, Vec3};
use hex_dynamics::{run_simulation, ChannelSelector, RunOutcome};
use hex_flight_data::{import_path, ChannelRole};
use hex_model::ImportOptions;
use hex_project::{
    import_flight_source, list_runs, save_report, save_run, save_telemetry_session, Project,
    Settings,
};
use hex_telemetry::{
    CalibrationSet, DeviceProfile, IngestionPipeline, ScriptedSource, TelemetryChannel,
};

/// A throwaway directory, removed when the guard is dropped.
struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("hexadof-journey-{}-{}", label, std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("the temporary root must be creatable");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A CSV flight log sampled from the simulation, with the channels a small
/// flight computer records.
///
/// The log is generated from the simulated state plus a deliberate measurement
/// offset, so the comparison has a real difference to report rather than an
/// exact match that would make the test meaningless.
fn flight_csv(outcome: &RunOutcome, decimation: usize) -> String {
    let mut text =
        String::from("time_s,ax,ay,az,gx,gy,gz,gnss_alt,gnss_speed,baro_pa,mag_x,mag_y,mag_z\n");
    let samples = &outcome.samples;
    let count = samples.len();
    for (i, sample) in samples.iter().enumerate() {
        if i % decimation != 0 {
            continue;
        }
        let time = sample.time;
        let previous = &samples[i.saturating_sub(1)];
        let next = &samples[(i + 1).min(count - 1)];
        let span = (next.time - previous.time).max(1e-9);
        let acceleration_world =
            (next.velocity_vec() - previous.velocity_vec()) / span + Vec3::new(0.0, 0.0, 9.80665);
        let specific_force = sample
            .attitude_quaternion()
            .rotate_inverse(acceleration_world);
        let rate = sample.angular_velocity_vec();

        // A barometer reads low by a constant offset, which is what a real
        // uncalibrated sensor does and what the data quality panel should see.
        let altitude = sample.position_vec()[2].max(0.0) + 0.4;
        let pressure = 101_325.0 * (1.0 - 2.255_77e-5 * altitude).powf(5.255_88);
        // A GNSS receiver reports a speed magnitude, never a signed vertical
        // velocity, so apogee has to come from the altitude peak instead.
        let speed = sample.velocity_vec().norm() * 1.01;

        text.push_str(&format!(
            "{:.4},{:.5},{:.5},{:.5},{:.6},{:.6},{:.6},{:.3},{:.3},{:.1},{:.2},{:.2},{:.2}\n",
            time,
            specific_force[0],
            specific_force[1],
            specific_force[2],
            rate[0],
            rate[1],
            rate[2],
            altitude,
            speed,
            pressure,
            21.0,
            -3.0,
            44.0,
        ));
    }
    text
}

/// The import request the flight analysis screen sends for a generated log.
fn flight_request(path: &Path) -> FlightImportRequest {
    FlightImportRequest {
        path: path.to_string_lossy().to_string(),
        delimiter: None,
        has_header: Some(true),
        timestamp_column: Some("time_s".to_string()),
        timestamp_unit: Some("seconds".to_string()),
        tick_rate: None,
        comment_prefix: None,
        max_rows: None,
        decimal_comma: None,
        mappings: Vec::new(),
        expected_rate_hz: Some(50.0),
        gap_factor: None,
        rollover_bits: None,
        accel_range: None,
        gyro_range: None,
        label: Some("Journey flight".to_string()),
        create_session: Some(true),
    }
}

fn channel(code: &str) -> MetricChannel {
    metric_channel_from_code(code).unwrap_or_else(|| panic!("{} is not a comparison channel", code))
}

#[test]
fn the_simulation_and_flight_workflow_runs_end_to_end() {
    let root = TempRoot::new("simulation");
    let settings = Settings::default();

    // 1. Create a project. Every artifact below lands inside it.
    let mut project = Project::create(root.path(), "Journey project").expect("project creation");
    project.save().expect("project metadata");
    assert!(project.layout_is_complete());

    // 2. Import the built-in example model and check the validation report.
    let text = model_example().expect("the built-in example model");
    let imported = hex_model::import_json(&text, &ImportOptions::default()).expect("model import");
    assert_eq!(imported.model_id, "builtin.example.rocket");
    assert!(
        !imported.validation.has_errors(),
        "the built-in example must be usable: {}",
        imported.validation.summary()
    );
    assert!(imported.validation.can_proceed());

    // The model file is copied into the project, not referenced in place.
    let document_path = root.path().join("example.dynamic.json");
    std::fs::write(&document_path, &text).expect("write the model source");
    let reference = project
        .import_model_file(&document_path, Some("example.dynamic.json"))
        .expect("model copy into the project");
    assert_eq!(reference, "models/example.dynamic.json");
    assert!(project.models_dir().join("example.dynamic.json").is_file());
    // The reference is part of the project metadata, so it is written with it.
    project
        .save()
        .expect("project metadata after the model import");

    // 3. Configure a vertical flight and run it.
    let request = ScenarioRequest {
        name: Some("Journey vertical flight".to_string()),
        // Long enough for the whole flight: up, over, and back to the ground.
        end_time: Some(12.0),
        output_interval: Some(0.02),
        rail_length: Some(1.5),
        ..ScenarioRequest::default()
    };
    let config = build_config(&request, &settings, Some(&imported));
    assert!(config.forces.thrust.is_some());
    assert!(config.forces.aero.is_some());

    let outcome = run_simulation(config.clone());
    // The ground plane is a collision surface, so the run ends where the
    // trajectory reaches it rather than continuing through the terrain.
    assert_eq!(outcome.status.label(), "Ended at ground impact");
    assert!(outcome.status.is_usable());
    assert!(outcome.samples.len() > 100, "the run produced a history");
    assert!(
        outcome.summary.maximum_altitude > 1.0,
        "a vertical flight climbs: {}",
        outcome.summary.maximum_altitude
    );
    assert!(outcome.summary.maximum_quaternion_norm_error < 1e-9);
    assert!(
        outcome.events.iter().any(|e| e.label.contains("Apogee")),
        "apogee must be detected in the run, at any output rate"
    );
    assert!(
        outcome
            .events
            .iter()
            .any(|e| e.label.contains("Ground impact")),
        "the ground impact must be reported as an event"
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.code == "flight.ground_impact"),
        "the run must say why it stopped early"
    );

    // 4. Save the run into the project. It is written once, then read only.
    let artifact = save_run(
        &project,
        "Journey vertical flight",
        &outcome.summary,
        &outcome.samples,
        &outcome.events,
        &config,
    )
    .expect("saving the run");
    assert!(artifact.path.join("results.hlog").is_file());

    // 5. Reopen the saved run and confirm the history survives the round trip.
    let listed = list_runs(&project);
    assert_eq!(listed.len(), 1);
    let reopened = &listed[0];
    let history = reopened.load_history().expect("the saved history");
    assert_eq!(history.len(), outcome.samples.len());
    let altitude = history
        .channel(ChannelSelector::Altitude)
        .expect("an altitude column");
    let last_altitude = altitude[altitude.len() - 1];
    assert!(
        last_altitude.abs() < 1e-6,
        "the flight returns to the ground"
    );
    let recovered_config = reopened.load_config().expect("the saved configuration");
    assert_eq!(recovered_config.end_time, 12.0);
    assert_eq!(
        reopened.load_events().expect("saved events").len(),
        outcome.events.len()
    );

    // 6. The results view the plot panel receives.
    let view = view_from_outcome(
        &outcome,
        Some(reopened.name.clone()),
        &ChannelRequest::default(),
    );
    assert!(!view.times.is_empty());
    assert_eq!(view.columns.len(), view.channel_names.len());
    assert!(selector_from_name("altitude").is_some());

    // 7. The 3D replay the viewport consumes.
    let replay = replay_from_samples(
        &outcome.samples,
        outcome.events.clone(),
        "Journey vertical flight",
        1000,
    );
    assert!(!replay.is_empty());
    assert!(replay.len() <= 1000);

    // 8. Write the generated flight log and import it through the same request
    //    the flight analysis screen builds.
    let flight_path = root.path().join("journey-flight.csv");
    std::fs::write(&flight_path, flight_csv(&outcome, 2)).expect("write the flight log");
    let mut flight_request = flight_request(&flight_path);
    flight_request.create_session = Some(true);
    let mappings = flight_mappings(&flight_request);
    assert!(
        mappings.is_empty(),
        "no explicit mapping is supplied, so the importer infers the roles"
    );
    let (mut session, report) = import_path(&flight_path, &csv_options(&flight_request), &mappings)
        .expect("flight log import");
    assert!(report.succeeded, "the import pipeline must succeed");
    assert!(session.times.len() > 50);
    assert!(session.channel(ChannelRole::GnssAltitude).is_some());
    assert!(session.channel(ChannelRole::AccelZ).is_some());
    assert!(session.channel(ChannelRole::GyroX).is_some());
    assert!(session.times.windows(2).all(|w| w[1] > w[0]));

    // 9. The log is copied into the project and never modified in place.
    let before = std::fs::read(&flight_path).expect("read the source log");
    let mut metadata = hex_project::FlightSessionMetadata {
        schema_version: String::new(),
        name: String::new(),
        label: "Journey flight".to_string(),
        source_file: String::new(),
        source_bytes: 0,
        source_hash: String::new(),
        source_format: format!("{:?}", session.source.format),
        imported_at: hex_project::now_iso8601(),
        sample_count: session.times.len(),
        duration_seconds: session.duration(),
        sample_rate_hz: session.sample_rate_hz(),
        channels: session
            .channels
            .iter()
            .map(|c| c.role.label().to_string())
            .collect(),
        derived_channels: Vec::new(),
        events: Vec::new(),
        quality_flags: session
            .quality
            .flags
            .iter()
            .map(|f| f.code.clone())
            .collect(),
        original_preserved: false,
    };
    let flight_artifact = import_flight_source(
        &project,
        "Journey flight",
        &flight_path,
        &mut metadata,
        &serde_json::json!({}),
        &serde_json::json!({}),
        &[],
        &[],
        &[],
    )
    .expect("the flight session is stored in the project");
    assert!(flight_artifact.metadata.original_preserved);
    assert!(flight_artifact.path.join("source.csv").is_file());
    assert_eq!(
        std::fs::read(&flight_path).expect("re-read the source log"),
        before,
        "the original log must never be modified"
    );

    // 10. Detect the flight events. Apogee comes from the altitude peak here,
    //     because a GNSS speed has no sign and cannot show a descent.
    let signals = FlightSignals {
        altitude: session.channel(ChannelRole::GnssAltitude).map(|c| {
            Signal::new(
                "gnss altitude",
                session.times.clone(),
                c.corrected_values(),
                "m",
            )
        }),
        vertical_velocity: session.channel(ChannelRole::GnssSpeed).map(|c| {
            Signal::new(
                "gnss speed",
                session.times.clone(),
                c.corrected_values(),
                "m/s",
            )
        }),
        specific_force: session.channel(ChannelRole::AccelZ).map(|c| {
            Signal::new(
                "specific force",
                session.times.clone(),
                c.corrected_values(),
                "m/s^2",
            )
        }),
        dynamic_pressure: derived_dynamic_pressure(&session),
        thrust: None,
        pressure: session.channel(ChannelRole::BaroPressure).map(|c| {
            Signal::new(
                "pressure",
                session.times.clone(),
                c.corrected_values(),
                "Pa",
            )
        }),
    };
    let detected = detect_events(&signals, &EventDetectionOptions::default());
    let apogee = detected
        .iter()
        .find(|e| e.code == "apogee")
        .expect("apogee must be detected");
    assert_eq!(apogee.channel, "gnss altitude");
    assert!(
        apogee
            .note
            .as_deref()
            .unwrap_or("")
            .contains("altitude peak"),
        "an inferred apogee must say how it was inferred"
    );
    assert!(
        detected.iter().any(|e| e.code == "max_q"),
        "dynamic pressure is derived from the GNSS speed"
    );
    let peak_q = detected.iter().find(|e| e.code == "max_q").expect("max q");
    assert!(
        peak_q.value > 1.0 && peak_q.value < 20_000.0,
        "a derived q must be a plausible aerodynamic pressure, not an ambient one: {}",
        peak_q.value
    );
    assert!(
        peak_q.channel.contains("derived"),
        "a derived event must carry its derivation: {}",
        peak_q.channel
    );

    // 10b. Mark an event by hand. A marker is an observation, not a detection,
    //      so it must stay labelled as one wherever it is shown.
    session.add_marker(apogee.time, "Visual apogee");
    let timeline = hexadof_desktop_lib::commands::analysis::flight_timeline(&session, None);
    let marker = timeline
        .events
        .iter()
        .find(|e| e.user_added)
        .expect("the marker must appear in the timeline");
    assert_eq!(marker.label, "Visual apogee");
    assert_eq!(marker.evidence, hex_analysis::EventEvidence::Manual);
    assert!(
        timeline
            .events
            .iter()
            .any(|e| e.code == "apogee" && !e.user_added),
        "the detected apogee is still there alongside the marker"
    );
    let stored_markers = flight_artifact
        .add_marker(apogee.time, "Visual apogee")
        .expect("the marker is stored with the session");
    assert_eq!(stored_markers.len(), 1);
    assert_eq!(flight_artifact.load_events().len(), 1);
    assert!(flight_artifact.source_is_intact());

    // 11. Align and compare. The flight log was sampled from the run, so the
    //     alignment offset must be small but the reported difference nonzero.
    let stored = StoredRun {
        outcome,
        artifact_name: Some(reopened.name.clone()),
    };
    let outcome = &stored.outcome;
    let channels = vec![
        channel("altitude"),
        channel("speed"),
        channel("acceleration"),
        channel("attitude"),
    ];
    let simulation = simulation_series(&stored, &channels);
    let flight = flight_series(&session, &channels);
    assert!(flight.has_position, "the log carries a measured altitude");
    let mut options = ComparisonOptions {
        method: AlignmentMethod::FirstSample,
        channels: channels.clone(),
        ..ComparisonOptions::default()
    };
    let comparison = compare(&simulation, &flight, &options).expect("the comparison builds");
    assert_eq!(comparison.simulation_name, "journey-vertical-flight");
    assert!(comparison.alignment.offset_seconds.abs() < 0.2);
    let altitude_row = comparison
        .metrics
        .rows
        .iter()
        .find(|r| r.channel == MetricChannel::Altitude)
        .expect("an altitude row");
    let altitude_metrics = altitude_row.scalar.as_ref().expect("altitude metrics");
    assert!(altitude_metrics.samples > 50);
    assert!(
        altitude_metrics.root_mean_square_error < 5.0,
        "the log was sampled from the run: {}",
        altitude_metrics.root_mean_square_error
    );
    assert!(
        altitude_metrics.root_mean_square_error > 0.0,
        "the measured offset must show up rather than being rounded away"
    );
    let attitude_row = comparison
        .metrics
        .rows
        .iter()
        .find(|r| r.channel == MetricChannel::Attitude)
        .expect("an attitude row");
    assert!(
        attitude_row.attitude.is_some(),
        "attitude is compared as a rotation distance"
    );

    // A relative attitude is a relative attitude, whatever it is compared with.
    assert!(
        comparison
            .warnings
            .iter()
            .any(|w| w.detail.to_lowercase().contains("gyro")
                || w.detail.to_lowercase().contains("relative")
                || w.detail.to_lowercase().contains("drift")),
        "the comparison must warn that the flight attitude is relative: {:?}",
        comparison.warnings
    );

    // Aligning on a detected event is offered and works.
    options.method = AlignmentMethod::Event {
        reference_event: hex_analysis::EventRef::new("apogee"),
        comparison_event: hex_analysis::EventRef::new("apogee"),
    };
    let event_aligned = compare(&simulation, &flight, &options).expect("event alignment");
    assert!(matches!(
        event_aligned.alignment_method,
        AlignmentMethod::Event { .. }
    ));

    // 12. Export a report. It travels away from the application, so it has to
    //     carry the positioning with it.
    let timeline = EventTimeline::new(
        hexadof_desktop_lib::commands::analysis::detected_from_run_events(&outcome.events),
        outcome.summary.duration,
    );
    let mut body = String::new();
    body.push_str("# Journey project report\n\n");
    body.push_str(
        "HexaDOF is an analysis and instrumentation tool. A simulation is a model, \
         not a test flight, and these results do not certify flight safety.\n\n",
    );
    body.push_str(&format!(
        "- Maximum altitude: {:.3} m\n",
        outcome.summary.maximum_altitude
    ));
    body.push_str(&format!("- Samples: {}\n\n", outcome.summary.sample_count));
    body.push_str(&timeline.to_text());
    body.push('\n');
    body.push_str(&comparison.to_text());
    let report_path =
        save_report(&project, "Journey report", &body).expect("the report is written");
    let report = std::fs::read_to_string(&report_path).expect("read the report");
    assert!(report.contains("do not certify flight safety"));
    assert!(report.contains("does not identify a single cause"));
    assert!(report.contains("Apogee"));

    // 13. The project inventory reflects everything that was produced.
    let inventory = project.inventory();
    assert_eq!(inventory.models, 1);
    assert_eq!(inventory.simulations, 1);
    assert_eq!(inventory.flight_logs, 1);
    assert_eq!(inventory.reports, 1);
    assert!(!inventory.is_empty());

    // The project reopens with its metadata intact.
    let reopened_project = Project::open(root.path()).expect("the project reopens");
    assert_eq!(reopened_project.metadata.name, "Journey project");
    assert_eq!(
        reopened_project.metadata.model_reference.as_deref(),
        Some("models/example.dynamic.json")
    );
}

#[test]
fn a_recorded_telemetry_session_survives_the_codec() {
    let root = TempRoot::new("telemetry");
    let mut project = Project::create(root.path(), "Telemetry project").expect("project creation");
    project.save().expect("project metadata");

    // A scripted device stream in the format the example profile declares.
    let mut text = String::new();
    for i in 0..400 {
        let time = i as f64 * 0.01;
        let rate = 0.2 + 0.4 * (i as f64 / 400.0);
        text.push_str(&format!(
            "{:.4},{:.5},{:.5},{:.5},{:.6},{:.6},{:.6},{:.2}\n",
            time,
            0.02 * (time * 6.0).sin(),
            0.02 * (time * 5.0).cos(),
            -9.80665 + 0.1 * (time * 8.0).sin(),
            0.01,
            0.0,
            rate,
            101_325.0 - 120.0 * time
        ));
    }

    let mut pipeline = IngestionPipeline::new(
        DeviceProfile::example_csv_profile(),
        CalibrationSet::default(),
        10_000,
    );
    pipeline.start_recording();
    let mut source = ScriptedSource::new(text.clone().into_bytes())
        .with_chunk(256)
        .with_description("scripted journey stream");
    while source.remaining() > 0 {
        pipeline
            .pump(&mut source, 512)
            .expect("the scripted source never fails");
    }
    pipeline.stop_recording();

    let recorder = pipeline.recorder();
    assert_eq!(recorder.len(), 400, "every packet is recorded");
    assert_eq!(
        pipeline.counters().samples_recorded,
        400,
        "the recorder holds every sample the decoder produced"
    );

    // The columns the live console shows.
    let channels = [
        TelemetryChannel::HostTime,
        TelemetryChannel::AccelX,
        TelemetryChannel::AccelY,
        TelemetryChannel::AccelZ,
        TelemetryChannel::GyroX,
        TelemetryChannel::GyroY,
        TelemetryChannel::GyroZ,
        TelemetryChannel::Pressure,
    ];
    let names: Vec<String> = channels.iter().map(|c| c.label().to_string()).collect();
    let columns: Vec<Vec<f64>> = channels
        .iter()
        .map(|c| c.values(recorder.samples()))
        .collect();
    let times = recorder.times();
    assert_eq!(columns[0].len(), times.len());
    assert!(times.windows(2).all(|w| w[1] > w[0]));
    eprintln!(
        "DEBUG recorder times first {:?} last {:?} count {}",
        &times[..3],
        &times[times.len() - 3..],
        times.len()
    );
    // Every packet decoded, and the display policy never changed what was kept.
    assert!(pipeline.counters().packets_decoded >= 400);
    assert_eq!(pipeline.counters().packets_rejected, 0);
    assert_eq!(pipeline.counters().samples_dropped_for_display, 0);

    let metadata = hex_project::TelemetrySessionMetadata {
        schema_version: String::new(),
        name: String::new(),
        label: "Journey telemetry".to_string(),
        profile_id: pipeline.profile().id.clone(),
        profile_name: pipeline.profile().name.clone(),
        port: "scripted".to_string(),
        baud_rate: pipeline.profile().serial.baud_rate,
        packet_format: pipeline.profile().format.label().to_string(),
        started_at: hex_project::now_iso8601(),
        duration_seconds: times[times.len() - 1] - times[0],
        sample_count: recorder.len() as u64,
        average_rate_hz: 100.0,
        packets_rejected: pipeline.counters().packets_rejected,
        bytes_received: text.len() as u64,
        display_downsampled: false,
        channels: names.clone(),
        calibration: pipeline.calibration().summary(),
        warnings: vec![
            "This session was recorded from a scripted stream, not from a device.".to_string(),
        ],
    };
    let artifact = save_telemetry_session(
        &project,
        "Journey telemetry",
        &metadata,
        &names,
        &columns,
        &times,
    )
    .expect("the session is saved");
    assert!(artifact.log_path.is_file());
    assert_eq!(
        artifact.metadata.warnings.len(),
        1,
        "a synthetic session must say so where it is stored"
    );

    // The same reader that serves a flight log reads the session back.
    let bytes = std::fs::read(&artifact.log_path).expect("read the session log");
    let (round_trip, report) = hex_flight_data::import_binary(&bytes, &[]).expect("decode the log");
    assert!(report.succeeded);
    assert_eq!(round_trip.times.len(), times.len());
    // Times inside a record are placed on the nominal interval, so the host
    // clock's own jitter is not stored. The error must stay far below one sample
    // interval, and a real gap must start a new record rather than be smeared.
    let worst = round_trip
        .times
        .iter()
        .zip(times.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f64, f64::max);
    assert!(
        worst < 0.001,
        "the reconstructed times drifted by {} s, which is not jitter",
        worst
    );
    assert!(round_trip.times.windows(2).all(|w| w[1] > w[0]));
    let stored = round_trip
        .channels
        .iter()
        .find(|c| c.name == names[3])
        .expect("the acceleration column survives under its own name");
    assert_eq!(stored.values.len(), columns[3].len());

    let listed = hex_project::list_telemetry_sessions(&project);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].metadata.sample_count, recorder.len() as u64);
}

#[test]
fn settings_round_trip_both_themes() {
    let root = TempRoot::new("settings");
    let path = root.path().join("settings.json");

    let mut settings = Settings::default();
    // The default follows the operating system, so both preferences resolve and
    // neither leaves the interface unthemed.
    assert_eq!(settings.appearance.theme, hex_project::Theme::System);
    assert_eq!(settings.resolved_theme(true).label(), "Dark");
    assert_eq!(settings.resolved_theme(false).label(), "Light");
    settings.appearance.theme = hex_project::Theme::Light;
    settings.appearance.reduced_motion = true;
    settings.save(&path).expect("settings are written");

    let (mut loaded, warnings) = Settings::load_or_default(&path);
    assert!(warnings.is_empty(), "{:?}", warnings);
    assert_eq!(loaded.appearance.theme, hex_project::Theme::Light);
    assert!(loaded.appearance.reduced_motion);
    // An explicit choice overrides the system preference in both directions.
    assert_eq!(loaded.resolved_theme(true).label(), "Light");
    assert_eq!(loaded.resolved_theme(false).label(), "Light");
    loaded.appearance.theme = hex_project::Theme::Dark;
    assert_eq!(loaded.resolved_theme(false).label(), "Dark");
    assert!(loaded.validate().is_empty());

    // A corrupt settings file falls back to the defaults rather than refusing to
    // start, and says so.
    std::fs::write(&path, "{ this is not json").expect("write a corrupt file");
    let (fallback, warnings) = Settings::load_or_default(&path);
    assert!(
        !warnings.is_empty(),
        "a discarded settings file must be reported"
    );
    assert_eq!(
        fallback.appearance.theme,
        Settings::default().appearance.theme
    );
}

#[test]
fn the_flight_attitude_estimate_is_never_presented_as_absolute() {
    let root = TempRoot::new("attitude");
    let _project = Project::create(root.path(), "Attitude project").expect("project creation");

    // A log with gyroscopes only: the attitude is integrated and therefore
    // relative, and every label has to say so.
    let mut text = String::from("time_s,gx,gy,gz\n");
    for i in 0..200 {
        let t = i as f64 * 0.01;
        text.push_str(&format!("{:.4},{:.6},{:.6},{:.6}\n", t, 0.01, 0.0, 0.5));
    }
    let path = root.path().join("gyro-only.csv");
    std::fs::write(&path, text).expect("write the gyro log");
    let request = FlightImportRequest {
        path: path.to_string_lossy().to_string(),
        has_header: Some(true),
        timestamp_column: Some("time_s".to_string()),
        timestamp_unit: Some("seconds".to_string()),
        label: None,
        create_session: Some(false),
        ..flight_request(&path)
    };
    let (session, report) = import_path(&path, &csv_options(&request), &flight_mappings(&request))
        .expect("gyro only import");
    assert!(report.succeeded);

    let series = flight_series(&session, &[MetricChannel::Attitude]);
    assert!(
        !series.has_position,
        "a gyro only log has no position to compare"
    );
    assert!(
        series
            .warnings
            .iter()
            .any(|w| w.contains("no measured position")),
        "the missing position must be stated: {:?}",
        series.warnings
    );
    let attitude = series
        .channels
        .iter()
        .find(|c| c.channel == MetricChannel::Attitude);
    assert!(
        attitude.is_none(),
        "attitude is carried as a rotation series, not as a scalar column"
    );
    let quaternions = hex_flight_data::propagate_gyro_attitude(
        &session.times,
        &(0..session.times.len())
            .map(|_| Vec3::new(0.01, 0.0, 0.5))
            .collect::<Vec<_>>(),
        Quaternion::identity(),
        Vec3::zeros(),
    );
    assert_eq!(quaternions.len(), session.times.len());
    let drift = quaternions[quaternions.len() - 1].angular_distance(&Quaternion::identity());
    assert!(
        drift > 0.5,
        "an unobservable yaw must drift visibly, not appear stable: {}",
        drift
    );
}

/// The crate must not expose a way to claim a simulated run is a flight.
#[test]
fn a_simulation_result_is_never_reported_as_measured() {
    // A real, if very short, run, so the provenance claim is checked against the
    // same outcome type the interface receives.
    let config = hex_dynamics::RunConfig {
        end_time: 0.05,
        output_interval: 0.01,
        ..hex_dynamics::RunConfig::default()
    };
    let outcome = run_simulation(config);
    assert!(!outcome.samples.is_empty());

    let series = simulation_series(
        &StoredRun {
            outcome,
            artifact_name: None,
        },
        &[MetricChannel::Altitude, MetricChannel::Attitude],
    );
    assert!(
        series.has_position,
        "a run always has a position to compare"
    );
    for channel in &series.channels {
        assert_eq!(
            channel.provenance,
            hex_analysis::Provenance::Simulated,
            "every simulation channel is simulated"
        );
    }
    let attitude = series.attitude.as_ref().expect("the attitude series");
    assert_eq!(attitude.provenance, hex_analysis::Provenance::Simulated);
}
