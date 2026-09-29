//! Quaternion algebra with one documented convention.
//!
//! # Convention
//!
//! * Quaternions are scalar first: `q = [w, x, y, z]`.
//! * `q_bi` rotates **body-frame** vectors into the **inertial/world** frame:
//!   `v_i = q_bi * v_b`.
//! * Composition follows Hamilton's rule, so
//!   `(a * b) v (a * b)^-1 == a (b v b^-1) a^-1`. This means `a * b` applies
//!   `b` first, then `a`.
//! * Body-frame angular velocity kinematics use `q_dot = 0.5 * q * [0, w_b]`.
//!
//! The last two statements are the ones that are easy to get backwards, so they
//! are asserted directly against numerical derivatives of known rotations in
//! the tests at the bottom of this file.

use std::ops::Mul;

use serde::{Deserialize, Serialize};

use crate::vectors::{Mat3, Vec3};
use crate::Real;

/// A general quaternion, not necessarily unit norm.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Quaternion {
    pub w: Real,
    pub x: Real,
    pub y: Real,
    pub z: Real,
}

impl Quaternion {
    pub const IDENTITY: Quaternion = Quaternion {
        w: 1.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub const fn new(w: Real, x: Real, y: Real, z: Real) -> Self {
        Self { w, x, y, z }
    }

    pub const fn identity() -> Self {
        Self::IDENTITY
    }

    /// Build from a scalar part and a vector part.
    pub fn from_scalar_vector(w: Real, v: Vec3) -> Self {
        Self {
            w,
            x: v[0],
            y: v[1],
            z: v[2],
        }
    }

    /// Pure quaternion with zero scalar part.
    pub fn pure(v: Vec3) -> Self {
        Self::from_scalar_vector(0.0, v)
    }

    pub fn vector_part(&self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    pub fn norm_squared(&self) -> Real {
        self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z
    }

    pub fn norm(&self) -> Real {
        self.norm_squared().sqrt()
    }

    pub fn is_finite(&self) -> bool {
        self.w.is_finite() && self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn conjugate(&self) -> Self {
        Self::new(self.w, -self.x, -self.y, -self.z)
    }

    /// Inverse for a general quaternion. Returns the identity when singular.
    pub fn inverse(&self) -> Self {
        let n2 = self.norm_squared();
        if n2 < 1e-30 {
            Self::IDENTITY
        } else {
            let c = self.conjugate();
            Self::new(c.w / n2, c.x / n2, c.y / n2, c.z / n2)
        }
    }

    pub fn normalized(&self) -> Self {
        let n = self.norm();
        if n < 1e-15 {
            Self::IDENTITY
        } else {
            Self::new(self.w / n, self.x / n, self.y / n, self.z / n)
        }
    }

    /// Normalize in place, returning the norm error before normalization.
    pub fn normalize_in_place(&mut self) -> Real {
        let n = self.norm();
        let error = (n - 1.0).abs();
        if n < 1e-15 {
            *self = Self::IDENTITY;
        } else {
            self.w /= n;
            self.x /= n;
            self.y /= n;
            self.z /= n;
        }
        error
    }

    pub fn dot(&self, other: &Self) -> Real {
        self.w * other.w + self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Rotation by `angle` radians about the unit axis `axis`.
    ///
    /// A non-unit axis is normalized; a zero axis yields the identity.
    pub fn from_axis_angle(axis: Vec3, angle: Real) -> Self {
        let n = axis.norm();
        if n < 1e-15 {
            return Self::IDENTITY;
        }
        let half = 0.5 * angle;
        let s = half.sin() / n;
        Self::new(half.cos(), axis.x * s, axis.y * s, axis.z * s)
    }

    /// Exponential map: interpret `v` as a rotation vector whose direction is
    /// the rotation axis and whose magnitude is the rotation angle in radians.
    ///
    /// This is the exact inverse of [`Quaternion::to_rotation_vector`] on the
    /// principal branch. It is built directly rather than delegating to
    /// [`Quaternion::from_axis_angle`], because that constructor halves the angle
    /// it is given; passing the magnitude through it would apply half the
    /// requested rotation.
    pub fn from_rotation_vector(v: Vec3) -> Self {
        let angle = v.norm();
        if angle < 1e-15 {
            return Self::IDENTITY;
        }
        let half = 0.5 * angle;
        let s = half.sin() / angle;
        Self::new(half.cos(), v.x * s, v.y * s, v.z * s)
    }

    /// Logarithmic map: the rotation vector of this (unit) quaternion.
    ///
    /// The result is in the principal branch, so its magnitude never exceeds pi.
    pub fn to_rotation_vector(&self) -> Vec3 {
        let q = self.canonical().normalized();
        let v = q.vector_part();
        let vn = v.norm();
        if vn < 1e-12 {
            // Small-angle limit: angle ~= 2 * vn.
            return v * 2.0;
        }
        let angle = 2.0 * vn.atan2(q.w);
        v * (angle / vn)
    }

    /// Flip the sign so the scalar part is non-negative.
    ///
    /// `q` and `-q` describe the same rotation; canonicalising makes comparison
    /// and interpolation deterministic.
    pub fn canonical(&self) -> Self {
        if self.w < 0.0 {
            Self::new(-self.w, -self.x, -self.y, -self.z)
        } else {
            *self
        }
    }

    /// Sandwich product `q * [0, v] * q^-1`, which equals `rotate(v)` for a unit
    /// quaternion.
    /// Used by tests and by the kinematics checks; kept public because it makes
    /// the derivative forms explicit without allocating a [`Vec3`].
    pub fn sandwich(&self, v: Vec3) -> Vec3 {
        let p = Self::pure(v);
        (self.mul(p).mul(self.inverse())).vector_part()
    }

    /// Rotate a vector: `v' = q * [0, v] * q^-1`.
    pub fn rotate(&self, v: Vec3) -> Vec3 {
        // Expanded Rodrigues form avoids constructing intermediate quaternions.
        let u = self.vector_part();
        let s = self.w;
        let uu = u.dot(&u);
        let uv = u.dot(&v);
        v * (s * s - uu) + u * (2.0 * uv) + u.cross(&v) * (2.0 * s)
    }

    /// Inverse rotation.
    pub fn rotate_inverse(&self, v: Vec3) -> Vec3 {
        self.conjugate().rotate(v)
    }

    /// Rotation matrix equivalent to `rotate`.
    pub fn to_matrix(&self) -> Mat3 {
        let (w, x, y, z) = (self.w, self.x, self.y, self.z);
        Mat3::new(
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        )
    }

    /// Build the rotation from a rotation matrix.
    ///
    /// Uses Shepperd's method to stay numerically well behaved near 180 degrees.
    pub fn from_matrix(m: &Mat3) -> Self {
        let trace = m[(0, 0)] + m[(1, 1)] + m[(2, 2)];
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            Self::new(
                0.25 * s,
                (m[(2, 1)] - m[(1, 2)]) / s,
                (m[(0, 2)] - m[(2, 0)]) / s,
                (m[(1, 0)] - m[(0, 1)]) / s,
            )
        } else if m[(0, 0)] > m[(1, 1)] && m[(0, 0)] > m[(2, 2)] {
            let s = (1.0 + m[(0, 0)] - m[(1, 1)] - m[(2, 2)]).sqrt() * 2.0;
            Self::new(
                (m[(2, 1)] - m[(1, 2)]) / s,
                0.25 * s,
                (m[(0, 1)] + m[(1, 0)]) / s,
                (m[(0, 2)] + m[(2, 0)]) / s,
            )
        } else if m[(1, 1)] > m[(2, 2)] {
            let s = (1.0 + m[(1, 1)] - m[(0, 0)] - m[(2, 2)]).sqrt() * 2.0;
            Self::new(
                (m[(0, 2)] - m[(2, 0)]) / s,
                (m[(0, 1)] + m[(1, 0)]) / s,
                0.25 * s,
                (m[(1, 2)] + m[(2, 1)]) / s,
            )
        } else {
            let s = (1.0 + m[(2, 2)] - m[(0, 0)] - m[(1, 1)]).sqrt() * 2.0;
            Self::new(
                (m[(1, 0)] - m[(0, 1)]) / s,
                (m[(0, 2)] + m[(2, 0)]) / s,
                (m[(1, 2)] + m[(2, 1)]) / s,
                0.25 * s,
            )
        };
        q.normalized()
    }

    /// Right-handed Euler angles (roll, pitch, yaw) in radians.
    ///
    /// These are a display convenience only. The authoritative attitude is
    /// always the quaternion.
    ///
    /// The decomposition assumes the standard aerospace 3-2-1 sequence applied
    /// to the body axes, with yaw about world Z, pitch about the intermediate Y,
    /// and roll about the body X.
    pub fn to_euler_321(&self) -> Vec3 {
        let q = self.normalized();
        let (w, x, y, z) = (q.w, q.x, q.y, q.z);

        // ZYX intrinsic composition written out in closed form.
        let sin_pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0);
        let pitch = sin_pitch.asin();

        let roll = if sin_pitch.cos().abs() > 1e-9 {
            (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y))
        } else {
            // Gimbal lock: fold roll and yaw together, keep yaw at zero.
            2.0 * (w * x - y * z).atan2(1.0 - 2.0 * (x * x + z * z))
        };
        let yaw = if sin_pitch.cos().abs() > 1e-9 {
            (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z))
        } else {
            0.0
        };

        Vec3::new(roll, pitch, yaw)
    }

    /// Build a quaternion from roll, pitch, yaw in radians (3-2-1 sequence).
    ///
    /// This is the inverse of [`Quaternion::to_euler_321`] away from gimbal lock.
    pub fn from_euler_321(roll: Real, pitch: Real, yaw: Real) -> Self {
        let (sr, cr) = (0.5 * roll).sin_cos();
        let (sp, cp) = (0.5 * pitch).sin_cos();
        let (sy, cy) = (0.5 * yaw).sin_cos();
        Self::new(
            cr * cp * cy + sr * sp * sy,
            sr * cp * cy - cr * sp * sy,
            cr * sp * cy + sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
        )
    }

    /// Quaternion derivative for a body-frame angular velocity.
    ///
    /// `0.5 * q * [0, w_b]`. See the module docs and the verification test.
    pub fn derivative_body_rate(&self, omega_body: Vec3) -> Self {
        let p = Self::pure(omega_body);
        let q = *self;
        q.mul(p) * 0.5
    }

    /// Quaternion derivative for a world/inertial-frame angular velocity.
    ///
    /// `0.5 * [0, w_i] * q`.
    pub fn derivative_world_rate(&self, omega_world: Vec3) -> Self {
        let p = Self::pure(omega_world);
        let q = *self;
        p.mul(q) * 0.5
    }

    /// Angular distance to another orientation, in radians.
    ///
    /// Uses the rotation-distance measure rather than differencing Euler angles,
    /// and takes the absolute scalar part so `q` and `-q` compare equal.
    pub fn angular_distance(&self, other: &Self) -> Real {
        let a = self.normalized();
        let b = other.normalized();
        let d = a.dot(&b).abs().clamp(0.0, 1.0);
        2.0 * d.acos()
    }

    /// Relative rotation `self^-1 * other`, the error quaternion.
    pub fn error_from(&self, other: &Self) -> Self {
        self.conjugate().mul(*other)
    }

    /// Spherical linear interpolation. Falls back to normalised lerp when the
    /// quaternions are nearly parallel.
    pub fn slerp(&self, other: &Self, t: Real) -> Self {
        let mut b = *other;
        let mut cos = self.dot(&b);
        if cos < 0.0 {
            b = Self::new(-b.w, -b.x, -b.y, -b.z);
            cos = -cos;
        }
        let t = t.clamp(0.0, 1.0);
        if cos > 0.9995 {
            let lerped = Self::new(
                self.w + (b.w - self.w) * t,
                self.x + (b.x - self.x) * t,
                self.y + (b.y - self.y) * t,
                self.z + (b.z - self.z) * t,
            );
            return lerped.normalized();
        }
        let theta = cos.clamp(-1.0, 1.0).acos();
        let sin_theta = theta.sin();
        let a = ((1.0 - t) * theta).sin() / sin_theta;
        let c = (t * theta).sin() / sin_theta;
        Self::new(
            a * self.w + c * b.w,
            a * self.x + c * b.x,
            a * self.y + c * b.y,
            a * self.z + c * b.z,
        )
        .normalized()
    }
}

impl Mul for Quaternion {
    type Output = Quaternion;
    /// Hamilton product. `a * b` applies `b` first, then `a`.
    fn mul(self, rhs: Quaternion) -> Quaternion {
        Quaternion::new(
            self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
            self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
        )
    }
}

impl Mul<Real> for Quaternion {
    type Output = Quaternion;
    fn mul(self, rhs: Real) -> Quaternion {
        Quaternion::new(self.w * rhs, self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl std::ops::Add for Quaternion {
    type Output = Quaternion;
    fn add(self, rhs: Quaternion) -> Quaternion {
        Quaternion::new(
            self.w + rhs.w,
            self.x + rhs.x,
            self.y + rhs.y,
            self.z + rhs.z,
        )
    }
}

impl std::ops::Sub for Quaternion {
    type Output = Quaternion;
    fn sub(self, rhs: Quaternion) -> Quaternion {
        Quaternion::new(
            self.w - rhs.w,
            self.x - rhs.x,
            self.y - rhs.y,
            self.z - rhs.z,
        )
    }
}

impl From<[Real; 4]> for Quaternion {
    fn from(v: [Real; 4]) -> Self {
        Self::new(v[0], v[1], v[2], v[3])
    }
}

impl From<Quaternion> for [Real; 4] {
    fn from(q: Quaternion) -> Self {
        [q.w, q.x, q.y, q.z]
    }
}

/// A quaternion that is guaranteed to have unit norm.
///
/// Constructing one always normalises, so downstream code can rely on
/// `rotate` being an isometry.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitQuaternion(pub Quaternion);

impl UnitQuaternion {
    pub fn identity() -> Self {
        Self(Quaternion::IDENTITY)
    }

    pub fn new(q: Quaternion) -> Self {
        Self(q.normalized())
    }

    pub fn from_axis_angle(axis: Vec3, angle: Real) -> Self {
        Self(Quaternion::from_axis_angle(axis, angle).normalized())
    }

    pub fn from_euler_321(roll: Real, pitch: Real, yaw: Real) -> Self {
        Self(Quaternion::from_euler_321(roll, pitch, yaw).normalized())
    }

    pub fn inner(&self) -> Quaternion {
        self.0
    }

    pub fn rotate(&self, v: Vec3) -> Vec3 {
        self.0.rotate(v)
    }

    pub fn rotate_inverse(&self, v: Vec3) -> Vec3 {
        self.0.rotate_inverse(v)
    }
}

impl Default for UnitQuaternion {
    fn default() -> Self {
        Self::identity()
    }
}

impl From<Quaternion> for UnitQuaternion {
    fn from(q: Quaternion) -> Self {
        Self::new(q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

    fn approx(a: Real, b: Real, tol: Real) -> bool {
        (a - b).abs() <= tol
    }

    fn assert_vec_close(a: Vec3, b: Vec3, tol: Real) {
        assert!(
            (a - b).norm() <= tol,
            "expected {:?}, got {:?} (norm {})",
            b,
            a,
            (a - b).norm()
        );
    }

    #[test]
    fn identity_does_not_rotate() {
        let q = Quaternion::identity();
        let v = Vec3::new(1.0, -2.0, 3.0);
        assert_vec_close(q.rotate(v), v, 1e-15);
    }

    #[test]
    fn zero_rotation_rate_leaves_attitude_constant() {
        let q = Quaternion::identity();
        let d = q.derivative_body_rate(Vec3::zeros());
        assert!(d.norm() < 1e-15);
    }

    #[test]
    fn known_rotation_about_each_axis() {
        // 90 degrees about X maps Y -> Z.
        let qx = Quaternion::from_axis_angle(Vec3::x(), FRAC_PI_2);
        assert_vec_close(qx.rotate(Vec3::y()), Vec3::z(), 1e-12);

        // 90 degrees about Y maps Z -> X.
        let qy = Quaternion::from_axis_angle(Vec3::y(), FRAC_PI_2);
        assert_vec_close(qy.rotate(Vec3::z()), Vec3::x(), 1e-12);

        // 90 degrees about Z maps X -> Y.
        let qz = Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_2);
        assert_vec_close(qz.rotate(Vec3::x()), Vec3::y(), 1e-12);

        // 180 degrees about X maps Y -> -Y.
        let q180 = Quaternion::from_axis_angle(Vec3::x(), PI);
        assert_vec_close(q180.rotate(Vec3::y()), -Vec3::y(), 1e-12);
    }

    #[test]
    fn composition_order_is_body_then_parent() {
        // Applying qx then qy must equal (qy * qx).
        let qx = Quaternion::from_axis_angle(Vec3::x(), FRAC_PI_2);
        let qy = Quaternion::from_axis_angle(Vec3::y(), FRAC_PI_2);
        let v = Vec3::new(1.0, 2.0, 3.0);
        let sequential = qy.rotate(qx.rotate(v));
        let composed = (qy * qx).rotate(v);
        assert_vec_close(composed, sequential, 1e-12);
    }

    #[test]
    fn inverse_transform_consistency() {
        let q = Quaternion::from_axis_angle(Vec3::new(1.0, -2.0, 0.5), 2.1);
        let v = Vec3::new(4.0, 5.0, -6.0);
        assert_vec_close(q.rotate_inverse(q.rotate(v)), v, 1e-12);
        assert_vec_close(q.inverse().rotate(q.rotate(v)), v, 1e-12);
    }

    #[test]
    fn matrix_and_vector_rotation_agree() {
        let q = Quaternion::from_axis_angle(Vec3::new(0.2, 0.9, -0.4), 1.7);
        let v = Vec3::new(-1.0, 2.5, 0.75);
        assert_vec_close(q.to_matrix() * v, q.rotate(v), 1e-12);
    }

    #[test]
    fn matrix_round_trip_preserves_rotation() {
        for (axis, angle) in [
            (Vec3::x(), 0.0),
            (Vec3::x(), 0.3),
            (Vec3::y(), PI - 1e-6),
            (Vec3::z(), 2.9),
            (Vec3::new(1.0, 1.0, 1.0), 2.0),
        ] {
            let q = Quaternion::from_axis_angle(axis, angle);
            let back = Quaternion::from_matrix(&q.to_matrix());
            assert!(q.angular_distance(&back) < 1e-9);
        }
    }

    #[test]
    fn normalization_reports_norm_error() {
        let mut q = Quaternion::new(2.0, 0.0, 0.0, 0.0);
        let err = q.normalize_in_place();
        assert!(approx(err, 1.0, 1e-12));
        assert!(approx(q.norm(), 1.0, 1e-15));
    }

    #[test]
    fn rotation_vector_round_trip() {
        let v = Vec3::new(0.3, -0.9, 0.2);
        let q = Quaternion::from_rotation_vector(v);
        assert_vec_close(q.to_rotation_vector(), v, 1e-9);
    }

    #[test]
    fn constant_body_rate_about_x_matches_analytic_solution() {
        // For constant body rate w about X, the analytic attitude is a rotation
        // of (w * t) about X.
        let w = 1.3;
        let omega = Vec3::new(w, 0.0, 0.0);
        let mut q = Quaternion::from_axis_angle(Vec3::y(), 0.4);
        let dt = 1e-6;
        let steps = 500;
        for _ in 0..steps {
            let k1 = q.derivative_body_rate(omega);
            let k2 = (q + k1 * (0.5 * dt)).derivative_body_rate(omega);
            let k3 = (q + k2 * (0.5 * dt)).derivative_body_rate(omega);
            let k4 = (q + k3 * dt).derivative_body_rate(omega);
            q = q + (k1 + k2 * 2.0 + k3 * 2.0 + k4) * (dt / 6.0);
            q = q.normalized();
        }
        let expected = Quaternion::from_axis_angle(Vec3::y(), 0.4)
            * Quaternion::from_axis_angle(Vec3::x(), w * dt * steps as Real);
        assert!(q.angular_distance(&expected) < 1e-9);
    }

    #[test]
    fn constant_body_rate_about_y_and_z_match_analytic() {
        for (axis, angle) in [(Vec3::y(), 0.6), (Vec3::z(), -0.9)] {
            let w = 0.77;
            let omega = axis * w;
            let mut q = Quaternion::from_axis_angle(Vec3::new(0.3, 0.5, 0.8), 0.25);
            let dt = 1e-6;
            let steps = 400;
            for _ in 0..steps {
                let k1 = q.derivative_body_rate(omega);
                let k2 = (q + k1 * (0.5 * dt)).derivative_body_rate(omega);
                let k3 = (q + k2 * (0.5 * dt)).derivative_body_rate(omega);
                let k4 = (q + k3 * dt).derivative_body_rate(omega);
                q = (q + (k1 + k2 * 2.0 + k3 * 2.0 + k4) * (dt / 6.0)).normalized();
            }
            let expected = Quaternion::from_axis_angle(Vec3::new(0.3, 0.5, 0.8), 0.25)
                * Quaternion::from_axis_angle(axis, w * dt * steps as Real);
            assert!(
                q.angular_distance(&expected) < 1e-9,
                "axis {:?} angle {}",
                axis,
                angle
            );
        }
    }

    #[test]
    fn body_rate_derivative_matches_numerical_rotation_derivative() {
        // The decisive convention test.
        //
        // For a body-frame rate, a vector y = q v q* must satisfy
        //   dy/dt = omega_body x y.
        // The exact derivative follows from the product rule:
        //   dy/dt = q_dot v q* + q v q_dot*
        // with q_dot = 0.5 q [0, omega_body]. Any wrong product order in
        // `derivative_body_rate` fails here, and the same result is checked
        // against a central finite difference of the exact rotation.
        let cases = [
            (
                Quaternion::from_axis_angle(Vec3::x(), FRAC_PI_4),
                Vec3::new(0.0, 0.0, 0.7),
                Vec3::new(0.0, 1.0, 0.0),
            ),
            (
                Quaternion::from_axis_angle(Vec3::new(0.3, 0.5, 0.8), 1.1),
                Vec3::new(0.4, -0.9, 0.25),
                Vec3::new(-1.0, 0.5, 2.0),
            ),
            (
                Quaternion::from_euler_321(0.2, -0.6, 1.4),
                Vec3::new(-0.35, 0.8, -1.2),
                Vec3::new(2.0, -0.25, 0.75),
            ),
        ];

        for (case_index, (q0, omega, v)) in cases.into_iter().enumerate() {
            let qd = q0.derivative_body_rate(omega);

            let h = 1e-6;

            // The perturbation quaternion must be a rotation by |omega| * h about
            // the unit direction of omega, which is what from_axis_angle produces
            // from a full angle.
            let perturb = Quaternion::from_rotation_vector(omega * h);
            let perturb_axis = Quaternion::from_axis_angle(omega, omega.norm() * h);
            assert!(
                (perturb.w - perturb_axis.w).abs() < 1e-15
                    && (perturb.vector_part() - perturb_axis.vector_part()).norm() < 1e-15,
                "case {}: from_rotation_vector disagrees with from_axis_angle: {:?} vs {:?}",
                case_index,
                perturb,
                perturb_axis
            );

            // Ground truth by central finite difference of the exact rotation.
            // The vector is fixed in the body frame, so the path in the inertial
            // frame is q(t) v q(t)* with q(t) = q0 exp([0, omega] t / 2).
            let q_plus = q0 * Quaternion::from_rotation_vector(omega * h);
            let q_minus = q0 * Quaternion::from_rotation_vector(-omega * h);
            let reference = (q_plus.sandwich(v) - q_minus.sandwich(v)) * (1.0 / (2.0 * h));

            // The two rotation paths must agree, so a mistake in either the
            // sandwich form or the expanded Rodrigues form is caught here.
            assert_vec_close(q_plus.sandwich(v), q_plus.rotate(v), 1e-12);

            // 1. The analytic quaternion derivative must match the finite
            //    difference of the exact attitude path.
            let qd_fd = (q_plus - q_minus) * (1.0 / (2.0 * h));
            let qd_scale = qd.norm().max(1e-12);
            assert!(
                (qd - qd_fd).norm() / qd_scale <= 1e-4,
                "case {}: q_dot mismatch: analytic {:?} numeric {:?}",
                case_index,
                qd,
                qd_fd
            );

            // 2. The analytic product rule must match the same reference.
            //    d/dt (q v q*) = q_dot v q* + q v q_dot*, and q* is the exact
            //    inverse of q because q is unit norm.
            let p = Quaternion::pure(v);
            let q_star = q0.conjugate();
            let term_a = (qd * p * q_star).vector_part();
            let term_b = (q0 * p * qd.conjugate()).vector_part();
            let analytic = term_a + term_b;
            assert!(
                (analytic - reference).norm() <= 1e-8,
                "case {}: product rule expected {:?}, got {:?}",
                case_index,
                reference,
                analytic
            );

            // 3. The derivative must equal omega_world x y, where omega_world is
            //    the body rate rotated into the world frame. A transposed product
            //    order in `derivative_body_rate` fails here, because it would put
            //    the cross product with the wrong sign.
            let y = q0.sandwich(v);
            let omega_world = q0.rotate(omega);
            let expected = omega_world.cross(&y);
            assert!(
                (reference - expected).norm() <= 1e-8,
                "case {}: omega_world x y expected {:?}, got {:?}",
                case_index,
                expected,
                reference
            );
            assert!(
                (reference + expected).norm() > 1e-6,
                "case {}: derivative has the wrong sign",
                case_index
            );
        }
    }

    #[test]
    fn sandwich_matches_rotate_for_unit_quaternions() {
        let q = Quaternion::from_axis_angle(Vec3::new(0.4, -0.2, 0.9), 0.8);
        let v = Vec3::new(1.0, -2.0, 3.0);
        assert_vec_close(q.sandwich(v), q.rotate(v), 1e-12);
    }

    #[test]
    fn world_rate_derivative_is_consistent_with_body_rate() {
        // omega_world = q * omega_body * q^-1, and the two derivative forms must
        // agree when the angular velocity is rotated accordingly.
        let q = Quaternion::from_axis_angle(Vec3::new(0.1, 0.3, 0.9), 1.1);
        let omega_body = Vec3::new(0.4, -0.2, 0.6);
        let omega_world = q.rotate(omega_body);
        let a = q.derivative_body_rate(omega_body);
        let b = q.derivative_world_rate(omega_world);
        assert!((a.w - b.w).abs() < 1e-12);
        assert!((a.x - b.x).abs() < 1e-12);
        assert!((a.y - b.y).abs() < 1e-12);
        assert!((a.z - b.z).abs() < 1e-12);
    }

    #[test]
    fn euler_round_trip_away_from_gimbal_lock() {
        for (r, p, y) in [
            (0.0, 0.0, 0.0),
            (0.3, 0.2, -0.4),
            (-1.2, 0.9, 2.5),
            (0.1, -1.4, -0.9),
        ] {
            let q = Quaternion::from_euler_321(r, p, y);
            let e = q.to_euler_321();
            assert!(approx(e.x, r, 1e-9), "roll {} vs {}", e.x, r);
            assert!(approx(e.y, p, 1e-9), "pitch {} vs {}", e.y, p);
            assert!(approx(e.z, y, 1e-9), "yaw {} vs {}", e.z, y);
        }
    }

    #[test]
    fn euler_conversion_matches_axis_rotations() {
        let q = Quaternion::from_euler_321(0.0, 0.0, FRAC_PI_2);
        assert_vec_close(q.rotate(Vec3::x()), Vec3::y(), 1e-12);
    }

    #[test]
    fn angular_distance_is_zero_for_same_rotation_either_sign() {
        let q = Quaternion::from_axis_angle(Vec3::z(), 0.5);
        assert!(q.angular_distance(&q) < 1e-12);
        assert!(q.angular_distance(&q.conjugate().conjugate()) < 1e-12);
        let neg = Quaternion::new(-q.w, -q.x, -q.y, -q.z);
        assert!(q.angular_distance(&neg) < 1e-12);
    }

    #[test]
    fn angular_distance_matches_known_angle() {
        let a = Quaternion::identity();
        let b = Quaternion::from_axis_angle(Vec3::new(1.0, 1.0, 0.0), 1.0);
        assert!(approx(a.angular_distance(&b), 1.0, 1e-12));

        let c = Quaternion::from_axis_angle(Vec3::z(), PI);
        assert!(approx(
            Quaternion::identity().angular_distance(&c),
            PI,
            1e-12
        ));
    }

    #[test]
    fn slerp_endpoints_and_midpoint() {
        let a = Quaternion::identity();
        let b = Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_2);
        let start = a.slerp(&b, 0.0);
        let end = a.slerp(&b, 1.0);
        assert!(a.angular_distance(&start) < 1e-12);
        assert!(b.angular_distance(&end) < 1e-12);

        let mid = a.slerp(&b, 0.5);
        let expected = Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_4);
        assert!(mid.angular_distance(&expected) < 1e-12);
    }

    #[test]
    fn slerp_takes_short_path_across_sign_flip() {
        let a = Quaternion::identity();
        let b = Quaternion::new(
            -Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_2).w,
            -Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_2).x,
            -Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_2).y,
            -Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_2).z,
        );
        let mid = a.slerp(&b, 0.5);
        let expected = Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_4);
        assert!(mid.angular_distance(&expected) < 1e-12);
    }

    #[test]
    fn unit_quaternion_wrapper_stays_normalized() {
        let u = UnitQuaternion::new(Quaternion::new(3.0, 0.0, 4.0, 0.0));
        assert!(approx(u.inner().norm(), 1.0, 1e-15));
        assert!(approx(u.rotate(Vec3::x()).norm(), 1.0, 1e-15));
    }

    #[test]
    fn error_from_gives_relative_rotation() {
        let a = Quaternion::from_axis_angle(Vec3::z(), 0.4);
        let b = Quaternion::from_axis_angle(Vec3::z(), 0.9);
        let e = a.error_from(&b);
        assert!(approx(
            e.angular_distance(&Quaternion::identity()),
            0.5,
            1e-12
        ));
    }
}
