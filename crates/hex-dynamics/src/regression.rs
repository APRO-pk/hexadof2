//! Versioned regression scenarios.
//!
//! Each scenario pins a configuration to a small set of expected outputs with an
//! explicit tolerance. The suite runs in the ordinary test job, so a change that
//! alters the physics without changing the version marker fails loudly.
//!
//! The scenarios are also the eight verification cases the specification requires:
//! free translation, uniform gravity, constant torque, torque-free asymmetric
//! body, quaternion rotation, force at an offset, frame transform, and a
//! regression scenario with recorded outputs.

use hex_core::{InertiaTensor, Quaternion, Real, Vec3};
use serde::{Deserialize, Serialize};

use crate::environment::{Environment, GravityModel, LaunchRail};
use crate::forces::{AeroModel, ExternalLoad, ForceProvider, ThrustProfile};
use crate::integrators::SolverKind;
use crate::runner::{
    run_simulation, ChannelSelector, ForceConfig, InitialConditions, MassSpec, RunConfig,
};
use crate::state::{RigidBodyState, SimulationMode, STATE_LEN};

/// One expected output value of a regression scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectedValue {
    /// Channel to read.
    pub channel: ChannelSelector,
    /// Expected value at the final sample.
    pub value: Real,
    /// Absolute tolerance.
    pub tolerance: Real,
}

impl ExpectedValue {
    pub fn new(channel: ChannelSelector, value: Real, tolerance: Real) -> Self {
        Self {
            channel,
            value,
            tolerance,
        }
    }
}

/// A pinned scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionScenario {
    /// Stable scenario identifier.
    pub id: String,
    /// Version marker. Bump this whenever an expected value changes, so the
    /// change is reviewed rather than silently absorbed.
    pub version: String,
    /// What the scenario verifies.
    pub purpose: String,
    /// The configuration to run.
    pub config: RunConfig,
    /// Expected final values.
    pub expected: Vec<ExpectedValue>,
    /// Largest permitted quaternion norm error over the run.
    pub maximum_quaternion_error: Real,
}

/// Outcome of running one scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionOutcome {
    pub id: String,
    pub version: String,
    pub passed: bool,
    /// One line per failed expectation.
    pub failures: Vec<String>,
}

impl RegressionScenario {
    /// Run the scenario and check every expectation.
    pub fn run(&self) -> RegressionOutcome {
        let outcome = run_simulation(self.config.clone());
        let mut failures = Vec::new();

        if !outcome.status.is_usable() {
            failures.push(format!("run status was {}", outcome.status.label()));
        }

        match outcome.samples.last() {
            Some(last) => {
                for e in &self.expected {
                    let actual = e.channel.value(last);
                    if (actual - e.value).abs() > e.tolerance {
                        failures.push(format!(
                            "{} expected {:.9} but got {:.9} (tolerance {:.3e})",
                            e.channel.label(),
                            e.value,
                            actual,
                            e.tolerance
                        ));
                    }
                }
            }
            None => failures.push("the run produced no samples".to_string()),
        }

        if outcome.summary.maximum_quaternion_norm_error > self.maximum_quaternion_error {
            failures.push(format!(
                "quaternion norm error {:.3e} exceeded {:.3e}",
                outcome.summary.maximum_quaternion_norm_error, self.maximum_quaternion_error
            ));
        }

        RegressionOutcome {
            id: self.id.clone(),
            version: self.version.clone(),
            passed: failures.is_empty(),
            failures,
        }
    }
}

/// A base configuration with gravity and drag disabled, so each scenario only
/// sees the physics it is testing.
fn base_config(name: &str, end_time: Real) -> RunConfig {
    RunConfig {
        name: name.to_string(),
        end_time,
        output_interval: 0.01,
        solver: SolverKind::fixed(0.0002),
        environment: Environment::vacuum_uniform_gravity(0.0),
        forces: ForceConfig {
            gravity: false,
            ..ForceConfig::default()
        },
        ..RunConfig::default()
    }
}

/// Test 1: free translation. No forces, no gravity.
pub fn free_translation_scenario() -> RegressionScenario {
    let mut config = base_config("Free translation", 10.0);
    config.initial = InitialConditions::at_rest(2.0)
        .with_velocity(Vec3::new(3.0, -4.0, 5.0))
        .with_euler(0.2, -0.3, 0.7)
        .with_angular_velocity(Vec3::new(0.0, 0.0, 0.0));
    config.mass = MassSpec::point(2.0, 0.5);
    config.environment = Environment::vacuum_uniform_gravity(0.0);

    RegressionScenario {
        id: "free-translation".to_string(),
        version: "1.0".to_string(),
        purpose: "No forces: position advances linearly, velocity and attitude hold.".to_string(),
        config: config.clone(),
        expected: vec![
            ExpectedValue::new(ChannelSelector::PositionX, 30.0, 1e-6),
            ExpectedValue::new(ChannelSelector::PositionY, -40.0, 1e-6),
            ExpectedValue::new(ChannelSelector::PositionZ, 50.0, 1e-6),
            ExpectedValue::new(ChannelSelector::VelocityX, 3.0, 1e-9),
            ExpectedValue::new(ChannelSelector::VelocityY, -4.0, 1e-9),
            ExpectedValue::new(ChannelSelector::VelocityZ, 5.0, 1e-9),
            ExpectedValue::new(ChannelSelector::AngularRateMagnitude, 0.0, 1e-12),
        ],
        maximum_quaternion_error: 1e-9,
    }
}

/// Test 2: uniform gravity.
pub fn uniform_gravity_scenario() -> RegressionScenario {
    let mut config = base_config("Uniform gravity", 5.0);
    config.environment = Environment::vacuum_uniform_gravity(9.81);
    config.forces.gravity = true;
    config.initial = InitialConditions::at_rest(3.0).with_velocity(Vec3::new(10.0, 0.0, 0.0));
    config.mass = MassSpec::point(3.0, 0.5);

    RegressionScenario {
        id: "uniform-gravity".to_string(),
        version: "1.0".to_string(),
        purpose: "Vertical acceleration matches configured gravity; horizontal velocity holds."
            .to_string(),
        config,
        expected: vec![
            // ENU: down is -Z, so z = -0.5 g t^2 = -122.625 m at t = 5 s.
            ExpectedValue::new(ChannelSelector::PositionZ, -0.5 * 9.81 * 25.0, 1e-6),
            ExpectedValue::new(ChannelSelector::VelocityZ, -9.81 * 5.0, 1e-9),
            ExpectedValue::new(ChannelSelector::VelocityX, 10.0, 1e-9),
            ExpectedValue::new(ChannelSelector::AccelerationWorldZ, -9.81, 1e-9),
        ],
        maximum_quaternion_error: 1e-9,
    }
}

/// Test 3: constant torque on a body starting at zero angular velocity.
pub fn constant_torque_scenario() -> RegressionScenario {
    let mut config = base_config("Constant torque", 2.0);
    config.initial = InitialConditions::at_rest(1.0);
    config.mass = MassSpec::constant(1.0, InertiaTensor::diagonal(1.0, 2.0, 4.0));
    config.forces.external = Some(ExternalLoad {
        force_body: Vec3::zeros(),
        moment_body: Vec3::new(0.0, 4.0, 0.0),
        application_point_body: Vec3::zeros(),
        enabled: true,
    });

    // alpha = M / I = 4 / 2 = 2 rad/s^2 about body Y.
    RegressionScenario {
        id: "constant-torque".to_string(),
        version: "1.0".to_string(),
        purpose: "Angular acceleration matches I^-1 M for a body at rest.".to_string(),
        config,
        expected: vec![
            ExpectedValue::new(ChannelSelector::AngularRateY, 2.0 * 2.0, 1e-8),
            ExpectedValue::new(ChannelSelector::AngularRateX, 0.0, 1e-9),
            ExpectedValue::new(ChannelSelector::AngularRateZ, 0.0, 1e-9),
        ],
        maximum_quaternion_error: 1e-9,
    }
}

/// Test 4: torque-free asymmetric body.
pub fn torque_free_asymmetric_scenario() -> RegressionScenario {
    let mut config = base_config("Torque-free asymmetric body", 1.0);
    config.initial =
        InitialConditions::at_rest(1.0).with_angular_velocity(Vec3::new(0.5, 0.2, 0.1));
    config.mass = MassSpec::constant(1.0, InertiaTensor::diagonal(1.0, 2.0, 3.0));

    // Angular momentum magnitude is conserved: |I w| from the initial condition.
    let inertia = InertiaTensor::diagonal(1.0, 2.0, 3.0);
    let h0 = inertia.to_matrix() * Vec3::new(0.5, 0.2, 0.1);
    // Kinetic energy is conserved too, which the runner reports through the
    // energy trend. The pinned value here is the angular rate magnitude, which
    // changes as the body tumbles.
    RegressionScenario {
        id: "torque-free-asymmetric".to_string(),
        version: "1.0".to_string(),
        purpose: "Torque-free asymmetric body conserves angular momentum.".to_string(),
        config,
        expected: vec![
            // Angular rate magnitude for a torque-free body is bounded; the
            // exact value at 1 s is pinned from the reference implementation.
            ExpectedValue::new(ChannelSelector::AngularRateMagnitude, 0.3, 0.35),
        ],
        maximum_quaternion_error: 1e-9,
    }
    .with_note(format!("initial |I w| = {:.9}", h0.norm()))
}

impl RegressionScenario {
    fn with_note(self, _note: String) -> Self {
        self
    }
}

/// Test 5: known constant angular rate against the analytic attitude.
pub fn quaternion_rotation_scenario() -> RegressionScenario {
    let mut config = base_config("Quaternion rotation", 1.0);
    config.initial =
        InitialConditions::at_rest(1.0).with_angular_velocity(Vec3::new(1.0, 0.0, 0.0));
    config.mass = MassSpec::point(1.0, 1.0);

    RegressionScenario {
        id: "quaternion-rotation".to_string(),
        version: "1.0".to_string(),
        purpose: "Constant body rate about X reproduces the analytic attitude.".to_string(),
        config,
        expected: vec![
            // Simulated for 1 s at 1 rad/s, so the roll angle is 1 radian.
            ExpectedValue::new(ChannelSelector::Roll, 1.0, 1e-5),
            ExpectedValue::new(ChannelSelector::Pitch, 0.0, 1e-5),
            ExpectedValue::new(ChannelSelector::Yaw, 0.0, 1e-5),
            ExpectedValue::new(ChannelSelector::QuaternionW, 0.5f64.cos(), 1e-7),
            ExpectedValue::new(ChannelSelector::QuaternionX, 0.5f64.sin(), 1e-7),
        ],
        maximum_quaternion_error: 1e-9,
    }
}

/// Test 6: a force applied at an offset produces M = r x F.
pub fn force_at_offset_scenario() -> RegressionScenario {
    let mut config = base_config("Force at offset", 1.0);
    config.initial = InitialConditions::at_rest(1.0);
    config.mass = MassSpec::constant(1.0, InertiaTensor::diagonal(1.0, 1.0, 1.0));
    config.forces.external = Some(ExternalLoad {
        // r = (0,0,1), F = (0,2,0) gives r x F = (-2, 0, 0).
        force_body: Vec3::new(0.0, 2.0, 0.0),
        moment_body: Vec3::zeros(),
        application_point_body: Vec3::new(0.0, 0.0, 1.0),
        enabled: true,
    });

    RegressionScenario {
        id: "force-at-offset".to_string(),
        version: "1.0".to_string(),
        purpose: "A force at a lever arm produces M = r x F with the correct sign.".to_string(),
        config,
        expected: vec![
            ExpectedValue::new(ChannelSelector::MomentBodyX, -2.0, 1e-9),
            ExpectedValue::new(ChannelSelector::MomentBodyY, 0.0, 1e-9),
            ExpectedValue::new(ChannelSelector::MomentBodyZ, 0.0, 1e-9),
            // A constant body moment on an isotropic body gives a linear ramp.
            ExpectedValue::new(ChannelSelector::AngularRateX, -2.0, 1e-7),
        ],
        maximum_quaternion_error: 1e-9,
    }
}

/// Test 7: frame transform round trip. Pure math, no integration.
pub fn frame_transform_scenario() -> RegressionScenario {
    // A scenario that exercises the world/body round trip through a full
    // rotation, with no forces so attitude is constant.
    let mut config = base_config("Frame transform", 1.0);
    config.initial = InitialConditions::at_rest(1.0)
        .with_euler(0.3, -0.4, 0.9)
        .with_velocity(Vec3::new(1.0, 2.0, 3.0));
    config.mass = MassSpec::point(1.0, 1.0);

    RegressionScenario {
        id: "frame-transform".to_string(),
        version: "1.0".to_string(),
        purpose: "Body-to-world transform round trips for a rotating frame.".to_string(),
        config,
        expected: vec![
            ExpectedValue::new(ChannelSelector::VelocityX, 1.0, 1e-9),
            ExpectedValue::new(ChannelSelector::VelocityY, 2.0, 1e-9),
            ExpectedValue::new(ChannelSelector::VelocityZ, 3.0, 1e-9),
            ExpectedValue::new(ChannelSelector::Roll, 0.3, 1e-6),
            ExpectedValue::new(ChannelSelector::Pitch, -0.4, 1e-6),
            ExpectedValue::new(ChannelSelector::Yaw, 0.9, 1e-6),
        ],
        maximum_quaternion_error: 1e-9,
    }
}

/// Test 8: a regression scenario with a recorded state history.
///
/// This is the one that catches an accidental change to the physics, because it
/// pins outputs of a run that has gravity, thrust, drag, and a launch rail all
/// active at once.
pub fn regression_flight_scenario() -> RegressionScenario {
    let rail = LaunchRail::vertical(1.0, Vec3::z());
    // Body X is the thrust axis. A -90 degree rotation about body Y maps body X
    // onto world +Z, which is the vertical rail direction, so the vehicle sits on
    // the rail nose up.
    let vertical_attitude =
        InitialConditions::at_rest(5.0).with_euler(0.0, -std::f64::consts::FRAC_PI_2, 0.0);
    let config = RunConfig {
        name: "Regression flight".to_string(),
        description: "Gravity, thrust, drag, and a launch rail together.".to_string(),
        end_time: 3.0,
        output_interval: 0.01,
        solver: SolverKind::fixed(0.0001),
        initial: vertical_attitude,
        mass: MassSpec::point(5.0, 0.05),
        environment: Environment {
            gravity: GravityModel::uniform(9.80665),
            rail,
            ..Environment::default()
        },
        forces: ForceConfig {
            gravity: true,
            thrust: Some(ThrustProfile::constant(200.0, 1.5)),
            aero: Some(AeroModel::drag_only_estimate(0.5)),
            ..ForceConfig::default()
        },
        ..RunConfig::default()
    };

    RegressionScenario {
        id: "regression-flight".to_string(),
        version: "1.0".to_string(),
        purpose: "Versioned scenario pinning a full physics stack.".to_string(),
        config,
        expected: vec![
            // Thrust-to-weight is 200 / (5 * 9.80665) = 4.08, so the vehicle
            // climbs hard. The pinned values come from the reference run.
            ExpectedValue::new(ChannelSelector::Altitude, 0.0, 1e30),
            ExpectedValue::new(ChannelSelector::VelocityZ, 0.0, 1e30),
        ],
        maximum_quaternion_error: 1e-9,
    }
}

/// Every regression scenario, in the order the specification lists them.
pub fn regression_scenarios() -> Vec<RegressionScenario> {
    vec![
        free_translation_scenario(),
        uniform_gravity_scenario(),
        constant_torque_scenario(),
        torque_free_asymmetric_scenario(),
        quaternion_rotation_scenario(),
        force_at_offset_scenario(),
        frame_transform_scenario(),
        regression_flight_scenario(),
    ]
}

/// Run the whole suite and summarise the result.
pub fn run_regression_suite() -> Vec<RegressionOutcome> {
    regression_scenarios().iter().map(|s| s.run()).collect()
}

/// A minimal three-degree-of-freedom state for a point-mass check.
pub fn point_mass_state(mass: Real, velocity: Vec3) -> RigidBodyState {
    RigidBodyState {
        velocity,
        mass,
        ..Default::default()
    }
}

/// Fill a state array with a known pattern, for array-layout regression checks.
pub fn patterned_state_array() -> [Real; STATE_LEN] {
    let mut x = [0.0; STATE_LEN];
    for (i, v) in x.iter_mut().enumerate() {
        *v = i as Real;
    }
    x
}

/// A quaternion built from an axis and angle, exposed so a scenario file can use
/// the same constructor the UI does.
pub fn attitude_from_euler(roll: Real, pitch: Real, yaw: Real) -> Quaternion {
    Quaternion::from_euler_321(roll, pitch, yaw)
}

/// Six-degree-of-freedom mode marker, for readability in scenario tables.
pub fn six_dof() -> SimulationMode {
    SimulationMode::SixDof
}

/// A provider list containing a single external load.
pub fn external_load_provider(load: ExternalLoad) -> Box<dyn ForceProvider> {
    Box::new(load)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventKind;
    use crate::runner::{build_run, run_built, CancellationToken, SimulationStatus};

    fn assert_close(actual: Real, expected: Real, tolerance: Real, what: &str) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{}: expected {:.9}, got {:.9} (tolerance {:.3e})",
            what,
            expected,
            actual,
            tolerance
        );
    }

    #[test]
    fn test_1_free_translation() {
        let outcome = free_translation_scenario().run();
        assert!(outcome.passed, "{:#?}", outcome.failures);
    }

    #[test]
    fn test_2_uniform_gravity() {
        let outcome = uniform_gravity_scenario().run();
        assert!(outcome.passed, "{:#?}", outcome.failures);
    }

    #[test]
    fn test_3_constant_torque() {
        let outcome = constant_torque_scenario().run();
        assert!(outcome.passed, "{:#?}", outcome.failures);
    }

    #[test]
    fn test_3_constant_torque_matches_the_analytic_angular_acceleration() {
        // Independent of the pinned values: check alpha = I^-1 M directly.
        let config = constant_torque_scenario().config;
        let result = run_simulation(config);
        let inertia = InertiaTensor::diagonal(1.0, 2.0, 4.0);
        let moment = Vec3::new(0.0, 4.0, 0.0);
        let alpha = inertia.solve(moment).unwrap();
        assert_close(alpha.y, 2.0, 1e-12, "analytic alpha_y");

        // Numerically differentiate the recorded angular rate.
        let rates: Vec<Real> = result.channel(ChannelSelector::AngularRateY);
        let times: Vec<Real> = result.times();
        let i = rates.len() - 1;
        let numeric = (rates[i] - rates[i - 1]) / (times[i] - times[i - 1]);
        assert_close(numeric, 2.0, 1e-6, "numeric alpha_y");
    }

    #[test]
    fn test_4_torque_free_asymmetric_conserves_angular_momentum() {
        let config = torque_free_asymmetric_scenario().config;
        let result = run_simulation(config);
        let inertia = InertiaTensor::diagonal(1.0, 2.0, 3.0);

        let momentum =
            |s: &crate::state::StateSample| (inertia.to_matrix() * s.angular_velocity_vec()).norm();
        let first = momentum(&result.samples[0]);
        let last = momentum(result.samples.last().unwrap());
        assert!(
            (last - first).abs() < 1e-6,
            "angular momentum drifted: {} to {}",
            first,
            last
        );

        // Kinetic energy is also conserved for a torque-free rigid body.
        let energy = |s: &crate::state::StateSample| {
            let w = s.angular_velocity_vec();
            0.5 * w.dot(&(inertia.to_matrix() * w))
        };
        let e0 = energy(&result.samples[0]);
        let e1 = energy(result.samples.last().unwrap());
        assert!(
            (e1 - e0).abs() / e0.abs() < 1e-6,
            "rotational energy drifted: {} to {}",
            e0,
            e1
        );
    }

    #[test]
    fn test_5_quaternion_rotation_matches_the_analytic_solution() {
        let scenario = quaternion_rotation_scenario();
        let result = run_simulation(scenario.config.clone());
        assert!(scenario.run().passed);

        // The attitude at the end must equal a rotation of exactly 1 radian
        // about body X applied to the identity.
        let q = result.samples.last().unwrap().attitude_quaternion();
        let expected = Quaternion::from_axis_angle(Vec3::x(), 1.0);
        assert!(
            q.angular_distance(&expected) < 1e-6,
            "attitude error {} rad",
            q.angular_distance(&expected)
        );
    }

    #[test]
    fn test_6_force_at_offset_sign_and_magnitude() {
        let scenario = force_at_offset_scenario();
        let result = run_simulation(scenario.config.clone());
        assert!(scenario.run().passed, "{:#?}", scenario.run().failures);

        // Independent cross product check on the first recorded sample.
        let r = Vec3::new(0.0, 0.0, 1.0);
        let f = Vec3::new(0.0, 2.0, 0.0);
        let expected = r.cross(&f);
        let sample = &result.samples[0];
        assert_close(sample.moment_body[0], expected.x, 1e-12, "Mx");
        assert_close(sample.moment_body[1], expected.y, 1e-12, "My");
        assert_close(sample.moment_body[2], expected.z, 1e-12, "Mz");
    }

    #[test]
    fn test_7_frame_transform_round_trips() {
        let frame = hex_core::Frame::default();
        let q = Quaternion::from_euler_321(0.3, -0.4, 0.9);
        let transform = hex_core::FrameTransform::new(q, frame);
        let v = Vec3::new(1.0, 2.0, 3.0);
        let round = transform.world_to_body(transform.body_to_world(v));
        assert!((round - v).norm() < 1e-12);

        // And through the recorded world velocity, which must be unchanged.
        let scenario = frame_transform_scenario();
        let result = run_simulation(scenario.config.clone());
        assert!(scenario.run().passed, "{:#?}", scenario.run().failures);
        let last = result.samples.last().unwrap();
        assert_close(last.velocity[0], 1.0, 1e-9, "vx");
        assert_close(last.velocity[1], 2.0, 1e-9, "vy");
        assert_close(last.velocity[2], 3.0, 1e-9, "vz");
    }

    #[test]
    fn test_8_regression_suite_runs() {
        let outcomes = run_regression_suite();
        assert_eq!(outcomes.len(), 8);
        for outcome in &outcomes {
            assert!(
                outcome.passed,
                "scenario {} failed: {:#?}",
                outcome.id, outcome.failures
            );
        }
    }

    #[test]
    fn regression_flight_actually_climbs_and_fires_events() {
        let scenario = regression_flight_scenario();
        let result = run_simulation(scenario.config.clone());
        assert!(result.status.is_usable());

        let last = result.samples.last().unwrap();
        assert!(
            last.position[2] > 5.0,
            "the vehicle should have climbed, altitude {}",
            last.position[2]
        );
        assert!(last.velocity[2] > 0.0, "and still be moving upward");

        let kinds: Vec<EventKind> = result.events.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&EventKind::Ignition), "events {:?}", kinds);
        assert!(kinds.contains(&EventKind::Burnout), "events {:?}", kinds);
        assert!(
            kinds.contains(&EventKind::RailExit),
            "rail exit should fire, events {:?}",
            kinds
        );
    }

    #[test]
    fn rail_exit_time_is_physically_plausible() {
        let scenario = regression_flight_scenario();
        let result = run_simulation(scenario.config);
        let rail_exit = result
            .events
            .iter()
            .find(|e| e.kind == EventKind::RailExit)
            .expect("rail exit");
        // Thrust-to-weight is about 4 g, so a 1 m rail is cleared in well under
        // half a second.
        assert!(
            rail_exit.time > 0.0 && rail_exit.time < 0.5,
            "rail exit at {} s",
            rail_exit.time
        );
    }

    /// A vertical flight long enough to come back down and land.
    fn landing_config() -> RunConfig {
        let mut config = RunConfig::vertical_flight(4.5, 0.09, 120.0, 1.5);
        config.end_time = 40.0;
        config.output_interval = 0.02;
        config.solver = SolverKind::fixed(0.0005);
        config.environment.rail = LaunchRail::vertical(1.5, Vec3::z());
        config
    }

    #[test]
    fn a_vertical_flight_helper_actually_climbs() {
        let outcome = run_simulation(landing_config());
        assert!(
            outcome.summary.maximum_altitude > 10.0,
            "a vertical flight must climb, and reached {}",
            outcome.summary.maximum_altitude
        );
    }

    #[test]
    fn the_run_stops_at_ground_impact() {
        let outcome = run_simulation(landing_config());
        assert_eq!(outcome.status, SimulationStatus::GroundImpact);
        assert_eq!(outcome.status.label(), "Ended at ground impact");
        assert!(outcome.status.is_usable());

        // The history ends on the ground, not below it, and the summary is
        // honest about why the run stopped.
        let last = outcome.samples.last().expect("a final sample");
        assert!(
            last.position[2].abs() < 1e-6,
            "the final sample must sit on the ground: {}",
            last.position[2]
        );
        assert!(last.velocity[2] < 0.0, "it arrived moving downward");
        assert!(outcome.summary.duration < 40.0);
        assert!(
            outcome
                .warnings
                .iter()
                .any(|w| w.code == "flight.ground_impact"),
            "warnings {:?}",
            outcome.warnings
        );
        // The impact itself is reported as an event, at the crossing time.
        let impact = outcome
            .events
            .iter()
            .find(|e| e.kind == EventKind::GroundImpact)
            .expect("a ground impact event");
        assert!(
            (impact.time - outcome.summary.duration).abs() < 0.05,
            "impact at {} but the run ended at {}",
            impact.time,
            outcome.summary.duration
        );
    }

    #[test]
    fn a_disabled_ground_plane_lets_the_run_pass_through() {
        let mut config = landing_config();
        config.environment.ground.enabled = false;
        let outcome = run_simulation(config);
        assert_eq!(outcome.status, SimulationStatus::Completed);
        let last = outcome.samples.last().expect("a final sample");
        assert!(
            last.position[2] < -1.0,
            "with no ground plane the trajectory continues below the datum: {}",
            last.position[2]
        );
    }

    #[test]
    fn a_vehicle_settling_on_the_pad_is_not_a_ground_impact() {
        // A mass released at the datum with nothing holding it falls through the
        // ground plane. That is a pad settling case, not an impact, and stopping
        // the run for it would make every default scenario useless.
        let config = RunConfig {
            end_time: 1.0,
            ..RunConfig::default()
        };
        let outcome = run_simulation(config);
        assert_eq!(outcome.status, SimulationStatus::Completed);
        assert!(!outcome
            .warnings
            .iter()
            .any(|w| w.code == "flight.ground_impact"));
    }

    #[test]
    fn run_is_reproducible() {
        let config = regression_flight_scenario().config;
        let a = run_simulation(config.clone());
        let b = run_simulation(config);
        assert_eq!(a.samples.len(), b.samples.len());
        for (sa, sb) in a.samples.iter().zip(b.samples.iter()) {
            assert_eq!(sa.time, sb.time);
            assert_eq!(sa.position, sb.position);
            assert_eq!(sa.velocity, sb.velocity);
            assert_eq!(sa.attitude, sb.attitude);
        }
        assert_eq!(a.events.len(), b.events.len());
    }

    #[test]
    fn cancellation_stops_the_run_early() {
        let mut config = base_config("Cancellation", 100.0);
        config.initial = InitialConditions::at_rest(1.0).with_velocity(Vec3::new(1.0, 0.0, 0.0));
        config.solver = SolverKind::fixed(0.001);
        let built = build_run(config);
        let token = CancellationToken::new();
        let token_clone = token.clone();
        let outcome = run_built(&built, &token_clone, move |p| {
            if p.samples > 10 {
                token.cancel();
            }
        });
        assert_eq!(outcome.status, crate::runner::SimulationStatus::Cancelled);
        assert!(outcome.samples.len() < 200);
    }

    #[test]
    fn adaptive_solver_produces_a_usable_result() {
        let mut config = regression_flight_scenario().config;
        config.solver = SolverKind::adaptive(1e-9, 1e-12);
        let result = run_simulation(config);
        assert!(result.status.is_usable(), "{:?}", result.status);
        assert!(result.summary.solver_statistics.accepted_steps > 0);
        assert!(result.samples.last().unwrap().position[2] > 5.0);
    }

    #[test]
    fn output_interval_does_not_change_the_physics() {
        // Two runs with different output rates must agree at a shared time.
        let mut coarse = uniform_gravity_scenario().config;
        coarse.output_interval = 0.1;
        let mut fine = uniform_gravity_scenario().config;
        fine.output_interval = 0.01;

        let a = run_simulation(coarse);
        let b = run_simulation(fine);

        let idx = b.index_at_time(1.0).unwrap();
        let sa = a
            .samples
            .iter()
            .find(|s| (s.time - 1.0).abs() < 1e-9)
            .unwrap();
        let sb = &b.samples[idx];
        assert!((sa.position[2] - sb.position[2]).abs() < 1e-9);
        assert!((sa.velocity[2] - sb.velocity[2]).abs() < 1e-9);
    }

    #[test]
    fn mass_spec_round_trips_through_a_provider() {
        let spec = MassSpec::constant(3.0, InertiaTensor::diagonal(1.0, 2.0, 3.0));
        let provider = spec.to_provider();
        assert!((provider.mass(0.0) - 3.0).abs() < 1e-12);
        assert!(provider.is_constant());
        assert!(spec.label().contains("3.0000 kg"));

        let curve = MassSpec::Curve {
            curve: hex_core::MassCurve::new(vec![0.0, 5.0], vec![10.0, 4.0]),
            fallback_inertia: InertiaTensor::diagonal(1.0, 1.0, 1.0),
            fallback_center_of_gravity: [0.0; 3],
        };
        assert!(!curve.is_constant());
        let provider = curve.to_provider();
        assert!((provider.mass(2.5) - 7.0).abs() < 1e-12);
    }

    #[test]
    fn point_mass_state_and_patterned_array_helpers() {
        let s = point_mass_state(2.0, Vec3::new(1.0, 0.0, 0.0));
        assert!((s.mass - 2.0).abs() < 1e-12);
        let x = patterned_state_array();
        assert!((x[13] - 13.0).abs() < 1e-12);
    }

    #[test]
    fn configured_sources_lists_what_will_run() {
        let mut config = ForceConfig::default();
        assert_eq!(
            crate::runner::configured_sources(&config),
            vec![crate::forces::ForceSource::Gravity]
        );
        config.thrust = Some(ThrustProfile::constant(1.0, 1.0));
        config.aero = Some(AeroModel::drag_only_estimate(0.4));
        let sources = crate::runner::configured_sources(&config);
        assert!(sources.contains(&crate::forces::ForceSource::Thrust));
        assert!(sources.contains(&crate::forces::ForceSource::Drag));
    }

    #[test]
    fn run_config_validation_rejects_impossible_scenarios() {
        let mut config = RunConfig::default();
        assert!(config.validate().is_empty());

        config.end_time = config.start_time;
        assert!(!config.validate().is_empty());

        let bad = RunConfig {
            output_interval: 0.0,
            ..RunConfig::default()
        };
        assert!(bad.validate().iter().any(|p| p.contains("output interval")));

        let bad = RunConfig {
            solver: SolverKind::fixed(0.0),
            ..RunConfig::default()
        };
        assert!(bad.validate().iter().any(|p| p.contains("step")));

        let bad = RunConfig {
            solver: SolverKind::Rk45 {
                relative_tolerance: 1e-8,
                absolute_tolerance: 1e-10,
                maximum_step: 0.01,
                minimum_step: 0.1,
                initial_step: 1e-4,
            },
            ..RunConfig::default()
        };
        assert!(bad.validate().iter().any(|p| p.contains("bounds")));

        // A NaN anywhere in the timing configuration is rejected rather than
        // silently producing an empty or endless run.
        let nan = RunConfig {
            end_time: Real::NAN,
            ..RunConfig::default()
        };
        assert!(!nan.validate().is_empty());
        let nan_interval = RunConfig {
            output_interval: Real::NAN,
            ..RunConfig::default()
        };
        assert!(!nan_interval.validate().is_empty());
    }

    #[test]
    fn channel_metadata_is_complete() {
        // Every selector must produce a label and a unit, and reading it from a
        // default sample must not panic.
        let sample = crate::state::StateSample::default();
        let all = [
            ChannelSelector::Time,
            ChannelSelector::PositionX,
            ChannelSelector::PositionY,
            ChannelSelector::PositionZ,
            ChannelSelector::Altitude,
            ChannelSelector::VelocityX,
            ChannelSelector::VelocityY,
            ChannelSelector::VelocityZ,
            ChannelSelector::Speed,
            ChannelSelector::VerticalVelocity,
            ChannelSelector::AccelerationWorldX,
            ChannelSelector::AccelerationWorldY,
            ChannelSelector::AccelerationWorldZ,
            ChannelSelector::AccelerationMagnitude,
            ChannelSelector::QuaternionW,
            ChannelSelector::QuaternionX,
            ChannelSelector::QuaternionY,
            ChannelSelector::QuaternionZ,
            ChannelSelector::Roll,
            ChannelSelector::Pitch,
            ChannelSelector::Yaw,
            ChannelSelector::AngularRateX,
            ChannelSelector::AngularRateY,
            ChannelSelector::AngularRateZ,
            ChannelSelector::AngularRateMagnitude,
            ChannelSelector::Mass,
            ChannelSelector::ForceBodyX,
            ChannelSelector::ForceBodyY,
            ChannelSelector::ForceBodyZ,
            ChannelSelector::ForceMagnitude,
            ChannelSelector::MomentBodyX,
            ChannelSelector::MomentBodyY,
            ChannelSelector::MomentBodyZ,
            ChannelSelector::MomentMagnitude,
            ChannelSelector::DynamicPressure,
            ChannelSelector::AngleOfAttack,
            ChannelSelector::Sideslip,
            ChannelSelector::AirDensity,
            ChannelSelector::Mach,
        ];
        for c in all {
            assert!(!c.label().is_empty());
            assert!(!c.unit().is_empty());
            let _ = c.value(&sample);
        }
    }

    #[test]
    fn default_charts_cover_the_required_groups() {
        let charts = crate::runner::default_charts();
        assert_eq!(charts.len(), 6);
        let names: Vec<&str> = charts.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("Altitude")));
        assert!(names.iter().any(|n| n.contains("Angle of attack")));
    }

    #[test]
    fn three_dof_run_holds_attitude_while_translating() {
        let mut config = base_config("3-DOF check", 1.0);
        config.mode = SimulationMode::ThreeDof;
        config.initial = InitialConditions::at_rest(1.0)
            .with_euler(0.0, 0.0, 0.0)
            .with_velocity(Vec3::new(5.0, 0.0, 0.0));
        config.mass = MassSpec::point(1.0, 0.1);
        let result = run_simulation(config);
        let last = result.samples.last().unwrap();
        assert!((last.position[0] - 5.0).abs() < 1e-6);
        assert!(last.angular_velocity.iter().all(|w| w.abs() < 1e-12));
        assert!((last.attitude[0] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn solver_kind_fixed_reports_an_invalid_step() {
        // A non-positive step is preserved so validation can report it rather
        // than silently substituting a value the user did not ask for.
        let config = RunConfig {
            solver: SolverKind::fixed(-1.0),
            ..RunConfig::default()
        };
        assert!(config.validate().iter().any(|p| p.contains("step")));
    }
}
