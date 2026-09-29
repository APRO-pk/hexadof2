/**
 * Self landing flights.
 *
 * The two landing examples the Dynamics screen offers depend on one piece of
 * physics: a landing burn that lights on the stopping-distance condition. This
 * test flies both of them and checks the thing that matters, which is the speed
 * at touchdown. A landing example that arrives at 40 m/s is not a landing.
 */
use hex_core::Vec3;
use hex_dynamics::{
    run_simulation, Environment, ForceConfig, InitialConditions, LandingBurn, MassSpec, RunConfig,
    SolverKind, ThrustProfile,
};
use hex_model::ImportOptions;
use hex_project::Settings;
use hexadof_desktop_lib::commands::model::model_example;
use hexadof_desktop_lib::commands::simulation::{
    build_config, LandingBurnRequest, ScenarioRequest,
};

/// The Starhopper style hop: a short burn, a low apogee, a long fall, and a
/// landing burn.
fn hop_request() -> ScenarioRequest {
    ScenarioRequest {
        name: Some("Starhopper hop".to_string()),
        mode: Some("six_dof".to_string()),
        end_time: Some(40.0),
        output_interval: Some(0.02),
        step: Some(0.0005),
        euler: Some([0.0, -std::f64::consts::FRAC_PI_2, 0.0]),
        mass: Some(4.5),
        inertia: Some([0.09, 0.09, 0.012, 0.0, 0.0, 0.0]),
        // A short, hard burn: up and then off.
        thrust: Some(140.0),
        burn_time: Some(1.6),
        rail_length: Some(0.0),
        drag_coefficient: Some(0.45),
        aero_source: Some("auto".to_string()),
        landing_burn: Some(LandingBurnRequest {
            thrust: 90.0,
            deceleration_margin: Some(1.0),
            maximum_ignition_altitude: Some(120.0),
            minimum_descent_rate: Some(0.2),
        }),
        ..ScenarioRequest::default()
    }
}

/// The hover slam: a higher apogee and a longer fall, so the landing burn has
/// more speed to take out.
fn hover_slam_request() -> ScenarioRequest {
    ScenarioRequest {
        name: Some("Self landing hover slam".to_string()),
        end_time: Some(60.0),
        output_interval: Some(0.01),
        thrust: Some(220.0),
        burn_time: Some(2.6),
        landing_burn: Some(LandingBurnRequest {
            thrust: 110.0,
            deceleration_margin: Some(1.5),
            maximum_ignition_altitude: Some(400.0),
            minimum_descent_rate: Some(0.2),
        }),
        ..hop_request()
    }
}

struct Landing {
    touchdown_speed: f64,
    apogee: f64,
    events: Vec<String>,
    /// How the run ended, which for a landing is a touchdown on the ground plane.
    status: String,
}

/// A quaternion for a rotation of `pitch` about body Y, scalar first.
fn quaternion_from_pitch(pitch: f64) -> [f64; 4] {
    let q = hex_core::Quaternion::from_axis_angle(Vec3::y(), pitch);
    [q.w, q.x, q.y, q.z]
}

fn fly(request: &ScenarioRequest) -> Landing {
    let settings = Settings::default();
    let text = model_example().expect("the built-in example model");
    let imported = hex_model::import_json(&text, &ImportOptions::default()).expect("model import");
    let config = build_config(request, &settings, Some(&imported));
    // The control case has no landing burn on purpose, so the assertion belongs
    // to the caller that expects one rather than here.
    assert_eq!(
        config.forces.landing_burn.is_some(),
        request.landing_burn.is_some(),
        "the scenario has to carry the landing burn it asked for"
    );

    let outcome = run_simulation(config);
    let last = outcome.samples.last().expect("a recorded history");
    let speed = Vec3::new(last.velocity[0], last.velocity[1], last.velocity[2]).norm();
    Landing {
        touchdown_speed: speed,
        apogee: outcome.summary.maximum_altitude,
        events: outcome.events.iter().map(|e| e.label.clone()).collect(),
        status: outcome.status.label().to_string(),
    }
}

#[test]
fn the_starhopper_hop_climbs_then_lands_gently() {
    let landing = fly(&hop_request());

    assert!(
        landing.apogee > 10.0,
        "the hop has to leave the ground: {} m",
        landing.apogee
    );
    assert!(
        landing.events.iter().any(|e| e == "Landing burn"),
        "the landing burn has to be in the record: {:?}",
        landing.events
    );
    assert!(
        landing.events.iter().any(|e| e == "Apogee"),
        "{:?}",
        landing.events
    );
    // The whole point of the example: it arrives softly.
    assert!(
        landing.touchdown_speed < 6.0,
        "a landing without a landing burn arrives at tens of metres per second: {:.2} m/s",
        landing.touchdown_speed
    );
    assert_eq!(
        landing.status, "Ended at ground impact",
        "the run has to reach the ground rather than hover above it"
    );
}

#[test]
fn the_hover_slam_takes_more_speed_out_than_it_would_fall_with() {
    let landing = fly(&hover_slam_request());
    assert!(
        landing.apogee > 50.0,
        "the hover slam has to fall from somewhere: {} m",
        landing.apogee
    );
    assert!(
        landing.events.iter().any(|e| e == "Landing burn"),
        "{:?}",
        landing.events
    );
    assert!(
        landing.touchdown_speed < 6.0,
        "touchdown was {:.2} m/s",
        landing.touchdown_speed
    );

    // And the comparison that gives the number its meaning: the same vehicle
    // falling from apogee with no landing burn.
    let mut ballistic = hover_slam_request();
    ballistic.landing_burn = None;
    ballistic.end_time = Some(120.0);
    let without = fly(&ballistic);
    assert!(
        without.touchdown_speed > 3.0 * landing.touchdown_speed,
        "the landing burn has to be doing the work: {:.2} m/s with it, {:.2} m/s without",
        landing.touchdown_speed,
        without.touchdown_speed
    );
    assert!(
        !without.events.iter().any(|e| e == "Landing burn"),
        "an unconfigured burn is never reported"
    );
}

#[test]
fn a_landing_burn_without_enough_thrust_is_refused_by_validation() {
    // 30 N on 4.5 kg is less than the vehicle's weight, so no stopping distance
    // exists. Validation has to say so rather than run a flight that cannot land.
    let mut request = hop_request();
    request.landing_burn = Some(LandingBurnRequest {
        thrust: 30.0,
        deceleration_margin: Some(1.0),
        maximum_ignition_altitude: None,
        minimum_descent_rate: None,
    });
    let settings = Settings::default();
    let text = model_example().unwrap();
    let imported = hex_model::import_json(&text, &ImportOptions::default()).unwrap();
    let config = build_config(&request, &settings, Some(&imported));

    // The build still produces a provider, and it refuses to fire.
    let burn = config.forces.landing_burn.clone().expect("a landing burn");
    assert!(!burn.can_stop(config.initial.mass, 9.80665));

    let outcome = run_simulation(config);
    assert!(
        !outcome.events.iter().any(|e| e.label == "Landing burn"),
        "a burn that cannot stop the vehicle must never light: {:?}",
        outcome
            .events
            .iter()
            .map(|e| e.label.clone())
            .collect::<Vec<_>>()
    );
    let last = outcome.samples.last().expect("a recorded history");
    let speed = Vec3::new(last.velocity[0], last.velocity[1], last.velocity[2]).norm();
    assert!(
        speed > 6.0,
        "and it has to arrive hard, which is the honest result: {:.2} m/s",
        speed
    );
}

#[test]
fn a_landing_burn_is_reported_as_its_own_force_source() {
    // The force breakdown names it, so a reader can see the landing pulse in the
    // forces rather than having to guess what moved the vehicle.
    let settings = Settings::default();
    let mut config = RunConfig {
        name: "landing source".to_string(),
        start_time: 0.0,
        end_time: 6.0,
        output_interval: 0.01,
        solver: SolverKind::Rk4 { step: 0.0005 },
        initial: InitialConditions {
            position: [0.0, 0.0, 12.0],
            velocity: [0.0, 0.0, -18.0],
            attitude: quaternion_from_pitch(-std::f64::consts::FRAC_PI_2),
            mass: 4.5,
            ..InitialConditions::at_rest(4.5)
        },
        mass: MassSpec::constant(4.5, hex_core::InertiaTensor::diagonal(0.09, 0.09, 0.012)),
        environment: Environment::default(),
        forces: ForceConfig {
            landing_burn: Some(LandingBurn {
                thrust: 90.0,
                deceleration_margin: 1.0,
                ..LandingBurn::default()
            }),
            ..ForceConfig::default()
        },
        ..RunConfig::default()
    };
    config.forces.thrust = Some(ThrustProfile::default());
    let outcome = run_simulation(config);
    let chain = outcome.summary.active_providers.join(", ");
    assert!(
        chain.contains("landing_burn"),
        "the active provider list names it: {}",
        chain
    );
    assert!(
        outcome
            .summary
            .force_models
            .iter()
            .any(|line| line.contains("Landing burn")),
        "and so does the model description: {:?}",
        outcome.summary.force_models
    );
    let _ = settings;
}
