/**
 * The example flight the Dynamics screen offers.
 *
 * The Configuration panel has one button that installs a complete example: the
 * built-in model and a sounding rocket scenario. That is a promise, and a promise
 * is only worth anything if those exact numbers fly. The scenario below mirrors
 * `exampleFlightScenario()` in the frontend store, and it is duplicated here on
 * purpose: a test that reads the numbers from the interface would not notice the
 * interface and the physics drifting apart.
 */
use hex_dynamics::run_simulation;
use hex_model::ImportOptions;
use hex_project::Settings;
use hexadof_desktop_lib::commands::model::model_example;
use hexadof_desktop_lib::commands::simulation::{build_config, ScenarioRequest};

/// A 4.5 kg sounding rocket: 180 N for 2.4 s from a 1.5 m rail, drag coefficient
/// 0.45 against the 125 mm body the example model declares.
fn example_flight_request() -> ScenarioRequest {
    ScenarioRequest {
        name: Some("Example sounding rocket".to_string()),
        mode: Some("six_dof".to_string()),
        start_time: Some(0.0),
        end_time: Some(20.0),
        output_interval: Some(0.02),
        step: Some(0.0005),
        position: Some([0.0, 0.0, 0.0]),
        velocity: Some([0.0, 0.0, 0.0]),
        // Nose up: -90 degrees of pitch points body X along world +Z.
        euler: Some([0.0, -std::f64::consts::FRAC_PI_2, 0.0]),
        angular_velocity: Some([0.0, 0.0, 0.0]),
        mass: Some(4.5),
        inertia: Some([0.09, 0.09, 0.012, 0.0, 0.0, 0.0]),
        thrust: Some(180.0),
        burn_time: Some(2.4),
        rail_length: Some(1.5),
        drag_coefficient: Some(0.45),
        aero_source: Some("auto".to_string()),
        ground_elevation: Some(0.0),
        ..ScenarioRequest::default()
    }
}

#[test]
fn the_drop_test_has_no_motor_and_still_finishes() {
    // The example list offers a pure dynamics case: released at 300 m with no
    // thrust at all. It has to validate, run, and reach the ground, and it has to
    // report energy drift, because that diagnostic is only meaningful without a
    // motor adding energy.
    let settings = Settings::default();
    let text = model_example().expect("the built-in example model");
    let imported = hex_model::import_json(&text, &ImportOptions::default()).expect("model import");

    let request = ScenarioRequest {
        name: Some("Drop test".to_string()),
        end_time: Some(30.0),
        output_interval: Some(0.02),
        position: Some([0.0, 0.0, 300.0]),
        velocity: Some([0.0, 0.0, 0.0]),
        thrust: None,
        burn_time: None,
        rail_length: Some(0.0),
        drag_coefficient: Some(0.45),
        aero_source: Some("estimate".to_string()),
        ..ScenarioRequest::default()
    };

    let config = build_config(&request, &settings, Some(&imported));
    assert!(config.forces.thrust.is_none(), "there is no motor");
    assert!(config.forces.aero.is_some(), "but there is drag");

    let outcome = run_simulation(config);
    assert!(
        outcome.summary.maximum_altitude <= 300.0,
        "it starts at 300 m and only falls: {}",
        outcome.summary.maximum_altitude
    );
    assert!(
        outcome.summary.energy_drift_ratio.is_some(),
        "a run with no thrust reports energy drift"
    );
    assert!(
        outcome
            .events
            .iter()
            .any(|event| event.label == "Ground impact"),
        "{:?}",
        outcome
            .events
            .iter()
            .map(|e| e.label.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_example_flight_is_a_plausible_sounding_rocket() {
    let settings = Settings::default();
    let text = model_example().expect("the built-in example model");
    let imported = hex_model::import_json(&text, &ImportOptions::default()).expect("model import");
    assert!(
        imported.validation.can_proceed(),
        "the example model must be usable: {}",
        imported.validation.summary()
    );

    let config = build_config(&example_flight_request(), &settings, Some(&imported));
    assert!(config.forces.thrust.is_some(), "the example has a motor");
    assert!(config.forces.aero.is_some(), "the example has aerodynamics");

    let outcome = run_simulation(config);

    // It has to leave the rail, climb to something worth watching, and stay a
    // sounding rocket rather than reaching orbit.
    assert!(
        outcome.summary.maximum_speed > 50.0,
        "the example has to leave the rail properly: {} m/s",
        outcome.summary.maximum_speed
    );
    assert!(
        outcome.summary.maximum_altitude > 150.0,
        "the example has to be worth watching: apogee was {} m",
        outcome.summary.maximum_altitude
    );
    assert!(
        outcome.summary.maximum_altitude < 2000.0,
        "and it has to stay a sounding rocket: {} m",
        outcome.summary.maximum_altitude
    );

    // Drag scales with the reference area, so the geometry the example model
    // declares is what keeps the flight physical. At the one square metre a
    // minimal document carries, this same drag coefficient would dominate the
    // whole flight and the example would be quietly wrong.
    assert!(
        (imported.reference_geometry.reference_area - 0.0123).abs() < 1e-12,
        "the example model must declare the area its coefficients are quoted against: {}",
        imported.reference_geometry.reference_area
    );

    // The timeline shades the flight between the events that delimit its phases,
    // so those events have to be in the record for the shading to mean anything.
    // The label is what the reader sees; the kind is what the timeline matches on.
    let labels: Vec<&str> = outcome
        .events
        .iter()
        .map(|event| event.label.as_str())
        .collect();
    let kinds: Vec<&str> = outcome
        .events
        .iter()
        .map(|event| event.kind.code())
        .collect();
    for expected in ["ignition", "burnout", "apogee", "ground_impact"] {
        assert!(
            kinds.contains(&expected),
            "the timeline draws a phase boundary at {}: {:?}",
            expected,
            kinds
        );
    }
    assert!(
        labels.iter().any(|label| label.contains("Lift-off")),
        "{:?}",
        labels
    );
    assert!(
        labels.iter().any(|label| label.contains("Motor burnout")),
        "{:?}",
        labels
    );
    assert!(
        labels.iter().any(|label| label.contains("Apogee")),
        "{:?}",
        labels
    );
    assert!(
        labels.iter().any(|label| label.contains("Ground impact")),
        "{:?}",
        labels
    );

    // And the run has to be numerically sound: a quaternion that drifts is a sign
    // the scenario is not the quiet vertical flight it claims to be.
    assert!(outcome.summary.maximum_quaternion_norm_error < 1e-9);
    assert!(outcome.samples.len() > 100, "the run produced a history");
}
