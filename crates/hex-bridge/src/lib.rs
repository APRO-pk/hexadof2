//! # hex-bridge
//!
//! Imports APRO Works platform artifacts as HexaDOF dynamics models.
//!
//! This is the consumer half of the platform's motivating example: mass properties
//! measured in aproCAD become a model HexaDOF can simulate.
//!
//! It lives in the hexadof2 repository because it depends on `hex-model` and `hex-core`.
//! The platform must never depend on an application, so the dependency points one way.
//!
//! ## What the consumer has to decide, and what it must not
//!
//! The payload is SI and says so, so [unit errors](apro_contracts::MassPropertiesV1::validate)
//! cannot slip through. Two things it cannot settle on its own:
//!
//! * **Where the body datum is.** Handled by
//!   [`MassPropertiesV1::center_of_gravity_in_datum`]; the producer declares the offset.
//! * **How the CAD axes map onto the HexaDOF body frame.** This is a *consumer* fact —
//!   the producing app knows its own axes, but only HexaDOF knows which of its body
//!   conventions it wants. [`CAD_TO_BODY`] states the mapping this crate applies, and
//!   [`to_model_document`] records it in the model metadata so the choice is visible
//!   rather than buried.

use std::collections::BTreeMap;

use apro_client::{AproStoreClient, ClientError, Selector, TypeId};
use apro_contracts::{
    ContractError, MassPropertiesV1, ReferenceGeometrySi, MASS_PROPERTIES_TYPE,
};

/// Re-exported so an application embedding this bridge does not have to pin the same SDK
/// version twice — and cannot pin two different ones.
pub use apro_client;
pub use apro_contracts;

/// The slug the hub launches HexaDOF under.
pub const APP_SLUG: &str = "hexadof";

// ---------------------------------------------------------------------------
// The axis mapping
// ---------------------------------------------------------------------------

/// How a CAD axis index maps onto the HexaDOF body axes.
///
/// `BODY_AXIS_OF_CAD_AXIS[i]` is the body axis that CAD axis `i` becomes, where the body
/// axes are `0 = forward, 1 = left, 2 = up` (HexaDOF's `ForwardLeftUp`).
///
/// aproCAD renders Y-up — its grid lies on the XZ plane — and models a rocket along +Z,
/// which is what its extrude axis and its own presets use. That gives
/// `(forward, left, up) = (Z_cad, X_cad, Y_cad)`, a cyclic permutation and therefore
/// right-handed, which is why the resulting tensor needs no reflection fix-up.
///
/// **This is a stated choice, not a law.** A design modelled along a different axis, or a
/// consumer that wants the classical Forward-Right-Down frame, changes this table and the
/// permutation below follows. It is recorded in the imported model's metadata so a user
/// can see which convention produced the numbers they are looking at.
pub const CAD_TO_BODY: [usize; 3] = [2, 0, 1];

/// The HexaDOF body convention those axes describe.
const BODY_FRAME: hex_core::BodyFrame = hex_core::BodyFrame::ForwardLeftUp;

/// Permute a vector from CAD axes into body axes.
pub fn permute_vector(cad: [f64; 3]) -> [f64; 3] {
    [
        cad[CAD_TO_BODY[0]],
        cad[CAD_TO_BODY[1]],
        cad[CAD_TO_BODY[2]],
    ]
}

/// Permute a tensor from CAD axes into body axes: `I_body = P · I_cad · Pᵀ`.
///
/// A pure axis relabelling over an orthonormal basis, so the result is an exact
/// rearrangement of the same numbers — no interpolation and no loss.
pub fn permute_tensor(cad: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0_f64; 3]; 3];
    for (body_row, cad_row) in CAD_TO_BODY.iter().enumerate() {
        for (body_col, cad_col) in CAD_TO_BODY.iter().enumerate() {
            out[body_row][body_col] = cad[*cad_row][*cad_col];
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// The payload did not declare SI. Refusing beats guessing by a factor of 1e6.
    #[error("{0}")]
    Contract(#[from] ContractError),

    #[error("could not reach the APRO Works store: {0}")]
    Client(#[from] ClientError),

    #[error("{0}")]
    Model(#[from] hex_model::ModelError),

    #[error("the artifact type {0:?} is not a valid type id")]
    TypeId(String),

    #[error("no artifact of type {type_id:?} instance {instance:?} has been published yet")]
    NotPublished { type_id: String, instance: String },

    /// The producer could not state its aerodynamic convention, and this consumer will
    /// not invent one: reference area is a choice, and a made-up value would go straight
    /// into a simulation.
    #[error(
        "the published mass properties carry no reference geometry, so the reference area \
         is unknown; supply it before importing"
    )]
    MissingReferenceGeometry,

    #[error("could not serialise the model document: {0}")]
    Json(#[from] serde_json::Error),

    #[error("the published payload is not valid UTF-8, so it is not a JSON contract payload")]
    NotUtf8,
}

pub type Result<T> = std::result::Result<T, BridgeError>;

// ---------------------------------------------------------------------------
// Reading from the platform
// ---------------------------------------------------------------------------

/// A published revision, with the metadata that says *which* revision it is.
///
/// The payload alone cannot answer "what did I just import?", which is exactly the
/// question a user asks when a model looks wrong.
#[derive(Debug, Clone)]
pub struct PublishedPayload {
    pub payload: MassPropertiesV1,
    pub revision_number: u32,
    pub content_hash: String,
    pub label: Option<String>,
}

/// Pull the newest revision of a published mass-properties artifact.
///
/// `Ok(None)` means the producer has not published this instance yet, which is a normal
/// state rather than a failure.
pub fn pull_mass_properties(
    client: &dyn AproStoreClient,
    instance: &str,
) -> Result<Option<PublishedPayload>> {
    let type_id = TypeId::parse(MASS_PROPERTIES_TYPE)
        .map_err(|err| BridgeError::TypeId(err.to_string()))?;

    let Some(fetched) = client.pull(&type_id, instance, Selector::Latest)? else {
        return Ok(None);
    };

    // The bytes came off the wire, so parse and validate before trusting any of it.
    let text = fetched.as_str().map_err(|_| BridgeError::NotUtf8)?;
    let payload = MassPropertiesV1::from_json_checked(text)?;

    Ok(Some(PublishedPayload {
        payload,
        revision_number: fetched.revision_number,
        content_hash: fetched.content_hash,
        label: fetched.label,
    }))
}

// ---------------------------------------------------------------------------
// Mapping onto a HexaDOF model
// ---------------------------------------------------------------------------

/// Map an SI payload onto HexaDOF's model document.
///
/// The result serialises to the JSON `hex_model::import_json` already knows how to read,
/// validate and convert, so the consumer needs no bespoke parsing path.
///
/// `reference` overrides the payload's own reference geometry. It exists because a
/// consumer may want to fly a vehicle against a different convention than the producer
/// assumed — and because a payload without geometry must be given one rather than
/// silently defaulted.
pub fn to_model_document(
    payload: &MassPropertiesV1,
    model_id: &str,
    name: &str,
    reference: Option<ReferenceGeometrySi>,
) -> Result<hex_model::ModelDocument> {
    payload.validate()?;

    let reference = reference
        .or_else(|| payload.reference_geometry.clone())
        .ok_or(BridgeError::MissingReferenceGeometry)?;
    reference.validate()?;

    let cg_cad = payload.center_of_gravity_in_datum();
    let cg = permute_vector(cg_cad);
    let inertia = permute_tensor(payload.inertia_kg_m2);

    let mut metadata = BTreeMap::new();
    // The model's frame fields are non-optional in the schema, so their presence cannot
    // distinguish a deliberate choice from a serde default. This flag can.
    metadata.insert("frame_confirmed".to_string(), serde_json::Value::Bool(true));
    metadata.insert(
        "source".to_string(),
        serde_json::json!({
            "platform": "APRO Works",
            "type": MASS_PROPERTIES_TYPE,
            "original_units": payload.source_units.as_str(),
            "note": payload.source_note,
        }),
    );
    // Record the axis mapping, so a user looking at a surprising orientation can see
    // which convention produced it instead of guessing.
    metadata.insert(
        "axis_mapping".to_string(),
        serde_json::json!({
            "cad_to_body": CAD_TO_BODY,
            "body_frame": BODY_FRAME.label(),
            "note": "body axes are 0 = forward, 1 = left, 2 = up; \
                     CAD_TO_BODY[i] is the body axis that CAD axis i becomes",
        }),
    );
    metadata.insert(
        "reference_geometry".to_string(),
        serde_json::json!({
            "source": if payload.reference_geometry.is_some() { "publisher" } else { "consumer" },
            "convention": reference.convention,
        }),
    );

    Ok(hex_model::ModelDocument {
        schema_version: hex_model::SCHEMA_VERSION.to_string(),
        model_id: model_id.to_string(),
        model_version: "1.0".to_string(),
        name: name.to_string(),
        description: Some(format!(
            "Mass properties published by the CAD application (authored in {}), \
             converted to SI at the publish boundary and rotated into the {} body frame.",
            payload.source_units.as_str(),
            BODY_FRAME.label()
        )),
        frame: hex_model::FrameDocument {
            world: hex_core::WorldFrame::Enu,
            body: BODY_FRAME,
        },
        reference_geometry: hex_model::ReferenceGeometryDocument {
            reference_area: reference.reference_area_m2,
            reference_length: reference.reference_length_m,
            body_diameter: reference.body_diameter_m,
        },
        mass_properties: hex_model::MassPropertiesDocument {
            mass: payload.mass_kg,
            center_of_gravity: hex_core::Vec3Record::new(cg[0], cg[1], cg[2]),
            inertia: hex_model::InertiaDocument {
                ixx: inertia[0][0],
                ixy: inertia[0][1],
                ixz: inertia[0][2],
                iyx: inertia[1][0],
                iyy: inertia[1][1],
                iyz: inertia[1][2],
                izx: inertia[2][0],
                izy: inertia[2][1],
                izz: inertia[2][2],
            },
            mass_curve: None,
        },
        mesh: None,
        thrust: None,
        aerodynamics: None,
        metadata,
    })
}

/// Pull a published vehicle and import it as a HexaDOF model in one step.
pub fn import_from_platform(
    client: &dyn AproStoreClient,
    instance: &str,
    name: &str,
    reference: Option<ReferenceGeometrySi>,
) -> Result<(PublishedPayload, hex_model::ImportedModel)> {
    let published = pull_mass_properties(client, instance)?.ok_or_else(|| {
        BridgeError::NotPublished {
            type_id: MASS_PROPERTIES_TYPE.to_string(),
            instance: instance.to_string(),
        }
    })?;

    // The model id is the platform instance, so re-importing an updated design replaces
    // the same model rather than accumulating near-duplicates.
    let document = to_model_document(&published.payload, instance, name, reference)?;
    let json = document.to_json_pretty()?;
    let imported = hex_model::import_json(&json, &hex_model::ImportOptions::si())?;
    Ok((published, imported))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use apro_contracts::{DocumentMassProperties, MASS_PROPERTIES_TYPE as TYPE_ID, SourceUnits};

    /// A 100 x 100 x 600 mm aluminium block, corner at the CAD origin.
    const EDGE_MM: f64 = 100.0;
    const LENGTH_MM: f64 = 600.0;
    const DENSITY_KG_PER_MM3: f64 = 2700.0 / 1.0e9;

    fn block_payload() -> MassPropertiesV1 {
        let volume = EDGE_MM * EDGE_MM * LENGTH_MM;
        let mass = DENSITY_KG_PER_MM3 * volume;
        let ixx = mass / 12.0 * (EDGE_MM * EDGE_MM + LENGTH_MM * LENGTH_MM);
        let iyy = mass / 12.0 * (EDGE_MM * EDGE_MM + LENGTH_MM * LENGTH_MM);
        let izz = mass / 12.0 * (EDGE_MM * EDGE_MM + EDGE_MM * EDGE_MM);

        MassPropertiesV1::from_document(
            DocumentMassProperties {
                volume,
                mass_kg: mass,
                center_of_mass: [EDGE_MM / 2.0, EDGE_MM / 2.0, LENGTH_MM / 2.0],
                inertia: [
                    [ixx, 0.0, 0.0],
                    [0.0, iyy, 0.0],
                    [0.0, 0.0, izz],
                ],
            },
            SourceUnits::Millimeters,
            [0.0; 3],
        )
        .with_reference_geometry(ReferenceGeometrySi::from_body_diameter(0.1, 0.6))
    }

    #[test]
    fn the_axis_mapping_is_a_right_handed_permutation() {
        // A cyclic permutation preserves handedness, so the inertia tensor needs no
        // reflection fix-up. An accidental swap of two axes would flip the sign of every
        // off-diagonal term and produce a mirror-image vehicle.
        let mut sorted = CAD_TO_BODY;
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 2], "must be a permutation of the three axes");

        let is_cyclic = CAD_TO_BODY == [2, 0, 1] || CAD_TO_BODY == [1, 2, 0];
        assert!(is_cyclic, "CAD_TO_BODY must be cyclic to stay right-handed");
    }

    #[test]
    fn vectors_are_permuted_onto_the_body_axes() {
        // CAD +Z is forward, +X is left, +Y is up.
        assert_eq!(permute_vector([0.0, 0.0, 1.0]), [1.0, 0.0, 0.0]);
        assert_eq!(permute_vector([1.0, 0.0, 0.0]), [0.0, 1.0, 0.0]);
        assert_eq!(permute_vector([0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
    }

    /// The centre of gravity must arrive in body axes. The block's CAD long axis is Z, so
    /// after the mapping the long-axis offset lands on the body's forward axis.
    #[test]
    fn the_centre_of_gravity_lands_on_the_body_forward_axis() {
        let payload = block_payload();
        let document = to_model_document(&payload, "veh-1", "Block", None).unwrap();
        let cg = document.mass_properties.center_of_gravity;
        assert!((cg.x - 0.3).abs() < 1e-12, "forward was {}", cg.x);
        assert!((cg.y - 0.05).abs() < 1e-12, "left was {}", cg.y);
        assert!((cg.z - 0.05).abs() < 1e-12, "up was {}", cg.z);
    }

    /// The inertia tensor is rotated by the same permutation as the vector. Getting this
    /// wrong leaves a diagonally-dominant, entirely plausible, wrong tensor.
    #[test]
    fn the_inertia_tensor_is_rotated_with_the_axes() {
        let payload = block_payload();
        let cad = payload.inertia_kg_m2;
        let document = to_model_document(&payload, "veh-1", "Block", None).unwrap();
        let inertia = &document.mass_properties.inertia;

        // body_x = cad_z, body_y = cad_x, body_z = cad_y.
        assert!((inertia.ixx - cad[2][2]).abs() < 1e-18);
        assert!((inertia.iyy - cad[0][0]).abs() < 1e-18);
        assert!((inertia.izz - cad[1][1]).abs() < 1e-18);

        // The block is long along CAD Z, so after the mapping the largest inertia is
        // about the two transverse body axes, not about forward.
        assert!(inertia.iyy > inertia.ixx);
        assert!(inertia.izz > inertia.ixx);
    }

    #[test]
    fn the_permuted_tensor_stays_symmetric() {
        let mut cad = [[0.0_f64; 3]; 3];
        for (row, values) in [[1.0, 2.0, 3.0], [2.0, 4.0, 5.0], [3.0, 5.0, 6.0]]
            .iter()
            .enumerate()
        {
            cad[row] = *values;
        }
        let body = permute_tensor(cad);
        for row in 0..3 {
            for col in 0..3 {
                assert!(
                    (body[row][col] - body[col][row]).abs() < 1e-18,
                    "asymmetric at {row},{col}"
                );
            }
        }
    }

    #[test]
    fn the_datum_offset_moves_the_centre_of_gravity() {
        let mut payload = block_payload();
        payload.datum_offset_m = [0.0, 0.0, -0.3];

        let document = to_model_document(&payload, "veh-1", "Block", None).unwrap();
        let cg = document.mass_properties.center_of_gravity;
        // CAD z 0.3 shifts by -0.3 to 0.0, and lands on the body forward axis.
        assert!(cg.x.abs() < 1e-12, "forward was {}", cg.x);

        // The inertia is about the centre of gravity and must not have moved.
        assert!((document.mass_properties.inertia.iyy - payload.inertia_kg_m2[0][0]).abs() < 1e-18);
    }

    #[test]
    fn the_result_imports_through_the_real_importer() {
        let payload = block_payload();
        let document = to_model_document(&payload, "veh-1", "Block", None).unwrap();
        let json = document.to_json_pretty().unwrap();
        let imported = hex_model::import_json(&json, &hex_model::ImportOptions::si()).unwrap();

        assert_eq!(imported.model_id, "veh-1");
        assert!(!imported.validation.has_errors(), "{:?}", imported.validation);
        assert!((imported.mass() - payload.mass_kg).abs() < 1e-12);
    }

    #[test]
    fn the_frame_choice_is_declared_and_recorded() {
        let payload = block_payload();
        let document = to_model_document(&payload, "veh-1", "Block", None).unwrap();

        assert_eq!(document.frame.body, hex_core::BodyFrame::ForwardLeftUp);
        assert_eq!(document.frame.world, hex_core::WorldFrame::Enu);
        assert_eq!(document.metadata["frame_confirmed"], serde_json::json!(true));

        let mapping = &document.metadata["axis_mapping"];
        assert_eq!(mapping["cad_to_body"], serde_json::json!([2, 0, 1]));
        assert!(mapping["body_frame"].as_str().unwrap().contains("forward"));
    }

    #[test]
    fn a_consumer_supplied_reference_geometry_overrides_the_publishers() {
        let payload = block_payload();
        let override_reference = ReferenceGeometrySi::from_body_diameter(0.2, 1.0);

        let document =
            to_model_document(&payload, "veh-1", "Block", Some(override_reference.clone()))
                .unwrap();
        assert!((document.reference_geometry.body_diameter - 0.2).abs() < 1e-12);
        assert_eq!(
            document.metadata["reference_geometry"]["source"],
            serde_json::json!("publisher"),
            "the payload still carried geometry, so its provenance is the publisher"
        );
    }

    #[test]
    fn a_payload_without_reference_geometry_is_refused_not_defaulted() {
        let mut payload = block_payload();
        payload.reference_geometry = None;

        match to_model_document(&payload, "veh-1", "Block", None) {
            Err(BridgeError::MissingReferenceGeometry) => {}
            other => panic!("expected MissingReferenceGeometry, got {other:?}"),
        }

        // ...and the caller can supply one instead.
        let supplied = ReferenceGeometrySi::from_body_diameter(0.1, 0.6);
        let document = to_model_document(&payload, "veh-1", "Block", Some(supplied)).unwrap();
        assert!((document.reference_geometry.body_diameter - 0.1).abs() < 1e-12);
    }

    #[test]
    fn a_payload_that_does_not_declare_si_is_refused() {
        let mut payload = block_payload();
        payload.units = "mm".into();
        assert!(matches!(
            to_model_document(&payload, "veh-1", "Block", None),
            Err(BridgeError::Contract(ContractError::NotSi { .. }))
        ));
    }

    #[test]
    fn the_model_id_is_the_platform_instance() {
        // Re-importing an updated design must replace the same model, not accumulate
        // near-duplicates under ever-changing display names.
        let payload = block_payload();
        let a = to_model_document(&payload, "veh-1", "Old name", None).unwrap();
        let b = to_model_document(&payload, "veh-1", "New name", None).unwrap();
        assert_eq!(a.model_id, b.model_id);
        assert_ne!(a.name, b.name);
    }

    #[test]
    fn the_contract_type_id_is_the_one_we_pull() {
        assert_eq!(MASS_PROPERTIES_TYPE, TYPE_ID);
        TypeId::parse(MASS_PROPERTIES_TYPE).unwrap();
    }
}
