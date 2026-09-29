//! Dynamics model schema, import, validation, and versioning.
//!
//! This crate owns everything about **reading** a dynamics model: the on-disk
//! document schema, the unit conversion at the file boundary, the validation
//! rules, and the schema version policy. It owns nothing about integrating the
//! model: no equations of motion, no state, no solver. The crate that integrates
//! a model consumes [`ImportedModel`], which is already SI-canonical.
//!
//! # Units
//!
//! Every conversion happens at this boundary, once, through
//! [`hex_core::QuantityKind::scale_for`]. Values leave this crate in SI: metres,
//! kilograms, seconds, newtons, and newton metres. A unit string that cannot be
//! resolved is never guessed; it produces a `units.unknown` warning and the value
//! is carried through unconverted so the mistake stays visible.
//!
//! # Validation
//!
//! Every import produces a [`hex_core::ValidationReport`]. The report is part of
//! the returned model, not an afterthought, because the UI must show it: an
//! import never silently succeeds on a model that failed a physics check.
//!
//! ```
//! use hex_model::{import_json, ImportOptions, ModelDocument};
//!
//! let doc = ModelDocument::minimal("demo", "Demo body", 10.0, [1.0, 2.0, 3.0]);
//! let text = doc.to_json_pretty().unwrap();
//! let model = import_json(&text, &ImportOptions::default()).unwrap();
//!
//! assert!(model.is_valid());
//! assert_eq!(model.mass(), 10.0);
//! ```
#![warn(missing_docs)]

pub mod error;
pub mod importer;
pub mod schema;
pub mod validator;
pub mod versioning;

pub use error::ModelError;
pub use importer::{
    hash_source, import_document, import_file, import_json, unit_system_from_str, AeroModel,
    ImportOptions, ImportedModel, MeshReference, ReferenceGeometry, ThrustModel,
};
pub use schema::{
    new_model_id, AeroDocument, FrameDocument, InertiaDocument, MassCurveDocument,
    MassPropertiesDocument, MeshDocument, ModelDocument, ReferenceGeometryDocument, ThrustDocument,
};
pub use validator::{
    finalize_report, frame_confirmation_issue, inertia_asymmetry_issue, validate_document,
    validate_imported, ASYMMETRY_REL_TOLERANCE, MASS_INCREASE_LIMIT, TRIANGLE_REL_TOLERANCE,
};
pub use versioning::{check_schema_version, migrate_document, ModelVersion, SCHEMA_VERSION};

/// Short alias for [`ModelError`], the error type of every fallible entry point.
pub type Error = ModelError;
