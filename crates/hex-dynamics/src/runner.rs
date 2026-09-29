//! Run configuration, the simulation loop, and the result artifact.
//!
//! The runner is deliberately free of any UI concern. It reports progress through
//! a callback, records every output sample, tracks solver statistics and health
//! metrics, and stops with a diagnostic when a state becomes non-finite.
//!
//! # Step sizes
//!
//! Three different rates are kept distinct, because conflating them is the most
//! common way a simulation result becomes misleading:
//!
//! * the integration step, chosen by the solver,
//! * the output sampling interval, chosen by the user,
//! * the rendering rate, which belongs to the frontend and never affects the
//!   numerics.
//!
//! # Reproducibility
//!
//! [`RunConfig`] holds every input that can change the result, and
//! [`RunSummary`] records all of them plus the derived statistics. Re-running the
//! same configuration with the same model produces the same history.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use hex_core::{InertiaTensor, MassCurve, Quaternion, Real, Vec3};
use serde::{Deserialize, Serialize};

use crate::environment::Environment;
use crate::equations::{BoundDynamics, DynamicsDiagnostics, ProviderChain, RigidBodyDynamics};
use crate::events::{sort_events, EventDetector, EventKind, EventRules, FlightEvent};
use crate::forces::{
    AeroForce, AeroModel, ControlForce, ControlMoment, ExternalLoad, ForceProvider, ForceSource,
    GravityForce, LandingBurn, ThrustForce, ThrustProfile,
};
use crate::integrators::{
    ErrorWeights, NormalizationPolicy, Rk4, Rk45, SolverKind, SolverStatistics,
};
use crate::state::{RigidBodyState, SimulationMode, StateSample, STATE_LEN};
use crate::vehicle::{
    ConstantMass, CurveMass, MassFlowProfile, MassProvider, ReferenceGeometry, Vehicle,
    VehicleModel,
};

/// Initial conditions for a run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InitialConditions {
    /// Position in the world frame, metres.
    pub position: [Real; 3],
    /// Velocity in the world frame, metres per second.
    pub velocity: [Real; 3],
    /// Body-to-world quaternion, scalar first.
    pub attitude: [Real; 4],
    /// Body angular velocity, radians per second. Ignored in 3-DOF mode.
    pub angular_velocity: [Real; 3],
    /// Initial mass in kilograms.
    pub mass: Real,
    /// Initial simulation time in seconds.
    pub time: Real,
}

impl Default for InitialConditions {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            velocity: [0.0; 3],
            attitude: [1.0, 0.0, 0.0, 0.0],
            angular_velocity: [0.0; 3],
            mass: 1.0,
            time: 0.0,
        }
    }
}

impl InitialConditions {
    /// A vehicle at rest with identity attitude.
    pub fn at_rest(mass: Real) -> Self {
        Self {
            mass,
            ..Self::default()
        }
    }

    /// Build from an Euler angle triple in radians, which is the friendly input
    /// the UI offers. The state itself is always a quaternion.
    pub fn with_euler(mut self, roll: Real, pitch: Real, yaw: Real) -> Self {
        let q = Quaternion::from_euler_321(roll, pitch, yaw);
        self.attitude = [q.w, q.x, q.y, q.z];
        self
    }

    /// Build from an explicit quaternion.
    pub fn with_quaternion(mut self, q: Quaternion) -> Self {
        let q = q.normalized();
        self.attitude = [q.w, q.x, q.y, q.z];
        self
    }

    pub fn with_position(mut self, p: Vec3) -> Self {
        self.position = [p.x, p.y, p.z];
        self
    }

    pub fn with_velocity(mut self, v: Vec3) -> Self {
        self.velocity = [v.x, v.y, v.z];
        self
    }

    pub fn with_angular_velocity(mut self, w: Vec3) -> Self {
        self.angular_velocity = [w.x, w.y, w.z];
        self
    }

    pub fn position_vec(&self) -> Vec3 {
        Vec3::new(self.position[0], self.position[1], self.position[2])
    }

    pub fn velocity_vec(&self) -> Vec3 {
        Vec3::new(self.velocity[0], self.velocity[1], self.velocity[2])
    }

    pub fn angular_velocity_vec(&self) -> Vec3 {
        Vec3::new(
            self.angular_velocity[0],
            self.angular_velocity[1],
            self.angular_velocity[2],
        )
    }

    pub fn attitude_quaternion(&self) -> Quaternion {
        Quaternion::new(
            self.attitude[0],
            self.attitude[1],
            self.attitude[2],
            self.attitude[3],
        )
        .normalized()
    }

    /// Build the state this configuration describes.
    pub fn to_state(&self) -> RigidBodyState {
        RigidBodyState {
            position: self.position_vec(),
            velocity: self.velocity_vec(),
            attitude: self.attitude_quaternion(),
            angular_velocity: self.angular_velocity_vec(),
            mass: self.mass,
        }
    }
}

/// The mass model a run uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MassSpec {
    /// A single mass value, treated as constant.
    Constant {
        mass: Real,
        center_of_gravity: [Real; 3],
        inertia: InertiaTensor,
    },
    /// A sampled mass curve, optionally with centre of gravity and inertia
    /// tracks.
    Curve {
        curve: MassCurve,
        fallback_inertia: InertiaTensor,
        fallback_center_of_gravity: [Real; 3],
    },
}

impl MassSpec {
    /// A point vehicle with an isotropic inertia.
    pub fn point(mass: Real, inertia: Real) -> Self {
        MassSpec::Constant {
            mass,
            center_of_gravity: [0.0; 3],
            inertia: InertiaTensor::diagonal(inertia, inertia, inertia),
        }
    }

    pub fn constant(mass: Real, inertia: InertiaTensor) -> Self {
        MassSpec::Constant {
            mass,
            center_of_gravity: [0.0; 3],
            inertia,
        }
    }

    /// The provider this specification describes.
    pub fn to_provider(&self) -> Box<dyn MassProvider> {
        match self.clone() {
            MassSpec::Constant {
                mass,
                center_of_gravity,
                inertia,
            } => Box::new(ConstantMass::new(
                mass,
                Vec3::new(
                    center_of_gravity[0],
                    center_of_gravity[1],
                    center_of_gravity[2],
                ),
                inertia,
            )),
            MassSpec::Curve {
                curve,
                fallback_inertia,
                fallback_center_of_gravity,
            } => Box::new(CurveMass {
                curve,
                fallback_inertia,
                fallback_center_of_gravity: Vec3::new(
                    fallback_center_of_gravity[0],
                    fallback_center_of_gravity[1],
                    fallback_center_of_gravity[2],
                ),
            }),
        }
    }

    /// Whether this specification describes a constant mass.
    pub fn is_constant(&self) -> bool {
        matches!(self, MassSpec::Constant { .. })
    }

    /// A label for the model panel.
    pub fn label(&self) -> String {
        match self {
            MassSpec::Constant { mass, .. } => format!("Constant mass, {:.4} kg", mass),
            MassSpec::Curve { curve, .. } => format!(
                "Mass curve, {} samples, {:.4} kg to {:.4} kg",
                curve.times.len(),
                curve.masses.first().copied().unwrap_or(0.0),
                curve.masses.last().copied().unwrap_or(0.0)
            ),
        }
    }
}

/// The force and moment models selected for a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForceConfig {
    /// Thrust profile, when thrust is modelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thrust: Option<ThrustProfile>,
    /// A landing burn that ignites on the stopping-distance condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landing_burn: Option<LandingBurn>,
    /// Use the vehicle model's thrust application point.
    #[serde(default)]
    pub thrust_at_vehicle_point: bool,
    /// Propellant mass flow, applied on top of the mass provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mass_flow: Option<MassFlowProfile>,
    /// Aerodynamic model, when aerodynamics are modelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aero: Option<AeroModel>,
    /// Control or thrust-vector moment profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlMoment>,
    /// A constant external load, for parametric studies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalLoad>,
    /// Whether gravity is included. Disabling it is a verification aid.
    #[serde(default = "default_true")]
    pub gravity: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ForceConfig {
    fn default() -> Self {
        Self {
            thrust: None,
            landing_burn: None,
            thrust_at_vehicle_point: false,
            mass_flow: None,
            aero: None,
            control: None,
            external: None,
            gravity: true,
        }
    }
}

impl ForceConfig {
    /// A drag-only configuration with no thrust, for a coasting case.
    pub fn coast(drag_coefficient: Real) -> Self {
        Self {
            aero: Some(AeroModel::drag_only_estimate(drag_coefficient)),
            ..Self::default()
        }
    }

    /// Build the provider chain in a fixed, documented order.
    ///
    /// Gravity first, then thrust, then the landing burn, then aerodynamics, then
    /// control, then any external load. The order does not change the totals
    /// because forces add, but it fixes the order of the contribution list in the
    /// results panel.
    pub fn to_chain(&self) -> ProviderChain {
        let mut chain = ProviderChain::new();
        if self.gravity {
            chain.push(Box::new(GravityForce));
        }
        if let Some(t) = self.thrust.clone() {
            let provider = if self.thrust_at_vehicle_point {
                ThrustForce::at_vehicle_point(t)
            } else {
                ThrustForce::new(t)
            };
            chain.push(Box::new(provider));
        }
        // The landing burn is its own provider rather than a second segment in the
        // thrust profile, because its ignition time is a function of the state and
        // a thrust profile is a function of time alone.
        if let Some(burn) = self.landing_burn.clone() {
            chain.push(Box::new(burn));
        }
        if let Some(a) = self.aero.clone() {
            chain.push(Box::new(AeroForce::new(a)));
        }
        if let Some(c) = self.control.clone() {
            chain.push(Box::new(ControlForce { profile: c }));
        }
        if let Some(e) = self.external {
            chain.push(Box::new(e));
        }
        chain
    }

    /// Names of the configured models, for the run header.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "Gravity: {}",
            if self.gravity { "included" } else { "disabled" }
        ));
        match &self.thrust {
            Some(t) => lines.push(format!(
                "Thrust: {} samples, burnout at {}",
                t.times.len(),
                t.burnout_time()
                    .map(|x| format!("{:.3} s", x))
                    .unwrap_or_else(|| "never".to_string())
            )),
            None => lines.push("Thrust: none".to_string()),
        }
        if let Some(burn) = &self.landing_burn {
            lines.push(burn.describe());
        }
        match &self.aero {
            Some(a) => lines.push(format!(
                "Aerodynamics: {}, coefficients from {}",
                a.fidelity_label(),
                a.source.label()
            )),
            None => lines.push("Aerodynamics: none".to_string()),
        }
        match &self.control {
            Some(c) => lines.push(format!(
                "Control: {}",
                if c.enabled { "enabled" } else { "disabled" }
            )),
            None => lines.push("Control: none".to_string()),
        }
        if let Some(e) = &self.external {
            lines.push(format!(
                "External load: {}",
                if e.enabled { "enabled" } else { "disabled" }
            ));
        }
        lines
    }
}

/// Warning thresholds used to flag a run without failing it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WarningThresholds {
    /// Quaternion norm error above which the run is flagged.
    pub quaternion_norm_error: Real,
    /// Maximum angular rate in rad/s above which the run is flagged.
    pub maximum_angular_rate: Real,
    /// Maximum acceleration in m/s^2 above which the run is flagged.
    pub maximum_acceleration: Real,
    /// Rejected-step ratio above which the adaptive solver is flagged.
    pub step_rejection_ratio: Real,
    /// Relative drift in total energy above which a force-free run is flagged.
    pub energy_drift_ratio: Real,
}

impl Default for WarningThresholds {
    fn default() -> Self {
        Self {
            quaternion_norm_error: 1e-6,
            maximum_angular_rate: 100.0,
            maximum_acceleration: 1000.0,
            step_rejection_ratio: 0.1,
            energy_drift_ratio: 1e-3,
        }
    }
}

/// A complete, reproducible run configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunConfig {
    /// Name shown in the run list.
    pub name: String,
    /// Optional scenario description.
    pub description: String,
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Start time in seconds.
    pub start_time: Real,
    /// End time in seconds.
    pub end_time: Real,
    /// Output sampling interval in seconds. Distinct from the integration step.
    pub output_interval: Real,
    /// Maximum number of output samples before the run stops.
    pub maximum_samples: usize,
    /// Integrator and its settings.
    pub solver: SolverKind,
    /// Quaternion normalisation policy.
    pub normalization: NormalizationPolicy,
    /// Initial conditions.
    pub initial: InitialConditions,
    /// Mass model.
    pub mass: MassSpec,
    /// Environment.
    pub environment: Environment,
    /// Force and moment models.
    pub forces: ForceConfig,
    /// Automatic event rules.
    pub event_rules: EventRules,
    /// Time at which recovery deployment is expected, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_time: Option<Real>,
    /// Warning thresholds.
    pub thresholds: WarningThresholds,
    /// Latitude of the launch site in degrees, recorded for reproducibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_latitude: Option<Real>,
    /// Longitude of the launch site in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_longitude: Option<Real>,
    /// Free-form user notes stored with the run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            name: "Run".to_string(),
            description: String::new(),
            mode: SimulationMode::SixDof,
            start_time: 0.0,
            end_time: 10.0,
            output_interval: 0.01,
            maximum_samples: 2_000_000,
            solver: SolverKind::fixed(0.0005),
            normalization: NormalizationPolicy::EveryStep,
            initial: InitialConditions::at_rest(1.0),
            mass: MassSpec::point(1.0, 0.1),
            environment: Environment::default(),
            forces: ForceConfig::default(),
            event_rules: EventRules::default(),
            recovery_time: None,
            thresholds: WarningThresholds::default(),
            launch_latitude: None,
            launch_longitude: None,
            notes: None,
        }
    }
}

impl RunConfig {
    /// A vertical flight with thrust and drag, which is the shape of a typical
    /// hobby rocket run.
    ///
    /// Body X is the thrust axis. A rotation of minus ninety degrees about body Y
    /// maps it onto world +Z, so the vehicle climbs rather than driving its nose
    /// into the ground.
    pub fn vertical_flight(mass: Real, inertia: Real, thrust: Real, burn: Real) -> Self {
        Self {
            name: "Vertical flight".to_string(),
            end_time: burn + 60.0,
            initial: InitialConditions::at_rest(mass).with_euler(
                0.0,
                -std::f64::consts::FRAC_PI_2,
                0.0,
            ),
            mass: MassSpec::point(mass, inertia),
            forces: ForceConfig {
                thrust: Some(ThrustProfile::constant(thrust, burn)),
                aero: Some(AeroModel::drag_only_estimate(0.45)),
                ..ForceConfig::default()
            },
            ..Self::default()
        }
    }

    /// The scenario end time must exceed the start time.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        // Each test is written so that a NaN is rejected rather than slipping
        // through: a NaN end time, interval, or step is not a valid scenario.
        if self.end_time.is_nan() || self.start_time.is_nan() || self.end_time <= self.start_time {
            problems.push("end time must be greater than the start time".to_string());
        }
        if self.output_interval.is_nan() || self.output_interval <= 0.0 {
            problems.push("output interval must be positive".to_string());
        }
        if self.maximum_samples == 0 {
            problems.push("maximum sample count must be positive".to_string());
        }
        if let SolverKind::Rk4 { step } = self.solver {
            if step.is_nan() || step <= 0.0 {
                problems.push("fixed integration step must be positive".to_string());
            }
        }
        if let SolverKind::Rk45 {
            minimum_step,
            maximum_step,
            ..
        } = self.solver
        {
            if minimum_step <= 0.0 || maximum_step < minimum_step {
                problems.push("adaptive step bounds are inconsistent".to_string());
            }
        }
        if !self.mass.is_constant() {
            if let MassSpec::Curve { curve, .. } = &self.mass {
                problems.extend(curve.validate());
            }
        }
        if self.initial.mass <= 0.0 && self.mass.is_constant() {
            problems.push("initial mass must be positive".to_string());
        }
        for v in [
            self.initial.position,
            self.initial.velocity,
            self.initial.angular_velocity,
        ] {
            if !v.iter().all(|x| x.is_finite()) {
                problems.push("initial conditions must be finite".to_string());
                break;
            }
        }
        problems
    }

    /// Number of output samples this configuration is expected to produce.
    pub fn expected_samples(&self) -> usize {
        if self.output_interval.is_nan() || self.output_interval <= 0.0 {
            // A non-positive interval is reported by `validate`, so this must stay
            // total rather than producing an infinite estimate.
            return 0;
        }
        let span = (self.end_time - self.start_time).max(0.0);
        ((span / self.output_interval).floor() as usize + 1).min(self.maximum_samples)
    }
}

/// Status of a completed run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationStatus {
    /// The run reached the configured end time.
    Completed,
    /// The state became non-finite and the run stopped with a diagnostic.
    Failed,
    /// The user cancelled the run.
    Cancelled,
    /// The output sample limit was reached.
    SampleLimitReached,
    /// The vehicle reached the ground plane, so the run ended there.
    GroundImpact,
}

impl SimulationStatus {
    pub fn label(self) -> &'static str {
        match self {
            SimulationStatus::Completed => "Completed",
            SimulationStatus::Failed => "Failed",
            SimulationStatus::Cancelled => "Cancelled",
            SimulationStatus::SampleLimitReached => "Sample limit reached",
            SimulationStatus::GroundImpact => "Ended at ground impact",
        }
    }

    /// Whether the run produced usable data.
    pub fn is_usable(self) -> bool {
        matches!(
            self,
            SimulationStatus::Completed
                | SimulationStatus::SampleLimitReached
                | SimulationStatus::GroundImpact
        )
    }
}

/// A single actionable finding from a completed run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunWarning {
    /// Machine code, for example `solver.step_rejections`.
    pub code: String,
    /// Short title.
    pub title: String,
    /// Explanation with the measured value.
    pub detail: String,
    /// Whether the finding blocks using the result.
    pub blocking: bool,
}

impl RunWarning {
    pub fn new(code: &str, title: &str, detail: impl Into<String>, blocking: bool) -> Self {
        Self {
            code: code.to_string(),
            title: title.to_string(),
            detail: detail.into(),
            blocking,
        }
    }
}

/// Lightweight progress report emitted during a run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RunProgress {
    /// Current simulation time in seconds.
    pub time: Real,
    /// Fraction of the time span completed, in the range 0 to 1.
    pub fraction: Real,
    /// Samples recorded so far.
    pub samples: usize,
    /// Steps accepted so far.
    pub accepted_steps: u64,
    /// Steps rejected so far.
    pub rejected_steps: u64,
    /// Events detected so far.
    pub events: usize,
}

/// Everything a completed run produces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    pub status: SimulationStatus,
    pub samples: Vec<StateSample>,
    pub events: Vec<FlightEvent>,
    pub summary: RunSummary,
    pub warnings: Vec<RunWarning>,
    /// Diagnostics from the final evaluation.
    pub final_diagnostics: DynamicsDiagnostics,
    /// Final state, for restarting or for the state inspector.
    pub final_state: RigidBodyState,
}

impl RunOutcome {
    /// Extract one channel across every sample, for plotting.
    pub fn channel(&self, selector: ChannelSelector) -> Vec<Real> {
        self.samples.iter().map(|s| selector.value(s)).collect()
    }

    /// Extract the time column.
    pub fn times(&self) -> Vec<Real> {
        self.samples.iter().map(|s| s.time).collect()
    }

    /// Event times of a given kind.
    pub fn event_times(&self, kind: EventKind) -> Vec<Real> {
        self.events
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| e.time)
            .collect()
    }

    /// Add a user event marker.
    pub fn add_marker(&mut self, time: Real, label: impl Into<String>) {
        self.events.push(FlightEvent::marker(time, label));
        sort_events(&mut self.events);
    }

    /// Index of the sample nearest a time, for the replay scrubber.
    pub fn index_at_time(&self, time: Real) -> Option<usize> {
        index_at_time(&self.samples, time)
    }

    /// Convert into the persistent result artifact.
    pub fn into_result(self) -> SimulationResult {
        SimulationResult {
            summary: self.summary,
            samples: self.samples,
            events: self.events,
        }
    }
}

/// A named scalar channel of a sample, used by the plot panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelSelector {
    Time,
    PositionX,
    PositionY,
    PositionZ,
    Altitude,
    VelocityX,
    VelocityY,
    VelocityZ,
    Speed,
    VerticalVelocity,
    AccelerationWorldX,
    AccelerationWorldY,
    AccelerationWorldZ,
    AccelerationMagnitude,
    QuaternionW,
    QuaternionX,
    QuaternionY,
    QuaternionZ,
    Roll,
    Pitch,
    Yaw,
    AngularRateX,
    AngularRateY,
    AngularRateZ,
    AngularRateMagnitude,
    Mass,
    ForceBodyX,
    ForceBodyY,
    ForceBodyZ,
    ForceMagnitude,
    MomentBodyX,
    MomentBodyY,
    MomentBodyZ,
    MomentMagnitude,
    DynamicPressure,
    AngleOfAttack,
    Sideslip,
    AirDensity,
    Mach,
}

impl ChannelSelector {
    /// Pull the channel value out of a sample.
    ///
    /// Altitude and world-frame components assume an ENU world frame, which is
    /// the default. A project that selects NED must map the axes at the display
    /// boundary; the recorded values are unchanged.
    pub fn value(&self, s: &StateSample) -> Real {
        match self {
            ChannelSelector::Time => s.time,
            ChannelSelector::PositionX => s.position[0],
            ChannelSelector::PositionY => s.position[1],
            ChannelSelector::PositionZ => s.position[2],
            ChannelSelector::Altitude => s.position[2],
            ChannelSelector::VelocityX => s.velocity[0],
            ChannelSelector::VelocityY => s.velocity[1],
            ChannelSelector::VelocityZ => s.velocity[2],
            ChannelSelector::Speed => s.speed(),
            ChannelSelector::VerticalVelocity => s.velocity[2],
            ChannelSelector::AccelerationWorldX => s.acceleration_world[0],
            ChannelSelector::AccelerationWorldY => s.acceleration_world[1],
            ChannelSelector::AccelerationWorldZ => s.acceleration_world[2],
            ChannelSelector::AccelerationMagnitude => Vec3::new(
                s.acceleration_world[0],
                s.acceleration_world[1],
                s.acceleration_world[2],
            )
            .norm(),
            ChannelSelector::QuaternionW => s.attitude[0],
            ChannelSelector::QuaternionX => s.attitude[1],
            ChannelSelector::QuaternionY => s.attitude[2],
            ChannelSelector::QuaternionZ => s.attitude[3],
            ChannelSelector::Roll => s.euler_321[0],
            ChannelSelector::Pitch => s.euler_321[1],
            ChannelSelector::Yaw => s.euler_321[2],
            ChannelSelector::AngularRateX => s.angular_velocity[0],
            ChannelSelector::AngularRateY => s.angular_velocity[1],
            ChannelSelector::AngularRateZ => s.angular_velocity[2],
            ChannelSelector::AngularRateMagnitude => s.angular_velocity_vec().norm(),
            ChannelSelector::Mass => s.mass,
            ChannelSelector::ForceBodyX => s.force_body[0],
            ChannelSelector::ForceBodyY => s.force_body[1],
            ChannelSelector::ForceBodyZ => s.force_body[2],
            ChannelSelector::ForceMagnitude => {
                Vec3::new(s.force_body[0], s.force_body[1], s.force_body[2]).norm()
            }
            ChannelSelector::MomentBodyX => s.moment_body[0],
            ChannelSelector::MomentBodyY => s.moment_body[1],
            ChannelSelector::MomentBodyZ => s.moment_body[2],
            ChannelSelector::MomentMagnitude => {
                Vec3::new(s.moment_body[0], s.moment_body[1], s.moment_body[2]).norm()
            }
            ChannelSelector::DynamicPressure => s.dynamic_pressure,
            ChannelSelector::AngleOfAttack => s.angle_of_attack,
            ChannelSelector::Sideslip => s.sideslip,
            ChannelSelector::AirDensity => s.air_density,
            ChannelSelector::Mach => s.mach,
        }
    }

    /// Branch name for the channel, shown in the plot legend.
    pub fn label(&self) -> &'static str {
        match self {
            ChannelSelector::Time => "Time",
            ChannelSelector::PositionX => "Position X",
            ChannelSelector::PositionY => "Position Y",
            ChannelSelector::PositionZ => "Position Z",
            ChannelSelector::Altitude => "Altitude",
            ChannelSelector::VelocityX => "Velocity X",
            ChannelSelector::VelocityY => "Velocity Y",
            ChannelSelector::VelocityZ => "Velocity Z",
            ChannelSelector::Speed => "Speed",
            ChannelSelector::VerticalVelocity => "Vertical velocity",
            ChannelSelector::AccelerationWorldX => "Acceleration X",
            ChannelSelector::AccelerationWorldY => "Acceleration Y",
            ChannelSelector::AccelerationWorldZ => "Acceleration Z",
            ChannelSelector::AccelerationMagnitude => "Acceleration",
            ChannelSelector::QuaternionW => "Quaternion W",
            ChannelSelector::QuaternionX => "Quaternion X",
            ChannelSelector::QuaternionY => "Quaternion Y",
            ChannelSelector::QuaternionZ => "Quaternion Z",
            ChannelSelector::Roll => "Roll",
            ChannelSelector::Pitch => "Pitch",
            ChannelSelector::Yaw => "Yaw",
            ChannelSelector::AngularRateX => "Angular rate X",
            ChannelSelector::AngularRateY => "Angular rate Y",
            ChannelSelector::AngularRateZ => "Angular rate Z",
            ChannelSelector::AngularRateMagnitude => "Angular rate",
            ChannelSelector::Mass => "Mass",
            ChannelSelector::ForceBodyX => "Force X",
            ChannelSelector::ForceBodyY => "Force Y",
            ChannelSelector::ForceBodyZ => "Force Z",
            ChannelSelector::ForceMagnitude => "Total force",
            ChannelSelector::MomentBodyX => "Moment X",
            ChannelSelector::MomentBodyY => "Moment Y",
            ChannelSelector::MomentBodyZ => "Moment Z",
            ChannelSelector::MomentMagnitude => "Total moment",
            ChannelSelector::DynamicPressure => "Dynamic pressure",
            ChannelSelector::AngleOfAttack => "Angle of attack",
            ChannelSelector::Sideslip => "Sideslip",
            ChannelSelector::AirDensity => "Air density",
            ChannelSelector::Mach => "Mach",
        }
    }

    /// Unit string for the channel.
    pub fn unit(&self) -> &'static str {
        match self {
            ChannelSelector::Time => "s",
            ChannelSelector::PositionX
            | ChannelSelector::PositionY
            | ChannelSelector::PositionZ
            | ChannelSelector::Altitude => "m",
            ChannelSelector::VelocityX
            | ChannelSelector::VelocityY
            | ChannelSelector::VelocityZ
            | ChannelSelector::Speed
            | ChannelSelector::VerticalVelocity => "m/s",
            ChannelSelector::AccelerationWorldX
            | ChannelSelector::AccelerationWorldY
            | ChannelSelector::AccelerationWorldZ
            | ChannelSelector::AccelerationMagnitude => "m/s^2",
            ChannelSelector::QuaternionW
            | ChannelSelector::QuaternionX
            | ChannelSelector::QuaternionY
            | ChannelSelector::QuaternionZ => "-",
            ChannelSelector::Roll | ChannelSelector::Pitch | ChannelSelector::Yaw => "deg",
            ChannelSelector::AngularRateX
            | ChannelSelector::AngularRateY
            | ChannelSelector::AngularRateZ
            | ChannelSelector::AngularRateMagnitude => "rad/s",
            ChannelSelector::Mass => "kg",
            ChannelSelector::ForceBodyX
            | ChannelSelector::ForceBodyY
            | ChannelSelector::ForceBodyZ
            | ChannelSelector::ForceMagnitude => "N",
            ChannelSelector::MomentBodyX
            | ChannelSelector::MomentBodyY
            | ChannelSelector::MomentBodyZ
            | ChannelSelector::MomentMagnitude => "N m",
            ChannelSelector::DynamicPressure => "Pa",
            ChannelSelector::AngleOfAttack | ChannelSelector::Sideslip => "deg",
            ChannelSelector::AirDensity => "kg/m^3",
            ChannelSelector::Mach => "-",
        }
    }
}

/// Every channel selector the results workspace can plot, with its label and unit.
pub fn all_channel_names() -> Vec<&'static str> {
    [
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
    ]
    .iter()
    .map(|c| c.label())
    .collect()
}

/// The default chart groups the results workspace opens with.
pub fn default_charts() -> Vec<(String, Vec<ChannelSelector>)> {
    vec![
        (
            "Altitude and vertical velocity".to_string(),
            vec![ChannelSelector::Altitude, ChannelSelector::VerticalVelocity],
        ),
        (
            "Speed and acceleration".to_string(),
            vec![
                ChannelSelector::Speed,
                ChannelSelector::AccelerationMagnitude,
            ],
        ),
        (
            "Roll, pitch, yaw".to_string(),
            vec![
                ChannelSelector::Roll,
                ChannelSelector::Pitch,
                ChannelSelector::Yaw,
            ],
        ),
        (
            "Angular rates".to_string(),
            vec![
                ChannelSelector::AngularRateX,
                ChannelSelector::AngularRateY,
                ChannelSelector::AngularRateZ,
            ],
        ),
        (
            "Total force and moment".to_string(),
            vec![
                ChannelSelector::ForceMagnitude,
                ChannelSelector::MomentMagnitude,
            ],
        ),
        (
            "Angle of attack and sideslip".to_string(),
            vec![ChannelSelector::AngleOfAttack, ChannelSelector::Sideslip],
        ),
    ]
}

/// The recorded summary of a run, which is what gets saved beside the samples.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    /// Run name.
    pub name: String,
    /// Scenario description.
    pub description: String,
    pub status: SimulationStatus,
    pub mode: SimulationMode,
    /// Simulated time span in seconds.
    pub duration: Real,
    /// Simulated time span divided by wall-clock time, when measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub real_time_factor: Option<Real>,
    /// Wall-clock duration of the run in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_clock_seconds: Option<Real>,
    /// Integrator description.
    pub solver: String,
    /// Integration step for a fixed-step solver, seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_step: Option<Real>,
    /// Relative tolerance for an adaptive solver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative_tolerance: Option<Real>,
    /// Absolute tolerance for an adaptive solver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub absolute_tolerance: Option<Real>,
    /// Output sampling interval, seconds.
    pub output_interval: Real,
    /// Effective output rate, hertz.
    pub output_rate_hz: Real,
    /// Samples recorded.
    pub sample_count: usize,
    /// Solver statistics.
    pub solver_statistics: SolverStatistics,
    /// Quaternion normalisation policy description.
    pub normalization: String,
    /// Mass model description.
    pub mass_model: String,
    /// Environment description, one line per component.
    pub environment: Vec<String>,
    /// Force and moment model descriptions.
    pub force_models: Vec<String>,
    /// Active force provider names.
    pub active_providers: Vec<String>,
    /// Initial conditions.
    pub initial: InitialConditions,
    /// Event rules description.
    pub event_rules: EventRules,
    /// Number of events detected.
    pub event_count: usize,
    /// Largest quaternion norm error seen.
    pub maximum_quaternion_norm_error: Real,
    /// Largest angular rate magnitude seen, rad/s.
    pub maximum_angular_rate: Real,
    /// Largest acceleration magnitude seen, m/s^2.
    pub maximum_acceleration: Real,
    /// Largest dynamic pressure seen, Pa.
    pub maximum_dynamic_pressure: Real,
    /// Highest altitude reached, metres.
    pub maximum_altitude: Real,
    /// Fastest speed reached, m/s.
    pub maximum_speed: Real,
    /// Largest Mach number reached.
    pub maximum_mach: Real,
    /// Total energy at the start of the run, joules.
    pub initial_energy: Real,
    /// Total energy at the end of the run, joules.
    pub final_energy: Real,
    /// Relative energy drift, when the run is force-free enough to be meaningful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub energy_drift_ratio: Option<Real>,
    /// Application version that produced the run.
    pub application_version: String,
    /// Timestamp the run was created, ISO 8601.
    pub created_at: String,
    /// Model identifier, when the vehicle came from an imported model.
    pub model_id: String,
    /// Model version.
    pub model_version: String,
    /// Reference geometry used.
    pub reference_geometry: ReferenceGeometry,
    /// Warnings raised by the run.
    pub warnings: Vec<RunWarning>,
    /// User notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// A complete simulation result: the summary plus the sample history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationResult {
    pub summary: RunSummary,
    pub samples: Vec<StateSample>,
    pub events: Vec<FlightEvent>,
}

impl SimulationResult {
    /// Channel data for plotting.
    pub fn channel(&self, selector: ChannelSelector) -> Vec<Real> {
        self.samples.iter().map(|s| selector.value(s)).collect()
    }

    pub fn times(&self) -> Vec<Real> {
        self.samples.iter().map(|s| s.time).collect()
    }

    /// Index of the sample nearest a time, for the shared time cursor.
    pub fn index_at_time(&self, time: Real) -> Option<usize> {
        index_at_time(&self.samples, time)
    }
}

/// Index of the sample nearest a time in a recorded history.
///
/// Used by the replay scrubber and by comparison alignment, which both need the
/// same nearest-neighbour rule.
pub fn index_at_time(samples: &[StateSample], time: Real) -> Option<usize> {
    if samples.is_empty() {
        return None;
    }
    let times: Vec<Real> = samples.iter().map(|s| s.time).collect();
    match times.binary_search_by(|t| t.partial_cmp(&time).unwrap_or(std::cmp::Ordering::Equal)) {
        Ok(i) => Some(i),
        Err(0) => Some(0),
        Err(i) if i >= times.len() => Some(times.len() - 1),
        Err(i) => {
            if (times[i] - time).abs() < (time - times[i - 1]).abs() {
                Some(i)
            } else {
                Some(i - 1)
            }
        }
    }
}

/// The scenario assembled from a configuration, ready to integrate.
pub struct BuiltRun {
    pub config: RunConfig,
    pub vehicle: Arc<Vehicle>,
    pub dynamics: RigidBodyDynamics,
}

impl std::fmt::Debug for BuiltRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltRun")
            .field("config", &self.config.name)
            .field("vehicle", &self.vehicle.model_id())
            .finish()
    }
}

/// Assemble the dynamics for a configuration.
///
/// The vehicle is built from the configuration's mass model and reference
/// geometry, so a run is fully reproducible from its `RunConfig` alone. When the
/// caller supplies an imported model, [`build_run_with_vehicle`] keeps the
/// model's identity in the metadata.
pub fn build_run(config: RunConfig) -> BuiltRun {
    let geometry = ReferenceGeometry::default();
    let vehicle = Vehicle {
        model_id: "scenario.inline".to_string(),
        model_version: "1.0".to_string(),
        display_name: config.name.clone(),
        frame: hex_core::Frame::default(),
        reference_geometry: geometry,
        mass: config.mass.to_provider(),
        mesh_reference: None,
        thrust_application_point: Vec3::zeros(),
    };
    build_run_with_vehicle(config, vehicle)
}

/// Assemble the dynamics for a configuration using an imported vehicle model.
pub fn build_run_with_vehicle(config: RunConfig, vehicle: Vehicle) -> BuiltRun {
    let chain = config.forces.to_chain();
    let launch_point = config.initial.position_vec();
    let mass_flow = config.forces.mass_flow.clone();
    let recovery_time = config.recovery_time;
    let mode = config.mode;
    let environment = config.environment.clone();

    let mut dynamics =
        RigidBodyDynamics::new(&vehicle, environment, chain, mode).with_launch_point(launch_point);
    if let Some(flow) = mass_flow {
        dynamics = dynamics.with_mass_flow(flow);
    }
    if let Some(t) = recovery_time {
        dynamics = dynamics.with_recovery_time(t);
    }
    BuiltRun {
        config,
        vehicle: Arc::new(vehicle),
        dynamics,
    }
}

/// Cancellation handle shared with the caller.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Request cancellation. The run stops at its next output boundary.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// A token that is never cancelled.
    pub fn never() -> Self {
        Self::new()
    }
}

/// Run a simulation described by an already assembled scenario.
///
/// `progress` is called at most once per output sample, so a slow UI can never
/// change the numerics.
pub fn run_built(
    built: &BuiltRun,
    cancel: &CancellationToken,
    mut progress: impl FnMut(RunProgress),
) -> RunOutcome {
    let started = std::time::Instant::now();
    let config = &built.config;
    let vehicle: &dyn VehicleModel = built.vehicle.as_ref();
    let dynamics = &built.dynamics;

    let bound = BoundDynamics { dynamics, vehicle };

    let mut state = config.initial.to_state();
    if built.dynamics.constant_mass {
        state.mass = built.dynamics.fixed_mass;
    }
    let mut time = config.start_time;

    let mut samples: Vec<StateSample> = Vec::with_capacity(config.expected_samples().min(200_000));
    let mut events: Vec<FlightEvent> = Vec::new();
    let mut warnings: Vec<RunWarning> = Vec::new();
    let mut statistics = SolverStatistics::default();
    let mut detector = EventDetector::new(config.event_rules);
    detector.recovery_time = config.recovery_time;

    let mut maximum_quaternion_error: Real = 0.0;
    let mut maximum_angular_rate: Real = 0.0;
    let mut maximum_acceleration: Real = 0.0;
    let mut maximum_dynamic_pressure: Real = 0.0;
    let mut maximum_altitude: Real = Real::NEG_INFINITY;
    let mut maximum_speed: Real = 0.0;
    let mut maximum_mach: Real = 0.0;

    let world_up = built.dynamics.world_up;
    let ground_elevation = config.environment.ground.elevation;

    // Record the initial sample before stepping.
    let (_, mut diagnostics) = dynamics.compute(time, &state, vehicle);
    let initial_energy = {
        let health = crate::equations::StateHealth::measure(
            &state,
            &diagnostics.inertia,
            config.environment.gravity.nominal_magnitude(),
            world_up,
            config.thresholds.quaternion_norm_error,
        );
        health.total_energy
    };

    let mut status = SimulationStatus::Completed;
    let mut next_output_time = config.start_time;
    let mut adaptive_step = match config.solver {
        SolverKind::Rk45 { initial_step, .. } => initial_step.max(1e-9),
        SolverKind::Rk4 { step } => step,
    };

    macro_rules! record {
        ($time:expr) => {{
            let mut sample = StateSample::from_state($time, &state);
            sample.force_body = diagnostics.force_body;
            sample.moment_body = diagnostics.moment_body;
            sample.force_world = diagnostics.force_world;
            sample.acceleration_world = diagnostics.acceleration_world;
            sample.dynamic_pressure = diagnostics.dynamic_pressure;
            sample.angle_of_attack = diagnostics.angle_of_attack;
            sample.sideslip = diagnostics.sideslip;
            sample.air_density = diagnostics.air_density;
            sample.mach = diagnostics.mach;
            samples.push(sample);

            maximum_quaternion_error = maximum_quaternion_error.max(state.quaternion_norm_error());
            maximum_angular_rate = maximum_angular_rate.max(
                state
                    .angular_velocity
                    .iter()
                    .fold(0.0, |a: Real, v| a.max(v.abs())),
            );
            maximum_acceleration = maximum_acceleration.max(
                Vec3::new(
                    diagnostics.acceleration_world[0],
                    diagnostics.acceleration_world[1],
                    diagnostics.acceleration_world[2],
                )
                .norm(),
            );
            maximum_dynamic_pressure = maximum_dynamic_pressure.max(diagnostics.dynamic_pressure);
            maximum_altitude =
                maximum_altitude.max(state.position.dot(&world_up) - ground_elevation);
            maximum_speed = maximum_speed.max(state.speed());
            maximum_mach = maximum_mach.max(diagnostics.mach);
        }};
    }

    record!(time);
    let mut thrust_now = config
        .forces
        .thrust
        .as_ref()
        .map(|t| t.thrust_at(time))
        .unwrap_or(0.0);
    let mut observation =
        dynamics.observation(time, &state, &diagnostics, thrust_now, ground_elevation);
    events.extend(detector.observe(observation));

    while time < config.end_time {
        if cancel.is_cancelled() {
            status = SimulationStatus::Cancelled;
            break;
        }
        if samples.len() >= config.maximum_samples {
            status = SimulationStatus::SampleLimitReached;
            break;
        }

        // Captured so a step that reaches the ground can be interpolated back to
        // the crossing instant instead of being reported below it.
        let time_before = time;
        let state_before = state;
        let altitude_before = state.position.dot(&world_up) - ground_elevation;

        let remaining = config.end_time - time;
        let mut x = state.to_array();
        if dynamics.constant_mass {
            x[13] = dynamics.fixed_mass;
        }

        match config.solver {
            SolverKind::Rk4 { step } => {
                let h = step.min(remaining).max(1e-12);
                let mut out = [0.0; STATE_LEN];
                Rk4.step(&bound, time, &x, h, &mut out);
                statistics.function_evaluations += 4;
                let mut next = RigidBodyState::from_array(&out);
                if dynamics.constant_mass {
                    next.mass = dynamics.fixed_mass;
                }
                config.normalization.apply(&mut next);
                state = next;
                time += h;
                statistics.accepted_steps += 1;
                statistics.minimum_step_used = if statistics.minimum_step_used == 0.0 {
                    h
                } else {
                    statistics.minimum_step_used.min(h)
                };
                statistics.maximum_step_used = statistics.maximum_step_used.max(h);
            }
            SolverKind::Rk45 {
                relative_tolerance,
                absolute_tolerance,
                maximum_step,
                minimum_step,
                ..
            } => {
                let h = adaptive_step.min(maximum_step).min(remaining).max(1e-12);
                let weights = ErrorWeights::default();
                let mut out = [0.0; STATE_LEN];
                let outcome = Rk45.step(
                    &bound,
                    time,
                    &x,
                    h,
                    &weights,
                    relative_tolerance,
                    absolute_tolerance,
                    &mut out,
                );
                statistics.function_evaluations += 6;
                if outcome.rejected {
                    statistics.rejected_steps += 1;
                    adaptive_step = outcome.suggested_next_step.max(minimum_step);
                    if adaptive_step >= h {
                        // The controller cannot reduce the step any further, so
                        // accept the step rather than spinning.
                        statistics.rejected_steps -= 1;
                        statistics.accepted_steps += 1;
                        statistics.maximum_step_used = statistics.maximum_step_used.max(h);
                        apply_step(
                            &mut state,
                            &out,
                            h,
                            &config.normalization,
                            dynamics.constant_mass,
                            dynamics.fixed_mass,
                        );
                        time += h;
                    }
                    continue;
                }
                statistics.accepted_steps += 1;
                statistics.minimum_step_used = if statistics.minimum_step_used == 0.0 {
                    h
                } else {
                    statistics.minimum_step_used.min(h)
                };
                statistics.maximum_step_used = statistics.maximum_step_used.max(h);
                apply_step(
                    &mut state,
                    &out,
                    h,
                    &config.normalization,
                    dynamics.constant_mass,
                    dynamics.fixed_mass,
                );
                time += h;
                adaptive_step = outcome
                    .suggested_next_step
                    .clamp(minimum_step, maximum_step);
            }
        }

        // Non-finite detection stops the run immediately with a diagnostic.
        if !state.is_finite() {
            status = SimulationStatus::Failed;
            warnings.push(RunWarning::new(
                "solver.non_finite",
                "State became non-finite",
                format!(
                    "At t = {:.6} s a state component became NaN or infinite. The integration step is likely too large for the dynamics present, or a model input is degenerate.",
                    time
                ),
                true,
            ));
            break;
        }

        // Ground intersection. The ground plane is a collision surface, so a
        // trajectory that has reached it ends there rather than continuing into
        // the terrain, where nothing about the motion is meaningful. A vehicle
        // that never climbed above a metre is settling on the pad rather than
        // impacting, so the same one metre floor the apogee rule uses applies.
        let altitude_after = state.position.dot(&world_up) - ground_elevation;
        if config.environment.ground.enabled
            && maximum_altitude > 1.0
            && altitude_before > 0.0
            && altitude_after <= 0.0
        {
            let fraction = altitude_before / (altitude_before - altitude_after);
            let impact_time = time_before + fraction * (time - time_before);
            // Interpolated to the crossing instant, so the final sample sits on
            // the ground rather than below it.
            state.position =
                state_before.position + (state.position - state_before.position) * fraction;
            state.velocity =
                state_before.velocity + (state.velocity - state_before.velocity) * fraction;
            state.attitude = state_before
                .attitude
                .slerp(&state.attitude, fraction)
                .normalized();
            state.angular_velocity = state_before.angular_velocity
                + (state.angular_velocity - state_before.angular_velocity) * fraction;
            state.mass = state_before.mass + (state.mass - state_before.mass) * fraction;
            time = impact_time;

            let (_, impact_diag) = dynamics.compute(time, &state, vehicle);
            diagnostics = impact_diag;
            record!(time);
            let mut impact_observation =
                dynamics.observation(time, &state, &diagnostics, 0.0, ground_elevation);
            impact_observation.altitude = 0.0;
            let fired = detector.observe(impact_observation);
            if !fired.is_empty() {
                events.extend(fired);
            }
            warnings.push(RunWarning::new(
                "flight.ground_impact",
                "The run ended at ground impact",
                format!(
                    "The vehicle reached the ground at t = {:.4} s with a vertical speed of {:.2} m/s. The trajectory after impact is not modelled, so the history stops at that instant.",
                    impact_time,
                    state.velocity.dot(&world_up)
                ),
                false,
            ));
            status = SimulationStatus::GroundImpact;
            break;
        }

        let (_, diag) = dynamics.compute(time, &state, vehicle);
        diagnostics = diag;

        if time + 1e-12 >= next_output_time {
            record!(time);
            thrust_now = config
                .forces
                .thrust
                .as_ref()
                .map(|t| t.thrust_at(time))
                .unwrap_or(0.0);
            observation =
                dynamics.observation(time, &state, &diagnostics, thrust_now, ground_elevation);
            let fired = detector.observe(observation);
            if !fired.is_empty() {
                events.extend(fired);
            }
            next_output_time += config.output_interval;
            if next_output_time <= time {
                next_output_time = time + config.output_interval;
            }
            progress(RunProgress {
                time,
                fraction: ((time - config.start_time) / (config.end_time - config.start_time))
                    .clamp(0.0, 1.0),
                samples: samples.len(),
                accepted_steps: statistics.accepted_steps,
                rejected_steps: statistics.rejected_steps,
                events: events.len(),
            });
        }
    }

    // Record the final sample if the loop ended between output boundaries.
    if samples.last().map(|s| s.time).unwrap_or(Real::NEG_INFINITY) < time - 1e-12 {
        let (_, diag) = dynamics.compute(time, &state, vehicle);
        diagnostics = diag;
        record!(time);
    }

    thrust_now = config
        .forces
        .thrust
        .as_ref()
        .map(|t| t.thrust_at(time))
        .unwrap_or(0.0);
    observation = dynamics.observation(time, &state, &diagnostics, thrust_now, ground_elevation);
    events.extend(detector.finish(observation));

    let final_energy = {
        let health = crate::equations::StateHealth::measure(
            &state,
            &diagnostics.inertia,
            config.environment.gravity.nominal_magnitude(),
            world_up,
            config.thresholds.quaternion_norm_error,
        );
        health.total_energy
    };

    // ---- Warnings ---------------------------------------------------------
    if statistics.is_struggling() {
        warnings.push(RunWarning::new(
            "solver.step_rejections",
            "Adaptive solver rejected many steps",
            format!(
                "{} of {} attempted steps were rejected ({:.1} percent). Consider a tighter tolerance or a smaller maximum step.",
                statistics.rejected_steps,
                statistics.accepted_steps + statistics.rejected_steps,
                100.0 * statistics.rejection_ratio()
            ),
            false,
        ));
    }
    if maximum_quaternion_error > config.thresholds.quaternion_norm_error {
        warnings.push(RunWarning::new(
            "attitude.norm_drift",
            "Quaternion norm drifted beyond tolerance",
            format!(
                "The largest quaternion norm error was {:.3e}, above the configured {:.3e}. The attitude history is still usable because the quaternion is normalized, but the integration step may be too large.",
                maximum_quaternion_error, config.thresholds.quaternion_norm_error
            ),
            false,
        ));
    }
    if maximum_angular_rate > config.thresholds.maximum_angular_rate {
        warnings.push(RunWarning::new(
            "dynamics.high_angular_rate",
            "High angular rate",
            format!(
                "The angular rate reached {:.2} rad/s, above the configured {:.2} rad/s. Small-step behaviour may not be resolved at the chosen output rate.",
                maximum_angular_rate, config.thresholds.maximum_angular_rate
            ),
            false,
        ));
    }
    if maximum_acceleration > config.thresholds.maximum_acceleration {
        warnings.push(RunWarning::new(
            "dynamics.high_acceleration",
            "High acceleration",
            format!(
                "The acceleration reached {:.2} m/s^2, above the configured {:.2} m/s^2.",
                maximum_acceleration, config.thresholds.maximum_acceleration
            ),
            false,
        ));
    }
    if let Some(a) = &config.forces.aero {
        // The source is named, because a drag table the vehicle designer
        // measured and a coefficient this tool chose are not the same evidence.
        if a.drag_only {
            warnings.push(RunWarning::new(
                "aero.simplified",
                "Simplified aerodynamic model",
                format!(
                    "This run used a drag-only aerodynamic model from {}. Lift and aerodynamic moments were not modelled. {}",
                    a.source.phrase(),
                    a.fidelity_label()
                ),
                false,
            ));
        } else {
            warnings.push(RunWarning::new(
                "aero.coefficients",
                "Aerodynamic coefficients, not a measurement",
                format!(
                    "This run used a coefficient model from {}. The forces and moments follow the coefficients, so the result is only as good as they are. {}",
                    a.source.phrase(),
                    a.fidelity_label()
                ),
                false,
            ));
        }
    }
    if config.environment.atmosphere.is_vacuum() && config.forces.aero.is_some() {
        warnings.push(RunWarning::new(
            "environment.vacuum_with_aero",
            "Aerodynamics enabled in vacuum",
            "An aerodynamic model is configured but the atmosphere is a vacuum, so it had no effect.",
            false,
        ));
    }

    // Energy drift is only a meaningful diagnostic for a run with no thrust,
    // because a thrusting run is deliberately adding energy.
    let energy_drift_ratio = if config.forces.thrust.is_none() && initial_energy.abs() > 1e-9 {
        let drift = (final_energy - initial_energy).abs() / initial_energy.abs();
        if drift > config.thresholds.energy_drift_ratio {
            warnings.push(RunWarning::new(
                "solver.energy_drift",
                "Energy drifted more than expected",
                format!(
                    "Total energy changed by {:.3} percent over a run with no thrust. For a conservative case this indicates the step size is too large for the dynamics.",
                    100.0 * drift
                ),
                false,
            ));
        }
        Some(drift)
    } else {
        None
    };

    if status == SimulationStatus::SampleLimitReached {
        warnings.push(RunWarning::new(
            "run.sample_limit",
            "Output sample limit reached",
            format!(
                "The run stopped after {} output samples, before the configured end time of {:.3} s. Increase the sample limit or the output interval to reach the end.",
                config.maximum_samples, config.end_time
            ),
            false,
        ));
    }

    // A landing burn ignites on the state rather than at a time, so the ignition
    // is read back from the recorded history: it is the first sample whose state
    // satisfies the trigger. Scanning the samples keeps this exact, and it can
    // only ever report a burn that was configured and that the run actually lit.
    if let Some(burn) = &config.forces.landing_burn {
        let already = events.iter().any(|e| e.kind == EventKind::LandingBurn);
        if !already {
            if let Some(sample) = samples.iter().find(|sample| {
                let position =
                    Vec3::new(sample.position[0], sample.position[1], sample.position[2]);
                let velocity =
                    Vec3::new(sample.velocity[0], sample.velocity[1], sample.velocity[2]);
                let gravity = config
                    .environment
                    .gravity
                    .acceleration(position, -world_up)
                    .norm();
                burn.fires_at(
                    &position,
                    &velocity,
                    sample.mass,
                    &world_up,
                    config.environment.ground.elevation,
                    gravity,
                )
            }) {
                events.push(
                    FlightEvent::new(EventKind::LandingBurn, sample.time, "Landing burn")
                        .with_trigger(burn.thrust, "N")
                        .with_description(
                            "The landing engine lit on the stopping-distance condition: the vehicle is descending, low, and close enough to the ground to stop at its own thrust-to-weight.",
                        ),
                );
            }
        }
    }

    sort_events(&mut events);

    let wall_clock = started.elapsed().as_secs_f64();
    let duration = (time - config.start_time).max(0.0);
    let real_time_factor = if wall_clock > 1e-9 {
        Some(duration / wall_clock)
    } else {
        None
    };

    let (integration_step, relative_tolerance, absolute_tolerance) = match config.solver {
        SolverKind::Rk4 { step } => (Some(step), None, None),
        SolverKind::Rk45 {
            relative_tolerance,
            absolute_tolerance,
            ..
        } => (None, Some(relative_tolerance), Some(absolute_tolerance)),
    };

    let summary = RunSummary {
        name: config.name.clone(),
        description: config.description.clone(),
        status,
        mode: config.mode,
        duration,
        real_time_factor,
        wall_clock_seconds: Some(wall_clock),
        solver: config.solver.label(),
        integration_step,
        relative_tolerance,
        absolute_tolerance,
        output_interval: config.output_interval,
        output_rate_hz: if config.output_interval > 0.0 {
            1.0 / config.output_interval
        } else {
            0.0
        },
        sample_count: samples.len(),
        solver_statistics: statistics,
        normalization: config.normalization.label(),
        mass_model: config.mass.label(),
        environment: config.environment.describe(),
        force_models: config.forces.describe(),
        active_providers: built.dynamics.providers.active_names(),
        initial: config.initial,
        event_rules: config.event_rules,
        event_count: events.len(),
        maximum_quaternion_norm_error: maximum_quaternion_error,
        maximum_angular_rate,
        maximum_acceleration,
        maximum_dynamic_pressure,
        maximum_altitude: if maximum_altitude.is_finite() {
            maximum_altitude
        } else {
            0.0
        },
        maximum_speed,
        maximum_mach,
        initial_energy,
        final_energy,
        energy_drift_ratio,
        application_version: hex_core::app_version().to_string(),
        created_at: chrono_free_timestamp(),
        model_id: built.vehicle.model_id().to_string(),
        model_version: built.vehicle.model_version().to_string(),
        reference_geometry: built.vehicle.reference_geometry(),
        warnings: warnings.clone(),
        notes: config.notes.clone(),
    };

    RunOutcome {
        status,
        samples,
        events,
        summary,
        warnings,
        final_diagnostics: diagnostics,
        final_state: state,
    }
}

fn apply_step(
    state: &mut RigidBodyState,
    out: &[Real; STATE_LEN],
    _h: Real,
    normalization: &NormalizationPolicy,
    constant_mass: bool,
    fixed_mass: Real,
) {
    let mut next = RigidBodyState::from_array(out);
    if constant_mass {
        next.mass = fixed_mass;
    }
    normalization.apply(&mut next);
    *state = next;
}

/// Run a simulation from a configuration, with no cancellation and no progress.
pub fn run_simulation(config: RunConfig) -> RunOutcome {
    let built = build_run(config);
    run_built(&built, &CancellationToken::never(), |_| {})
}

/// Run a simulation with a progress callback.
pub fn run_simulation_with_progress(
    config: RunConfig,
    progress: impl FnMut(RunProgress),
) -> RunOutcome {
    let built = build_run(config);
    run_built(&built, &CancellationToken::never(), progress)
}

/// A timestamp without pulling a timezone database into the dynamics crate.
///
/// The project layer replaces this with a full ISO 8601 value from `chrono`.
fn chrono_free_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{}", now)
}

/// Convenience for a provider list, used by tests and the CLI harness.
pub fn providers_from_config(config: &ForceConfig) -> ProviderChain {
    config.to_chain()
}

/// The names of the force sources a configuration will evaluate.
pub fn configured_sources(config: &ForceConfig) -> Vec<ForceSource> {
    let mut v = Vec::new();
    if config.gravity {
        v.push(ForceSource::Gravity);
    }
    if config.thrust.is_some() {
        v.push(ForceSource::Thrust);
    }
    if config.aero.is_some() {
        v.push(ForceSource::Drag);
    }
    if config.control.is_some() {
        v.push(ForceSource::Control);
    }
    if config.external.is_some() {
        v.push(ForceSource::External);
    }
    v
}

/// A provider that does nothing, for building scenarios with no forces.
pub struct NoForce;

impl ForceProvider for NoForce {
    fn name(&self) -> &str {
        "none"
    }

    fn evaluate(
        &self,
        _ctx: &crate::forces::ForceContext,
        _out: &mut crate::forces::ForceAccumulator,
    ) {
    }

    fn is_active(&self) -> bool {
        false
    }
}
