//! End-to-end import tests over real files on disk.
//!
//! These tests use `std::env::temp_dir` with a unique name per run, so they need
//! no fixture directory and no extra dependency.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use hex_core::Severity;
use hex_model::{
    import_document, import_file, validate_imported, ImportOptions, InertiaDocument, ModelDocument,
};

/// A unique path inside the system temporary directory.
fn temp_model_path(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before the Unix epoch")
        .as_nanos();
    env::temp_dir().join(format!(
        "hex-model-{tag}-{}-{nanos}.json",
        std::process::id()
    ))
}

/// A small, complete model: 12.5 kg with a plausible diagonal inertia.
fn small_model() -> ModelDocument {
    let mut doc = ModelDocument::minimal(
        "integration-1",
        "Integration test body",
        12.5,
        [0.42, 2.10, 2.10],
    );
    doc.model_version = "1.2.0".to_string();
    doc.description = Some("Written by the import integration test".to_string());
    doc.reference_geometry.reference_area = 0.031_416;
    doc.reference_geometry.reference_length = 0.2;
    doc
}

#[test]
fn imports_a_written_model_file_and_validates_it() {
    let path = temp_model_path("valid");
    let doc = small_model();
    fs::write(&path, doc.to_json_pretty().unwrap()).expect("could not write the test model");

    let model = import_file(&path, &ImportOptions::default()).expect("import failed");
    fs::remove_file(&path).ok();

    assert_eq!(model.model_id, "integration-1");
    assert_eq!(model.model_version, "1.2.0");
    assert_eq!(model.name, "Integration test body");

    // "Valid" means no blocking and no suspicious finding. The status is
    // "valid with warnings" because informational notes are still issues: the
    // importer records the assumptions it made rather than hiding them.
    assert!(model.is_valid());
    assert_eq!(model.validation.error_count(), 0);
    assert_eq!(model.validation.warning_count(), 0);
    assert!(model.validation.can_proceed());
    assert!(model
        .validation
        .issues
        .iter()
        .all(|i| i.severity == Severity::Info));
    assert!(model.validation.issues.iter().any(|i| i.code == "model.ok"));

    // Re-validating an imported model reproduces the report it came with.
    assert_eq!(validate_imported(&model), model.validation);
    assert!(!model.source_hash.is_empty());
}

#[test]
fn a_negative_definite_inertia_is_a_blocking_error() {
    let path = temp_model_path("negative-inertia");
    let mut doc = small_model();
    doc.mass_properties.inertia = InertiaDocument::diagonal(-1.0, -1.0, -1.0);
    fs::write(&path, doc.to_json_pretty().unwrap()).expect("could not write the test model");

    let model = import_file(&path, &ImportOptions::default()).expect("import failed");
    fs::remove_file(&path).ok();

    let issue = model
        .validation
        .issues
        .iter()
        .find(|issue| issue.code == "inertia.not_positive_definite")
        .expect("no inertia.not_positive_definite finding");
    assert_eq!(issue.severity, Severity::Error);
    assert_eq!(
        issue.detail,
        "The supplied tensor is not positive definite. Check the mass properties export and body reference frame."
    );

    assert!(!model.is_valid());
    assert!(!model.validation.can_proceed());
    let err = model
        .check_valid()
        .expect_err("check_valid accepted a bad model");
    assert!(err.is_blocking());
    assert_eq!(err.code(), "validation.failed");
}

#[test]
fn an_unknown_unit_warns_through_a_whole_file_import() {
    let path = temp_model_path("unknown-unit");
    let doc = small_model();
    fs::write(&path, doc.to_json_pretty().unwrap()).expect("could not write the test model");

    let options = ImportOptions {
        mass_unit: Some("stone".to_string()),
        ..ImportOptions::default()
    };
    let model = import_file(&path, &options).expect("import failed");
    fs::remove_file(&path).ok();

    let warning = model
        .validation
        .issues
        .iter()
        .find(|issue| issue.code == "units.unknown")
        .expect("no units.unknown finding");
    assert_eq!(warning.severity, Severity::Warning);
    // The value was carried through unchanged rather than guessed.
    assert!((model.mass() - 12.5).abs() < 1e-12);
    assert!(model.is_valid());
}

#[test]
fn importing_a_document_matches_importing_its_file() {
    let path = temp_model_path("round-trip");
    let doc = small_model();
    let text = doc.to_json_pretty().unwrap();
    fs::write(&path, &text).expect("could not write the test model");

    let from_file = import_file(&path, &ImportOptions::default()).expect("import failed");
    fs::remove_file(&path).ok();
    let from_document = import_document(&doc, &ImportOptions::default()).expect("import failed");

    assert_eq!(from_file.mass_properties, from_document.mass_properties);
    assert_eq!(from_file.frame, from_document.frame);
    assert_eq!(from_file.validation, from_document.validation);
    assert_eq!(from_file.source_hash, hex_model::hash_source(&text));
    assert_ne!(from_file.source_hash, from_document.source_hash);
}
