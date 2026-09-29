//! Explicit coordinate-frame definitions and transforms.
//!
//! HexaDOF never assumes that a sensor axis, a renderer axis, and a body axis
//! are the same thing. Each of those is a named frame here, and every vector
//! that crosses a boundary carries a documented frame.

use serde::{Deserialize, Serialize};

use crate::quaternion::Quaternion;
use crate::vectors::{Mat3, Vec3};
use crate::Real;

/// The inertial reference frame a project uses.
///
/// Both supported frames are right-handed. `Ned` is the classical aerospace
/// convention; `Enu` is common in robotics and is easier to map onto renderer
/// axes directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldFrame {
    /// X north, Y east, Z down. Up is negative Z.
    Ned,
    /// X east, Y north, Z up. Up is positive Z.
    #[default]
    Enu,
}

impl WorldFrame {
    pub fn label(self) -> &'static str {
        match self {
            WorldFrame::Ned => "NED (N, E, D)",
            WorldFrame::Enu => "ENU (E, N, U)",
        }
    }

    /// Unit vector, expressed in this frame, that points away from the Earth.
    pub fn up_axis(self) -> Vec3 {
        match self {
            WorldFrame::Ned => Vec3::new(0.0, 0.0, -1.0),
            WorldFrame::Enu => Vec3::new(0.0, 0.0, 1.0),
        }
    }

    /// Unit vector, expressed in this frame, that points down toward the Earth.
    pub fn down_axis(self) -> Vec3 {
        -self.up_axis()
    }

    /// Gravity acceleration vector in this frame for magnitude `g`.
    pub fn gravity_vector(self, g: Real) -> Vec3 {
        self.down_axis() * g
    }

    /// Rotation taking a vector expressed in the other world frame into this one.
    pub fn from_frame(self) -> WorldFrame {
        self
    }
}

/// The body-fixed frame convention a model declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyFrame {
    /// X forward, Y right, Z down. Right-handed, classical aerospace.
    #[default]
    ForwardRightDown,
    /// X forward, Y left, Z up. Right-handed, common in some simulation tools.
    ForwardLeftUp,
}

impl BodyFrame {
    pub fn label(self) -> &'static str {
        match self {
            BodyFrame::ForwardRightDown => "X forward, Y right, Z down",
            BodyFrame::ForwardLeftUp => "X forward, Y left, Z up",
        }
    }

    /// Whether this convention uses Z-up, which matters for mesh orientation.
    pub fn is_z_up(self) -> bool {
        matches!(self, BodyFrame::ForwardLeftUp)
    }
}

/// Native axes of a physical sensor package.
///
/// A device may mount its accelerometer with any permutation and sign of the
/// body axes. This type records that permutation explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorAxes {
    /// Sensor X maps to body X, Y to Y, Z to Z, all positive.
    Xyz,
    /// Sensor X maps to body X, sensor Y to body Y, sensor Z to negative body Z.
    XyzNegZ,
    /// Sensor X maps to body Y, Y to body X, Z to negative body Z.
    YxzNegZ,
    /// Sensor X maps to negative body Y, Y to body X, Z to body Z.
    NegYxz,
    /// A fully explicit custom permutation.
    #[default]
    Custom,
}

/// A signed axis permutation plus an optional mounting rotation.
///
/// This is the structure stored in device profiles and flight-log mappings. It
/// is the only supported way to relate sensor readings to body axes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AxisMapping {
    /// For each body axis, which sensor axis supplies it (0 = X, 1 = Y, 2 = Z).
    pub body_to_sensor: [u8; 3],
    /// Sign applied when reading each body axis from the selected sensor axis.
    pub sign: [Real; 3],
    /// Optional fixed mounting rotation applied after the permutation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mounting: Option<Quaternion>,
}

impl Default for AxisMapping {
    fn default() -> Self {
        Self::identity()
    }
}

impl AxisMapping {
    pub const fn identity() -> Self {
        Self {
            body_to_sensor: [0, 1, 2],
            sign: [1.0, 1.0, 1.0],
            mounting: None,
        }
    }

    /// Build the mapping implied by a named sensor axis convention.
    pub fn from_sensor_axes(axes: SensorAxes) -> Self {
        match axes {
            SensorAxes::Xyz => Self::identity(),
            SensorAxes::XyzNegZ => Self {
                body_to_sensor: [0, 1, 2],
                sign: [1.0, 1.0, -1.0],
                mounting: None,
            },
            SensorAxes::YxzNegZ => Self {
                body_to_sensor: [1, 0, 2],
                sign: [1.0, 1.0, -1.0],
                mounting: None,
            },
            SensorAxes::NegYxz => Self {
                body_to_sensor: [1, 0, 2],
                sign: [-1.0, 1.0, 1.0],
                mounting: None,
            },
            SensorAxes::Custom => Self {
                body_to_sensor: [0, 1, 2],
                sign: [1.0, 1.0, 1.0],
                mounting: None,
            },
        }
    }

    /// True when the mapping is a valid signed permutation.
    pub fn is_valid(&self) -> bool {
        let mut seen = [false; 3];
        for &a in &self.body_to_sensor {
            if a > 2 {
                return false;
            }
            if seen[a as usize] {
                return false;
            }
            seen[a as usize] = true;
        }
        self.sign
            .iter()
            .all(|s| (*s - 1.0).abs() < 1e-12 || (*s + 1.0).abs() < 1e-12)
            && self.mounting.map(|m| m.is_finite()).unwrap_or(true)
    }

    /// Read a body-frame vector from a sensor-frame vector.
    pub fn sensor_to_body(&self, sensor: Vec3) -> Vec3 {
        let raw = Vec3::new(
            self.sign[0] * sensor[self.body_to_sensor[0] as usize],
            self.sign[1] * sensor[self.body_to_sensor[1] as usize],
            self.sign[2] * sensor[self.body_to_sensor[2] as usize],
        );
        match self.mounting {
            Some(q) => q.rotate(raw),
            None => raw,
        }
    }

    /// Rotation matrix form of the mapping, useful for covariance transforms.
    pub fn to_matrix(&self) -> Mat3 {
        let mut m = Mat3::zeros();
        for (body_axis, (&sensor_axis, &s)) in
            self.body_to_sensor.iter().zip(self.sign.iter()).enumerate()
        {
            m[(body_axis, sensor_axis as usize)] = s;
        }
        match self.mounting {
            Some(q) => q.to_matrix() * m,
            None => m,
        }
    }
}

/// A declared frame triple for a project or imported artefact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub world: WorldFrame,
    pub body: BodyFrame,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            world: WorldFrame::Enu,
            body: BodyFrame::ForwardRightDown,
        }
    }
}

impl Frame {
    pub const fn new(world: WorldFrame, body: BodyFrame) -> Self {
        Self { world, body }
    }
}

/// A quaternion that has been explicitly labelled with the frames it relates.
///
/// Constructing one of these is the only sanctioned way to move attitude across
/// a module boundary, so a convention mistake becomes a type error rather than a
/// silent sign flip.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrameTransform {
    /// Rotation mapping body-frame vectors into the world frame.
    pub body_to_world: Quaternion,
    pub frames: Frame,
}

impl FrameTransform {
    pub fn identity(frames: Frame) -> Self {
        Self {
            body_to_world: Quaternion::identity(),
            frames,
        }
    }

    pub fn new(body_to_world: Quaternion, frames: Frame) -> Self {
        Self {
            body_to_world: body_to_world.normalized(),
            frames,
        }
    }

    /// Rotate a body-frame vector into the world frame.
    pub fn body_to_world(&self, v: Vec3) -> Vec3 {
        self.body_to_world.rotate(v)
    }

    /// Rotate a world-frame vector into the body frame.
    pub fn world_to_body(&self, v: Vec3) -> Vec3 {
        self.body_to_world.inverse().rotate(v)
    }

    /// The inverse transform, useful when checking round-trip consistency.
    pub fn inverse(&self) -> Self {
        Self {
            body_to_world: self.body_to_world.inverse(),
            frames: self.frames,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    #[test]
    fn world_up_and_gravity_agree() {
        let ned = WorldFrame::Ned;
        assert_eq!(ned.up_axis(), Vec3::new(0.0, 0.0, -1.0));
        assert_eq!(ned.gravity_vector(9.81), Vec3::new(0.0, 0.0, 9.81));

        let enu = WorldFrame::Enu;
        assert_eq!(enu.up_axis(), Vec3::new(0.0, 0.0, 1.0));
        assert_eq!(enu.gravity_vector(9.81), Vec3::new(0.0, 0.0, -9.81));
    }

    #[test]
    fn frame_transform_round_trips() {
        let q = Quaternion::from_axis_angle(Vec3::new(1.0, 2.0, -0.5), 1.234);
        let t = FrameTransform::new(q, Frame::default());
        let v = Vec3::new(3.0, -4.0, 5.0);
        let round = t.world_to_body(t.body_to_world(v));
        assert!((round - v).norm() < 1e-12);
    }

    #[test]
    fn inverse_transform_is_exact_inverse() {
        let q = Quaternion::from_axis_angle(Vec3::z(), 0.7);
        let t = FrameTransform::new(q, Frame::default());
        let inv = t.inverse();
        let v = Vec3::new(1.0, 1.0, 0.0);
        assert!((inv.body_to_world(t.body_to_world(v)) - v).norm() < 1e-12);
    }

    #[test]
    fn axis_mapping_identity_and_negated() {
        let id = AxisMapping::identity();
        let v = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(id.sensor_to_body(v), v);
        assert!(id.is_valid());

        let neg_z = AxisMapping::from_sensor_axes(SensorAxes::XyzNegZ);
        assert_eq!(neg_z.sensor_to_body(v), Vec3::new(1.0, 2.0, -3.0));
        assert!(neg_z.is_valid());

        // Swapping X and Y must be its own inverse.
        let yxz = AxisMapping::from_sensor_axes(SensorAxes::YxzNegZ);
        let swapped = yxz.sensor_to_body(v);
        assert_eq!(swapped, Vec3::new(2.0, 1.0, -3.0));
        assert!(yxz.is_valid());
    }

    #[test]
    fn axis_mapping_matrix_matches_vector_form() {
        let m = AxisMapping::from_sensor_axes(SensorAxes::NegYxz);
        let v = Vec3::new(0.5, -1.5, 2.5);
        assert!((m.to_matrix() * v - m.sensor_to_body(v)).norm() < 1e-12);
    }

    #[test]
    fn axis_mapping_rejects_duplicate_axis() {
        let bad = AxisMapping {
            body_to_sensor: [0, 0, 2],
            sign: [1.0, 1.0, 1.0],
            mounting: None,
        };
        assert!(!bad.is_valid());
    }

    #[test]
    fn axis_mapping_with_mounting_rotation() {
        let m = AxisMapping {
            body_to_sensor: [0, 1, 2],
            sign: [1.0, 1.0, 1.0],
            mounting: Some(Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_2)),
        };
        let out = m.sensor_to_body(Vec3::new(1.0, 0.0, 0.0));
        assert!((out - Vec3::new(0.0, 1.0, 0.0)).norm() < 1e-12);
    }
}
