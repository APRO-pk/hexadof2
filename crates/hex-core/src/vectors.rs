//! Small fixed-size linear algebra aliases and helpers.
//!
//! The dynamics core works with 3-vectors and 3x3 matrices only. These aliases
//! keep signatures readable, and the `ops` module collects the handful of
//! numerical helpers that would otherwise be re-derived in every crate.
//!
//! Mass properties live in [`crate::mass`].

use nalgebra::{Matrix3, Vector3};
use serde::{Deserialize, Serialize};

use crate::Real;

/// A 3-vector in whatever frame the caller documents.
pub type Vec3 = Vector3<Real>;

/// A 3x3 matrix, used for inertia tensors, rotations, and covariance.
pub type Mat3 = Matrix3<Real>;

/// Component-wise relative comparison with an absolute floor.
pub fn approx_eq(a: Real, b: Real, rel: Real, abs: Real) -> bool {
    if a == b {
        return true;
    }
    (a - b).abs() <= (rel * a.abs().max(b.abs())).max(abs)
}

/// True when the vector has no NaN or infinite component.
pub fn is_finite(v: &Vec3) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// True when every entry of the matrix is finite.
pub fn matrix_is_finite(m: &Mat3) -> bool {
    m.iter().all(|x| x.is_finite())
}

/// Angle between two vectors in radians, clamped for numerical safety.
pub fn angle_between(a: &Vec3, b: &Vec3) -> Real {
    let na = a.norm();
    let nb = b.norm();
    if na < 1e-15 || nb < 1e-15 {
        return 0.0;
    }
    (a.dot(b) / (na * nb)).clamp(-1.0, 1.0).acos()
}

/// Project `v` onto the plane orthogonal to `n`.
pub fn reject_from(v: &Vec3, n: &Vec3) -> Vec3 {
    let nn = n.norm_squared();
    if nn < 1e-30 {
        *v
    } else {
        v - n * (v.dot(n) / nn)
    }
}

/// Unit vector in the direction of `v`, or `fallback` when `v` is degenerate.
///
/// Used everywhere a direction is taken from user input, so a zero vector
/// produces a documented default instead of a NaN.
pub fn normalized_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let n = v.norm();
    if n.is_finite() && n > 1e-12 {
        v / n
    } else {
        fallback
    }
}

/// Unit vector in the direction of `v`, falling back to world +X.
pub fn normalized(v: Vec3) -> Vec3 {
    normalized_or(v, Vec3::x())
}

/// Skew-symmetric matrix `[v]x` such that `[v]x * u == v x u`.
pub fn skew(v: &Vec3) -> Mat3 {
    Mat3::new(0.0, -v.z, v.y, v.z, 0.0, -v.x, -v.y, v.x, 0.0)
}

/// A serialisable 3-vector for on-disk formats.
///
/// `nalgebra` vectors serialise as arrays, which is compact but opaque in a
/// hand-editable project file. Project and model schemas use this type instead so
/// a user can read and edit coordinates directly.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3Record {
    pub x: Real,
    pub y: Real,
    pub z: Real,
}

impl Vec3Record {
    pub const fn new(x: Real, y: Real, z: Real) -> Self {
        Self { x, y, z }
    }

    pub fn to_vec3(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    pub fn from_vec3(v: Vec3) -> Self {
        Self::new(v.x, v.y, v.z)
    }

    pub fn is_finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl From<Vec3> for Vec3Record {
    fn from(v: Vec3) -> Self {
        Self::from_vec3(v)
    }
}

impl From<Vec3Record> for Vec3 {
    fn from(v: Vec3Record) -> Self {
        v.to_vec3()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skew_matrix_matches_cross_product() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(-4.0, 5.0, 0.5);
        assert!((skew(&a) * b - a.cross(&b)).norm() < 1e-15);
    }

    #[test]
    fn angle_between_is_clamped() {
        let a = Vec3::new(1.0, 0.0, 0.0);
        assert!((angle_between(&a, &a) - 0.0).abs() < 1e-12);
        assert!((angle_between(&a, &(-a)) - std::f64::consts::PI).abs() < 1e-12);
    }

    #[test]
    fn reject_removes_normal_component() {
        let v = Vec3::new(2.0, 3.0, 4.0);
        let r = reject_from(&v, &Vec3::z());
        assert!(r.z.abs() < 1e-15);
    }

    #[test]
    fn vec3_record_round_trips() {
        let v = Vec3::new(1.5, -2.5, 3.25);
        assert_eq!(Vec3::from(Vec3Record::from(v)), v);
    }

    #[test]
    fn finite_checks_detect_nan() {
        assert!(!is_finite(&Vec3::new(Real::NAN, 0.0, 0.0)));
        assert!(!matrix_is_finite(&Mat3::new(
            Real::INFINITY,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            1.0
        )));
    }
}
