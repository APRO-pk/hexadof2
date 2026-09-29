//! The equations of motion for the 6-DOF and 3-DOF modes.
//!
//! # Translational dynamics
//!
//! ```text
//! dr_I/dt = v_I
//! m * dv_I/dt = F_total_I
//! ```
//!
//! Forces are summed in the world frame. Body-frame forces are rotated into the
//! world frame by each provider before they are stored, so the sum never mixes
//! frames.
//!
//! # Rotational dynamics
//!
//! Angular velocity is in the body frame:
//!
//! ```text
//! I_B * dw_B/dt + w_B x (I_B * w_B) = M_total_B
//! dw_B/dt = I_B^-1 * (M_total_B - w_B x (I_B * w_B))
//! ```
//!
//! The inertia tensor is evaluated about the centre of mass and expressed in the
//! body frame, which is the same frame as the moments.
//!
//! # Attitude kinematics
//!
//! ```text
//! dq_BI/dt = 0.5 * q_BI * [0, w_B]
//! ```
//!
//! `q_BI` maps body vectors into the world frame, as fixed in `hex-core`.
//!
//! # Mass
//!
//! ```text
//! dm/dt = -m_dot_propellant
//! ```
//!
//! When the vehicle model declares constant mass the mass derivative is forced to
//! zero so the state dimension stays fixed.

use hex_core::{InertiaTensor, Quaternion, Real, Vec3};
use serde::{Deserialize, Serialize};

use crate::environment::{AirProperties, Environment};
use crate::events::EventObservation;
use crate::forces::{accumulate, ForceAccumulator, ForceProvider, ForceSource, GravityForce};
use crate::integrators::Derivative;
use crate::state::{RigidBodyState, SimulationMode, STATE_LEN};
use crate::vehicle::{MassFlowProfile, VehicleModel};

/// A constraint that restricts motion to a line until it releases.
///
/// The rail is applied as a projection on the translational acceleration and
/// velocity, and it freezes rotation. That is the physically meaningful part of a
/// rail: the vehicle cannot pitch or yaw while the buttons are engaged.
#[derive(Debug, Clone, Copy, PartialEq)]
struct RailConstraint {
    direction: Vec3,
    launch_point: Vec3,
    length: Real,
    active: bool,
}

impl RailConstraint {
    fn new(direction: Vec3, launch_point: Vec3, length: Real, enabled: bool) -> Self {
        Self {
            direction,
            launch_point,
            length,
            active: enabled && length > 0.0,
        }
    }

    /// Project a world-frame vector onto the rail direction.
    fn project(&self, v: Vec3) -> Vec3 {
        self.direction * v.dot(&self.direction)
    }

    /// Components of `v` perpendicular to the rail.
    fn perpendicular(&self, v: Vec3) -> Vec3 {
        v - self.project(v)
    }
}

/// An ordered list of force providers.
///
/// A named type rather than a bare `Vec` so the run configuration reads clearly
/// and so the order of evaluation is explicit in one place.
#[derive(Default)]
pub struct ProviderChain {
    providers: Vec<Box<dyn ForceProvider>>,
}

impl ProviderChain {
    pub fn new() -> Self {
        Self::default()
    }

    /// A chain that always starts with gravity.
    pub fn with_gravity() -> Self {
        let mut c = Self::new();
        c.push(Box::new(GravityForce));
        c
    }

    pub fn push(&mut self, provider: Box<dyn ForceProvider>) -> &mut Self {
        self.providers.push(provider);
        self
    }

    pub fn with(mut self, provider: Box<dyn ForceProvider>) -> Self {
        self.providers.push(provider);
        self
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn ForceProvider> {
        self.providers.iter().map(|p| p.as_ref())
    }

    /// One line per provider, for the run metadata panel.
    pub fn describe(&self) -> Vec<String> {
        self.providers
            .iter()
            .map(|p| format!("{}: {}", p.name(), p.describe()))
            .collect()
    }

    /// Names of the providers that can affect the outcome.
    pub fn active_names(&self) -> Vec<String> {
        self.providers
            .iter()
            .filter(|p| p.is_active())
            .map(|p| p.name().to_string())
            .collect()
    }
}

/// Everything needed to evaluate the equations of motion.
pub struct RigidBodyDynamics {
    /// Simulation mode.
    pub mode: SimulationMode,
    /// Unit up axis of the world frame.
    pub world_up: Vec3,
    /// Unit down axis of the world frame.
    pub world_down: Vec3,
    /// Force and moment providers, evaluated in order.
    pub providers: ProviderChain,
    /// Environment for this run.
    pub environment: Environment,
    /// Optional propellant mass flow applied on top of the mass provider.
    pub mass_flow: Option<MassFlowProfile>,
    /// Whether the mass provider is constant, so mass is not integrated.
    pub constant_mass: bool,
    /// Constant mass value when `constant_mass` is set.
    pub fixed_mass: Real,
    /// Optional recovery deployment time, passed to the event detector.
    pub recovery_time: Option<Real>,
    rail: RailConstraint,
    /// Diagnostics from the most recent derivative evaluation.
    last: std::sync::Mutex<DynamicsDiagnostics>,
}

/// Values recorded during the most recent evaluation of the equations.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct DynamicsDiagnostics {
    /// Total force in the world frame, N.
    pub force_world: [Real; 3],
    /// Total force in the body frame, N.
    pub force_body: [Real; 3],
    /// Total moment about the centre of gravity, body frame, N m.
    pub moment_body: [Real; 3],
    /// Acceleration in the world frame, m/s^2.
    pub acceleration_world: [Real; 3],
    /// Angular acceleration in the body frame, rad/s^2.
    pub angular_acceleration_body: [Real; 3],
    /// Dynamic pressure, Pa.
    pub dynamic_pressure: Real,
    /// Angle of attack, rad.
    pub angle_of_attack: Real,
    /// Sideslip, rad.
    pub sideslip: Real,
    /// Air density, kg/m^3.
    pub air_density: Real,
    /// Mach number.
    pub mach: Real,
    /// Inertia tensor used, body frame.
    pub inertia: InertiaTensor,
    /// Whether the rail constraint was active.
    pub on_rail: bool,
    /// Distance along the rail, m.
    pub rail_distance: Real,
    /// Air properties used.
    pub air: AirProperties,
}

impl DynamicsDiagnostics {
    pub fn force_world_vec(&self) -> Vec3 {
        Vec3::new(
            self.force_world[0],
            self.force_world[1],
            self.force_world[2],
        )
    }

    pub fn force_body_vec(&self) -> Vec3 {
        Vec3::new(self.force_body[0], self.force_body[1], self.force_body[2])
    }

    pub fn moment_body_vec(&self) -> Vec3 {
        Vec3::new(
            self.moment_body[0],
            self.moment_body[1],
            self.moment_body[2],
        )
    }
}

impl RigidBodyDynamics {
    /// Build the dynamics for a run.
    pub fn new(
        vehicle: &dyn VehicleModel,
        environment: Environment,
        providers: ProviderChain,
        mode: SimulationMode,
    ) -> Self {
        let (world_up, world_down) = match vehicle.frame().world {
            hex_core::WorldFrame::Enu => (Vec3::z(), -Vec3::z()),
            hex_core::WorldFrame::Ned => (-Vec3::z(), Vec3::z()),
        };
        let rail = RailConstraint::new(
            environment.rail.unit_direction(),
            Vec3::zeros(),
            environment.rail.length,
            environment.rail.enabled,
        );
        let constant_mass = vehicle.mass_provider().is_constant();
        let fixed_mass = vehicle.mass_provider().mass(0.0);
        Self {
            mode,
            world_up,
            world_down,
            providers,
            environment,
            mass_flow: None,
            constant_mass,
            fixed_mass,
            recovery_time: None,
            rail,
            last: std::sync::Mutex::new(DynamicsDiagnostics::default()),
        }
    }

    /// Set the launch point the rail is measured from.
    pub fn with_launch_point(mut self, launch_point: Vec3) -> Self {
        self.rail.launch_point = launch_point;
        self
    }

    /// Attach an explicit propellant mass flow.
    ///
    /// A flow only has an effect when the mass provider does not already encode
    /// it, so attaching one clears the constant-mass shortcut. Otherwise the
    /// mass derivative would be forced to zero and the flow would be ignored.
    pub fn with_mass_flow(mut self, flow: MassFlowProfile) -> Self {
        if !flow.already_in_mass_curve {
            self.constant_mass = false;
        }
        self.mass_flow = Some(flow);
        self
    }

    pub fn with_recovery_time(mut self, time: Real) -> Self {
        self.recovery_time = Some(time);
        self
    }

    /// Whether the rail constraint is currently engaged for a state.
    pub fn is_on_rail(&self, state: &RigidBodyState) -> bool {
        self.rail.active && !self.has_released(state)
    }

    fn has_released(&self, state: &RigidBodyState) -> bool {
        !self.rail.active
            || (state.position - self.rail.launch_point).dot(&self.rail.direction)
                >= self.rail.length
    }

    /// Diagnostics from the last evaluation.
    pub fn last_diagnostics(&self) -> DynamicsDiagnostics {
        *self
            .last
            .lock()
            .expect("dynamics diagnostics mutex poisoned")
    }

    /// Evaluate the equations of motion for a vehicle.
    ///
    /// This is separated from the [`Derivative`] implementation so tests and the
    /// stepper can call it directly and keep the diagnostics.
    pub fn compute(
        &self,
        t: Real,
        state: &RigidBodyState,
        vehicle: &dyn VehicleModel,
    ) -> ([Real; STATE_LEN], DynamicsDiagnostics) {
        let ctx = crate::forces::ForceContext::new(
            t,
            state,
            vehicle,
            &self.environment,
            self.world_up,
            self.world_down,
        );
        let acc = accumulate(&self.providers, &ctx);

        let force_world = acc.total_force_world();
        let force_body = acc.total_force_body();
        let moment_body = acc.total_moment_body();

        let on_rail = self.is_on_rail(state);
        let inertia = ctx.inertia;

        // Translational acceleration.
        let acceleration = if state.mass.abs() > 1e-12 {
            force_world / state.mass
        } else {
            Vec3::zeros()
        };

        // Rotational acceleration.
        let angular_acceleration = if self.mode == SimulationMode::ThreeDof || on_rail {
            Vec3::zeros()
        } else {
            let iw = inertia.to_matrix() * state.angular_velocity;
            let gyroscopic = state.angular_velocity.cross(&iw);
            // Solve I * alpha = M - w x (I w) rather than forming an inverse.
            inertia
                .solve(moment_body - gyroscopic)
                .unwrap_or_else(Vec3::zeros)
        };

        let mut velocity_derivative = acceleration;
        let mut angular_velocity = state.angular_velocity;

        if on_rail {
            // Only motion along the rail is allowed while the constraint holds,
            // and rotation is frozen.
            let along = self.rail.direction;
            let normal = self.rail.perpendicular(force_world);
            velocity_derivative = (force_world - normal) / state.mass.max(1e-12);
            let _ = along;
            angular_velocity = Vec3::zeros();
        }

        // Quaternion kinematics from the body rate.
        let q = if self.mode == SimulationMode::ThreeDof {
            Quaternion::identity()
        } else {
            state.attitude
        };
        let q_dot = q.derivative_body_rate(angular_velocity);

        // Mass rate. A constant-mass model declares a zero derivative rather than
        // silently using a curve.
        let mass_rate = if self.constant_mass {
            0.0
        } else {
            let flow = self
                .mass_flow
                .as_ref()
                .map(|f| f.effective_rate(t))
                .unwrap_or(0.0);
            let curve_rate = vehicle.mass_provider().mass_rate(t);
            // The curve rate already has the correct sign. An explicit flow adds
            // to it and is subtracted.
            if flow.abs() > 0.0 {
                curve_rate - flow
            } else {
                curve_rate
            }
        };

        let mut dx = [0.0; STATE_LEN];
        dx[0] = state.velocity.x;
        dx[1] = state.velocity.y;
        dx[2] = state.velocity.z;
        dx[3] = velocity_derivative.x;
        dx[4] = velocity_derivative.y;
        dx[5] = velocity_derivative.z;
        dx[6] = q_dot.w;
        dx[7] = q_dot.x;
        dx[8] = q_dot.y;
        dx[9] = q_dot.z;
        dx[10] = angular_acceleration.x;
        dx[11] = angular_acceleration.y;
        dx[12] = angular_acceleration.z;
        dx[13] = mass_rate;

        if self.mode == SimulationMode::ThreeDof {
            // Attitude is held, so its derivative is zero and the state does not
            // rotate away from the initial condition.
            dx[6] = 0.0;
            dx[7] = 0.0;
            dx[8] = 0.0;
            dx[9] = 0.0;
        }

        let rail_distance = (state.position - self.rail.launch_point).dot(&self.rail.direction);
        let diagnostics = DynamicsDiagnostics {
            force_world: [force_world.x, force_world.y, force_world.z],
            force_body: [force_body.x, force_body.y, force_body.z],
            moment_body: [moment_body.x, moment_body.y, moment_body.z],
            acceleration_world: [
                velocity_derivative.x,
                velocity_derivative.y,
                velocity_derivative.z,
            ],
            angular_acceleration_body: [
                angular_acceleration.x,
                angular_acceleration.y,
                angular_acceleration.z,
            ],
            dynamic_pressure: ctx.dynamic_pressure,
            angle_of_attack: ctx.angle_of_attack(),
            sideslip: ctx.sideslip(),
            air_density: ctx.air.density,
            mach: ctx.mach,
            inertia,
            on_rail,
            rail_distance,
            air: ctx.air,
        };

        if let Ok(mut guard) = self.last.lock() {
            *guard = diagnostics;
        }

        (dx, diagnostics)
    }

    /// Build the event observation for a state, using the diagnostics.
    pub fn observation(
        &self,
        t: Real,
        state: &RigidBodyState,
        diagnostics: &DynamicsDiagnostics,
        thrust: Real,
        ground_elevation: Real,
    ) -> EventObservation {
        let altitude = state.position.dot(&self.world_up) - ground_elevation;
        let vertical_velocity = state.velocity.dot(&self.world_up);
        let rail_distance = (state.position - self.rail.launch_point).dot(&self.rail.direction);
        EventObservation {
            time: t,
            altitude,
            vertical_velocity,
            airspeed: (state.velocity - self.environment.wind_at(state.position, self.world_up))
                .norm(),
            dynamic_pressure: diagnostics.dynamic_pressure,
            thrust,
            mass: state.mass,
            rail_distance,
            on_rail: self.rail.active && rail_distance < self.rail.length,
        }
    }

    /// Extract the thrust magnitude from an accumulator, for event detection.
    pub fn thrust_from(acc: &ForceAccumulator) -> Real {
        acc.get(ForceSource::Thrust)
            .filter(|c| c.active)
            .map(|c| c.force_body.norm())
            .unwrap_or(0.0)
    }

    /// Provider that always contributes gravity, for convenience.
    pub fn gravity_provider() -> Box<dyn ForceProvider> {
        Box::new(GravityForce)
    }
}

/// The dynamics bound to a vehicle, which is what the integrators consume.
pub struct BoundDynamics<'a> {
    pub dynamics: &'a RigidBodyDynamics,
    pub vehicle: &'a dyn VehicleModel,
}

impl Derivative for BoundDynamics<'_> {
    fn derivative(&self, t: Real, x: &[Real; STATE_LEN], dx: &mut [Real; STATE_LEN]) {
        let mut state = RigidBodyState::from_array(x);
        if self.dynamics.constant_mass {
            state.mass = self.dynamics.fixed_mass;
        }
        let (computed, _) = self.dynamics.compute(t, &state, self.vehicle);
        dx.copy_from_slice(&computed);
        if self.dynamics.constant_mass {
            dx[13] = 0.0;
        }
    }
}

/// Diagnostics that describe the quality of a state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StateHealth {
    /// Quaternion norm error.
    pub quaternion_norm_error: Real,
    /// Kinetic energy in joules, when the inertia tensor is known.
    pub kinetic_energy: Real,
    /// Total mechanical energy including gravity potential, when applicable.
    pub total_energy: Real,
    /// Linear momentum magnitude.
    pub linear_momentum: Real,
    /// Angular momentum magnitude in the body frame.
    pub angular_momentum: Real,
    /// Maximum absolute angular rate, rad/s.
    pub maximum_angular_rate: Real,
    /// Maximum absolute acceleration, m/s^2.
    pub maximum_acceleration: Real,
    /// Whether every value is finite.
    pub finite: bool,
    /// Whether the quaternion norm error exceeds the supplied threshold.
    pub quaternion_out_of_tolerance: bool,
}

impl StateHealth {
    /// Evaluate a state.
    pub fn measure(
        state: &RigidBodyState,
        inertia: &InertiaTensor,
        gravity_magnitude: Real,
        world_up: Vec3,
        quaternion_tolerance: Real,
    ) -> Self {
        let norm_error = state.quaternion_norm_error();
        let kinetic = state.kinetic_energy(inertia);
        let potential = state.mass * gravity_magnitude * state.altitude_above(world_up);
        let max_rate = state
            .angular_velocity
            .iter()
            .fold(0.0 as Real, |acc, v| acc.max(v.abs()));
        Self {
            quaternion_norm_error: norm_error,
            kinetic_energy: kinetic,
            total_energy: kinetic + potential,
            linear_momentum: state.linear_momentum().norm(),
            angular_momentum: state.angular_momentum_body(inertia).norm(),
            maximum_angular_rate: max_rate,
            maximum_acceleration: 0.0,
            finite: state.is_finite(),
            quaternion_out_of_tolerance: norm_error > quaternion_tolerance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::Environment;
    use crate::forces::{AeroForce, AeroModel, ExternalLoad, ThrustForce, ThrustProfile};
    use crate::vehicle::Vehicle;
    use std::f64::consts::FRAC_PI_2;

    fn providers(extra: Vec<Box<dyn ForceProvider>>) -> ProviderChain {
        let mut chain = ProviderChain::with_gravity();
        for p in extra {
            chain.push(p);
        }
        chain
    }

    fn dynamics_for(
        vehicle: &dyn VehicleModel,
        env: Environment,
        extra: Vec<Box<dyn ForceProvider>>,
        mode: SimulationMode,
    ) -> RigidBodyDynamics {
        RigidBodyDynamics::new(vehicle, env, providers(extra), mode)
    }

    #[test]
    fn free_translation_has_no_acceleration() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let mut env = Environment::vacuum_uniform_gravity(0.0);
        env.gravity = crate::environment::GravityModel::uniform(0.0);
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);

        let state = RigidBodyState {
            velocity: Vec3::new(3.0, -4.0, 5.0),
            mass: 1.0,
            ..Default::default()
        };
        let (dx, diag) = dynamics.compute(0.0, &state, &vehicle);

        assert!(diag.force_world_vec().norm() < 1e-12);
        assert!((dx[0] - 3.0).abs() < 1e-12);
        assert!((dx[1] + 4.0).abs() < 1e-12);
        assert!((dx[2] - 5.0).abs() < 1e-12);
        for (i, component) in dx.iter().enumerate().skip(3).take(11) {
            assert!(component.abs() < 1e-12, "component {} = {}", i, component);
        }
    }

    #[test]
    fn uniform_gravity_gives_the_configured_acceleration() {
        let vehicle = Vehicle::simple(2.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);
        let state = RigidBodyState::at_rest(2.0);
        let (dx, _) = dynamics.compute(0.0, &state, &vehicle);
        // ENU world frame: down is -Z.
        assert!((dx[5] + 9.81).abs() < 1e-9, "az = {}", dx[5]);
        assert!(dx[3].abs() < 1e-12);
        assert!(dx[4].abs() < 1e-12);
    }

    #[test]
    fn constant_torque_gives_the_expected_angular_acceleration() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(0.0);
        let load = ExternalLoad {
            force_body: Vec3::zeros(),
            moment_body: Vec3::new(0.0, 2.0, 0.0),
            application_point_body: Vec3::zeros(),
            enabled: true,
        };
        let dynamics = dynamics_for(&vehicle, env, vec![Box::new(load)], SimulationMode::SixDof);

        // Inertia of a point vehicle is isotropic, so alpha = M / I.
        let state = RigidBodyState::at_rest(1.0);
        let (dx, diag) = dynamics.compute(0.0, &state, &vehicle);
        let expected = 2.0 / diag.inertia.iyy;
        assert!((dx[11] - expected).abs() < 1e-12, "alpha_y = {}", dx[11]);
        assert!(dx[10].abs() < 1e-12);
        assert!(dx[12].abs() < 1e-12);
    }

    #[test]
    fn asymmetric_torque_free_body_conserves_angular_momentum() {
        // Euler's equations with no torque conserve |I w| in the body frame.
        let mut vehicle = Vehicle::simple(1.0, 0.1);
        vehicle.mass = Box::new(crate::vehicle::ConstantMass::new(
            1.0,
            Vec3::zeros(),
            InertiaTensor::diagonal(1.0, 2.0, 3.0),
        ));
        let env = Environment::vacuum_uniform_gravity(0.0);
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);

        let state = RigidBodyState {
            angular_velocity: Vec3::new(0.3, 0.2, 0.1),
            mass: 1.0,
            ..Default::default()
        };
        let inertia = InertiaTensor::diagonal(1.0, 2.0, 3.0);
        let h0 = state.angular_momentum_body(&inertia);

        // Integrate with RK4 and check conservation.
        let bound = BoundDynamics {
            dynamics: &dynamics,
            vehicle: &vehicle,
        };
        let mut x = state.to_array();
        let h = 1e-4;
        let mut t = 0.0;
        for _ in 0..10_000 {
            let mut out = [0.0; STATE_LEN];
            crate::integrators::Rk4.step(&bound, t, &x, h, &mut out);
            x = out;
            t += h;
            let mut q = Quaternion::new(x[6], x[7], x[8], x[9]);
            q.normalize_in_place();
            x[6] = q.w;
            x[7] = q.x;
            x[8] = q.y;
            x[9] = q.z;
        }
        let s = RigidBodyState::from_array(&x);
        let h1 = s.angular_momentum_body(&inertia);
        assert!(
            (h1.norm() - h0.norm()).abs() < 1e-8,
            "|h| {} vs {}",
            h1.norm(),
            h0.norm()
        );

        // Kinetic energy is also conserved for a torque-free rigid body.
        let e0 = state.kinetic_energy(&inertia);
        let e1 = s.kinetic_energy(&inertia);
        assert!((e1 - e0).abs() < 1e-9, "energy {} vs {}", e1, e0);
    }

    #[test]
    fn gyroscopic_term_appears_for_an_asymmetric_body() {
        // With no external torque, an asymmetric body must still accelerate.
        let mut vehicle = Vehicle::simple(1.0, 0.1);
        vehicle.mass = Box::new(crate::vehicle::ConstantMass::new(
            1.0,
            Vec3::zeros(),
            InertiaTensor::diagonal(1.0, 2.0, 3.0),
        ));
        let env = Environment::vacuum_uniform_gravity(0.0);
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);
        let state = RigidBodyState {
            angular_velocity: Vec3::new(1.0, 1.0, 0.0),
            mass: 1.0,
            ..Default::default()
        };
        let (dx, _) = dynamics.compute(0.0, &state, &vehicle);
        let alpha = Vec3::new(dx[10], dx[11], dx[12]);
        assert!(
            alpha.norm() > 1e-6,
            "expected a gyroscopic term, got {:?}",
            alpha
        );
    }

    #[test]
    fn quaternion_kinematics_match_a_constant_body_rate() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(0.0);
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);
        let state = RigidBodyState {
            attitude: Quaternion::identity(),
            angular_velocity: Vec3::new(0.0, 0.0, 1.0),
            mass: 1.0,
            ..Default::default()
        };
        let (dx, _) = dynamics.compute(0.0, &state, &vehicle);
        let qd = Quaternion::new(dx[6], dx[7], dx[8], dx[9]);
        let expected = Quaternion::identity().derivative_body_rate(Vec3::new(0.0, 0.0, 1.0));
        assert!((qd.w - expected.w).abs() < 1e-15);
        assert!((qd.z - expected.z).abs() < 1e-15);
        assert!((qd.z - 0.5).abs() < 1e-15);
    }

    #[test]
    fn three_dof_mode_holds_attitude_and_angular_rate() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::ThreeDof);
        let state = RigidBodyState {
            angular_velocity: Vec3::new(1.0, 2.0, 3.0),
            attitude: Quaternion::from_axis_angle(Vec3::y(), 0.4),
            mass: 1.0,
            ..Default::default()
        };
        let (dx, _) = dynamics.compute(0.0, &state, &vehicle);
        for (i, component) in dx.iter().enumerate().skip(6).take(7) {
            assert!(component.abs() < 1e-15, "component {} = {}", i, component);
        }
        // Translation still works.
        assert!((dx[5] + 9.81).abs() < 1e-9);
    }

    #[test]
    fn rail_constraint_removes_cross_rail_acceleration() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let mut env = Environment::vacuum_uniform_gravity(9.81);
        // A vertical rail.
        env.rail = crate::environment::LaunchRail::vertical(5.0, Vec3::z());
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);

        // Tilted vehicle so gravity has a cross-rail component without the rail.
        let state = RigidBodyState {
            attitude: Quaternion::from_axis_angle(Vec3::y(), 0.3),
            mass: 1.0,
            ..Default::default()
        };
        let (dx, diag) = dynamics.compute(0.0, &state, &vehicle);
        assert!(diag.on_rail);
        // Acceleration must be purely along +/-Z.
        assert!(dx[3].abs() < 1e-9, "ax = {}", dx[3]);
        assert!(dx[4].abs() < 1e-9, "ay = {}", dx[4]);
        assert!(
            dx[11].abs() < 1e-12,
            "angular rate must be frozen on the rail"
        );
    }

    #[test]
    fn rail_releases_after_the_configured_length() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let mut env = Environment::vacuum_uniform_gravity(9.81);
        env.rail = crate::environment::LaunchRail::vertical(1.0, Vec3::z());
        // A body-frame thrust along a tilted body X axis, so the cross-rail
        // component is visible once the constraint releases.
        let dynamics = dynamics_for(
            &vehicle,
            env,
            vec![Box::new(crate::forces::ThrustForce::new(
                crate::forces::ThrustProfile::constant(50.0, 10.0),
            ))],
            SimulationMode::SixDof,
        );

        let below = RigidBodyState {
            position: Vec3::new(0.1, 0.0, 0.5),
            attitude: Quaternion::from_axis_angle(Vec3::y(), 0.3),
            mass: 1.0,
            ..Default::default()
        };
        let (dx_below, diag_below) = dynamics.compute(0.0, &below, &vehicle);
        assert!(diag_below.on_rail);
        // On the rail all acceleration is along the rail, so neither world X nor
        // world Y accelerates.
        assert!(dx_below[3].abs() < 1e-9, "ax on rail = {}", dx_below[3]);
        assert!(dx_below[4].abs() < 1e-9, "ay on rail = {}", dx_below[4]);

        let above = RigidBodyState {
            position: Vec3::new(0.1, 0.0, 2.0),
            velocity: Vec3::new(0.0, 0.0, 10.0),
            attitude: Quaternion::from_axis_angle(Vec3::y(), 0.3),
            mass: 1.0,
            ..Default::default()
        };
        let (dx_above, diag_above) = dynamics.compute(0.0, &above, &vehicle);
        assert!(!diag_above.on_rail);
        // Off the rail the tilted thrust produces a world X acceleration.
        assert!(dx_above[3].abs() > 1e-6, "ax off rail = {}", dx_above[3]);
    }

    #[test]
    fn variable_mass_curve_drives_the_mass_derivative() {
        let curve = hex_core::MassCurve::new(vec![0.0, 10.0], vec![10.0, 5.0]);
        let mut vehicle = Vehicle::simple(1.0, 0.1);
        vehicle.mass = Box::new(crate::vehicle::CurveMass::new(curve));
        let env = Environment::vacuum_uniform_gravity(0.0);
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);
        let state = RigidBodyState::at_rest(10.0);
        let (dx, _) = dynamics.compute(0.0, &state, &vehicle);
        assert!((dx[13] + 0.5).abs() < 1e-12, "dm/dt = {}", dx[13]);
    }

    #[test]
    fn explicit_mass_flow_reduces_the_mass() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(0.0);
        let dynamics =
            RigidBodyDynamics::new(&vehicle, env, providers(vec![]), SimulationMode::SixDof)
                .with_mass_flow(MassFlowProfile::constant(0.25));
        let state = RigidBodyState::at_rest(1.0);
        let (dx, _) = dynamics.compute(0.0, &state, &vehicle);
        assert!(dx[13] != 0.0);
    }

    #[test]
    fn constant_mass_model_forces_a_zero_mass_derivative() {
        let vehicle = Vehicle::simple(3.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(0.0);
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);
        assert!(dynamics.constant_mass);
        let bound = BoundDynamics {
            dynamics: &dynamics,
            vehicle: &vehicle,
        };
        let state = RigidBodyState::at_rest(3.0);
        let mut dx = [0.0; STATE_LEN];
        bound.derivative(0.0, &state.to_array(), &mut dx);
        assert!(dx[13].abs() < 1e-15);
    }

    #[test]
    fn thrust_changes_the_world_force_with_attitude() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(0.0);
        let dynamics = dynamics_for(
            &vehicle,
            env,
            vec![Box::new(ThrustForce::new(ThrustProfile::constant(
                10.0, 1.0,
            )))],
            SimulationMode::SixDof,
        );

        // Pointing up: a 90 degree rotation about body Y maps body X to world Z.
        let state = RigidBodyState {
            attitude: Quaternion::from_axis_angle(Vec3::y(), -FRAC_PI_2),
            mass: 1.0,
            ..Default::default()
        };
        let (dx, diag) = dynamics.compute(0.0, &state, &vehicle);
        assert!(
            diag.force_world_vec().z > 9.0,
            "world force {:?}",
            diag.force_world_vec()
        );
        assert!((dx[5] - 10.0).abs() < 1e-9, "az = {}", dx[5]);
    }

    #[test]
    fn diagnostics_record_aero_quantities() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::default();
        let dynamics = dynamics_for(
            &vehicle,
            env,
            vec![Box::new(AeroForce::new(AeroModel::drag_only_estimate(0.4)))],
            SimulationMode::SixDof,
        );
        let state = RigidBodyState {
            velocity: Vec3::new(200.0, 0.0, -200.0),
            mass: 1.0,
            ..Default::default()
        };
        let (_, diag) = dynamics.compute(0.0, &state, &vehicle);
        assert!(diag.dynamic_pressure > 0.0);
        assert!(diag.air_density > 0.0);
        assert!(diag.mach > 0.0);
        assert!(diag.angle_of_attack.abs() > 0.0);
        assert!(diag.force_body_vec().norm() > 0.0);
    }

    #[test]
    fn observation_reports_altitude_and_rail_state() {
        let vehicle = Vehicle::simple(1.0, 0.1);
        let mut env = Environment::vacuum_uniform_gravity(9.81);
        env.rail = crate::environment::LaunchRail::vertical(2.0, Vec3::z());
        env.ground.elevation = 100.0;
        let ground_elevation = env.ground.elevation;
        let dynamics = dynamics_for(&vehicle, env, vec![], SimulationMode::SixDof);
        let state = RigidBodyState {
            position: Vec3::new(0.0, 0.0, 150.0),
            velocity: Vec3::new(0.0, 0.0, 20.0),
            mass: 1.0,
            ..Default::default()
        };
        let (_, diag) = dynamics.compute(0.0, &state, &vehicle);
        let obs = dynamics.observation(0.0, &state, &diag, 0.0, ground_elevation);
        assert!((obs.altitude - 50.0).abs() < 1e-9);
        assert!((obs.vertical_velocity - 20.0).abs() < 1e-9);
        assert!(
            !obs.on_rail,
            "rail distance {} should exceed 2 m",
            obs.rail_distance
        );
    }

    #[test]
    fn state_health_tracks_quaternion_drift() {
        let inertia = InertiaTensor::diagonal(1.0, 1.0, 1.0);
        let good = RigidBodyState::default();
        let health = StateHealth::measure(&good, &inertia, 9.81, Vec3::z(), 1e-9);
        assert!(!health.quaternion_out_of_tolerance);
        assert!(health.finite);

        let bad = RigidBodyState {
            attitude: Quaternion::new(1.01, 0.0, 0.0, 0.0),
            ..Default::default()
        };
        let health = StateHealth::measure(&bad, &inertia, 9.81, Vec3::z(), 1e-9);
        assert!(health.quaternion_out_of_tolerance);
    }

    #[test]
    fn state_health_reports_energy_and_momentum() {
        let inertia = InertiaTensor::diagonal(1.0, 2.0, 3.0);
        let state = RigidBodyState {
            position: Vec3::new(0.0, 0.0, 10.0),
            velocity: Vec3::new(3.0, 4.0, 0.0),
            angular_velocity: Vec3::new(1.0, 0.0, 0.0),
            mass: 2.0,
            ..Default::default()
        };
        let health = StateHealth::measure(&state, &inertia, 10.0, Vec3::z(), 1e-9);
        // 0.5 * 2 * 25 = 25 J translational, 0.5 * 1 * 1 = 0.5 J rotational.
        assert!((health.kinetic_energy - 25.5).abs() < 1e-9);
        // Potential: m g h = 2 * 10 * 10 = 200 J.
        assert!((health.total_energy - 225.5).abs() < 1e-9);
        assert!((health.linear_momentum - 10.0).abs() < 1e-9);
        assert!((health.angular_momentum - 1.0).abs() < 1e-9);
        assert!((health.maximum_angular_rate - 1.0).abs() < 1e-12);
    }
}
