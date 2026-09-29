//! Force models and the per-step force accumulation.
//!
//! Every provider returns a [`ForceContribution`] that names its source. The
//! accumulation is kept so the results workspace can show which forces and
//! moments dominated, which is usually the first question asked of a surprising
//! trajectory.

use hex_core::{InertiaTensor, Real, Vec3};
use serde::{Deserialize, Serialize};

use crate::environment::Environment;
use crate::state::RigidBodyState;
use crate::vehicle::VehicleModel;

/// One named force acting on the vehicle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ForceContribution {
    /// Source label shown in the results panel, for example `thrust`.
    pub source: ForceSource,
    /// Force expressed in the body frame, newtons.
    pub force_body: Vec3,
    /// Force expressed in the world frame, newtons.
    pub force_world: Vec3,
    /// Application point relative to the centre of gravity, body frame, metres.
    ///
    /// A force with a non-zero lever arm produces a moment.
    pub application_point_body: Vec3,
    /// Moment produced directly by this source, not via its lever arm, body
    /// frame, newton metres. Engines with a gimbal offset and aero coefficients
    /// with an explicit moment both use this.
    pub direct_moment_body: Vec3,
    /// Whether the contribution is active at this instant.
    pub active: bool,
}

impl ForceContribution {
    /// A force applied at the centre of gravity with no direct moment.
    pub fn at_center_of_gravity(
        source: ForceSource,
        force_body: Vec3,
        attitude: &hex_core::Quaternion,
    ) -> Self {
        Self {
            source,
            force_body,
            force_world: attitude.rotate(force_body),
            application_point_body: Vec3::zeros(),
            direct_moment_body: Vec3::zeros(),
            active: true,
        }
    }

    /// A force applied at an offset from the centre of gravity.
    pub fn at_point(
        source: ForceSource,
        force_body: Vec3,
        application_point_body: Vec3,
        attitude: &hex_core::Quaternion,
    ) -> Self {
        Self {
            source,
            force_body,
            force_world: attitude.rotate(force_body),
            application_point_body,
            direct_moment_body: Vec3::zeros(),
            active: true,
        }
    }

    /// A moment with no net force, for example a pure control torque.
    pub fn moment_only(source: ForceSource, moment_body: Vec3) -> Self {
        Self {
            source,
            force_body: Vec3::zeros(),
            force_world: Vec3::zeros(),
            application_point_body: Vec3::zeros(),
            direct_moment_body: moment_body,
            active: true,
        }
    }

    /// An inactive placeholder, so a provider with no effect still names itself.
    pub fn inactive(source: ForceSource) -> Self {
        Self {
            source,
            force_body: Vec3::zeros(),
            force_world: Vec3::zeros(),
            application_point_body: Vec3::zeros(),
            direct_moment_body: Vec3::zeros(),
            active: false,
        }
    }

    /// Moment produced by this force about the centre of gravity, body frame.
    pub fn moment_body(&self) -> Vec3 {
        self.application_point_body.cross(&self.force_body) + self.direct_moment_body
    }

    /// True when this contribution changes anything.
    pub fn has_effect(&self) -> bool {
        self.force_body.norm() > 1e-15 || self.direct_moment_body.norm() > 1e-15
    }
}

/// Where a force came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceSource {
    Gravity,
    Thrust,
    Drag,
    Lift,
    SideForce,
    AerodynamicMoment,
    Control,
    LandingBurn,
    External,
}

impl ForceSource {
    pub fn label(self) -> &'static str {
        match self {
            ForceSource::Gravity => "Gravity",
            ForceSource::Thrust => "Thrust",
            ForceSource::Drag => "Drag",
            ForceSource::Lift => "Lift",
            ForceSource::SideForce => "Side force",
            ForceSource::AerodynamicMoment => "Aerodynamic moment",
            ForceSource::Control => "Control",
            ForceSource::LandingBurn => "Landing burn",
            ForceSource::External => "External",
        }
    }
}

/// Totals for one evaluation of the dynamics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForceAccumulator {
    /// Every contribution that was evaluated, including inactive ones.
    pub contributions: Vec<ForceContribution>,
}

impl Default for ForceAccumulator {
    fn default() -> Self {
        Self {
            contributions: Vec::with_capacity(8),
        }
    }
}

impl ForceAccumulator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, c: ForceContribution) {
        self.contributions.push(c);
    }

    /// Sum of every contribution's force in the body frame.
    pub fn total_force_body(&self) -> Vec3 {
        self.contributions
            .iter()
            .filter(|c| c.active)
            .fold(Vec3::zeros(), |acc, c| acc + c.force_body)
    }

    /// Sum of every contribution's force in the world frame.
    pub fn total_force_world(&self) -> Vec3 {
        self.contributions
            .iter()
            .filter(|c| c.active)
            .fold(Vec3::zeros(), |acc, c| acc + c.force_world)
    }

    /// Sum of every contribution's moment about the centre of gravity.
    pub fn total_moment_body(&self) -> Vec3 {
        self.contributions
            .iter()
            .filter(|c| c.active)
            .fold(Vec3::zeros(), |acc, c| acc + c.moment_body())
    }

    /// Look up a source's contribution, if it was evaluated.
    pub fn get(&self, source: ForceSource) -> Option<&ForceContribution> {
        self.contributions.iter().find(|c| c.source == source)
    }

    /// Sources that actually changed the totals, for the results panel.
    pub fn active_sources(&self) -> Vec<ForceSource> {
        self.contributions
            .iter()
            .filter(|c| c.active && c.has_effect())
            .map(|c| c.source)
            .collect()
    }
}

/// The inputs a force provider reads.
///
/// Passing a context struct rather than a long argument list keeps providers
/// small and makes it obvious which environment values a model actually used.
pub struct ForceContext<'a> {
    /// Current simulation time, seconds.
    pub time: Real,
    /// Current rigid-body state.
    pub state: &'a RigidBodyState,
    /// The vehicle being simulated.
    pub vehicle: &'a dyn VehicleModel,
    /// Inertia at the current mass, body frame.
    pub inertia: InertiaTensor,
    /// Environment for this run.
    pub environment: &'a Environment,
    /// Unit up axis of the world frame.
    pub world_up: Vec3,
    /// Unit down axis of the world frame.
    pub world_down: Vec3,
    /// Air properties at the current position.
    pub air: crate::environment::AirProperties,
    /// Wind velocity in the world frame at the current position.
    pub wind_world: Vec3,
    /// Velocity relative to the air, world frame.
    pub relative_velocity_world: Vec3,
    /// Velocity relative to the air, body frame.
    pub relative_velocity_body: Vec3,
    /// Dynamic pressure in pascals.
    pub dynamic_pressure: Real,
    /// Mach number.
    pub mach: Real,
}

impl<'a> ForceContext<'a> {
    /// Build the context for one evaluation.
    pub fn new(
        time: Real,
        state: &'a RigidBodyState,
        vehicle: &'a dyn VehicleModel,
        environment: &'a Environment,
        world_up: Vec3,
        world_down: Vec3,
    ) -> Self {
        let inertia = vehicle.mass_provider().inertia(time);
        let air = environment.air_at(state.position, world_up);
        let wind_world = environment.wind_at(state.position, world_up);
        let relative_velocity_world = state.velocity - wind_world;
        let relative_velocity_body = state.attitude.rotate_inverse(relative_velocity_world);
        let speed = relative_velocity_world.norm();
        let dynamic_pressure = 0.5 * air.density * speed * speed;
        let mach = if air.speed_of_sound > 1e-6 {
            speed / air.speed_of_sound
        } else {
            0.0
        };
        Self {
            time,
            state,
            vehicle,
            inertia,
            environment,
            world_up,
            world_down,
            air,
            wind_world,
            relative_velocity_world,
            relative_velocity_body,
            dynamic_pressure,
            mach,
        }
    }

    /// Angle of attack in radians: the angle between the body X axis and the
    /// projection of the relative wind onto the body X-Z plane.
    pub fn angle_of_attack(&self) -> Real {
        let v = self.relative_velocity_body;
        let forward = v.x;
        let down = v.z;
        if forward.abs() < 1e-9 && down.abs() < 1e-9 {
            return 0.0;
        }
        down.atan2(forward)
    }

    /// Sideslip angle in radians: the angle between the body X axis and the
    /// projection of the relative wind onto the body X-Y plane.
    pub fn sideslip(&self) -> Real {
        let v = self.relative_velocity_body;
        let forward = v.x;
        let right = v.y;
        if forward.abs() < 1e-9 && right.abs() < 1e-9 {
            return 0.0;
        }
        right.atan2(forward)
    }

    /// Total airspeed relative to the vehicle, m/s.
    pub fn airspeed(&self) -> Real {
        self.relative_velocity_world.norm()
    }
}

/// A source of forces and moments.
pub trait ForceProvider: Send + Sync {
    /// Stable name used in the configuration and the results panel.
    fn name(&self) -> &str;

    /// Evaluate this provider, pushing zero or more contributions.
    fn evaluate(&self, ctx: &ForceContext, out: &mut ForceAccumulator);

    /// Whether this provider can change the outcome at all.
    fn is_active(&self) -> bool {
        true
    }

    /// One-line description for the run metadata.
    fn describe(&self) -> String {
        self.name().to_string()
    }
}

/// Gravity, always evaluated in the world frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GravityForce;

impl ForceProvider for GravityForce {
    fn name(&self) -> &str {
        "gravity"
    }

    fn evaluate(&self, ctx: &ForceContext, out: &mut ForceAccumulator) {
        let acceleration = ctx
            .environment
            .gravity
            .acceleration(ctx.state.position, ctx.world_down);
        let force_world = acceleration * ctx.state.mass;
        // Gravity acts at the centre of gravity, so it contributes no moment.
        out.push(ForceContribution {
            source: ForceSource::Gravity,
            force_body: ctx.state.attitude.rotate_inverse(force_world),
            force_world,
            application_point_body: Vec3::zeros(),
            direct_moment_body: Vec3::zeros(),
            active: true,
        });
    }

    fn describe(&self) -> String {
        "Gravity, applied at the centre of gravity".to_string()
    }
}

/// A thrust source defined by a time profile and a body-frame direction.
///
/// The profile is sampled and held at its end values, with one exception: thrust
/// is exactly zero at and beyond the final sample time. That makes the final
/// sample the documented cutoff, so a profile that ends at burnout does not
/// report thrust at the burnout instant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThrustProfile {
    /// Sample times in seconds, strictly increasing.
    pub times: Vec<Real>,
    /// Thrust magnitude in newtons at each sample time.
    pub thrusts: Vec<Real>,
    /// Thrust direction in the body frame. Normalised when used.
    pub direction: Vec3,
    /// Application point relative to the centre of gravity, body frame, metres.
    pub application_point: Vec3,
    /// Constant misalignment rotation applied to the thrust direction.
    pub misalignment: Option<hex_core::Quaternion>,
    /// Optional gimbal deflection about body Y, radians. Positive tilts the
    /// thrust toward body Z.
    pub gimbal_y: Option<Real>,
    /// Optional gimbal deflection about body Z, radians.
    pub gimbal_z: Option<Real>,
    /// Engine moment not produced by the thrust line, body frame, N m.
    pub engine_moment: Vec3,
    /// Relative standard uncertainty on the thrust magnitude, for reporting.
    pub relative_uncertainty: Real,
}

impl Default for ThrustProfile {
    fn default() -> Self {
        Self {
            times: vec![0.0],
            thrusts: vec![0.0],
            direction: Vec3::x(),
            application_point: Vec3::zeros(),
            misalignment: None,
            gimbal_y: None,
            gimbal_z: None,
            engine_moment: Vec3::zeros(),
            relative_uncertainty: 0.0,
        }
    }
}

impl ThrustProfile {
    /// A constant thrust along body X starting at `ignition_time`.
    pub fn constant(magnitude: Real, duration: Real) -> Self {
        Self {
            times: vec![0.0, duration, duration + 1e-9],
            thrusts: vec![magnitude, magnitude, 0.0],
            direction: Vec3::x(),
            ..Default::default()
        }
    }

    /// A linearly tapering thrust profile, as a rough motors approximation.
    pub fn tapering(initial: Real, final_thrust: Real, duration: Real) -> Self {
        Self {
            times: vec![0.0, duration, duration + 1e-9],
            thrusts: vec![initial, final_thrust, 0.0],
            direction: Vec3::x(),
            ..Default::default()
        }
    }

    /// Interpolate the thrust magnitude at time `t`, zero outside the profile.
    pub fn thrust_at(&self, t: Real) -> Real {
        let n = self.times.len();
        if n == 0 || n != self.thrusts.len() {
            return 0.0;
        }
        if n == 1 {
            return if t >= self.times[0] {
                self.thrusts[0]
            } else {
                0.0
            };
        }
        if t < self.times[0] || t > self.times[n - 1] {
            return 0.0;
        }
        // The final sample marks the end of the profile, so thrust is exactly
        // zero at and beyond it. Without this rule a profile that steps straight
        // from full thrust to zero would still report thrust at burnout.
        if t >= self.times[n - 1] {
            return 0.0;
        }
        let idx = match self.times.partition_point(|x| *x <= t) {
            0 => 0,
            i if i >= n => n - 2,
            i => i - 1,
        };
        let span = self.times[idx + 1] - self.times[idx];
        if span <= 0.0 {
            return self.thrusts[idx];
        }
        let f = (t - self.times[idx]) / span;
        (self.thrusts[idx] * (1.0 - f) + self.thrusts[idx + 1] * f).max(0.0)
    }

    /// Time of the last non-zero thrust sample, used for burnout events.
    pub fn burnout_time(&self) -> Option<Real> {
        self.times
            .iter()
            .zip(self.thrusts.iter())
            .filter(|(_, th)| **th > 0.0)
            .map(|(t, _)| *t)
            .next_back()
    }

    /// Time of the first non-zero thrust sample, used for ignition events.
    pub fn ignition_time(&self) -> Option<Real> {
        self.times
            .iter()
            .zip(self.thrusts.iter())
            .find(|(_, th)| **th > 0.0)
            .map(|(t, _)| *t)
    }

    /// The thrust direction after misalignment and gimbal deflections.
    pub fn effective_direction(&self) -> Vec3 {
        let mut d = if self.direction.norm() > 1e-12 {
            hex_core::normalized(self.direction)
        } else {
            Vec3::x()
        };
        if let Some(q) = self.misalignment {
            d = q.rotate(d);
        }
        if let Some(gz) = self.gimbal_z {
            d = hex_core::Quaternion::from_axis_angle(Vec3::z(), gz).rotate(d);
        }
        if let Some(gy) = self.gimbal_y {
            d = hex_core::Quaternion::from_axis_angle(Vec3::y(), gy).rotate(d);
        }
        if d.norm() > 1e-12 {
            hex_core::normalized(d)
        } else {
            Vec3::x()
        }
    }

    /// Whether the profile can produce thrust at all.
    pub fn is_active(&self) -> bool {
        self.thrusts.iter().any(|t| *t > 0.0)
    }
}

/// Applies a [`ThrustProfile`], or the vehicle's own application point when the
/// profile does not specify one.
#[derive(Debug, Clone, PartialEq)]
pub struct ThrustForce {
    pub profile: ThrustProfile,
    /// Use the vehicle model's thrust application point instead of the profile's.
    pub use_vehicle_application_point: bool,
}

impl ThrustForce {
    pub fn new(profile: ThrustProfile) -> Self {
        Self {
            profile,
            use_vehicle_application_point: false,
        }
    }

    pub fn at_vehicle_point(profile: ThrustProfile) -> Self {
        Self {
            profile,
            use_vehicle_application_point: true,
        }
    }
}

impl ForceProvider for ThrustForce {
    fn name(&self) -> &str {
        "thrust"
    }

    fn evaluate(&self, ctx: &ForceContext, out: &mut ForceAccumulator) {
        let magnitude = self.profile.thrust_at(ctx.time);
        if magnitude <= 0.0 {
            out.push(ForceContribution::inactive(ForceSource::Thrust));
            return;
        }
        let direction = self.profile.effective_direction();
        let force_body = direction * magnitude;
        let application_point = if self.use_vehicle_application_point {
            ctx.vehicle.thrust_application_point()
        } else {
            self.profile.application_point
        };
        out.push(ForceContribution {
            source: ForceSource::Thrust,
            force_body,
            force_world: ctx.state.attitude.rotate(force_body),
            application_point_body: application_point,
            direct_moment_body: self.profile.engine_moment,
            active: true,
        });
    }

    fn is_active(&self) -> bool {
        self.profile.is_active()
    }

    fn describe(&self) -> String {
        match self.profile.burnout_time() {
            Some(t) => format!(
                "Thrust profile, {} samples, burnout at {:.3} s",
                self.profile.times.len(),
                t
            ),
            None => "Thrust profile, no thrust defined".to_string(),
        }
    }
}

/// A landing burn that ignites on the stopping-distance condition.
///
/// This is the classic hover-slam trigger: the engine lights as late as it can
/// and still bring the vehicle to rest at the ground, so the landing is gentle
/// without any throttle modulation. The condition is checked against the state
/// on every evaluation:
///
/// * the vehicle is descending faster than [`Self::minimum_descent_rate`],
/// * it is below [`Self::maximum_ignition_altitude`], and
/// * its height above the ground is no more than the distance it needs to stop
///   at its own thrust-to-weight, less [`Self::deceleration_margin`].
///
/// It is a trigger, not a guidance law. Nothing steers: the vehicle has to
/// already be upright and over the point it means to land on, and the burn is a
/// single on or off engine, so a vehicle that over-brakes cuts the burn and
/// falls again rather than hovering on a throttle. Drag is not in the stopping
/// distance, which biases the ignition slightly early, and a slightly early
/// burn is the safe direction to be wrong in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandingBurn {
    /// Thrust magnitude while the engine is lit, newtons.
    pub thrust: Real,
    /// Thrust direction in the body frame. Normalised when used.
    pub direction: Vec3,
    /// Application point relative to the centre of gravity, body frame, metres.
    pub application_point: Vec3,
    /// Deceleration held back from the stopping distance, metres per second
    /// squared. A larger margin lights the engine earlier and higher.
    pub deceleration_margin: Real,
    /// The burn is refused above this height, metres. It exists so a misconfigured
    /// trigger cannot fire a landing burn on the way up.
    pub maximum_ignition_altitude: Real,
    /// Descent rate below which the burn is not lit, metres per second. Positive
    /// numbers mean falling.
    pub minimum_descent_rate: Real,
}

impl Default for LandingBurn {
    fn default() -> Self {
        Self {
            thrust: 0.0,
            direction: Vec3::x(),
            application_point: Vec3::zeros(),
            deceleration_margin: 1.0,
            maximum_ignition_altitude: 500.0,
            minimum_descent_rate: 0.2,
        }
    }
}

impl LandingBurn {
    /// A landing burn of the given magnitude along body X.
    pub fn constant(thrust: Real) -> Self {
        Self {
            thrust,
            ..Self::default()
        }
    }

    /// Height above the ground for a world-frame position.
    pub fn height_above_ground(position: &Vec3, world_up: &Vec3, ground_elevation: Real) -> Real {
        position.dot(world_up) + ground_elevation
    }

    /// Whether the burn would be lit in this state.
    ///
    /// Public so a recorded history can be scanned for the ignition time without
    /// running the solver again.
    #[allow(clippy::too_many_arguments)]
    pub fn fires_at(
        &self,
        position: &Vec3,
        velocity: &Vec3,
        mass: Real,
        world_up: &Vec3,
        ground_elevation: Real,
        gravity: Real,
    ) -> bool {
        if self.thrust <= 0.0 || mass <= 0.0 || !gravity.is_finite() {
            return false;
        }
        let height = Self::height_above_ground(position, world_up, ground_elevation);
        if height > self.maximum_ignition_altitude {
            return false;
        }
        // Positive descent rate means falling.
        let descent = -velocity.dot(world_up);
        if descent < self.minimum_descent_rate {
            return false;
        }
        // Net upward acceleration available while the engine is lit.
        let deceleration = self.thrust / mass - gravity - self.deceleration_margin;
        if deceleration <= 1e-6 {
            // The engine cannot even hold the vehicle up, so there is no
            // stopping distance to compute and no point lighting it.
            return false;
        }
        let stopping_distance = descent * descent / (2.0 * deceleration);
        // A vehicle already at or below the ground is not a landing burn.
        height <= stopping_distance && height > 0.0
    }

    /// Whether the burn can ever stop this vehicle, for validation.
    pub fn can_stop(&self, mass: Real, gravity: Real) -> bool {
        self.thrust / mass.max(1e-9) - gravity - self.deceleration_margin > 1e-6
    }
}

impl ForceProvider for LandingBurn {
    fn name(&self) -> &str {
        "landing_burn"
    }

    fn evaluate(&self, ctx: &ForceContext, out: &mut ForceAccumulator) {
        let mass = ctx.vehicle.mass_provider().mass(ctx.time);
        let gravity = ctx
            .environment
            .gravity
            .acceleration(ctx.state.position, ctx.world_down)
            .norm();
        if !self.fires_at(
            &ctx.state.position,
            &ctx.state.velocity,
            mass,
            &ctx.world_up,
            ctx.environment.ground.elevation,
            gravity,
        ) {
            out.push(ForceContribution::inactive(ForceSource::LandingBurn));
            return;
        }
        let direction = if self.direction.norm() > 1e-12 {
            hex_core::normalized(self.direction)
        } else {
            Vec3::x()
        };
        let force_body = direction * self.thrust;
        out.push(ForceContribution {
            source: ForceSource::LandingBurn,
            force_body,
            force_world: ctx.state.attitude.rotate(force_body),
            application_point_body: self.application_point,
            direct_moment_body: Vec3::zeros(),
            active: true,
        });
    }

    fn is_active(&self) -> bool {
        self.thrust > 0.0
    }

    fn describe(&self) -> String {
        format!(
            "Landing burn, {:.1} N, lit on the stopping-distance condition",
            self.thrust
        )
    }
}

/// Aerodynamic coefficients as functions of Mach, angle of attack, and sideslip.
///
/// The MVP uses a table of coefficient samples with linear interpolation and a
/// documented zero-angle fallback. It is explicitly an approximation; the UI
/// labels any run that uses it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AeroCoefficientTable {
    /// Mach breakpoints, strictly increasing.
    pub mach: Vec<Real>,
    /// Coefficient values at each Mach breakpoint.
    pub values: Vec<Real>,
    /// Coefficient at zero Mach when the table is empty.
    pub fallback: Real,
}

impl AeroCoefficientTable {
    pub fn constant(value: Real) -> Self {
        Self {
            mach: vec![0.0],
            values: vec![value],
            fallback: value,
        }
    }

    pub fn from_samples(mach: Vec<Real>, values: Vec<Real>) -> Self {
        let fallback = values.first().copied().unwrap_or(0.0);
        Self {
            mach,
            values,
            fallback,
        }
    }

    /// Interpolate the coefficient at a Mach number.
    pub fn at_mach(&self, mach: Real) -> Real {
        let n = self.mach.len();
        if n == 0 || n != self.values.len() {
            return self.fallback;
        }
        if n == 1 {
            return self.values[0];
        }
        if !mach.is_finite() {
            return self.fallback;
        }
        if mach <= self.mach[0] {
            return self.values[0];
        }
        if mach >= self.mach[n - 1] {
            return self.values[n - 1];
        }
        let idx = match self.mach.partition_point(|m| *m <= mach) {
            0 => 0,
            i if i >= n => n - 2,
            i => i - 1,
        };
        let span = self.mach[idx + 1] - self.mach[idx];
        if span <= 0.0 {
            return self.values[idx];
        }
        let f = (mach - self.mach[idx]) / span;
        self.values[idx] * (1.0 - f) + self.values[idx + 1] * f
    }

    pub fn is_usable(&self) -> bool {
        !self.mach.is_empty() && self.mach.len() == self.values.len()
    }
}

/// Where a set of aerodynamic coefficients came from.
///
/// A run has to say this, because a built-in estimate and coefficients the
/// vehicle designer measured are not the same evidence, even when they agree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AeroSource {
    /// A built-in estimate, chosen when the model declares nothing.
    #[default]
    Estimate,
    /// Coefficients declared by the imported dynamics model.
    ImportedModel,
}

impl AeroSource {
    pub fn label(self) -> &'static str {
        match self {
            AeroSource::Estimate => "built-in estimate",
            AeroSource::ImportedModel => "imported model",
        }
    }

    /// A phrase that fits a sentence about where the numbers came from.
    pub fn phrase(self) -> &'static str {
        match self {
            AeroSource::Estimate => "a built-in estimate",
            AeroSource::ImportedModel => "the imported dynamics model",
        }
    }
}

/// The dimensionless derivatives that make up a coefficient aerodynamic model.
///
/// This is the shape an imported model supplies, kept separate from
/// [`AeroModel`] so a caller does not have to name every field to build one.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AeroDerivatives {
    /// Lift-curve slope per radian of angle of attack.
    pub lift_slope: Real,
    /// Lift coefficient at zero angle of attack.
    pub lift_zero: Real,
    /// Side-force slope per radian of sideslip.
    pub side_slope: Real,
    /// Pitching moment coefficient at zero angle of attack.
    pub pitch_zero: Real,
    /// Pitching moment slope per radian of angle of attack. Negative is stable.
    pub pitch_slope: Real,
    /// Yawing moment slope per radian of sideslip.
    pub yaw_slope: Real,
    /// Rolling moment slope per radian of sideslip.
    pub roll_slope: Real,
    /// Pitch damping with body pitch rate, per radian per second.
    pub pitch_damping: Real,
    /// Yaw damping with body yaw rate, per radian per second.
    pub yaw_damping: Real,
    /// Roll damping with body roll rate, per radian per second.
    pub roll_damping: Real,
}

impl AeroDerivatives {
    /// Whether any derivative can produce a force or moment other than drag.
    ///
    /// A model can declare `drag_only: false` and still carry zeros everywhere,
    /// which computes exactly the drag-only motion. Reporting that as a full
    /// model would overstate what was actually computed.
    pub fn has_non_drag_terms(&self) -> bool {
        [
            self.lift_slope,
            self.lift_zero,
            self.side_slope,
            self.pitch_zero,
            self.pitch_slope,
            self.yaw_slope,
            self.roll_slope,
            self.pitch_damping,
            self.yaw_damping,
            self.roll_damping,
        ]
        .iter()
        .any(|term| term.abs() > 1e-12)
    }
}

/// A modular aerodynamic model built from coefficient tables.
///
/// Force conventions, all in the body frame with X forward, Y right, Z down:
///
/// * Drag acts along the negative relative-wind direction.
/// * Lift acts perpendicular to the relative wind in the body X-Z plane.
/// * Side force acts along body Y.
/// * Moments use the standard roll, pitch, yaw coefficient form scaled by
///   `q_dyn * S * L`.
///
/// This is a coefficient model, not a CFD solution. The UI must present it as a
/// simplification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AeroModel {
    /// Drag coefficient, tabulated against Mach.
    pub drag: AeroCoefficientTable,
    /// Lift coefficient slope per radian of angle of attack.
    pub lift_slope: Real,
    /// Lift coefficient at zero angle of attack.
    pub lift_zero: Real,
    /// Side-force slope per radian of sideslip.
    pub side_slope: Real,
    /// Rolling moment coefficient derivative with sideslip, per radian.
    pub roll_damping: Real,
    /// Pitching moment coefficient at zero angle of attack.
    pub pitch_zero: Real,
    /// Pitching moment slope per radian of angle of attack. Negative is stable.
    pub pitch_slope: Real,
    /// Yawing moment coefficient derivative with sideslip, per radian.
    pub yaw_slope: Real,
    /// Pitch damping derivative with body pitch rate, per radian per second.
    pub pitch_damping: Real,
    /// Yaw damping derivative with body yaw rate, per radian per second.
    pub yaw_damping: Real,
    /// Roll damping derivative with body roll rate, per radian per second.
    pub roll_damping_rate: Real,
    /// Centre of pressure offset behind the centre of gravity, metres.
    ///
    /// Used only for the documented simplified drag moment; the pitching moment
    /// coefficients already account for the aerodynamic centre.
    pub center_of_pressure_offset: Real,
    /// Whether the model ignores angle of attack and uses drag only.
    pub drag_only: bool,
    /// Relative uncertainty on the coefficients, for reporting.
    pub relative_uncertainty: Real,
    /// Where the coefficients came from.
    #[serde(default)]
    pub source: AeroSource,
}

impl Default for AeroModel {
    fn default() -> Self {
        Self::drag_only_estimate(0.45)
    }
}

impl AeroModel {
    /// A drag-only model with a constant drag coefficient. This is the safe
    /// default because it makes the fewest assumptions.
    pub fn drag_only_estimate(drag_coefficient: Real) -> Self {
        Self {
            drag: AeroCoefficientTable::constant(drag_coefficient),
            lift_slope: 0.0,
            lift_zero: 0.0,
            side_slope: 0.0,
            roll_damping: 0.0,
            pitch_zero: 0.0,
            pitch_slope: 0.0,
            yaw_slope: 0.0,
            pitch_damping: 0.0,
            yaw_damping: 0.0,
            roll_damping_rate: 0.0,
            center_of_pressure_offset: 0.0,
            drag_only: true,
            relative_uncertainty: 0.2,
            source: AeroSource::Estimate,
        }
    }

    /// An estimate that adds lift and moments so a scenario can show what those
    /// terms do. The derivatives are conventions, not measurements.
    pub fn estimate_with_lift(drag_coefficient: Real, derivatives: AeroDerivatives) -> Self {
        Self {
            drag: AeroCoefficientTable::constant(drag_coefficient),
            lift_slope: derivatives.lift_slope,
            lift_zero: derivatives.lift_zero,
            side_slope: derivatives.side_slope,
            roll_damping: derivatives.roll_slope,
            pitch_zero: derivatives.pitch_zero,
            pitch_slope: derivatives.pitch_slope,
            yaw_slope: derivatives.yaw_slope,
            pitch_damping: derivatives.pitch_damping,
            yaw_damping: derivatives.yaw_damping,
            roll_damping_rate: derivatives.roll_damping,
            center_of_pressure_offset: 0.0,
            drag_only: !derivatives.has_non_drag_terms(),
            relative_uncertainty: 0.35,
            source: AeroSource::Estimate,
        }
    }

    /// Build from a Mach-indexed drag table and the derivatives an imported
    /// model supplies.
    ///
    /// `declared_drag_only` is what the model itself claims. The result is
    /// drag-only when that is set, or when every derivative is zero, because a
    /// model that carries no lift or moment term computes exactly the drag-only
    /// motion whatever it calls itself.
    pub fn from_table(
        drag: AeroCoefficientTable,
        derivatives: AeroDerivatives,
        declared_drag_only: bool,
        uncertainty: Real,
    ) -> Self {
        Self {
            drag,
            lift_slope: derivatives.lift_slope,
            lift_zero: derivatives.lift_zero,
            side_slope: derivatives.side_slope,
            roll_damping: derivatives.roll_slope,
            pitch_zero: derivatives.pitch_zero,
            pitch_slope: derivatives.pitch_slope,
            yaw_slope: derivatives.yaw_slope,
            pitch_damping: derivatives.pitch_damping,
            yaw_damping: derivatives.yaw_damping,
            roll_damping_rate: derivatives.roll_damping,
            center_of_pressure_offset: 0.0,
            drag_only: declared_drag_only || !derivatives.has_non_drag_terms(),
            relative_uncertainty: uncertainty.clamp(0.0, 1.0),
            source: AeroSource::ImportedModel,
        }
    }

    /// True when the model has any non-drag term.
    pub fn has_lift_or_moments(&self) -> bool {
        !self.drag_only
            && (self.lift_slope.abs() > 1e-12
                || self.side_slope.abs() > 1e-12
                || self.pitch_slope.abs() > 1e-12
                || self.yaw_slope.abs() > 1e-12)
    }

    /// Whether the drag coefficient varies with Mach.
    pub fn has_mach_dependent_drag(&self) -> bool {
        self.drag.mach.len() > 1
    }

    /// A label that states clearly how simplified the model is.
    pub fn fidelity_label(&self) -> &'static str {
        match (self.drag_only, self.has_mach_dependent_drag()) {
            (true, false) => "Simplified: drag only, constant coefficient",
            (true, true) => "Simplified: drag only, tabulated against Mach",
            (false, false) => {
                "Simplified: coefficient model with lift and moments, constant drag coefficient"
            }
            (false, true) => {
                "Simplified: coefficient model with lift and moments, drag tabulated against Mach"
            }
        }
    }
}

/// Applies an [`AeroModel`] using the vehicle's reference geometry.
#[derive(Debug, Clone, PartialEq)]
pub struct AeroForce {
    pub model: AeroModel,
}

impl AeroForce {
    pub fn new(model: AeroModel) -> Self {
        Self { model }
    }
}

impl ForceProvider for AeroForce {
    fn name(&self) -> &str {
        "aerodynamics"
    }

    fn evaluate(&self, ctx: &ForceContext, out: &mut ForceAccumulator) {
        let geometry = ctx.vehicle.reference_geometry();
        let s_ref = geometry.reference_area;
        let l_ref = geometry.reference_length;

        let speed = ctx.relative_velocity_world.norm();
        if ctx.air.density <= 0.0 || speed < 1e-9 || s_ref <= 0.0 {
            // A single inactive entry names this provider without inflating the
            // contribution list with sources the model cannot produce.
            out.push(ForceContribution::inactive(ForceSource::Drag));
            return;
        }

        let q_dyn = ctx.dynamic_pressure;
        let v_body = ctx.relative_velocity_body;
        let speed_body = v_body.norm();
        if speed_body < 1e-12 {
            out.push(ForceContribution::inactive(ForceSource::Drag));
            return;
        }
        let wind_dir_body = v_body / speed_body;

        let cd = self.model.drag.at_mach(ctx.mach);
        let drag_body = wind_dir_body * (-q_dyn * s_ref * cd);
        out.push(ForceContribution::at_center_of_gravity(
            ForceSource::Drag,
            drag_body,
            &ctx.state.attitude,
        ));

        if self.model.drag_only {
            return;
        }

        let alpha = ctx.angle_of_attack();
        let beta = ctx.sideslip();
        let q_over_v = if speed > 1e-9 { q_dyn / speed } else { 0.0 };

        // Lift acts perpendicular to the relative wind, in the plane containing
        // body X and the wind, rotating the wind direction toward body X.
        let cl = self.model.lift_zero + self.model.lift_slope * alpha;
        let lift_dir = {
            let plane_normal = wind_dir_body.cross(&Vec3::y());
            let n = plane_normal.norm();
            if n < 1e-9 {
                Vec3::zeros()
            } else {
                // Choose the sign so positive alpha gives positive lift along -Z.
                let candidate = hex_core::normalized(plane_normal);
                if candidate.z > 0.0 {
                    -candidate
                } else {
                    candidate
                }
            }
        };
        let lift_body = lift_dir * (q_dyn * s_ref * cl);
        out.push(ForceContribution::at_center_of_gravity(
            ForceSource::Lift,
            lift_body,
            &ctx.state.attitude,
        ));

        let cy = self.model.side_slope * beta;
        let side_body = Vec3::y() * (q_dyn * s_ref * cy);
        out.push(ForceContribution::at_center_of_gravity(
            ForceSource::SideForce,
            side_body,
            &ctx.state.attitude,
        ));

        // Moments. Rates are non-dimensionalised by 2 * V, which is the standard
        // coefficient convention.
        let omega = ctx.state.angular_velocity;
        let pitch_rate_term = if speed > 1e-9 {
            self.model.pitch_damping * omega.y * l_ref / (2.0 * speed)
        } else {
            0.0
        };
        let yaw_rate_term = if speed > 1e-9 {
            self.model.yaw_damping * omega.z * l_ref / (2.0 * speed)
        } else {
            0.0
        };
        let roll_rate_term = if speed > 1e-9 {
            self.model.roll_damping_rate * omega.x * l_ref / (2.0 * speed)
        } else {
            0.0
        };

        let cl_moment = self.model.roll_damping * beta + roll_rate_term;
        let cm_moment = self.model.pitch_zero + self.model.pitch_slope * alpha + pitch_rate_term;
        let cn_moment = self.model.yaw_slope * beta + yaw_rate_term;

        let scale = q_dyn * s_ref * l_ref;
        let moment_body = Vec3::new(cl_moment * scale, cm_moment * scale, cn_moment * scale);
        let _ = q_over_v;

        out.push(ForceContribution::moment_only(
            ForceSource::AerodynamicMoment,
            moment_body,
        ));
    }

    fn describe(&self) -> String {
        format!(
            "{} (Cd table with {} entries, reference length used for moments)",
            self.model.fidelity_label(),
            self.model.drag.values.len()
        )
    }
}

/// A control or thrust-vector moment defined by an explicit body-frame torque
/// profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlMoment {
    /// Sample times in seconds, strictly increasing.
    pub times: Vec<Real>,
    /// Moment vectors in the body frame, newton metres.
    pub moments: Vec<Vec3>,
    /// Whether the control is enabled for this run.
    pub enabled: bool,
}

impl Default for ControlMoment {
    fn default() -> Self {
        Self {
            times: vec![0.0],
            moments: vec![Vec3::zeros()],
            enabled: false,
        }
    }
}

impl ControlMoment {
    /// A constant body-frame moment active over an interval.
    pub fn constant(moment: Vec3, duration: Real) -> Self {
        Self {
            times: vec![0.0, duration, duration + 1e-9],
            moments: vec![moment, moment, Vec3::zeros()],
            enabled: true,
        }
    }

    /// Interpolate the moment at time `t`, zero outside the profile.
    pub fn moment_at(&self, t: Real) -> Vec3 {
        let n = self.times.len();
        if n == 0 || n != self.moments.len() {
            return Vec3::zeros();
        }
        if n == 1 {
            return if t >= self.times[0] {
                self.moments[0]
            } else {
                Vec3::zeros()
            };
        }
        if t < self.times[0] || t > self.times[n - 1] {
            return Vec3::zeros();
        }
        let idx = match self.times.partition_point(|x| *x <= t) {
            0 => 0,
            i if i >= n => n - 2,
            i => i - 1,
        };
        let span = self.times[idx + 1] - self.times[idx];
        if span <= 0.0 {
            return self.moments[idx];
        }
        let f = (t - self.times[idx]) / span;
        self.moments[idx] * (1.0 - f) + self.moments[idx + 1] * f
    }
}

/// A provider that applies a [`ControlMoment`] profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlForce {
    pub profile: ControlMoment,
}

impl ForceProvider for ControlForce {
    fn name(&self) -> &str {
        "control"
    }

    fn evaluate(&self, ctx: &ForceContext, out: &mut ForceAccumulator) {
        if !self.profile.enabled {
            out.push(ForceContribution::inactive(ForceSource::Control));
            return;
        }
        let m = self.profile.moment_at(ctx.time);
        if m.norm() < 1e-15 {
            out.push(ForceContribution::inactive(ForceSource::Control));
            return;
        }
        out.push(ForceContribution::moment_only(ForceSource::Control, m));
    }

    fn is_active(&self) -> bool {
        self.profile.enabled
    }

    fn describe(&self) -> String {
        if self.profile.enabled {
            format!(
                "Control moment profile, {} samples",
                self.profile.times.len()
            )
        } else {
            "Control moment disabled".to_string()
        }
    }
}

/// A constant external force and moment, for parametric studies.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExternalLoad {
    /// Force in the body frame, newtons.
    pub force_body: Vec3,
    /// Moment in the body frame, newton metres.
    pub moment_body: Vec3,
    /// Application point relative to the centre of gravity, body frame, metres.
    pub application_point_body: Vec3,
    /// Whether the load is applied.
    pub enabled: bool,
}

impl Default for ExternalLoad {
    fn default() -> Self {
        Self {
            force_body: Vec3::zeros(),
            moment_body: Vec3::zeros(),
            application_point_body: Vec3::zeros(),
            enabled: false,
        }
    }
}

impl ForceProvider for ExternalLoad {
    fn name(&self) -> &str {
        "external"
    }

    fn evaluate(&self, ctx: &ForceContext, out: &mut ForceAccumulator) {
        if !self.enabled {
            out.push(ForceContribution::inactive(ForceSource::External));
            return;
        }
        out.push(ForceContribution {
            source: ForceSource::External,
            force_body: self.force_body,
            force_world: ctx.state.attitude.rotate(self.force_body),
            application_point_body: self.application_point_body,
            direct_moment_body: self.moment_body,
            active: true,
        });
    }

    fn is_active(&self) -> bool {
        self.enabled
    }
}

/// Evaluate every provider in a chain, producing the totals for one step.
pub fn accumulate(
    providers: &crate::equations::ProviderChain,
    ctx: &ForceContext,
) -> ForceAccumulator {
    let mut acc = ForceAccumulator::new();
    for p in providers.iter() {
        p.evaluate(ctx, &mut acc);
    }
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::{Environment, WindModel};
    use crate::state::RigidBodyState;
    use crate::vehicle::Vehicle;
    use hex_core::Quaternion;

    fn context<'a>(
        state: &'a RigidBodyState,
        vehicle: &'a dyn VehicleModel,
        env: &'a Environment,
    ) -> ForceContext<'a> {
        ForceContext::new(0.0, state, vehicle, env, Vec3::z(), -Vec3::z())
    }

    /* ------------------------------------------------------- landing burn */

    /// A 1 tonne vehicle falling at 40 m/s, 100 m up, with a 30 kN engine.
    ///
    /// Stopping needs 400 / (2 * (30 - 9.81)) = 9.9 m of altitude, so a 100 m
    /// fall must not light the engine and an 8 m fall must.
    #[test]
    fn the_landing_burn_lights_only_inside_the_stopping_distance() {
        let burn = LandingBurn {
            thrust: 30_000.0,
            deceleration_margin: 0.0,
            ..LandingBurn::default()
        };
        let up = Vec3::z();

        let high = Vec3::new(0.0, 0.0, 100.0);
        let falling = Vec3::new(0.0, 0.0, -40.0);
        assert!(!burn.fires_at(&high, &falling, 1000.0, &up, 0.0, 9.81));

        let low = Vec3::new(0.0, 0.0, 8.0);
        assert!(burn.fires_at(&low, &falling, 1000.0, &up, 0.0, 9.81));
    }

    #[test]
    fn the_landing_burn_does_not_light_while_climbing_or_hovering() {
        let burn = LandingBurn {
            thrust: 30_000.0,
            deceleration_margin: 0.0,
            ..LandingBurn::default()
        };
        let up = Vec3::z();
        let low = Vec3::new(0.0, 0.0, 5.0);

        // Climbing: the vehicle is going the right way already.
        assert!(!burn.fires_at(&low, &Vec3::new(0.0, 0.0, 2.0), 1000.0, &up, 0.0, 9.81));
        // Hovering: not descending, so there is nothing to arrest.
        assert!(!burn.fires_at(&low, &Vec3::zeros(), 1000.0, &up, 0.0, 9.81));
        // On the ground: too late, and not a landing burn.
        assert!(!burn.fires_at(
            &Vec3::zeros(),
            &Vec3::new(0.0, 0.0, -1.0),
            1000.0,
            &up,
            0.0,
            9.81
        ));
    }

    #[test]
    fn a_landing_burn_that_cannot_beat_gravity_never_lights() {
        // 8 kN on a tonne is less than weight, so no stopping distance exists.
        let burn = LandingBurn {
            thrust: 8_000.0,
            deceleration_margin: 0.0,
            ..LandingBurn::default()
        };
        assert!(!burn.can_stop(1000.0, 9.81));
        assert!(!burn.fires_at(
            &Vec3::new(0.0, 0.0, 1.0),
            &Vec3::new(0.0, 0.0, -20.0),
            1000.0,
            &Vec3::z(),
            0.0,
            9.81
        ));
    }

    #[test]
    fn the_ignition_ceiling_is_absolute() {
        // A trigger that would otherwise fire at 300 m, refused above 50 m.
        let burn = LandingBurn {
            thrust: 30_000.0,
            deceleration_margin: 0.0,
            maximum_ignition_altitude: 50.0,
            ..LandingBurn::default()
        };
        let up = Vec3::z();
        // Falling at 100 m/s, the stopping distance is 62 m, so 60 m of altitude
        // would satisfy the trigger if the ceiling allowed it.
        let position = Vec3::new(0.0, 0.0, 60.0);
        let velocity = Vec3::new(0.0, 0.0, -100.0);
        assert!(!burn.fires_at(&position, &velocity, 1000.0, &up, 0.0, 9.81));
        let lower = Vec3::new(0.0, 0.0, 40.0);
        assert!(burn.fires_at(&lower, &velocity, 1000.0, &up, 0.0, 9.81));
    }

    #[test]
    fn the_deceleration_margin_lights_the_engine_earlier() {
        let tight = LandingBurn {
            thrust: 30_000.0,
            deceleration_margin: 0.0,
            ..LandingBurn::default()
        };
        let cautious = LandingBurn {
            thrust: 30_000.0,
            deceleration_margin: 10.0,
            ..LandingBurn::default()
        };
        let up = Vec3::z();
        let velocity = Vec3::new(0.0, 0.0, -40.0);
        // The tight trigger needs 39.6 m of altitude, the cautious one 78.5 m, so
        // 50 m is inside one stopping distance and outside the other.
        let position = Vec3::new(0.0, 0.0, 50.0);
        assert!(!tight.fires_at(&position, &velocity, 1000.0, &up, 0.0, 9.81));
        assert!(cautious.fires_at(&position, &velocity, 1000.0, &up, 0.0, 9.81));
    }

    #[test]
    fn ground_elevation_moves_the_trigger_with_the_ground() {
        let burn = LandingBurn {
            thrust: 30_000.0,
            deceleration_margin: 0.0,
            ..LandingBurn::default()
        };
        let up = Vec3::z();
        let velocity = Vec3::new(0.0, 0.0, -40.0);
        // A world position 100 m below the launch point, on a site 200 m up, is
        // 100 m above the ground there.
        let position = Vec3::new(0.0, 0.0, -100.0);
        assert!(!burn.fires_at(&position, &velocity, 1000.0, &up, 200.0, 9.81));
        assert!(burn.fires_at(&position, &velocity, 1000.0, &up, 105.0, 9.81));
    }

    #[test]
    fn the_landing_burn_contributes_force_along_the_body_axis() {
        let state = RigidBodyState {
            position: Vec3::new(0.0, 0.0, 5.0),
            velocity: Vec3::new(0.0, 0.0, -30.0),
            // Nose up: -90 degrees of pitch points body X along world +Z.
            attitude: Quaternion::from_axis_angle(Vec3::y(), -std::f64::consts::FRAC_PI_2),
            mass: 1000.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(1000.0, 1.0);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let burn = LandingBurn {
            thrust: 30_000.0,
            deceleration_margin: 0.0,
            ..LandingBurn::default()
        };
        let mut acc = ForceAccumulator::new();
        burn.evaluate(&ctx, &mut acc);
        let thrust = acc
            .contributions
            .iter()
            .find(|c| c.source == ForceSource::LandingBurn)
            .expect("the landing burn was evaluated");
        assert!(thrust.active);
        assert!((thrust.force_body - Vec3::new(30_000.0, 0.0, 0.0)).norm() < 1e-9);
        // Body X is up, so the world force is up.
        assert!((thrust.force_world - Vec3::new(0.0, 0.0, 30_000.0)).norm() < 1e-6);
        assert!((acc.total_force_body() - Vec3::new(30_000.0, 0.0, 0.0)).norm() < 1e-9);
    }

    #[test]
    fn gravity_force_is_mass_times_g_and_has_no_moment() {
        let state = RigidBodyState::at_rest(2.0);
        let vehicle = Vehicle::simple(2.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        GravityForce.evaluate(&ctx, &mut acc);

        let f = acc.total_force_world();
        assert!((f - Vec3::new(0.0, 0.0, -19.62)).norm() < 1e-9);
        assert!(acc.total_moment_body().norm() < 1e-12);
        // In the ENU world frame, pointing down is -Z, so body Z is +Z here.
        assert!((acc.total_force_body() - Vec3::new(0.0, 0.0, -19.62)).norm() < 1e-9);
    }

    #[test]
    fn gravity_rotates_into_the_body_frame() {
        let state = RigidBodyState {
            attitude: Quaternion::from_axis_angle(Vec3::y(), std::f64::consts::FRAC_PI_2),
            mass: 1.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(10.0);
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        GravityForce.evaluate(&ctx, &mut acc);

        // World down is -Z. A 90 degree rotation about body Y maps body Z to
        // world X, so world -Z corresponds to body +X.
        let fb = acc.total_force_body();
        assert!(
            (fb - Vec3::new(10.0, 0.0, 0.0)).norm() < 1e-9,
            "got {:?}",
            fb
        );
    }

    #[test]
    fn thrust_profile_interpolates_and_ends() {
        let p = ThrustProfile::constant(100.0, 2.0);
        assert!((p.thrust_at(-1.0)).abs() < 1e-12);
        assert!((p.thrust_at(0.0) - 100.0).abs() < 1e-12);
        assert!((p.thrust_at(1.0) - 100.0).abs() < 1e-12);
        assert!((p.thrust_at(1.999) - 100.0).abs() < 1e-9);
        // The final sample marks the end of the profile.
        assert!((p.thrust_at(2.0 + 1e-9)).abs() < 1e-12);
        assert!((p.thrust_at(5.0)).abs() < 1e-12);
        assert_eq!(p.ignition_time(), Some(0.0));
        assert_eq!(p.burnout_time(), Some(2.0));
        assert!(p.is_active());
    }

    #[test]
    fn thrust_tapering_profile_has_the_configured_burnout() {
        let p = ThrustProfile::tapering(100.0, 40.0, 3.0);
        assert!((p.thrust_at(0.0) - 100.0).abs() < 1e-9);
        assert!((p.thrust_at(3.0 + 1e-9)).abs() < 1e-12);
        assert_eq!(p.burnout_time(), Some(3.0));
        assert_eq!(p.ignition_time(), Some(0.0));
    }

    #[test]
    fn zero_thrust_profile_is_not_active() {
        let profile = ThrustProfile {
            times: vec![0.0, 1.0],
            thrusts: vec![0.0, 0.0],
            ..Default::default()
        };
        assert!(!profile.is_active());
        assert!(profile.burnout_time().is_none());
        assert!(profile.ignition_time().is_none());
        assert!(profile.thrust_at(0.5).abs() < 1e-15);
    }

    #[test]
    fn thrust_force_along_body_x_is_direction_times_magnitude() {
        let state = RigidBodyState::at_rest(1.0);
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        ThrustForce::new(ThrustProfile::constant(50.0, 1.0)).evaluate(&ctx, &mut acc);
        assert!((acc.total_force_body() - Vec3::new(50.0, 0.0, 0.0)).norm() < 1e-12);
        assert!(acc.total_moment_body().norm() < 1e-12);
    }

    #[test]
    fn thrust_at_an_offset_produces_the_correct_moment() {
        let state = RigidBodyState::at_rest(1.0);
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let mut profile = ThrustProfile::constant(10.0, 1.0);
        profile.application_point = Vec3::new(0.0, 0.0, 0.5);
        let mut acc = ForceAccumulator::new();
        ThrustForce::new(profile).evaluate(&ctx, &mut acc);
        // r x F = (0,0,0.5) x (10,0,0) = (0, 5, 0)
        assert!((acc.total_moment_body() - Vec3::new(0.0, 5.0, 0.0)).norm() < 1e-12);
    }

    #[test]
    fn gimbal_deflection_tilts_the_thrust_vector() {
        let mut profile = ThrustProfile::constant(10.0, 1.0);
        profile.gimbal_z = Some(0.1);
        let d = profile.effective_direction();
        assert!(d.y > 0.09 && d.y < 0.11);
        assert!((d.norm() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn inactive_thrust_reports_no_effect() {
        let state = RigidBodyState::at_rest(1.0);
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        let profile = ThrustProfile {
            times: vec![0.0],
            thrusts: vec![0.0],
            ..Default::default()
        };
        ThrustForce::new(profile).evaluate(&ctx, &mut acc);
        assert!(acc.total_force_body().norm() < 1e-15);
        assert!(acc.active_sources().is_empty());
    }

    #[test]
    fn drag_opposes_the_relative_wind() {
        let state = RigidBodyState {
            velocity: Vec3::new(100.0, 0.0, 0.0),
            mass: 1.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::default();
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        AeroForce::new(AeroModel::drag_only_estimate(0.5)).evaluate(&ctx, &mut acc);

        let f = acc.total_force_body();
        assert!(f.x < 0.0, "drag should oppose motion, got {:?}", f);
        assert!(f.y.abs() < 1e-9 && f.z.abs() < 1e-9);
        // Magnitude check: q_dyn * S * Cd.
        let expected = 0.5
            * ctx.air.density
            * 100.0
            * 100.0
            * vehicle.reference_geometry().reference_area
            * 0.5;
        assert!((f.norm() - expected).abs() < 1e-9);
    }

    #[test]
    fn a_tabulated_model_reports_where_its_coefficients_came_from() {
        let table = AeroCoefficientTable::from_samples(vec![0.0, 0.8, 1.5], vec![0.35, 0.42, 0.6]);
        let derivatives = AeroDerivatives {
            lift_slope: 2.4,
            pitch_slope: -1.8,
            ..AeroDerivatives::default()
        };

        let imported = AeroModel::from_table(table.clone(), derivatives, false, 0.1);
        assert_eq!(imported.source, AeroSource::ImportedModel);
        assert_eq!(imported.source.label(), "imported model");
        assert!(!imported.drag_only);
        assert!(imported.has_mach_dependent_drag());
        assert!((imported.drag.at_mach(0.4) - 0.385).abs() < 1e-12);
        assert_eq!(
            imported.fidelity_label(),
            "Simplified: coefficient model with lift and moments, drag tabulated against Mach"
        );

        // A model that calls itself complete but carries no derivative computes
        // the drag-only motion, and the label has to say so.
        let empty = AeroModel::from_table(table, AeroDerivatives::default(), false, 0.1);
        assert!(empty.drag_only);
        assert!(!empty.has_lift_or_moments());
        assert_eq!(
            empty.fidelity_label(),
            "Simplified: drag only, tabulated against Mach"
        );

        // The estimate is always labelled as an estimate.
        let estimate = AeroModel::drag_only_estimate(0.45);
        assert_eq!(estimate.source, AeroSource::Estimate);
        assert_eq!(estimate.source.phrase(), "a built-in estimate");
    }

    #[test]
    fn the_estimate_with_lift_reports_no_terms_when_every_derivative_is_zero() {
        let model = AeroModel::estimate_with_lift(0.45, AeroDerivatives::default());
        assert!(model.drag_only);
        assert_eq!(model.source, AeroSource::Estimate);
    }

    #[test]
    fn a_tabulated_drag_coefficient_changes_the_force_with_mach() {
        let table = AeroCoefficientTable::from_samples(vec![0.0, 2.0], vec![0.2, 0.8]);
        let model = AeroModel::from_table(
            table,
            AeroDerivatives {
                lift_slope: 2.0,
                ..AeroDerivatives::default()
            },
            false,
            0.1,
        );
        assert!((model.drag.at_mach(0.0) - 0.2).abs() < 1e-12);
        assert!((model.drag.at_mach(2.0) - 0.8).abs() < 1e-12);
        assert!((model.drag.at_mach(1.0) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn drag_is_zero_in_vacuum() {
        let state = RigidBodyState {
            velocity: Vec3::new(100.0, 0.0, 0.0),
            mass: 1.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        AeroForce::new(AeroModel::drag_only_estimate(0.5)).evaluate(&ctx, &mut acc);
        assert!(acc.total_force_body().norm() < 1e-15);
    }

    #[test]
    fn drag_uses_the_wind_relative_velocity() {
        let state = RigidBodyState {
            velocity: Vec3::new(50.0, 0.0, 0.0),
            mass: 1.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment {
            wind: WindModel::constant(50.0, Vec3::x()),
            ..Environment::default()
        };
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        AeroForce::new(AeroModel::drag_only_estimate(0.5)).evaluate(&ctx, &mut acc);
        // Airspeed relative to the wind is zero, so there is no drag.
        assert!(acc.total_force_body().norm() < 1e-12);
    }

    #[test]
    fn angle_of_attack_and_sideslip_signs() {
        let state = RigidBodyState {
            // Wind arriving from below and from the right.
            velocity: Vec3::new(10.0, 2.0, -3.0),
            mass: 1.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let alpha = ctx.angle_of_attack();
        let beta = ctx.sideslip();
        assert!(alpha < 0.0, "alpha {} should be negative", alpha);
        assert!(beta > 0.0, "beta {} should be positive", beta);
        assert!((alpha - (-3.0f64).atan2(10.0)).abs() < 1e-12);
        assert!((beta - 2.0f64.atan2(10.0)).abs() < 1e-12);
    }

    #[test]
    fn lift_is_perpendicular_to_the_relative_wind() {
        let state = RigidBodyState {
            velocity: Vec3::new(100.0, 0.0, -10.0),
            mass: 1.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::default();
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        let model = AeroModel {
            drag_only: false,
            lift_slope: 2.0,
            ..AeroModel::drag_only_estimate(0.3)
        };
        AeroForce::new(model).evaluate(&ctx, &mut acc);

        let lift = acc.get(ForceSource::Lift).unwrap().force_body;
        assert!(lift.norm() > 0.0);
        assert!(
            lift.dot(&ctx.relative_velocity_body).abs() < 1e-9 * lift.norm().max(1.0),
            "lift must be perpendicular to the wind"
        );
    }

    #[test]
    fn pitching_moment_is_negative_for_positive_alpha_with_stable_slope() {
        // Body Z points down and the body is nose-up, so the relative wind
        // arrives from below: the relative velocity has a positive body-Z
        // component, which makes alpha positive.
        let state = RigidBodyState {
            velocity: Vec3::new(100.0, 0.0, 5.0),
            mass: 1.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::default();
        let ctx = context(&state, &vehicle, &env);
        assert!(ctx.angle_of_attack() > 0.0);
        let mut acc = ForceAccumulator::new();
        let model = AeroModel {
            drag_only: false,
            pitch_slope: -1.5,
            ..AeroModel::drag_only_estimate(0.3)
        };
        AeroForce::new(model).evaluate(&ctx, &mut acc);
        let m = acc.total_moment_body();
        assert!(
            m.y < 0.0,
            "stable pitch moment should be negative, got {:?}",
            m
        );
    }

    #[test]
    fn coefficient_table_interpolates_and_clamps() {
        let t = AeroCoefficientTable::from_samples(vec![0.0, 1.0, 2.0], vec![0.4, 0.5, 0.6]);
        assert!((t.at_mach(0.0) - 0.4).abs() < 1e-12);
        assert!((t.at_mach(0.5) - 0.45).abs() < 1e-12);
        assert!((t.at_mach(-5.0) - 0.4).abs() < 1e-12);
        assert!((t.at_mach(99.0) - 0.6).abs() < 1e-12);
        assert!(t.is_usable());
    }

    #[test]
    fn coefficient_table_rejects_mismatched_lengths() {
        let t = AeroCoefficientTable {
            mach: vec![0.0, 1.0],
            values: vec![0.4],
            fallback: 0.42,
        };
        assert!(!t.is_usable());
        assert!((t.at_mach(0.5) - 0.42).abs() < 1e-12);
    }

    #[test]
    fn control_moment_profile_switches_on_and_off() {
        let p = ControlMoment::constant(Vec3::new(0.0, 1.0, 0.0), 1.0);
        assert!(p.enabled);
        assert!((p.moment_at(0.5) - Vec3::new(0.0, 1.0, 0.0)).norm() < 1e-12);
        assert!(p.moment_at(2.0).norm() < 1e-12);
    }

    #[test]
    fn disabled_control_contributes_nothing() {
        let state = RigidBodyState::at_rest(1.0);
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        ControlForce {
            profile: ControlMoment::default(),
        }
        .evaluate(&ctx, &mut acc);
        assert!(acc.total_moment_body().norm() < 1e-15);
    }

    #[test]
    fn external_load_applies_force_and_offset_moment() {
        let state = RigidBodyState::at_rest(1.0);
        let vehicle = Vehicle::simple(1.0, 0.1);
        let env = Environment::vacuum_uniform_gravity(9.81);
        let ctx = context(&state, &vehicle, &env);
        let mut acc = ForceAccumulator::new();
        ExternalLoad {
            force_body: Vec3::new(0.0, 2.0, 0.0),
            moment_body: Vec3::new(1.0, 0.0, 0.0),
            application_point_body: Vec3::new(0.0, 0.0, 1.0),
            enabled: true,
        }
        .evaluate(&ctx, &mut acc);
        // r x F = (0,0,1) x (0,2,0) = (-2,0,0), plus the direct moment (1,0,0).
        assert!((acc.total_moment_body() - Vec3::new(-1.0, 0.0, 0.0)).norm() < 1e-12);
        assert!((acc.total_force_body() - Vec3::new(0.0, 2.0, 0.0)).norm() < 1e-12);
    }

    #[test]
    fn accumulator_totals_every_active_source() {
        let state = RigidBodyState {
            velocity: Vec3::new(80.0, 0.0, 0.0),
            mass: 2.0,
            ..Default::default()
        };
        let vehicle = Vehicle::simple(2.0, 0.1);
        let env = Environment::default();
        let ctx = context(&state, &vehicle, &env);
        let providers = crate::equations::ProviderChain::with_gravity()
            .with(Box::new(ThrustForce::new(ThrustProfile::constant(
                40.0, 1.0,
            ))))
            .with(Box::new(AeroForce::new(AeroModel::drag_only_estimate(0.4))))
            .with(Box::new(ExternalLoad::default()));
        let acc = accumulate(&providers, &ctx);
        assert_eq!(acc.contributions.len(), 4);
        assert_eq!(acc.active_sources().len(), 3);
        assert!(acc.get(ForceSource::Gravity).is_some());
        assert!(!acc.get(ForceSource::External).unwrap().active);
        let sources = acc.active_sources();
        assert!(sources.contains(&ForceSource::Gravity));
        assert!(sources.contains(&ForceSource::Thrust));
        assert!(sources.contains(&ForceSource::Drag));
        assert!(!sources.contains(&ForceSource::External));
    }

    #[test]
    fn moment_only_contribution_has_no_force() {
        let c = ForceContribution::moment_only(ForceSource::Control, Vec3::new(1.0, 2.0, 3.0));
        assert!(c.force_body.norm() < 1e-15);
        assert!((c.moment_body() - Vec3::new(1.0, 2.0, 3.0)).norm() < 1e-15);
        assert!(c.has_effect());
    }
}
