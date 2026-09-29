//! Mass properties: mass, centre of gravity, and the inertia tensor.

use serde::{Deserialize, Serialize};

use crate::vectors::{Mat3, Vec3};
use crate::Real;

/// A symmetric 3x3 inertia tensor expressed in a body frame.
///
/// The tensor is stored as six independent components. An importer that reads a
/// full 3x3 matrix must symmetrise explicitly and report the asymmetry through
/// [`InertiaTensor::asymmetry`] so a frame mistake is visible rather than
/// silently absorbed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InertiaTensor {
    pub ixx: Real,
    pub iyy: Real,
    pub izz: Real,
    pub ixy: Real,
    pub ixz: Real,
    pub iyz: Real,
}

impl Default for InertiaTensor {
    fn default() -> Self {
        Self::diagonal(1.0, 1.0, 1.0)
    }
}

impl InertiaTensor {
    pub const fn diagonal(ixx: Real, iyy: Real, izz: Real) -> Self {
        Self {
            ixx,
            iyy,
            izz,
            ixy: 0.0,
            ixz: 0.0,
            iyz: 0.0,
        }
    }

    pub const fn new(ixx: Real, iyy: Real, izz: Real, ixy: Real, ixz: Real, iyz: Real) -> Self {
        Self {
            ixx,
            iyy,
            izz,
            ixy,
            ixz,
            iyz,
        }
    }

    /// Read the symmetric lower/upper average of a full 3x3 matrix.
    pub fn from_matrix(m: &Mat3) -> Self {
        Self {
            ixx: m[(0, 0)],
            iyy: m[(1, 1)],
            izz: m[(2, 2)],
            ixy: 0.5 * (m[(0, 1)] + m[(1, 0)]),
            ixz: 0.5 * (m[(0, 2)] + m[(2, 0)]),
            iyz: 0.5 * (m[(1, 2)] + m[(2, 1)]),
        }
    }

    pub fn to_matrix(&self) -> Mat3 {
        Mat3::new(
            self.ixx, self.ixy, self.ixz, self.ixy, self.iyy, self.iyz, self.ixz, self.iyz,
            self.izz,
        )
    }

    /// Largest absolute difference between mirrored entries of a full matrix.
    pub fn asymmetry(m: &Mat3) -> Real {
        let mut worst: Real = 0.0;
        for i in 0..3 {
            for j in 0..3 {
                worst = worst.max((m[(i, j)] - m[(j, i)]).abs());
            }
        }
        worst
    }

    /// All six components are finite.
    pub fn all_finite(&self) -> bool {
        [self.ixx, self.iyy, self.izz, self.ixy, self.ixz, self.iyz]
            .iter()
            .all(|v| v.is_finite())
    }

    pub fn determinant(&self) -> Real {
        self.to_matrix().determinant()
    }

    /// Sylvester's criterion for a symmetric matrix.
    pub fn is_positive_definite(&self) -> bool {
        let m = self.to_matrix();
        if m[(0, 0)] <= 0.0 {
            return false;
        }
        let d2 = m[(0, 0)] * m[(1, 1)] - m[(0, 1)] * m[(1, 0)];
        if d2 <= 0.0 {
            return false;
        }
        m.determinant() > 0.0
    }

    /// Principal moments, sorted ascending.
    pub fn principal_moments(&self) -> [Real; 3] {
        let eig = self.to_matrix().symmetric_eigen();
        let mut values = [eig.eigenvalues[0], eig.eigenvalues[1], eig.eigenvalues[2]];
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        values
    }

    /// Triangle inequalities satisfied by every real rigid body.
    ///
    /// Returns the labels of the inequalities that are violated.
    pub fn triangle_violations(&self, rel_tol: Real) -> Vec<String> {
        let [a, b, c] = self.principal_moments();
        let scale = a.abs().max(b.abs()).max(c.abs()).max(1e-30);
        let tol = rel_tol * scale;
        let mut out = Vec::new();
        for (p, q, r, label) in [
            (a, b, c, "Ix + Iy >= Iz"),
            (a, c, b, "Ix + Iz >= Iy"),
            (b, c, a, "Iy + Iz >= Ix"),
        ] {
            if p + q < r - tol {
                out.push(label.to_string());
            }
        }
        out
    }

    /// Solve `I * x = b`, returning `None` when the tensor is singular.
    pub fn solve(&self, b: Vec3) -> Option<Vec3> {
        self.to_matrix().lu().solve(&b)
    }

    /// Largest principal moment, used to set stability thresholds.
    pub fn max_moment(&self) -> Real {
        self.principal_moments()[2]
    }
}

/// Mass properties at one instant, referenced to a body-frame point.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MassProperties {
    /// Total mass in kilograms.
    pub mass: Real,
    /// Centre of gravity position in the body frame, in metres, measured from
    /// the body datum the model declares.
    pub center_of_gravity: Vec3,
    /// Inertia about the centre of gravity, in the body frame.
    pub inertia: InertiaTensor,
}

impl MassProperties {
    pub fn new(mass: Real, center_of_gravity: Vec3, inertia: InertiaTensor) -> Self {
        Self {
            mass,
            center_of_gravity,
            inertia,
        }
    }

    pub fn is_finite(&self) -> bool {
        self.mass.is_finite()
            && crate::vectors::is_finite(&self.center_of_gravity)
            && self.inertia.all_finite()
    }
}

/// A time-varying mass curve sampled from an imported model.
///
/// The MVP supports a piecewise-linear table. Values must be monotonic in time
/// and physically plausible; those checks live in the model validator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MassCurve {
    /// Sample times in seconds, strictly increasing.
    pub times: Vec<Real>,
    /// Mass in kilograms at each sample time.
    pub masses: Vec<Real>,
    /// Optional centre-of-gravity track, one entry per sample time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub centers_of_gravity: Vec<Vec3>,
    /// Optional inertia track, one entry per sample time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inertias: Vec<InertiaTensor>,
}

impl MassCurve {
    pub fn new(times: Vec<Real>, masses: Vec<Real>) -> Self {
        Self {
            times,
            masses,
            centers_of_gravity: Vec::new(),
            inertias: Vec::new(),
        }
    }

    /// Whether the curve has enough samples to be interpolated.
    pub fn is_usable(&self) -> bool {
        self.times.len() == self.masses.len() && self.times.len() >= 2
    }

    /// Linear interpolation of mass, clamped outside the sampled range.
    pub fn mass_at(&self, t: Real) -> Option<Real> {
        let n = self.times.len();
        if n == 0 || n != self.masses.len() {
            return None;
        }
        if n == 1 {
            return Some(self.masses[0]);
        }
        if t <= self.times[0] {
            return Some(self.masses[0]);
        }
        if t >= self.times[n - 1] {
            return Some(self.masses[n - 1]);
        }
        let idx = match self.times.partition_point(|x| *x <= t) {
            0 => 0,
            i if i >= n => n - 2,
            i => i - 1,
        };
        let t0 = self.times[idx];
        let t1 = self.times[idx + 1];
        let span = t1 - t0;
        if span <= 0.0 {
            return Some(self.masses[idx]);
        }
        let f = (t - t0) / span;
        Some(self.masses[idx] * (1.0 - f) + self.masses[idx + 1] * f)
    }

    /// Interpolate the centre of gravity when a track is present.
    pub fn cg_at(&self, t: Real) -> Option<Vec3> {
        self.interp_vec3(&self.centers_of_gravity, t)
    }

    /// Interpolate the inertia tensor when a track is present.
    pub fn inertia_at(&self, t: Real) -> Option<InertiaTensor> {
        let n = self.inertias.len();
        if n != self.times.len() || n == 0 {
            return None;
        }
        let (i, f) = self.bracket(t)?;
        if i + 1 >= n {
            return Some(self.inertias[n - 1]);
        }
        let a = self.inertias[i];
        let b = self.inertias[i + 1];
        let lerp = |x: Real, y: Real| x * (1.0 - f) + y * f;
        Some(InertiaTensor::new(
            lerp(a.ixx, b.ixx),
            lerp(a.iyy, b.iyy),
            lerp(a.izz, b.izz),
            lerp(a.ixy, b.ixy),
            lerp(a.ixz, b.ixz),
            lerp(a.iyz, b.iyz),
        ))
    }

    fn interp_vec3(&self, track: &[Vec3], t: Real) -> Option<Vec3> {
        if track.len() != self.times.len() || track.is_empty() {
            return None;
        }
        let (i, f) = self.bracket(t)?;
        if i + 1 >= track.len() {
            return Some(track[track.len() - 1]);
        }
        Some(track[i] * (1.0 - f) + track[i + 1] * f)
    }

    /// Return the lower sample index and the fraction into the interval.
    fn bracket(&self, t: Real) -> Option<(usize, Real)> {
        let n = self.times.len();
        if n < 2 {
            return None;
        }
        if t <= self.times[0] {
            return Some((0, 0.0));
        }
        if t >= self.times[n - 1] {
            return Some((n - 1, 0.0));
        }
        let idx = match self.times.partition_point(|x| *x <= t) {
            0 => 0,
            i if i >= n => n - 2,
            i => i - 1,
        };
        let span = self.times[idx + 1] - self.times[idx];
        let f = if span <= 0.0 {
            0.0
        } else {
            (t - self.times[idx]) / span
        };
        Some((idx, f))
    }

    /// Verify that time is strictly increasing and mass stays positive.
    ///
    /// Returns descriptions of each problem found.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.times.len() != self.masses.len() {
            problems.push(format!(
                "mass curve has {} time samples but {} mass samples",
                self.times.len(),
                self.masses.len()
            ));
            return problems;
        }
        if self.times.len() < 2 {
            problems.push("mass curve needs at least two samples".to_string());
            return problems;
        }
        for i in 1..self.times.len() {
            let previous = self.times[i - 1];
            let current = self.times[i];
            // A NaN time is rejected by the explicit check, because every
            // comparison against a NaN is false and it would slip through a
            // plain ordering test.
            if current.is_nan() || previous.is_nan() || current <= previous {
                problems.push(format!(
                    "mass curve time is not strictly increasing at index {}",
                    i
                ));
            }
        }
        for (i, m) in self.masses.iter().enumerate() {
            if !m.is_finite() || *m <= 0.0 {
                problems.push(format!("mass sample {} is not a positive finite value", i));
            }
        }
        for (i, cg) in self.centers_of_gravity.iter().enumerate() {
            if !crate::vectors::is_finite(cg) {
                problems.push(format!("centre of gravity sample {} is not finite", i));
            }
        }
        if !self.centers_of_gravity.is_empty() && self.centers_of_gravity.len() != self.times.len()
        {
            problems.push("centre of gravity track length does not match the time samples".into());
        }
        if !self.inertias.is_empty() && self.inertias.len() != self.times.len() {
            problems.push("inertia track length does not match the time samples".into());
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagonal_tensor_is_positive_definite() {
        let t = InertiaTensor::diagonal(2.0, 3.0, 4.0);
        assert!(t.is_positive_definite());
        assert!(t.triangle_violations(1e-9).is_empty());
        assert!((t.determinant() - 24.0).abs() < 1e-12);
    }

    #[test]
    fn non_positive_definite_tensor_is_detected() {
        assert!(!InertiaTensor::diagonal(1.0, -1.0, 2.0).is_positive_definite());
        assert!(!InertiaTensor::diagonal(-1.0, -1.0, -1.0).is_positive_definite());
    }

    #[test]
    fn triangle_inequality_violation_is_detected() {
        // Principal moments are (1, 1, 5), so only Ix + Iy >= Iz is violated.
        let t = InertiaTensor::diagonal(1.0, 1.0, 5.0);
        assert!(t.is_positive_definite());
        let v = t.triangle_violations(1e-9);
        assert_eq!(v.len(), 1, "violations: {:?}", v);
        assert!(v[0].contains("Ix + Iy"), "got {}", v[0]);
    }

    #[test]
    fn principal_moments_are_sorted() {
        let t = InertiaTensor::new(4.0, 2.0, 3.0, 0.0, 0.0, 0.0);
        let p = t.principal_moments();
        assert!((p[0] - 2.0).abs() < 1e-12);
        assert!((p[1] - 3.0).abs() < 1e-12);
        assert!((p[2] - 4.0).abs() < 1e-12);
    }

    #[test]
    fn principal_moments_are_rotation_invariant() {
        let base = InertiaTensor::diagonal(1.0, 2.0, 3.0);
        let r = crate::quaternion::Quaternion::from_axis_angle(Vec3::new(0.3, -0.7, 0.2), 0.9)
            .to_matrix();
        let rotated = InertiaTensor::from_matrix(&(r * base.to_matrix() * r.transpose()));
        let a = base.principal_moments();
        let b = rotated.principal_moments();
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < 1e-9);
        }
    }

    #[test]
    fn solve_recovers_angular_acceleration() {
        let t = InertiaTensor::diagonal(1.0, 2.0, 4.0);
        let torque = Vec3::new(1.0, 2.0, 4.0);
        let alpha = t.solve(torque).unwrap();
        assert!((alpha - Vec3::new(1.0, 1.0, 1.0)).norm() < 1e-12);
    }

    #[test]
    fn mass_curve_interpolates_linearly() {
        let c = MassCurve::new(vec![0.0, 1.0, 2.0], vec![10.0, 8.0, 6.0]);
        assert!(c.is_usable());
        assert!((c.mass_at(0.0).unwrap() - 10.0).abs() < 1e-12);
        assert!((c.mass_at(0.5).unwrap() - 9.0).abs() < 1e-12);
        assert!((c.mass_at(1.25).unwrap() - 7.5).abs() < 1e-12);
        assert!((c.mass_at(9.0).unwrap() - 6.0).abs() < 1e-12);
        assert!((c.mass_at(-3.0).unwrap() - 10.0).abs() < 1e-12);
    }

    #[test]
    fn mass_curve_validates_monotonic_time_and_positive_mass() {
        let good = MassCurve::new(vec![0.0, 1.0], vec![1.0, 0.5]);
        assert!(good.validate().is_empty());

        let bad = MassCurve::new(vec![0.0, 0.0], vec![1.0, -1.0]);
        let problems = bad.validate();
        assert!(problems.len() >= 2);
    }

    #[test]
    fn mass_curve_rejects_mismatched_lengths() {
        let bad = MassCurve::new(vec![0.0, 1.0, 2.0], vec![1.0]);
        assert!(!bad.is_usable());
        assert!(bad.mass_at(0.5).is_none());
        assert!(!bad.validate().is_empty());
    }

    #[test]
    fn mass_curve_interpolates_inertia_track() {
        let mut c = MassCurve::new(vec![0.0, 1.0], vec![2.0, 1.0]);
        c.inertias = vec![
            InertiaTensor::diagonal(1.0, 1.0, 1.0),
            InertiaTensor::diagonal(3.0, 3.0, 3.0),
        ];
        let i = c.inertia_at(0.5).unwrap();
        assert!((i.ixx - 2.0).abs() < 1e-12);
        assert!(c.validate().is_empty());
    }
}
