//! The on-disk dynamics model document.
//!
//! Every field that can legitimately be absent has a documented serde default so
//! a hand-written minimal file still parses. Values are stored in whatever unit
//! system the document declares; conversion to SI happens in
//! [`crate::importer`], never here, so a document round-trips byte-for-byte in
//! the units it was written in.
//!
//! Defaults are deliberately inert rather than convenient. A missing mass is
//! `0.0` and a missing inertia tensor is all zeros, so an incomplete file fails
//! validation loudly instead of silently acquiring a plausible-looking tensor
//! that the user never wrote.

use std::collections::BTreeMap;

use hex_core::{
    normalized_or, BodyFrame, Frame, InertiaTensor, MassCurve, Mat3, Quaternion, Real, Vec3,
    Vec3Record, WorldFrame,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::ModelError;
use crate::versioning::SCHEMA_VERSION;

/// A fresh unique model id, used when the editor creates a model from scratch.
pub fn new_model_id() -> String {
    Uuid::new_v4().to_string()
}

/// Linear interpolation over a table of `(x, y)` samples, clamped at both ends.
///
/// Clamping matters for aerodynamic tables: a Mach number beyond the last sample
/// must reuse the last coefficient rather than extrapolate into nonsense.
pub(crate) fn interp_table(table: &[(Real, Real)], x: Real) -> Option<Real> {
    match table.len() {
        0 => None,
        1 => Some(table[0].1),
        _ => {
            let last = table[table.len() - 1];
            if x <= table[0].0 {
                return Some(table[0].1);
            }
            if x >= last.0 {
                return Some(last.1);
            }
            for window in table.windows(2) {
                let (x0, y0) = window[0];
                let (x1, y1) = window[1];
                if x >= x0 && x <= x1 {
                    let span = x1 - x0;
                    if span <= 0.0 {
                        return Some(y0);
                    }
                    let f = (x - x0) / span;
                    return Some(y0 * (1.0 - f) + y1 * f);
                }
            }
            Some(last.1)
        }
    }
}

/// The complete dynamics model document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelDocument {
    /// Schema version this file was written against.
    pub schema_version: String,
    /// Stable identity of the model across versions.
    pub model_id: String,
    /// Version of the model itself, chosen by its author.
    pub model_version: String,
    /// Human-readable name.
    pub name: String,
    /// Optional longer description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Coordinate frames the document uses.
    pub frame: FrameDocument,
    /// Reference geometry for aerodynamic coefficients.
    pub reference_geometry: ReferenceGeometryDocument,
    /// Mass, centre of gravity, inertia, and an optional mass curve.
    pub mass_properties: MassPropertiesDocument,
    /// Optional visual or collision mesh reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshDocument>,
    /// Optional thrust profile.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thrust: Option<ThrustDocument>,
    /// Optional aerodynamic model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aerodynamics: Option<AeroDocument>,
    /// Free-form metadata. The validator looks for `frame_confirmed` here.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

impl Default for ModelDocument {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            model_id: String::new(),
            model_version: "1.0".to_string(),
            name: String::new(),
            description: None,
            frame: FrameDocument::default(),
            reference_geometry: ReferenceGeometryDocument::default(),
            mass_properties: MassPropertiesDocument::default(),
            mesh: None,
            thrust: None,
            aerodynamics: None,
            metadata: BTreeMap::new(),
        }
    }
}

impl ModelDocument {
    /// A small, complete, self-consistent document.
    ///
    /// The returned document is the shape the built-in example uses: unit
    /// reference area and length, zero centre of gravity, the supplied diagonal
    /// inertia, and the frame marked as confirmed because the caller declares it.
    pub fn minimal(
        model_id: impl Into<String>,
        name: impl Into<String>,
        mass: Real,
        inertia_xx_yy_zz: [Real; 3],
    ) -> Self {
        let mut metadata = BTreeMap::new();
        metadata.insert("frame_confirmed".to_string(), serde_json::Value::Bool(true));
        Self {
            schema_version: SCHEMA_VERSION.to_string(),
            model_id: model_id.into(),
            model_version: "1.0".to_string(),
            name: name.into(),
            description: None,
            frame: FrameDocument::default(),
            reference_geometry: ReferenceGeometryDocument {
                reference_area: 1.0,
                reference_length: 1.0,
                body_diameter: 0.0,
            },
            mass_properties: MassPropertiesDocument {
                mass,
                center_of_gravity: Vec3Record::default(),
                inertia: InertiaDocument::diagonal(
                    inertia_xx_yy_zz[0],
                    inertia_xx_yy_zz[1],
                    inertia_xx_yy_zz[2],
                ),
                mass_curve: None,
            },
            mesh: None,
            thrust: None,
            aerodynamics: None,
            metadata,
        }
    }

    /// Serialise to indented JSON, the form the editor writes.
    pub fn to_json_pretty(&self) -> Result<String, ModelError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Parse a document and verify the fields that have no default.
    pub fn from_json(text: &str) -> Result<Self, ModelError> {
        let doc: Self = serde_json::from_str(text)?;
        doc.check_required_fields()?;
        Ok(doc)
    }

    /// Fail when a field with no sensible default is blank.
    pub fn check_required_fields(&self) -> Result<(), ModelError> {
        if self.model_id.trim().is_empty() {
            return Err(ModelError::MissingField {
                field: "model_id".to_string(),
            });
        }
        Ok(())
    }

    /// The frames as the shared frame type.
    pub fn frame(&self) -> Frame {
        Frame::new(self.frame.world, self.frame.body)
    }

    /// Whether the author confirmed the frame convention in the metadata.
    ///
    /// The frame fields are non-optional in the schema, so their presence cannot
    /// distinguish a deliberate choice from a serde default. This flag can.
    pub fn is_frame_confirmed(&self) -> bool {
        matches!(
            self.metadata.get("frame_confirmed"),
            Some(serde_json::Value::Bool(true)) | Some(serde_json::Value::String(_))
        )
    }
}

/// Coordinate frames the document declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameDocument {
    /// Inertial frame the dynamics run in.
    pub world: WorldFrame,
    /// Body-fixed frame convention for mass properties, geometry, and forces.
    pub body: BodyFrame,
}

/// Reference quantities that scale aerodynamic coefficients.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ReferenceGeometryDocument {
    /// Reference area, in the document's length unit squared.
    pub reference_area: Real,
    /// Reference length, in the document's length unit.
    pub reference_length: Real,
    /// Body diameter, in the document's length unit. Zero when not axisymmetric.
    pub body_diameter: Real,
}

/// The full 3x3 inertia tensor, stored as nine named entries.
///
/// A physical tensor is symmetric, but the source of an import is often not.
/// Keeping all nine entries means an asymmetry survives the round trip and can be
/// reported, instead of being averaged away at parse time where nobody sees it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InertiaDocument {
    /// Moment about the body X axis.
    pub ixx: Real,
    /// Off-diagonal XY entry.
    pub ixy: Real,
    /// Off-diagonal XZ entry.
    pub ixz: Real,
    /// Off-diagonal YX entry.
    pub iyx: Real,
    /// Moment about the body Y axis.
    pub iyy: Real,
    /// Off-diagonal YZ entry.
    pub iyz: Real,
    /// Off-diagonal ZX entry.
    pub izx: Real,
    /// Off-diagonal ZY entry.
    pub izy: Real,
    /// Moment about the body Z axis.
    pub izz: Real,
}

impl InertiaDocument {
    /// A diagonal tensor with zero products of inertia.
    pub const fn diagonal(ixx: Real, iyy: Real, izz: Real) -> Self {
        Self {
            ixx,
            ixy: 0.0,
            ixz: 0.0,
            iyx: 0.0,
            iyy,
            iyz: 0.0,
            izx: 0.0,
            izy: 0.0,
            izz,
        }
    }

    /// A tensor with every entry written out.
    #[allow(clippy::too_many_arguments)]
    pub const fn full(
        ixx: Real,
        ixy: Real,
        ixz: Real,
        iyx: Real,
        iyy: Real,
        iyz: Real,
        izx: Real,
        izy: Real,
        izz: Real,
    ) -> Self {
        Self {
            ixx,
            ixy,
            ixz,
            iyx,
            iyy,
            iyz,
            izx,
            izy,
            izz,
        }
    }

    /// Copy a symmetric tensor back into document form.
    pub fn from_tensor(tensor: &InertiaTensor) -> Self {
        Self::full(
            tensor.ixx, tensor.ixy, tensor.ixz, tensor.ixy, tensor.iyy, tensor.iyz, tensor.ixz,
            tensor.iyz, tensor.izz,
        )
    }

    /// The nine entries as a matrix, asymmetry preserved.
    pub fn to_matrix(&self) -> Mat3 {
        Mat3::new(
            self.ixx, self.ixy, self.ixz, self.iyx, self.iyy, self.iyz, self.izx, self.izy,
            self.izz,
        )
    }

    /// The symmetric tensor the dynamics use.
    ///
    /// Mirrored entries are averaged, which is the same rule
    /// [`InertiaTensor::from_matrix`] applies, so the value here always matches
    /// what the core would compute from [`InertiaDocument::to_matrix`].
    pub fn to_tensor(&self) -> InertiaTensor {
        InertiaTensor::from_matrix(&self.to_matrix())
    }

    /// Largest absolute difference between mirrored entries.
    pub fn asymmetry(&self) -> Real {
        InertiaTensor::asymmetry(&self.to_matrix())
    }

    /// Largest absolute entry, used to make the symmetry tolerance relative.
    pub fn scale(&self) -> Real {
        [
            self.ixx, self.ixy, self.ixz, self.iyx, self.iyy, self.iyz, self.izx, self.izy,
            self.izz,
        ]
        .iter()
        .fold(0.0, |acc, v| acc.max(v.abs()))
    }

    /// Whether all nine entries are finite.
    pub fn all_finite(&self) -> bool {
        [
            self.ixx, self.ixy, self.ixz, self.iyx, self.iyy, self.iyz, self.izx, self.izy,
            self.izz,
        ]
        .iter()
        .all(|v| v.is_finite())
    }
}

/// Mass properties at the reference instant, plus an optional time track.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MassPropertiesDocument {
    /// Total mass in the document's mass unit.
    pub mass: Real,
    /// Centre of gravity in the body frame, in the document's length unit.
    pub center_of_gravity: Vec3Record,
    /// Inertia about the centre of gravity, in the document's inertia unit.
    pub inertia: InertiaDocument,
    /// Optional mass curve for models that change mass.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mass_curve: Option<MassCurveDocument>,
}

/// A piecewise-linear mass curve, optionally with centre-of-gravity and inertia
/// tracks.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MassCurveDocument {
    /// Sample times, in the document's time unit, strictly increasing.
    pub times: Vec<Real>,
    /// Mass at each sample time.
    pub masses: Vec<Real>,
    /// Optional centre of gravity per sample time.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub centers_of_gravity: Vec<Vec3Record>,
    /// Optional inertia tensor per sample time.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inertias: Vec<InertiaDocument>,
}

impl MassCurveDocument {
    /// Interpret the document curve as the shared mass-curve type.
    pub fn to_curve(&self) -> MassCurve {
        MassCurve {
            times: self.times.clone(),
            masses: self.masses.clone(),
            centers_of_gravity: self
                .centers_of_gravity
                .iter()
                .map(|v| v.to_vec3())
                .collect(),
            inertias: self
                .inertias
                .iter()
                .map(InertiaDocument::to_tensor)
                .collect(),
        }
    }
}

/// A mesh reference, resolved against a model-relative path.
///
/// `scale` defaults to zero rather than one so a mesh block that forgot to state
/// its scale is reported as unknown instead of being silently drawn at 1:1.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MeshDocument {
    /// Model-relative or asset-relative path to the mesh.
    pub reference: String,
    /// Size of one mesh unit in the document's length unit.
    pub scale: Real,
    /// Fixed orientation offset as a rotation vector, in the document's angle unit.
    pub orientation: Vec3Record,
}

/// A thrust profile with its application point and direction.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ThrustDocument {
    /// Application point in the body frame.
    pub application_point: Vec3Record,
    /// Thrust direction in the body frame. Normalised on import.
    pub direction: Vec3Record,
    /// Optional fixed misalignment applied to the nominal direction, `[w, x, y, z]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub misalignment_quaternion: Option<[Real; 4]>,
    /// Thrust sample times, in the document's time unit, strictly increasing.
    pub times: Vec<Real>,
    /// Thrust magnitude at each sample time.
    pub thrusts: Vec<Real>,
    /// Constant moment the engine applies about the centre of gravity.
    pub engine_moment: Vec3Record,
}

impl ThrustDocument {
    /// The misalignment rotation, when one was declared.
    pub fn misalignment(&self) -> Option<Quaternion> {
        self.misalignment_quaternion.map(Quaternion::from)
    }

    /// Unit thrust direction in the body frame.
    ///
    /// A zero or non-finite direction falls back to body +X, which is forward in
    /// both supported body conventions. A fallback is used rather than a NaN so
    /// the rest of the import can still produce a report.
    pub fn unit_direction(&self) -> Vec3 {
        normalized_or(self.direction.to_vec3(), Vec3::x())
    }
}

/// Aerodynamic coefficients, all dimensionless except the angles they multiply.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AeroDocument {
    /// Drag coefficient against Mach number, ascending in Mach.
    pub drag_coefficient_at_mach: Vec<(Real, Real)>,
    /// Lift-curve slope per radian.
    pub lift_slope: Real,
    /// Lift at zero angle of attack.
    pub lift_zero: Real,
    /// Side-force slope per radian.
    pub side_slope: Real,
    /// Pitching moment at zero angle of attack.
    pub pitch_zero: Real,
    /// Pitching-moment slope per radian.
    pub pitch_slope: Real,
    /// Yawing-moment slope per radian.
    pub yaw_slope: Real,
    /// Pitch damping derivative.
    pub pitch_damping: Real,
    /// Yaw damping derivative.
    pub yaw_damping: Real,
    /// Roll damping derivative.
    pub roll_damping: Real,
    /// Whether only the drag table is meaningful for this model.
    pub drag_only: bool,
}

impl AeroDocument {
    /// Drag coefficient at a Mach number, clamped to the sampled range.
    pub fn drag_coefficient_at(&self, mach: Real) -> Option<Real> {
        interp_table(&self.drag_coefficient_at_mach, mach)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ModelDocument {
        let mut doc = ModelDocument::minimal("rocket-1", "Sounding rocket", 12.5, [0.9, 0.9, 0.1]);
        doc.description = Some("Single stage".to_string());
        doc.reference_geometry.reference_area = 0.0314;
        doc.reference_geometry.reference_length = 0.2;
        doc.reference_geometry.body_diameter = 0.2;
        doc.mass_properties.center_of_gravity = Vec3Record::new(0.0, 0.0, -0.05);
        doc.metadata.insert(
            "source".to_string(),
            serde_json::Value::String("openrocket".to_string()),
        );
        doc
    }

    #[test]
    fn minimal_document_round_trips_through_json() {
        let doc = sample();
        let json = doc.to_json_pretty().unwrap();
        let back = ModelDocument::from_json(&json).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn json_keys_are_snake_case() {
        let json = ModelDocument::minimal("m", "n", 1.0, [1.0, 1.0, 1.0])
            .to_json_pretty()
            .unwrap();
        for key in [
            "schema_version",
            "model_id",
            "model_version",
            "reference_geometry",
            "reference_area",
            "reference_length",
            "body_diameter",
            "mass_properties",
            "center_of_gravity",
            "izz",
        ] {
            assert!(json.contains(&format!("\"{key}\"")), "missing key {key}");
        }
        assert!(!json.contains("\"modelId\""));
    }

    #[test]
    fn minimal_json_parses_with_defaults() {
        let doc = ModelDocument::from_json(r#"{"model_id":"only-an-id"}"#).unwrap();
        assert_eq!(doc.schema_version, SCHEMA_VERSION);
        assert_eq!(doc.model_version, "1.0");
        assert_eq!(doc.name, "");
        assert_eq!(doc.description, None);
        assert_eq!(doc.frame.world, WorldFrame::Enu);
        assert_eq!(doc.frame.body, BodyFrame::ForwardRightDown);
        assert_eq!(doc.mass_properties.mass, 0.0);
        assert_eq!(doc.mass_properties.inertia.ixx, 0.0);
        assert!(doc.mesh.is_none());
        assert!(doc.thrust.is_none());
        assert!(doc.aerodynamics.is_none());
        assert!(doc.metadata.is_empty());
    }

    #[test]
    fn missing_model_id_is_reported_not_defaulted() {
        let err = ModelDocument::from_json("{}").unwrap_err();
        match err {
            ModelError::MissingField { field } => assert_eq!(field, "model_id"),
            other => panic!("expected MissingField, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_is_reported() {
        assert!(matches!(
            ModelDocument::from_json("{ not json"),
            Err(ModelError::Json(_))
        ));
    }

    #[test]
    fn unknown_keys_are_ignored_for_forward_compatibility() {
        let doc =
            ModelDocument::from_json(r#"{"model_id":"m","future_field":{"nested":1},"name":"n"}"#)
                .unwrap();
        assert_eq!(doc.name, "n");
    }

    #[test]
    fn inertia_document_preserves_asymmetry_on_disk() {
        let mut doc = sample();
        doc.mass_properties.inertia = InertiaDocument::full(
            1.0, 0.2, 0.0, //
            0.0, 2.0, 0.0, //
            0.0, 0.0, 3.0,
        );
        let back = ModelDocument::from_json(&doc.to_json_pretty().unwrap()).unwrap();
        assert_eq!(back.mass_properties.inertia, doc.mass_properties.inertia);
        assert!((back.mass_properties.inertia.asymmetry() - 0.2).abs() < 1e-12);
    }

    #[test]
    fn inertia_document_symmetrises_and_keeps_diagonal() {
        let doc = InertiaDocument::full(1.0, 0.2, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0);
        let tensor = doc.to_tensor();
        assert!((tensor.ixx - 1.0).abs() < 1e-12);
        assert!((tensor.iyy - 2.0).abs() < 1e-12);
        assert!((tensor.izz - 3.0).abs() < 1e-12);
        assert!((tensor.ixy - 0.1).abs() < 1e-12);
        assert!((doc.scale() - 3.0).abs() < 1e-12);
    }

    #[test]
    fn inertia_document_default_is_zero_not_identity() {
        let doc = InertiaDocument::default();
        assert_eq!(doc.ixx, 0.0);
        assert_eq!(doc.izz, 0.0);
        assert!(doc.all_finite());
        assert!((doc.asymmetry() - 0.0).abs() < 1e-15);
    }

    #[test]
    fn inertia_document_tensor_round_trip() {
        let tensor = InertiaTensor::new(4.0, 2.0, 3.0, 0.5, -0.25, 0.125);
        let doc = InertiaDocument::from_tensor(&tensor);
        assert_eq!(doc.to_tensor(), tensor);
    }

    #[test]
    fn inertia_document_detects_non_finite_entries() {
        let doc = InertiaDocument::diagonal(Real::NAN, 1.0, 1.0);
        assert!(!doc.all_finite());
    }

    #[test]
    fn mass_curve_document_converts_every_track() {
        let doc = MassCurveDocument {
            times: vec![0.0, 1.0],
            masses: vec![10.0, 6.0],
            centers_of_gravity: vec![
                Vec3Record::new(0.0, 0.0, 0.0),
                Vec3Record::new(0.0, 0.0, 0.1),
            ],
            inertias: vec![
                InertiaDocument::diagonal(1.0, 1.0, 1.0),
                InertiaDocument::diagonal(0.5, 0.5, 0.5),
            ],
        };
        let curve = doc.to_curve();
        assert_eq!(curve.times.len(), 2);
        assert_eq!(curve.masses, vec![10.0, 6.0]);
        assert_eq!(curve.centers_of_gravity.len(), 2);
        assert_eq!(curve.inertias.len(), 2);
        assert!((curve.mass_at(0.5).unwrap() - 8.0).abs() < 1e-12);
        assert!(curve.validate().is_empty());
    }

    #[test]
    fn mass_curve_document_without_optional_tracks_is_valid() {
        let doc = MassCurveDocument {
            times: vec![0.0, 1.0, 2.0],
            masses: vec![10.0, 9.0, 8.0],
            ..Default::default()
        };
        let curve = doc.to_curve();
        assert!(curve.centers_of_gravity.is_empty());
        assert!(curve.inertias.is_empty());
        assert!(curve.validate().is_empty());
    }

    #[test]
    fn thrust_document_normalises_direction() {
        let doc = ThrustDocument {
            direction: Vec3Record::new(0.0, 0.0, 5.0),
            ..Default::default()
        };
        assert!((doc.unit_direction() - Vec3::z()).norm() < 1e-12);
    }

    #[test]
    fn thrust_document_falls_back_to_forward_for_a_zero_direction() {
        let doc = ThrustDocument::default();
        assert!((doc.unit_direction() - Vec3::x()).norm() < 1e-12);
        assert!(doc.misalignment().is_none());
    }

    #[test]
    fn thrust_document_reads_misalignment() {
        let doc = ThrustDocument {
            misalignment_quaternion: Some([0.0, 1.0, 0.0, 0.0]),
            ..Default::default()
        };
        let q = doc.misalignment().unwrap();
        assert!((q.norm() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn aero_document_interpolates_drag_table() {
        let doc = AeroDocument {
            drag_coefficient_at_mach: vec![(0.0, 0.02), (1.0, 0.03)],
            ..Default::default()
        };
        assert!((doc.drag_coefficient_at(0.5).unwrap() - 0.025).abs() < 1e-12);
    }

    #[test]
    fn aero_document_clamps_drag_table_ends() {
        let doc = AeroDocument {
            drag_coefficient_at_mach: vec![(0.5, 0.02), (1.0, 0.03)],
            ..Default::default()
        };
        assert!((doc.drag_coefficient_at(-2.0).unwrap() - 0.02).abs() < 1e-12);
        assert!((doc.drag_coefficient_at(9.0).unwrap() - 0.03).abs() < 1e-12);
    }

    #[test]
    fn aero_document_without_a_table_has_no_drag() {
        let doc = AeroDocument {
            drag_only: true,
            ..Default::default()
        };
        assert!(doc.drag_coefficient_at(0.3).is_none());
        assert!(doc.drag_only);
    }

    #[test]
    fn mesh_document_defaults_to_an_unknown_scale() {
        let doc = MeshDocument {
            reference: "rocket.glb".to_string(),
            ..Default::default()
        };
        assert_eq!(doc.scale, 0.0);
        assert_eq!(doc.orientation, Vec3Record::default());
    }

    #[test]
    fn metadata_survives_the_round_trip() {
        let doc = sample();
        let back = ModelDocument::from_json(&doc.to_json_pretty().unwrap()).unwrap();
        assert_eq!(
            back.metadata.get("source"),
            Some(&serde_json::Value::String("openrocket".to_string()))
        );
        assert!(back.is_frame_confirmed());
    }

    #[test]
    fn frame_confirmation_requires_a_truthy_flag() {
        let mut doc = sample();
        doc.metadata.clear();
        assert!(!doc.is_frame_confirmed());

        doc.metadata.insert(
            "frame_confirmed".to_string(),
            serde_json::Value::Bool(false),
        );
        assert!(!doc.is_frame_confirmed());
    }

    #[test]
    fn frame_accessor_mirrors_the_document() {
        let mut doc = sample();
        doc.frame.world = WorldFrame::Ned;
        doc.frame.body = BodyFrame::ForwardLeftUp;
        let frame = doc.frame();
        assert_eq!(frame.world, WorldFrame::Ned);
        assert_eq!(frame.body, BodyFrame::ForwardLeftUp);
    }

    #[test]
    fn new_model_ids_are_unique() {
        assert_ne!(new_model_id(), new_model_id());
    }

    #[test]
    fn optional_blocks_are_omitted_when_absent() {
        let json = ModelDocument::minimal("m", "n", 1.0, [1.0, 1.0, 1.0])
            .to_json_pretty()
            .unwrap();
        assert!(!json.contains("\"mesh\""));
        assert!(!json.contains("\"thrust\""));
        assert!(!json.contains("\"aerodynamics\""));
        assert!(!json.contains("\"description\""));
    }

    #[test]
    fn described_document_keeps_its_description() {
        let doc = sample();
        assert_eq!(doc.description.as_deref(), Some("Single stage"));
        let back = ModelDocument::from_json(&doc.to_json_pretty().unwrap()).unwrap();
        assert_eq!(back.description, doc.description);
    }
}
