//! The rigid-body state vector and its conversion to and from a flat array.
//!
//! # State layout
//!
//! ```text
//! x = [ r_x  r_y  r_z     position in the world frame, m
//!       v_x  v_y  v_z     velocity in the world frame, m/s
//!       q_w  q_x  q_y  q_z   body-to-world quaternion, scalar first
//!       w_x  w_y  w_z     angular velocity in the body frame, rad/s
//!       m ]               mass, kg
//! ```
//!
//! Fourteen stored values, with one unit-norm constraint on the quaternion. When
//! the vehicle model declares constant mass the mass entry is still carried so
//! the state dimension never changes mid-run, but its derivative is forced to
//! zero.
//!
//! The quaternion is normalised after every accepted integrator step. Euler
//! angles are provided as display-only derived values.

use hex_core::{Frame, FrameTransform, Quaternion, Real, Vec3};
use serde::{Deserialize, Serialize};

/// Number of stored values in the rigid-body state.
pub const STATE_LEN: usize = 14;

/// Offsets of each block inside the flat state array.
pub mod index {
    /// Position block offset.
    pub const POSITION: usize = 0;
    /// Velocity block offset.
    pub const VELOCITY: usize = 3;
    /// Quaternion block offset.
    pub const QUATERNION: usize = 6;
    /// Body angular velocity block offset.
    pub const ANGULAR_VELOCITY: usize = 10;
    /// Mass offset.
    pub const MASS: usize = 13;
}

/// Simulation mode: full rigid body or simplified point mass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationMode {
    /// Six degree of freedom rigid-body dynamics.
    #[default]
    SixDof,
    /// Three degree of freedom point mass. Attitude is fixed or prescribed.
    ThreeDof,
}

impl SimulationMode {
    pub fn label(self) -> &'static str {
        match self {
            SimulationMode::SixDof => "Rigid body 6-DOF",
            SimulationMode::ThreeDof => "Point mass 3-DOF",
        }
    }

    /// Short label used in run metadata.
    pub fn short(self) -> &'static str {
        match self {
            SimulationMode::SixDof => "6-DOF",
            SimulationMode::ThreeDof => "3-DOF",
        }
    }
}

/// The complete dynamic state of the vehicle at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RigidBodyState {
    /// Position in the world frame, metres.
    pub position: Vec3,
    /// Velocity in the world frame, metres per second.
    pub velocity: Vec3,
    /// Body-to-world attitude quaternion, scalar first.
    pub attitude: Quaternion,
    /// Angular velocity in the body frame, radians per second.
    pub angular_velocity: Vec3,
    /// Vehicle mass in kilograms.
    pub mass: Real,
}

impl Default for RigidBodyState {
    fn default() -> Self {
        Self {
            position: Vec3::zeros(),
            velocity: Vec3::zeros(),
            attitude: Quaternion::identity(),
            angular_velocity: Vec3::zeros(),
            mass: 1.0,
        }
    }
}

impl RigidBodyState {
    /// A vehicle at rest at the origin with identity attitude.
    pub fn at_rest(mass: Real) -> Self {
        Self {
            mass,
            ..Self::default()
        }
    }

    /// True when every component is finite.
    pub fn is_finite(&self) -> bool {
        self.position.iter().all(|v| v.is_finite())
            && self.velocity.iter().all(|v| v.is_finite())
            && self.attitude.is_finite()
            && self.angular_velocity.iter().all(|v| v.is_finite())
            && self.mass.is_finite()
    }

    /// Attitude as an explicitly labelled body-to-world transform.
    pub fn transform(&self, frame: Frame) -> FrameTransform {
        FrameTransform::new(self.attitude, frame)
    }

    pub fn speed(&self) -> Real {
        self.velocity.norm()
    }

    pub fn altitude_above(&self, world_up: Vec3) -> Real {
        self.position.dot(&world_up)
    }

    /// Roll, pitch, and yaw in radians, derived for display only.
    pub fn euler_321(&self) -> Vec3 {
        self.attitude.to_euler_321()
    }

    /// Quaternion norm error, which should stay near zero.
    pub fn quaternion_norm_error(&self) -> Real {
        (self.attitude.norm() - 1.0).abs()
    }

    /// Normalise the quaternion in place and return the error before doing so.
    pub fn normalize_attitude(&mut self) -> Real {
        let mut q = self.attitude;
        let error = q.normalize_in_place();
        self.attitude = q;
        error
    }

    /// Kinetic energy: translational plus rotational, in joules.
    pub fn kinetic_energy(&self, inertia: &hex_core::InertiaTensor) -> Real {
        let translational = 0.5 * self.mass * self.velocity.norm_squared();
        let iw = inertia.to_matrix() * self.angular_velocity;
        let rotational = 0.5 * self.angular_velocity.dot(&iw);
        translational + rotational
    }

    /// Angular momentum about the centre of mass, expressed in the body frame.
    pub fn angular_momentum_body(&self, inertia: &hex_core::InertiaTensor) -> Vec3 {
        inertia.to_matrix() * self.angular_velocity
    }

    /// Linear momentum in the world frame.
    pub fn linear_momentum(&self) -> Vec3 {
        self.velocity * self.mass
    }

    /// Flatten into the integrator's state array.
    pub fn to_array(&self) -> [Real; STATE_LEN] {
        let mut x = [0.0; STATE_LEN];
        x[index::POSITION..index::POSITION + 3].copy_from_slice(self.position.as_slice());
        x[index::VELOCITY..index::VELOCITY + 3].copy_from_slice(self.velocity.as_slice());
        x[index::QUATERNION] = self.attitude.w;
        x[index::QUATERNION + 1] = self.attitude.x;
        x[index::QUATERNION + 2] = self.attitude.y;
        x[index::QUATERNION + 3] = self.attitude.z;
        x[index::ANGULAR_VELOCITY..index::ANGULAR_VELOCITY + 3]
            .copy_from_slice(self.angular_velocity.as_slice());
        x[index::MASS] = self.mass;
        x
    }

    /// Rebuild from the integrator's state array.
    pub fn from_array(x: &[Real; STATE_LEN]) -> Self {
        Self {
            position: Vec3::new(x[0], x[1], x[2]),
            velocity: Vec3::new(x[3], x[4], x[5]),
            attitude: Quaternion::new(x[6], x[7], x[8], x[9]),
            angular_velocity: Vec3::new(x[10], x[11], x[12]),
            mass: x[13],
        }
    }
}

/// A single recorded output sample.
///
/// The plot panel and the state inspector read these. Every derived quantity is
/// optional so a 3-DOF run can leave attitude-dependent channels absent rather
/// than fabricating them.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StateSample {
    /// Simulation time in seconds.
    pub time: Real,
    /// Position in the world frame, metres.
    pub position: [Real; 3],
    /// Velocity in the world frame, metres per second.
    pub velocity: [Real; 3],
    /// Body-to-world quaternion, scalar first.
    pub attitude: [Real; 4],
    /// Angular velocity in the body frame, radians per second.
    pub angular_velocity: [Real; 3],
    /// Mass in kilograms.
    pub mass: Real,
    /// Total force in the body frame, newtons.
    pub force_body: [Real; 3],
    /// Total moment in the body frame, newton metres.
    pub moment_body: [Real; 3],
    /// Total force in the world frame, newtons.
    pub force_world: [Real; 3],
    /// Acceleration in the world frame, metres per second squared.
    pub acceleration_world: [Real; 3],
    /// Dynamic pressure in pascals.
    pub dynamic_pressure: Real,
    /// Angle of attack in radians.
    pub angle_of_attack: Real,
    /// Sideslip angle in radians.
    pub sideslip: Real,
    /// Ambient density in kilograms per cubic metre.
    pub air_density: Real,
    /// Mach number, when a speed of sound is available.
    pub mach: Real,
    /// Roll, pitch, and yaw in radians, derived for display.
    pub euler_321: [Real; 3],
    /// Names of the events that fired exactly at this sample.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
}

impl StateSample {
    /// Build a sample from the state and the per-step diagnostics.
    pub fn from_state(time: Real, state: &RigidBodyState) -> Self {
        let e = state.euler_321();
        Self {
            time,
            position: [state.position.x, state.position.y, state.position.z],
            velocity: [state.velocity.x, state.velocity.y, state.velocity.z],
            attitude: [
                state.attitude.w,
                state.attitude.x,
                state.attitude.y,
                state.attitude.z,
            ],
            angular_velocity: [
                state.angular_velocity.x,
                state.angular_velocity.y,
                state.angular_velocity.z,
            ],
            mass: state.mass,
            euler_321: [e.x, e.y, e.z],
            ..Self::default()
        }
    }

    pub fn position_vec(&self) -> Vec3 {
        Vec3::new(self.position[0], self.position[1], self.position[2])
    }

    pub fn velocity_vec(&self) -> Vec3 {
        Vec3::new(self.velocity[0], self.velocity[1], self.velocity[2])
    }

    pub fn attitude_quaternion(&self) -> Quaternion {
        Quaternion::new(
            self.attitude[0],
            self.attitude[1],
            self.attitude[2],
            self.attitude[3],
        )
    }

    pub fn angular_velocity_vec(&self) -> Vec3 {
        Vec3::new(
            self.angular_velocity[0],
            self.angular_velocity[1],
            self.angular_velocity[2],
        )
    }

    pub fn speed(&self) -> Real {
        self.velocity_vec().norm()
    }

    /// Altitude measured along the supplied world up axis.
    pub fn altitude(&self, world_up: Vec3) -> Real {
        self.position_vec().dot(&world_up)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_finite_and_normalized() {
        let s = RigidBodyState::default();
        assert!(s.is_finite());
        assert!(s.quaternion_norm_error() < 1e-15);
        assert!((s.speed()).abs() < 1e-15);
    }

    #[test]
    fn array_round_trip_preserves_every_field() {
        let s = RigidBodyState {
            position: Vec3::new(1.0, -2.0, 3.0),
            velocity: Vec3::new(4.0, 5.0, -6.0),
            attitude: Quaternion::new(0.5, 0.5, 0.5, 0.5),
            angular_velocity: Vec3::new(0.1, 0.2, 0.3),
            mass: 7.5,
        };
        let back = RigidBodyState::from_array(&s.to_array());
        assert_eq!(back, s);
    }

    #[test]
    fn array_layout_matches_the_documented_offsets() {
        let s = RigidBodyState {
            position: Vec3::new(1.0, 2.0, 3.0),
            velocity: Vec3::new(4.0, 5.0, 6.0),
            attitude: Quaternion::new(7.0, 8.0, 9.0, 10.0),
            angular_velocity: Vec3::new(11.0, 12.0, 13.0),
            mass: 14.0,
        };
        let x = s.to_array();
        for (i, v) in x.iter().enumerate() {
            assert!((*v - (i as Real + 1.0)).abs() < 1e-15, "index {}", i);
        }
        assert_eq!(STATE_LEN, 14);
    }

    #[test]
    fn non_finite_state_is_detected() {
        let mut s = RigidBodyState::default();
        s.position.x = Real::NAN;
        assert!(!s.is_finite());

        let mut s2 = RigidBodyState::default();
        s2.angular_velocity.z = Real::INFINITY;
        assert!(!s2.is_finite());
    }

    #[test]
    fn normalization_recovers_a_scaled_quaternion() {
        let mut s = RigidBodyState {
            attitude: Quaternion::new(2.0, 0.0, 0.0, 0.0),
            ..Default::default()
        };
        let error = s.normalize_attitude();
        assert!((error - 1.0).abs() < 1e-12);
        assert!(s.quaternion_norm_error() < 1e-15);
    }

    #[test]
    fn kinetic_energy_of_translation_only() {
        let s = RigidBodyState {
            velocity: Vec3::new(3.0, 4.0, 0.0),
            mass: 2.0,
            ..Default::default()
        };
        let inertia = hex_core::InertiaTensor::diagonal(1.0, 1.0, 1.0);
        // 0.5 * 2 * 25 = 25 J
        assert!((s.kinetic_energy(&inertia) - 25.0).abs() < 1e-12);
    }

    #[test]
    fn kinetic_energy_of_rotation_only() {
        let s = RigidBodyState {
            velocity: Vec3::zeros(),
            angular_velocity: Vec3::new(0.0, 0.0, 2.0),
            mass: 1.0,
            ..Default::default()
        };
        let inertia = hex_core::InertiaTensor::diagonal(1.0, 1.0, 0.5);
        // 0.5 * 0.5 * 4 = 1 J
        assert!((s.kinetic_energy(&inertia) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn angular_momentum_body_scales_with_each_axis() {
        let s = RigidBodyState {
            angular_velocity: Vec3::new(1.0, 1.0, 1.0),
            ..Default::default()
        };
        let inertia = hex_core::InertiaTensor::diagonal(1.0, 2.0, 3.0);
        let h = s.angular_momentum_body(&inertia);
        assert!((h - Vec3::new(1.0, 2.0, 3.0)).norm() < 1e-15);
    }

    #[test]
    fn linear_momentum_is_mass_times_velocity() {
        let s = RigidBodyState {
            velocity: Vec3::new(1.0, 2.0, 3.0),
            mass: 2.0,
            ..Default::default()
        };
        assert!((s.linear_momentum() - Vec3::new(2.0, 4.0, 6.0)).norm() < 1e-15);
    }

    #[test]
    fn sample_captures_state_and_derived_euler_angles() {
        let s = RigidBodyState {
            position: Vec3::new(0.0, 0.0, 100.0),
            velocity: Vec3::new(10.0, 0.0, 0.0),
            attitude: Quaternion::from_euler_321(0.1, 0.2, 0.3),
            angular_velocity: Vec3::new(0.0, 0.0, 0.5),
            mass: 5.0,
        };
        let sample = StateSample::from_state(1.25, &s);
        assert!((sample.time - 1.25).abs() < 1e-15);
        assert!((sample.speed() - 10.0).abs() < 1e-12);
        assert!((sample.altitude(Vec3::z()) - 100.0).abs() < 1e-12);
        assert!((sample.euler_321[1] - 0.2).abs() < 1e-12);
        assert!((sample.mass - 5.0).abs() < 1e-15);
    }

    #[test]
    fn altitude_uses_the_supplied_up_axis() {
        let s = RigidBodyState {
            position: Vec3::new(0.0, 0.0, -30.0),
            ..Default::default()
        };
        assert!((s.altitude_above(Vec3::new(0.0, 0.0, -1.0)) - 30.0).abs() < 1e-12);
        assert!((s.altitude_above(Vec3::z()) + 30.0).abs() < 1e-12);
    }

    #[test]
    fn mode_labels_are_distinct() {
        assert_ne!(
            SimulationMode::SixDof.label(),
            SimulationMode::ThreeDof.label()
        );
        assert_eq!(SimulationMode::default(), SimulationMode::SixDof);
    }
}
