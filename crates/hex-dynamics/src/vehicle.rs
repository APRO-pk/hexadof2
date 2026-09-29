//! Vehicle mass properties and the model interface the dynamics core consumes.
//!
//! The dynamics engine never reads an imported file directly: it talks to
//! [`VehicleModel`]. That keeps the physics independent of the on-disk schema,
//! and lets tests build a vehicle from a few numbers.

use hex_core::mass::{InertiaTensor, MassCurve};
use hex_core::{Frame, Real, Vec3};
use serde::{Deserialize, Serialize};

/// Reference geometry and aerodynamic bookkeeping for a vehicle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ReferenceGeometry {
    /// Aerodynamic reference area in square metres.
    pub reference_area: Real,
    /// Aerodynamic reference length in metres, used for moment coefficients.
    pub reference_length: Real,
    /// Nominal body diameter in metres, used for visualisation only.
    pub body_diameter: Real,
}

impl Default for ReferenceGeometry {
    fn default() -> Self {
        Self {
            reference_area: 0.01,
            reference_length: 1.0,
            body_diameter: 0.1,
        }
    }
}

impl ReferenceGeometry {
    /// Reference area implied by a circular body diameter.
    pub fn from_diameter(diameter: Real) -> Self {
        Self {
            reference_area: std::f64::consts::PI * 0.25 * diameter * diameter,
            reference_length: diameter * 10.0,
            body_diameter: diameter,
        }
    }
}

/// A source of mass properties over time.
pub trait MassProvider: Send + Sync {
    /// Mass in kilograms at time `t`.
    fn mass(&self, t: Real) -> Real;

    /// Centre of gravity in the body frame at time `t`.
    fn center_of_gravity(&self, t: Real) -> Vec3;

    /// Inertia about the centre of gravity, in the body frame, at time `t`.
    fn inertia(&self, t: Real) -> InertiaTensor;

    /// Mass flow rate in kilograms per second at time `t`.
    ///
    /// The sign convention is the rate of change of vehicle mass, so a burning
    /// motor returns a negative value. The default treats mass as constant.
    fn mass_rate(&self, _t: Real) -> Real {
        0.0
    }

    /// True when the provider treats mass as a constant.
    fn is_constant(&self) -> bool {
        true
    }

    /// Human-readable description shown in the run metadata.
    fn describe(&self) -> String {
        "constant mass".to_string()
    }
}

/// Constant mass properties. The simplest and most common case.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ConstantMass {
    pub mass: Real,
    pub center_of_gravity: Vec3,
    pub inertia: InertiaTensor,
}

impl ConstantMass {
    pub fn new(mass: Real, center_of_gravity: Vec3, inertia: InertiaTensor) -> Self {
        Self {
            mass,
            center_of_gravity,
            inertia,
        }
    }

    /// A point mass with a diagonal inertia, convenient for tests.
    pub fn point(mass: Real, inertia: Real) -> Self {
        Self {
            mass,
            center_of_gravity: Vec3::zeros(),
            inertia: InertiaTensor::diagonal(inertia, inertia, inertia),
        }
    }
}

impl MassProvider for ConstantMass {
    fn mass(&self, _t: Real) -> Real {
        self.mass
    }

    fn center_of_gravity(&self, _t: Real) -> Vec3 {
        self.center_of_gravity
    }

    fn inertia(&self, _t: Real) -> InertiaTensor {
        self.inertia
    }

    fn describe(&self) -> String {
        format!("constant mass {:.4} kg", self.mass)
    }
}

/// Mass properties driven by a sampled mass curve, optionally with centre of
/// gravity and inertia tracks.
///
/// Outside the sampled range the first or last sample is held, and the mass rate
/// falls back to the nearest interval slope rather than being reported as zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveMass {
    pub curve: MassCurve,
    /// Inertia used before and after the track, or when no track is present.
    pub fallback_inertia: InertiaTensor,
    /// Centre of gravity used when the curve has no centre-of-gravity track.
    pub fallback_center_of_gravity: Vec3,
}

impl CurveMass {
    pub fn new(curve: MassCurve) -> Self {
        let last_inertia = curve
            .inertias
            .last()
            .copied()
            .unwrap_or(InertiaTensor::diagonal(1.0, 1.0, 1.0));
        let last_cg = curve
            .centers_of_gravity
            .last()
            .copied()
            .unwrap_or_else(Vec3::zeros);
        Self {
            curve,
            fallback_inertia: last_inertia,
            fallback_center_of_gravity: last_cg,
        }
    }

    /// Slope of the mass curve at time `t`, in kilograms per second.
    pub fn slope_at(&self, t: Real) -> Real {
        let times = &self.curve.times;
        let masses = &self.curve.masses;
        let n = times.len();
        if n < 2 || n != masses.len() {
            return 0.0;
        }
        let idx = if t <= times[0] {
            0
        } else if t >= times[n - 1] {
            n - 2
        } else {
            match times.partition_point(|x| *x <= t) {
                0 => 0,
                i if i >= n => n - 2,
                i => i - 1,
            }
        };
        let dt = times[idx + 1] - times[idx];
        if dt <= 0.0 {
            0.0
        } else {
            (masses[idx + 1] - masses[idx]) / dt
        }
    }
}

impl MassProvider for CurveMass {
    fn mass(&self, t: Real) -> Real {
        self.curve.mass_at(t).unwrap_or(1.0)
    }

    fn center_of_gravity(&self, t: Real) -> Vec3 {
        self.curve
            .cg_at(t)
            .unwrap_or(self.fallback_center_of_gravity)
    }

    fn inertia(&self, t: Real) -> InertiaTensor {
        self.curve.inertia_at(t).unwrap_or(self.fallback_inertia)
    }

    fn mass_rate(&self, t: Real) -> Real {
        self.slope_at(t)
    }

    fn is_constant(&self) -> bool {
        false
    }

    fn describe(&self) -> String {
        format!(
            "curve mass, {} samples, {:.4} kg to {:.4} kg",
            self.curve.times.len(),
            self.curve.masses.first().copied().unwrap_or(0.0),
            self.curve.masses.last().copied().unwrap_or(0.0)
        )
    }
}

/// A propellant mass flow profile supplied by the propulsion source.
///
/// Thrust and mass flow are usually supplied together. This type records that
/// pairing so the run metadata can state whether the two were consistent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MassFlowProfile {
    /// Sample times in seconds, strictly increasing.
    pub times: Vec<Real>,
    /// Mass flow rate in kilograms per second at each sample time, positive for
    /// propellant leaving the vehicle.
    pub flow_rates: Vec<Real>,
    /// Whether the propulsion source already accounted for this flow when
    /// producing its mass curve, so the integrator must not subtract it twice.
    pub already_in_mass_curve: bool,
}

impl MassFlowProfile {
    pub fn constant(flow_rate: Real) -> Self {
        Self {
            times: vec![0.0],
            flow_rates: vec![flow_rate],
            already_in_mass_curve: false,
        }
    }

    /// Interpolate the flow rate at time `t`, holding the end values.
    pub fn flow_at(&self, t: Real) -> Real {
        let n = self.times.len();
        if n == 0 || n != self.flow_rates.len() {
            return 0.0;
        }
        if n == 1 || t <= self.times[0] {
            return self.flow_rates[0];
        }
        if t >= self.times[n - 1] {
            return self.flow_rates[n - 1];
        }
        let idx = match self.times.partition_point(|x| *x <= t) {
            0 => 0,
            i if i >= n => n - 2,
            i => i - 1,
        };
        let dt = self.times[idx + 1] - self.times[idx];
        if dt <= 0.0 {
            self.flow_rates[idx]
        } else {
            let f = (t - self.times[idx]) / dt;
            self.flow_rates[idx] * (1.0 - f) + self.flow_rates[idx + 1] * f
        }
    }

    /// Net mass flow rate to apply, or zero when the mass curve already has it.
    pub fn effective_rate(&self, t: Real) -> Real {
        if self.already_in_mass_curve {
            0.0
        } else {
            self.flow_at(t)
        }
    }
}

/// Everything the dynamics engine needs to know about the vehicle.
pub trait VehicleModel: Send + Sync {
    /// Stable identifier from the imported model.
    fn model_id(&self) -> &str;

    /// Version string from the imported model.
    fn model_version(&self) -> &str;

    /// Human-readable name for the UI.
    fn display_name(&self) -> &str;

    /// The declared body and world frame conventions.
    fn frame(&self) -> Frame;

    /// Reference geometry for aerodynamic coefficients.
    fn reference_geometry(&self) -> ReferenceGeometry;

    /// Mass properties over time.
    fn mass_provider(&self) -> &dyn MassProvider;

    /// Optional display mesh reference.
    fn mesh_reference(&self) -> Option<&str> {
        None
    }

    /// Application points of the declared thrust source, relative to the centre
    /// of gravity, expressed in the body frame.
    fn thrust_application_point(&self) -> Vec3 {
        Vec3::zeros()
    }

    /// A short summary used in run metadata and the model status card.
    fn summary(&self) -> String {
        format!(
            "{} v{} ({}), mass {}",
            self.display_name(),
            self.model_version(),
            self.model_id(),
            self.mass_provider().describe()
        )
    }
}

/// A concrete vehicle assembled from an imported model or built in a test.
///
/// The mass provider is a trait object, so this type cannot derive `Clone` or
/// `PartialEq`. Run metadata is compared through the summary fields instead.
pub struct Vehicle {
    pub model_id: String,
    pub model_version: String,
    pub display_name: String,
    pub frame: Frame,
    pub reference_geometry: ReferenceGeometry,
    pub mass: Box<dyn MassProvider>,
    pub mesh_reference: Option<String>,
    pub thrust_application_point: Vec3,
}

impl Vehicle {
    /// A simple vehicle with constant mass, for tests and quick scenarios.
    pub fn simple(mass: Real, inertia: Real) -> Self {
        Self {
            model_id: "builtin.simple".to_string(),
            model_version: "1.0".to_string(),
            display_name: "Simple test vehicle".to_string(),
            frame: Frame::default(),
            reference_geometry: ReferenceGeometry::default(),
            mass: Box::new(ConstantMass::point(mass, inertia)),
            mesh_reference: None,
            thrust_application_point: Vec3::zeros(),
        }
    }

    pub fn with_mass(mut self, mass: Box<dyn MassProvider>) -> Self {
        self.mass = mass;
        self
    }
}

impl std::fmt::Debug for Vehicle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vehicle")
            .field("model_id", &self.model_id)
            .field("model_version", &self.model_version)
            .field("display_name", &self.display_name)
            .field("mass", &self.mass.describe())
            .finish()
    }
}

impl VehicleModel for Vehicle {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn model_version(&self) -> &str {
        &self.model_version
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn frame(&self) -> Frame {
        self.frame
    }

    fn reference_geometry(&self) -> ReferenceGeometry {
        self.reference_geometry
    }

    fn mass_provider(&self) -> &dyn MassProvider {
        self.mass.as_ref()
    }

    fn mesh_reference(&self) -> Option<&str> {
        self.mesh_reference.as_deref()
    }

    fn thrust_application_point(&self) -> Vec3 {
        self.thrust_application_point
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_mass_reports_a_fixed_value() {
        let m = ConstantMass::point(2.5, 0.4);
        assert!((m.mass(0.0) - 2.5).abs() < 1e-15);
        assert!((m.mass(100.0) - 2.5).abs() < 1e-15);
        assert!((m.mass_rate(5.0)).abs() < 1e-15);
        assert!(m.is_constant());
    }

    #[test]
    fn curve_mass_interpolates_and_reports_rate() {
        let curve = MassCurve::new(vec![0.0, 1.0, 2.0], vec![10.0, 8.0, 1.0]);
        let m = CurveMass::new(curve);
        assert!((m.mass(0.5) - 9.0).abs() < 1e-12);
        assert!((m.mass_rate(0.5) + 2.0).abs() < 1e-12);
        assert!((m.mass_rate(1.5) + 7.0).abs() < 1e-12);
        assert!(!m.is_constant());
    }

    #[test]
    fn curve_mass_holds_last_value_beyond_the_curve() {
        let curve = MassCurve::new(vec![0.0, 1.0], vec![10.0, 4.0]);
        let m = CurveMass::new(curve);
        assert!((m.mass(50.0) - 4.0).abs() < 1e-12);
    }

    #[test]
    fn mass_flow_profile_interpolates_and_can_be_suppressed() {
        let p = MassFlowProfile {
            times: vec![0.0, 1.0],
            flow_rates: vec![2.0, 0.0],
            already_in_mass_curve: false,
        };
        assert!((p.flow_at(0.0) - 2.0).abs() < 1e-12);
        assert!((p.flow_at(0.5) - 1.0).abs() < 1e-12);
        assert!((p.effective_rate(0.5) - 1.0).abs() < 1e-12);

        let suppressed = MassFlowProfile {
            already_in_mass_curve: true,
            ..p
        };
        assert!((suppressed.effective_rate(0.5)).abs() < 1e-12);
    }

    #[test]
    fn reference_geometry_from_diameter() {
        let g = ReferenceGeometry::from_diameter(0.1);
        assert!((g.reference_area - std::f64::consts::PI * 0.0025).abs() < 1e-15);
        assert!((g.reference_length - 1.0).abs() < 1e-15);
    }

    #[test]
    fn simple_vehicle_reports_its_summary() {
        let v = Vehicle::simple(3.0, 0.5);
        assert!(v.summary().contains("3.0000 kg"));
        assert_eq!(v.model_id(), "builtin.simple");
    }
}
