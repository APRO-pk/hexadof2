//! Dynamics model import and validation commands.

use hex_core::UnitSystem;
use hex_core::ValidationReport;
use hex_model::{ImportOptions, ImportedModel, ModelDocument};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::CommandError;
use crate::events;
use crate::state::{emit_notice, AppState};

/// A model summary for the model panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelView {
    /// Model identifier.
    pub model_id: String,
    /// Model version.
    pub model_version: String,
    /// Display name.
    pub name: String,
    /// Description, when present.
    pub description: Option<String>,
    /// Unit system label.
    pub unit_system: String,
    /// World frame label.
    pub world_frame: String,
    /// Body frame label.
    pub body_frame: String,
    /// Total mass in kilograms.
    pub mass: f64,
    /// Centre of gravity in the body frame, metres.
    pub center_of_gravity: [f64; 3],
    /// Inertia tensor about the centre of gravity.
    pub inertia: [f64; 6],
    /// Reference area in square metres.
    pub reference_area: f64,
    /// Reference length in metres.
    pub reference_length: f64,
    /// Mesh reference, when the model declares one.
    pub mesh_reference: Option<String>,
    /// Whether the model has a thrust profile.
    pub has_thrust: bool,
    /// Whether the model has a mass curve.
    pub has_mass_curve: bool,
    /// Whether the model declares aerodynamic coefficients.
    pub has_aerodynamics: bool,
    /// One line describing the aerodynamics block, when there is one.
    pub aerodynamics_summary: Option<String>,
    /// Drag table entries the model declares.
    pub drag_table_entries: usize,
    /// Whether the aerodynamics block carries lift or moment derivatives.
    pub has_lift_or_moments: bool,
    /// Content hash of the source.
    pub source_hash: String,
    /// Whether validation found no blocking errors.
    pub valid: bool,
    /// Validation summary line.
    pub validation_summary: String,
    /// Number of blocking errors.
    pub error_count: usize,
    /// Number of warnings.
    pub warning_count: usize,
    /// The full validation report.
    pub validation: ValidationReport,
}

/// Whether an aerodynamics block carries any lift, side-force, or moment term.
fn aero_has_lift_or_moments(aero: &hex_model::AeroModel) -> bool {
    [
        aero.lift_slope,
        aero.lift_zero,
        aero.side_slope,
        aero.pitch_slope,
        aero.pitch_zero,
        aero.yaw_slope,
        aero.pitch_damping,
        aero.yaw_damping,
        aero.roll_damping,
    ]
    .iter()
    .any(|term| term.abs() > 1e-12)
}

/// One line describing what a model's aerodynamics block can supply.
fn aero_summary(aero: &hex_model::AeroModel) -> String {
    let drag = match aero.drag_coefficient_at_mach.len() {
        0 => "no drag table".to_string(),
        1 => format!("constant Cd {:.4}", aero.drag_coefficient_at_mach[0].1),
        n => format!("drag table with {n} Mach entries"),
    };
    let lift = if aero_has_lift_or_moments(aero) {
        "lift and moment derivatives"
    } else {
        "no lift or moment derivatives"
    };
    format!("{drag}, {lift}, drag only: {}", aero.drag_only)
}

impl ModelView {
    fn from_imported(model: &ImportedModel) -> Self {
        let inertia = model.inertia();
        let reflected = model.reference_geometry;
        Self {
            model_id: model.model_id.clone(),
            model_version: model.model_version.clone(),
            name: model.name.clone(),
            description: model.description.clone(),
            unit_system: model.unit_system.label().to_string(),
            world_frame: model.frame.world.label().to_string(),
            body_frame: model.frame.body.label().to_string(),
            mass: model.mass(),
            center_of_gravity: {
                let cg = model.center_of_gravity();
                [cg.x, cg.y, cg.z]
            },
            inertia: [
                inertia.ixx,
                inertia.iyy,
                inertia.izz,
                inertia.ixy,
                inertia.ixz,
                inertia.iyz,
            ],
            reference_area: reflected.reference_area,
            reference_length: reflected.reference_length,
            mesh_reference: model.mesh.as_ref().map(|m| m.reference.clone()),
            has_thrust: model.thrust.is_some(),
            has_mass_curve: model.mass_curve.is_some(),
            has_aerodynamics: model.aerodynamics.is_some(),
            aerodynamics_summary: model.aerodynamics.as_ref().map(aero_summary),
            drag_table_entries: model
                .aerodynamics
                .as_ref()
                .map(|a| a.drag_coefficient_at_mach.len())
                .unwrap_or(0),
            has_lift_or_moments: model
                .aerodynamics
                .as_ref()
                .map(aero_has_lift_or_moments)
                .unwrap_or(false),
            source_hash: model.source_hash.clone(),
            valid: !model.validation.has_errors(),
            validation_summary: model.validation.summary(),
            error_count: model.validation.error_count(),
            warning_count: model.validation.warning_count(),
            validation: model.validation.clone(),
        }
    }
}

/// What a model import produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelImportResult {
    /// The interpreted model.
    pub model: ModelView,
    /// Whether the model is usable for a run.
    pub usable: bool,
    /// Store kept for the frontend to inspect the raw document.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_json: Option<String>,
}

/// Import options supplied by the model import dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelImportRequest {
    /// Absolute path to the model file, or a raw JSON body when `text` is set.
    pub path: Option<String>,
    /// Raw JSON text, when the frontend already read the file.
    pub text: Option<String>,
    /// Unit system to assume. `unknown` lets the document metadata decide.
    pub unit_system: Option<String>,
    /// Override the mesh scale.
    pub mesh_scale: Option<f64>,
}

impl Default for ModelImportRequest {
    fn default() -> Self {
        Self {
            path: None,
            text: None,
            unit_system: Some("si".to_string()),
            mesh_scale: None,
        }
    }
}

fn parse_unit_system(value: Option<&str>) -> UnitSystem {
    match value.map(|v| v.trim().to_ascii_lowercase()) {
        Some(v) if v == "imperial" => UnitSystem::Imperial,
        Some(v) if v == "metric_degrees" || v == "metric" => UnitSystem::MetricDegrees,
        Some(v) if v == "unknown" || v == "auto" => UnitSystem::Unknown,
        _ => UnitSystem::Si,
    }
}

impl ModelImportRequest {
    /// The importer options this request describes.
    pub fn to_options(&self) -> ImportOptions {
        ImportOptions {
            unit_system: parse_unit_system(self.unit_system.as_deref()),
            mesh_scale_override: self.mesh_scale,
            ..ImportOptions::default()
        }
    }
}

/// Import a dynamics model from a file or from JSON text.
#[tauri::command]
pub fn model_import(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ModelImportRequest,
) -> Result<ModelImportResult, CommandError> {
    let options = request.to_options();
    let (imported, document_json) = match (&request.path, &request.text) {
        (Some(path), _) if !path.trim().is_empty() => {
            let text = std::fs::read_to_string(path).map_err(|e| {
                CommandError::new(
                    "model.io",
                    "The model file could not be read",
                    format!("{} could not be opened: {}", path, e),
                    "Check that the file exists and that it is readable.",
                )
            })?;
            let imported = hex_model::import_file(std::path::Path::new(path), &options)?;
            (imported, Some(text))
        }
        (_, Some(text)) => {
            let imported = hex_model::import_json(text, &options)?;
            (imported, Some(text.to_string()))
        }
        _ => {
            return Err(crate::error::invalid(
                "Supply a model path or the model JSON.",
                "Choose a file in the import dialog.",
            ))
        }
    };

    let view = ModelView::from_imported(&imported);
    let usable = view.valid;

    state.with(|s| {
        s.models.push(imported);
        Ok::<(), CommandError>(())
    })??;

    if view.valid {
        emit_notice(
            &app,
            events::NoticeEvent::info(
                "Model imported",
                format!(
                    "{} v{} ({}).",
                    view.name, view.model_version, view.validation_summary
                ),
            ),
        );
    } else {
        emit_notice(
            &app,
            events::NoticeEvent::error(
                "Model imported with blocking errors",
                format!("{} has {} blocking error(s).", view.name, view.error_count),
            ),
        );
    }

    Ok(ModelImportResult {
        model: view,
        usable,
        document_json,
    })
}

/// Validate a model without importing it into the session.
#[tauri::command]
pub fn model_validate(
    app: AppHandle,
    request: ModelImportRequest,
) -> Result<ValidationReport, CommandError> {
    let options = request.to_options();
    let imported = match (&request.path, &request.text) {
        (Some(path), _) if !path.trim().is_empty() => {
            hex_model::import_file(std::path::Path::new(path), &options)?
        }
        (_, Some(text)) => hex_model::import_json(text, &options)?,
        _ => {
            return Err(crate::error::invalid(
                "Supply a model path or the model JSON.",
                "Choose a file in the import dialog.",
            ))
        }
    };

    emit_notice(
        &app,
        events::NoticeEvent::info("Validation finished", imported.validation.summary()),
    );
    crate::state::emit(
        &app,
        events::channel::VALIDATION_RESULT,
        events::ValidationResultEvent {
            subject: imported.model_id.clone(),
            status: imported.validation.status.label().to_string(),
            errors: imported.validation.error_count(),
            warnings: imported.validation.warning_count(),
        },
    );
    Ok(imported.validation)
}

/// Report the imported models, newest last.
#[tauri::command]
pub fn model_list(state: State<'_, AppState>) -> Result<Vec<ModelView>, CommandError> {
    state.with(|s| s.models.iter().map(ModelView::from_imported).collect())
}

/// The most recently imported model.
#[tauri::command]
pub fn model_current(state: State<'_, AppState>) -> Result<Option<ModelView>, CommandError> {
    state.with(|s| s.models.last().map(ModelView::from_imported))
}

/// Drop every imported model.
#[tauri::command]
pub fn model_clear(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.with(|s| {
        s.models.clear();
    })
}

/// The example model shipped with the application, so a new user can run something
/// without importing anything.
///
/// The geometry is declared rather than left to the generic minimal document,
/// because reference area and length scale every aerodynamic force and moment. At
/// the one square metre a minimal document carries, a 4.5 kg rocket would fly as
/// if it were a sheet of plywood, and the drag coefficient would have to be
/// fudged to hide it.
#[tauri::command]
pub fn model_example() -> Result<String, CommandError> {
    let mut document = ModelDocument::minimal(
        "builtin.example.rocket",
        "Built-in example rocket",
        4.5,
        [0.09, 0.09, 0.012],
    );
    // A 125 mm sounding rocket, 0.61 m long. A slender body of this mass has
    // about this frontal area, and every coefficient is quoted against it.
    document.reference_geometry = hex_model::ReferenceGeometryDocument {
        reference_area: 0.0123,
        reference_length: 0.61,
        body_diameter: 0.125,
    };
    document.to_json_pretty().map_err(|e| {
        CommandError::new(
            "model.example",
            "The example model could not be produced",
            e.to_string(),
            "Report this as a bug.",
        )
    })
}

/// Import the built-in example model.
#[tauri::command]
pub fn model_import_example(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<ModelImportResult, CommandError> {
    let text = model_example()?;
    model_import(
        app,
        state,
        ModelImportRequest {
            path: None,
            text: Some(text.clone()),
            unit_system: Some("si".to_string()),
            mesh_scale: None,
        },
    )
    .map(|mut result| {
        result.document_json = Some(text);
        result
    })
}

/// Save the most recent model into the open project.
#[tauri::command]
pub fn model_save_to_project(state: State<'_, AppState>) -> Result<String, CommandError> {
    let project = state.require_project()?;
    let model = state.require_model()?;
    let text = ModelDocument::minimal(&model.model_id, &model.name, model.mass(), {
        let p = model.inertia().principal_moments();
        [p[0], p[1], p[2]]
    })
    .to_json_pretty()
    .map_err(|e| {
        CommandError::new(
            "model.save",
            "The model could not be written",
            e.to_string(),
            "Report this as a bug.",
        )
    })?;
    let file_name = format!(
        "{}.dynamic.json",
        hex_project::io::slugify(&model.model_id, "model")
    );
    let directory = project.models_dir();
    std::fs::create_dir_all(&directory)
        .map_err(|e| hex_project::ProjectError::io(&directory, e))?;
    let path = directory.join(&file_name);
    hex_project::io::write_atomic(&path, text.as_bytes())?;
    Ok(format!("models/{}", file_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_request_defaults_to_si() {
        let request = ModelImportRequest::default();
        assert_eq!(request.to_options().unit_system, UnitSystem::Si);
        assert!(request.path.is_none());
    }

    #[test]
    fn unit_system_parsing_accepts_the_documented_spellings() {
        assert_eq!(parse_unit_system(Some("si")), UnitSystem::Si);
        assert_eq!(parse_unit_system(Some("SI")), UnitSystem::Si);
        assert_eq!(parse_unit_system(Some("imperial")), UnitSystem::Imperial);
        assert_eq!(
            parse_unit_system(Some("metric_degrees")),
            UnitSystem::MetricDegrees
        );
        assert_eq!(parse_unit_system(Some("unknown")), UnitSystem::Unknown);
        assert_eq!(parse_unit_system(Some("auto")), UnitSystem::Unknown);
        assert_eq!(parse_unit_system(None), UnitSystem::Si);
        assert_eq!(parse_unit_system(Some("nonsense")), UnitSystem::Si);
    }

    #[test]
    fn the_example_model_is_valid_and_importable() {
        let text = model_example().unwrap();
        let imported = hex_model::import_json(&text, &ImportOptions::default()).unwrap();
        let view = ModelView::from_imported(&imported);
        assert!(view.valid, "{:?}", view.validation);
        assert_eq!(view.error_count, 0);
        assert!((view.mass - 4.5).abs() < 1e-9);
        assert!(view.inertia[0] > 0.0);
        assert!(!view.source_hash.is_empty());
        assert!(view.validation_summary.contains("error"));
    }

    #[test]
    fn the_example_model_declares_the_geometry_its_coefficients_are_quoted_against() {
        // A minimal document carries a one square metre reference area, which is
        // not a rocket. Aerodynamic force scales with it, so the example has to
        // say what it is: a 125 mm body.
        let text = model_example().unwrap();
        let imported = hex_model::import_json(&text, &ImportOptions::default()).unwrap();
        let geometry = imported.reference_geometry;
        assert!(
            (geometry.reference_area - 0.0123).abs() < 1e-12,
            "reference area was {}",
            geometry.reference_area
        );
        assert!((geometry.reference_length - 0.61).abs() < 1e-12);
        assert!((geometry.body_diameter - 0.125).abs() < 1e-12);

        // The reference length has to be the body length for the moment
        // coefficients to mean anything.
        assert!(geometry.reference_length > geometry.body_diameter);
    }

    #[test]
    fn a_model_with_a_bad_inertia_reports_blocking_errors() {
        let mut document = ModelDocument::minimal("bad", "Bad", 1.0, [1.0, 1.0, 1.0]);
        document.mass_properties.inertia = hex_model::InertiaDocument {
            ixx: -1.0,
            ixy: 0.0,
            ixz: 0.0,
            iyx: 0.0,
            iyy: 1.0,
            iyz: 0.0,
            izx: 0.0,
            izy: 0.0,
            izz: 1.0,
        };
        let text = document.to_json_pretty().unwrap();
        let imported = hex_model::import_json(&text, &ImportOptions::default()).unwrap();
        let view = ModelView::from_imported(&imported);
        assert!(!view.valid);
        assert!(view.error_count > 0);
        assert!(view
            .validation
            .issues
            .iter()
            .any(|i| i.code.contains("inertia")));
    }

    #[test]
    fn model_view_frames_are_human_readable() {
        let text = model_example().unwrap();
        let imported = hex_model::import_json(&text, &ImportOptions::default()).unwrap();
        let view = ModelView::from_imported(&imported);
        assert!(view.world_frame.contains("ENU"));
        assert!(view.body_frame.contains("forward"));
        assert_eq!(view.unit_system, "SI (m, kg, s, rad)");
    }
}
