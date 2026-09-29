//! Validation rules for a dynamics model.
//!
//! Two entry points share one rule set: [`validate_document`] checks a document
//! straight off disk, and [`validate_imported`] checks the SI-canonical model the
//! importer produced. Both set the report subject to the model id.
//!
//! Every finding carries a stable code, a short title, a plain-language
//! explanation, a suggested fix, and the technical detail the UI shows only when
//! the user expands the row.

use hex_core::{
    BodyFrame, InertiaTensor, MassCurve, Real, ValidationIssue, ValidationReport, WorldFrame,
};

use crate::importer::ImportedModel;
use crate::schema::{InertiaDocument, MassCurveDocument, ModelDocument};

/// Relative tolerance for the inertia symmetry check.
pub const ASYMMETRY_REL_TOLERANCE: Real = 1e-9;

/// Relative tolerance passed to the principal-moment triangle check.
pub const TRIANGLE_REL_TOLERANCE: Real = 1e-9;

/// Relative mass growth between two samples that counts as suspicious.
///
/// One percent is not a physical limit; it is well above the rounding noise of a
/// mass-properties export and well below any real vehicle's mass growth.
pub const MASS_INCREASE_LIMIT: Real = 0.01;

/// Findings that describe the source document and cannot be re-derived from the
/// interpreted SI values, so [`validate_imported`] replays them from the report
/// recorded at import time.
const SOURCE_LEVEL_CODES: [&str; 3] = ["units.unknown", "inertia.asymmetric", "frame.unconfirmed"];

/// Append the `model.ok` note when the report has no error and no warning.
///
/// Informational notes do not stop the summary: a model whose only findings are
/// documented assumptions still passed every check.
pub fn finalize_report(report: &mut ValidationReport) {
    if report.error_count() == 0 && report.warning_count() == 0 {
        report.push(ValidationIssue::info(
            "model.ok",
            "Model passed every check",
            "No blocking or suspicious findings were recorded. The notes above describe the assumptions the importer made.",
        ));
    }
}

/// Warning for an inertia tensor whose mirrored entries disagree.
///
/// `asymmetry` is the largest mirrored difference and `scale` the largest entry,
/// so the tolerance stays meaningful for tensors of any magnitude.
pub fn inertia_asymmetry_issue(inertia: &InertiaDocument) -> Option<ValidationIssue> {
    let asymmetry = inertia.asymmetry();
    let scale = inertia.scale();
    let tolerance = ASYMMETRY_REL_TOLERANCE * scale.max(Real::MIN_POSITIVE);
    // A non-finite mismatch is not a shape problem, so only a real excess counts.
    if asymmetry.is_nan() || asymmetry <= tolerance {
        return None;
    }
    Some(
        ValidationIssue::warning(
            "inertia.asymmetric",
            "Inertia tensor is not symmetric",
            "The nine supplied entries do not mirror across the diagonal. A physical inertia tensor is always symmetric, so the mismatch points at a transcription or axis-order problem in the source data.",
        )
        .with_context("mass_properties.inertia")
        .with_suggestion(
            "Re-export the inertia tensor, or check that the exported body axes match the frame declared in this model.",
        )
        .with_technical(format!(
            "largest mirrored mismatch = {asymmetry}, tensor scale = {scale}, relative tolerance = {ASYMMETRY_REL_TOLERANCE}"
        )),
    )
}

/// Warning when the document never confirmed which frames it uses.
///
/// The frame fields are non-optional, so a file that simply omits them parses
/// into the defaults. An accidental default frame is the most common source of
/// sign errors, so it is worth one warning.
pub fn frame_confirmation_issue(doc: &ModelDocument) -> Option<ValidationIssue> {
    let frame = doc.frame();
    let is_default_frame =
        frame.body == BodyFrame::ForwardRightDown && frame.world == WorldFrame::Enu;
    if !is_default_frame || doc.is_frame_confirmed() {
        return None;
    }
    Some(
        ValidationIssue::warning(
            "frame.unconfirmed",
            "Frame convention is not confirmed",
            "This model uses the default frames (world ENU, body forward-right-down) without a frame_confirmed marker in its metadata, so the choice may be an unstated default rather than a deliberate one.",
        )
        .with_context("frame")
        .with_suggestion(
            "Add \"frame_confirmed\": true to the model metadata once the frames have been checked against the source data.",
        )
        .with_technical(format!(
            "world = {:?}, body = {:?}, metadata key frame_confirmed = absent or false",
            frame.world, frame.body
        )),
    )
}

/// Validate a document exactly as it was read from disk.
///
/// The report subject is set to the model id, and the report is finalized, so a
/// caller that prints the report straight away sees the same summary the UI does.
pub fn validate_document(doc: &ModelDocument, report: &mut ValidationReport) {
    report.subject = doc.model_id.clone();

    check_mass(doc.mass_properties.mass, report);
    check_cg_finite(doc.mass_properties.center_of_gravity.is_finite(), report);
    check_inertia_finite(doc.mass_properties.inertia.all_finite(), report);
    if doc.mass_properties.inertia.all_finite() {
        if let Some(issue) = inertia_asymmetry_issue(&doc.mass_properties.inertia) {
            report.push(issue);
        }
        check_inertia_shape(&doc.mass_properties.inertia.to_tensor(), report);
    }

    let curve = doc
        .mass_properties
        .mass_curve
        .as_ref()
        .map(MassCurveDocument::to_curve);
    check_mass_curve(curve.as_ref(), report);

    if let Some(mesh) = &doc.mesh {
        check_mesh_scale(mesh.scale, report);
    }
    if let Some(thrust) = &doc.thrust {
        check_thrust(&thrust.times, &thrust.thrusts, report);
    }
    if let Some(aero) = &doc.aerodynamics {
        check_aerodynamics(
            &aero.drag_coefficient_at_mach,
            aero.drag_only,
            &AeroDerivativeSummary::from_document(aero),
            report,
        );
    }

    check_geometry(
        doc.reference_geometry.reference_area,
        doc.reference_geometry.reference_length,
        report,
    );

    if let Some(issue) = frame_confirmation_issue(doc) {
        report.push(issue);
    }
    report.push(application_points_issue());
    finalize_report(report);
}

/// Validate an imported model from its SI-canonical values.
///
/// Source-level findings recorded during import (`units.unknown`,
/// `inertia.asymmetric`, `frame.unconfirmed`) are replayed from
/// `model.validation`, because they cannot be recovered from converted numbers.
/// A model built by hand without an import therefore has none of them.
pub fn validate_imported(model: &ImportedModel) -> ValidationReport {
    let mut report = ValidationReport::new(model.model_id.clone());

    check_mass(model.mass_properties.mass, &mut report);
    check_cg_finite(
        hex_core::vectors::is_finite(&model.mass_properties.center_of_gravity),
        &mut report,
    );
    check_inertia_finite(model.mass_properties.inertia.all_finite(), &mut report);
    if model.mass_properties.inertia.all_finite() {
        check_inertia_shape(&model.mass_properties.inertia, &mut report);
    }

    check_mass_curve(model.mass_curve.as_ref(), &mut report);

    if let Some(mesh) = &model.mesh {
        check_mesh_scale(mesh.scale, &mut report);
    }
    if let Some(thrust) = &model.thrust {
        check_thrust(&thrust.times, &thrust.thrusts, &mut report);
    }
    if let Some(aero) = &model.aerodynamics {
        check_aerodynamics(
            &aero.drag_coefficient_at_mach,
            aero.drag_only,
            &AeroDerivativeSummary::from_model(aero),
            &mut report,
        );
    }

    check_geometry(
        model.reference_geometry.reference_area,
        model.reference_geometry.reference_length,
        &mut report,
    );

    report.extend(
        model
            .validation
            .issues
            .iter()
            .filter(|issue| SOURCE_LEVEL_CODES.contains(&issue.code.as_str()))
            .cloned(),
    );

    report.push(application_points_issue());
    finalize_report(&mut report);
    report
}

fn check_mass(mass: Real, report: &mut ValidationReport) {
    if !mass.is_finite() {
        report.push(
            ValidationIssue::error(
                "mass.non_finite",
                "Mass is not a finite number",
                "The total mass is not a number or is infinite, so no equation of motion can be formed from it.",
            )
            .with_context("mass_properties.mass")
            .with_suggestion("Export the mass properties again and check for an empty or failed calculation.")
            .with_technical(format!("mass = {mass}")),
        );
        return;
    }
    if mass <= 0.0 {
        report.push(
            ValidationIssue::error(
                "mass.non_positive",
                "Mass is zero or negative",
                "A rigid body must have a positive total mass. A zero or negative value usually means the mass export failed or the wrong field was read.",
            )
            .with_context("mass_properties.mass")
            .with_suggestion("Set the total mass to a positive value.")
            .with_technical(format!("mass = {mass}")),
        );
    }
}

fn check_cg_finite(is_finite: bool, report: &mut ValidationReport) {
    if !is_finite {
        report.push(
            ValidationIssue::error(
                "cg.non_finite",
                "Centre of gravity is not a finite position",
                "At least one centre-of-gravity coordinate is not a number or is infinite, so the body datum cannot be located.",
            )
            .with_context("mass_properties.center_of_gravity")
            .with_suggestion("Check the centre-of-gravity coordinates in the source export.")
            .with_technical("center_of_gravity contains NaN or an infinity"),
        );
    }
}

fn check_inertia_finite(all_finite: bool, report: &mut ValidationReport) {
    if !all_finite {
        report.push(
            ValidationIssue::error(
                "inertia.non_finite",
                "Inertia tensor contains a non-finite value",
                "At least one of the nine inertia entries is not a number or is infinite, so angular motion cannot be integrated.",
            )
            .with_context("mass_properties.inertia")
            .with_suggestion("Re-export the inertia tensor and check for an empty or failed calculation.")
            .with_technical("at least one of ixx..izz is NaN or infinite"),
        );
    }
}

/// Positive-definiteness and the physical triangle inequalities.
///
/// Both checks need finite entries: the eigenvalues of a tensor containing NaN
/// are meaningless, so the caller skips them once the finiteness check has fired.
fn check_inertia_shape(inertia: &InertiaTensor, report: &mut ValidationReport) {
    if !inertia.is_positive_definite() {
        let moments = inertia.principal_moments();
        report.push(
            ValidationIssue::error(
                "inertia.not_positive_definite",
                "Inertia tensor is not positive definite",
                "The supplied tensor is not positive definite. Check the mass properties export and body reference frame.",
            )
            .with_context("mass_properties.inertia")
            .with_suggestion("Re-export the mass properties, then import the model again.")
            .with_technical(format!(
                "principal moments = [{}, {}, {}], determinant = {}",
                moments[0],
                moments[1],
                moments[2],
                inertia.determinant()
            )),
        );
        // The triangle check is only meaningful for a positive definite tensor.
        return;
    }

    let violations = inertia.triangle_violations(TRIANGLE_REL_TOLERANCE);
    if !violations.is_empty() {
        let moments = inertia.principal_moments();
        report.push(
            ValidationIssue::error(
                "inertia.triangle_inequality",
                "Principal moments violate the rigid-body triangle inequality",
                "The principal moments of a real rigid body must satisfy Ix + Iy >= Iz for every permutation. A tensor that breaks this cannot describe any distribution of mass.",
            )
            .with_context("mass_properties.inertia")
            .with_suggestion(
                "Check that the tensor is taken about the centre of gravity and expressed in the declared body frame.",
            )
            .with_technical(format!(
                "violated: {}; principal moments = [{}, {}, {}]",
                violations.join(", "),
                moments[0],
                moments[1],
                moments[2]
            )),
        );
    }
}

fn check_geometry(reference_area: Real, reference_length: Real, report: &mut ValidationReport) {
    // A NaN fails both tests, which is what a missing reference quantity deserves.
    let area_ok = reference_area > 0.0;
    let length_ok = reference_length > 0.0;
    if !area_ok || !length_ok {
        report.push(
            ValidationIssue::error(
                "geometry.non_positive",
                "Reference geometry is not positive",
                "Reference area and reference length scale every aerodynamic force and moment, so both must be positive finite numbers.",
            )
            .with_context("reference_geometry")
            .with_suggestion("Set the reference area and reference length to the values the aerodynamic coefficients were built for.")
            .with_technical(format!(
                "reference_area = {reference_area}, reference_length = {reference_length}"
            )),
        );
    }
}

/// The aerodynamic derivatives a model declares, reduced to what the checks need.
struct AeroDerivativeSummary {
    lift_slope: Real,
    side_slope: Real,
    pitch_slope: Real,
    yaw_slope: Real,
    roll_slope: Real,
    pitch_damping: Real,
    yaw_damping: Real,
    roll_damping: Real,
}

impl AeroDerivativeSummary {
    fn from_document(doc: &crate::schema::AeroDocument) -> Self {
        Self {
            lift_slope: doc.lift_slope,
            side_slope: doc.side_slope,
            pitch_slope: doc.pitch_slope,
            yaw_slope: doc.yaw_slope,
            // The document declares one roll derivative, the rate one.
            roll_slope: 0.0,
            pitch_damping: doc.pitch_damping,
            yaw_damping: doc.yaw_damping,
            roll_damping: doc.roll_damping,
        }
    }

    fn from_model(aero: &crate::importer::AeroModel) -> Self {
        Self {
            lift_slope: aero.lift_slope,
            side_slope: aero.side_slope,
            pitch_slope: aero.pitch_slope,
            yaw_slope: aero.yaw_slope,
            roll_slope: 0.0,
            pitch_damping: aero.pitch_damping,
            yaw_damping: aero.yaw_damping,
            roll_damping: aero.roll_damping,
        }
    }

    fn has_non_drag_terms(&self) -> bool {
        [
            self.lift_slope,
            self.side_slope,
            self.pitch_slope,
            self.yaw_slope,
            self.roll_slope,
            self.pitch_damping,
            self.yaw_damping,
            self.roll_damping,
        ]
        .iter()
        .any(|term| term.abs() > 1e-12)
    }
}

/// Check a drag table and the derivatives that go with it.
///
/// The table is read by interpolation, which assumes strictly increasing Mach
/// breakpoints. A table out of order does not fail, it silently reads the wrong
/// coefficient, so it is rejected here rather than producing a plausible run.
fn check_aerodynamics(
    drag_coefficient_at_mach: &[(Real, Real)],
    drag_only: bool,
    derivatives: &AeroDerivativeSummary,
    report: &mut ValidationReport,
) {
    let mut previous: Option<Real> = None;
    for (index, (mach, coefficient)) in drag_coefficient_at_mach.iter().enumerate() {
        if !mach.is_finite() || !coefficient.is_finite() {
            report.push(
                ValidationIssue::error(
                    "aero.non_finite",
                    "Aerodynamic coefficient is not finite",
                    "The drag table holds a Mach number or a coefficient that is not a finite number.",
                )
                .with_context(format!("aerodynamics.drag_coefficient_at_mach[{index}]"))
                .with_suggestion("Remove the entry or replace it with a measured value.")
                .with_technical(format!("mach = {mach}, cd = {coefficient}")),
            );
            return;
        }
        if let Some(previous) = previous {
            if *mach <= previous {
                report.push(
                    ValidationIssue::error(
                        "aero.table_order",
                        "Drag table is not in ascending Mach order",
                        "The drag coefficient is interpolated against Mach, which needs strictly increasing breakpoints. A table out of order would silently read the wrong coefficient.",
                    )
                    .with_context(format!("aerodynamics.drag_coefficient_at_mach[{index}]"))
                    .with_suggestion(
                        "Sort the table by Mach number and remove duplicate breakpoints.",
                    )
                    .with_technical(format!("mach = {mach}, previous = {previous}")),
                );
                return;
            }
        }
        previous = Some(*mach);
    }

    if drag_coefficient_at_mach.is_empty() {
        report.push(
            ValidationIssue::warning(
                "aero.no_drag_table",
                "No drag coefficient is declared",
                "The model declares an aerodynamics block but no drag table, so the run cannot take a drag coefficient from it.",
            )
            .with_context("aerodynamics")
            .with_suggestion(
                "Add at least one drag coefficient against Mach, or let the scenario use the built-in estimate.",
            ),
        );
    }

    if !drag_only && !derivatives.has_non_drag_terms() {
        report.push(
            ValidationIssue::info(
                "aero.no_derivatives",
                "The aerodynamics block has no lift or moment derivatives",
                "The model does not mark itself as drag only, but every lift, side force, and moment derivative is zero, so the run computes the drag-only motion either way.",
            )
            .with_context("aerodynamics")
            .with_suggestion(
                "Fill in the derivatives, or set drag_only so the run summary describes the model accurately.",
            ),
        );
    }
}

fn check_mesh_scale(scale: Real, report: &mut ValidationReport) {
    if !scale.is_finite() || scale <= 0.0 {
        report.push(
            ValidationIssue::warning(
                "mesh.scale_unknown",
                "Mesh scale is unknown",
                "A mesh is referenced but its scale is missing, zero, or not a number, so nothing can place the geometry at a known size.",
            )
            .with_context("mesh.scale")
            .with_suggestion(
                "Set mesh.scale to the size of one mesh unit, or pass mesh_scale_override when importing.",
            )
            .with_technical(format!("scale = {scale}")),
        );
    }
}

fn check_mass_curve(curve: Option<&MassCurve>, report: &mut ValidationReport) {
    let Some(curve) = curve else {
        return;
    };

    let problems = curve.validate();
    if !problems.is_empty() {
        report.push(
            ValidationIssue::error(
                "mass_curve.invalid",
                "Mass curve cannot be used",
                "Sample times must increase strictly and every mass sample must be a positive finite number, otherwise the curve cannot be interpolated.",
            )
            .with_context("mass_properties.mass_curve")
            .with_suggestion("Check the time and mass columns of the curve in the source data.")
            .with_technical(problems.join("; ")),
        );
    }

    // A model that is not refuelled cannot gain mass, so any rise beyond the
    // rounding noise of an export means the samples do not belong together.
    let gains: Vec<String> = curve
        .masses
        .windows(2)
        .filter(|window| window[0].is_finite() && window[1].is_finite() && window[0] > 0.0)
        .filter(|window| window[1] - window[0] > MASS_INCREASE_LIMIT * window[0])
        .map(|window| format!("{} kg then {} kg", window[0], window[1]))
        .collect();
    if !gains.is_empty() {
        report.push(
            ValidationIssue::warning(
                "mass_curve.suspicious",
                "Mass increases along the curve",
                "The sampled mass grows by more than one percent between two samples. Without refuelling a model cannot gain mass, so the samples may come from different configurations or the time axis may be reversed.",
            )
            .with_context("mass_properties.mass_curve")
            .with_suggestion("Check that the mass curve belongs to one configuration and that time runs forwards.")
            .with_technical(gains.join("; ")),
        );
    }
}

fn check_thrust(times: &[Real], thrusts: &[Real], report: &mut ValidationReport) {
    if times.is_empty() && thrusts.is_empty() {
        return;
    }

    if times.len() != thrusts.len() {
        report.push(
            ValidationIssue::error(
                "thrust.profile_invalid",
                "Thrust profile lengths do not match",
                format!(
                    "The profile has {} time samples but {} thrust samples, so the two columns cannot be paired.",
                    times.len(),
                    thrusts.len()
                ),
            )
            .with_context("thrust.times")
            .with_suggestion("Check the thrust table in the source data; every time needs exactly one thrust value.")
            .with_technical(format!("times = {}, thrusts = {}", times.len(), thrusts.len())),
        );
        return;
    }

    for index in 1..times.len() {
        let increases = times[index] > times[index - 1];
        if !increases {
            report.push(
                ValidationIssue::error(
                    "thrust.profile_invalid",
                    "Thrust profile times are not increasing",
                    "Thrust sample times must increase strictly, otherwise the profile cannot be interpolated.",
                )
                .with_context("thrust.times")
                .with_suggestion("Sort the thrust table by time and remove duplicate samples.")
                .with_technical(format!(
                    "index {}: {} s then {} s",
                    index,
                    times[index - 1],
                    times[index]
                )),
            );
            break;
        }
    }

    if let Some(first) = times.first() {
        if first.is_finite() && *first < 0.0 {
            report.push(
                ValidationIssue::error(
                    "thrust.profile_invalid",
                    "Thrust starts before the run begins",
                    "The first thrust sample sits at a negative time, which would mean ignition happens before the simulation starts.",
                )
                .with_context("thrust.times")
                .with_suggestion("Shift the thrust profile so ignition is at or after zero seconds.")
                .with_technical(format!("first sample = {first} s")),
            );
        }
    }

    if let (Some(first), Some(last)) = (times.first(), times.last()) {
        if last < first {
            report.push(
                ValidationIssue::error(
                    "thrust.profile_invalid",
                    "Thrust burnout precedes ignition",
                    "The last thrust sample is earlier than the first, so the profile runs backwards.",
                )
                .with_context("thrust.times")
                .with_suggestion("Check the time column of the thrust profile.")
                .with_technical(format!("ignition = {first} s, burnout = {last} s")),
            );
        }
    }
}

/// The one assumption the document cannot prove.
///
/// It is reported as a note on every model rather than hidden, because a force
/// applied at a point expressed in another frame is a silent wrong answer.
fn application_points_issue() -> ValidationIssue {
    ValidationIssue::info(
        "frame.application_points_assumed",
        "Force application points are assumed to share the CG frame",
        "Thrust and aerodynamic application points are interpreted in the same body frame as the centre of gravity. The document alone cannot prove that, so the assumption is recorded here.",
    )
    .with_context("thrust.application_point")
    .with_suggestion("Check that the exporter wrote every position in the declared body frame.")
    .with_technical("positions = declared body frame, origin = declared body datum")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importer::{import_document, ImportOptions};
    use crate::schema::{AeroDocument, MeshDocument, ThrustDocument};
    use hex_core::{Severity, Vec3Record};

    fn codes(report: &ValidationReport) -> Vec<String> {
        report.issues.iter().map(|i| i.code.clone()).collect()
    }

    fn has_code(report: &ValidationReport, code: &str) -> bool {
        report.issues.iter().any(|i| i.code == code)
    }

    fn finding<'a>(report: &'a ValidationReport, code: &str) -> &'a ValidationIssue {
        report
            .issues
            .iter()
            .find(|i| i.code == code)
            .unwrap_or_else(|| panic!("no issue with code {code}; got {:?}", codes(report)))
    }

    fn document() -> ModelDocument {
        ModelDocument::minimal("rocket-1", "Sounding rocket", 12.5, [0.9, 0.9, 0.1])
    }

    fn validate(doc: &ModelDocument) -> ValidationReport {
        let mut report = ValidationReport::new("");
        validate_document(doc, &mut report);
        report
    }

    #[test]
    fn clean_document_reports_only_notes() {
        let report = validate(&document());
        assert!(!report.has_errors());
        assert_eq!(report.warning_count(), 0);
        assert!(has_code(&report, "model.ok"));
        assert!(has_code(&report, "frame.application_points_assumed"));
        assert_eq!(report.status, hex_core::ValidationStatus::ValidWithWarnings);
    }

    #[test]
    fn report_subject_is_the_model_id() {
        let report = validate(&document());
        assert_eq!(report.subject, "rocket-1");
    }

    #[test]
    fn zero_mass_is_an_error() {
        let mut doc = document();
        doc.mass_properties.mass = 0.0;
        let report = validate(&doc);
        assert_eq!(
            finding(&report, "mass.non_positive").severity,
            Severity::Error
        );
        assert!(!has_code(&report, "model.ok"));
    }

    #[test]
    fn negative_mass_is_an_error() {
        let mut doc = document();
        doc.mass_properties.mass = -3.0;
        assert!(has_code(&validate(&doc), "mass.non_positive"));
    }

    #[test]
    fn non_finite_mass_is_one_issue_not_two() {
        let mut doc = document();
        doc.mass_properties.mass = Real::NAN;
        let report = validate(&doc);
        assert!(has_code(&report, "mass.non_finite"));
        assert!(!has_code(&report, "mass.non_positive"));
    }

    #[test]
    fn non_finite_centre_of_gravity_is_an_error() {
        let mut doc = document();
        doc.mass_properties.center_of_gravity = Vec3Record::new(Real::INFINITY, 0.0, 0.0);
        let report = validate(&doc);
        assert_eq!(finding(&report, "cg.non_finite").severity, Severity::Error);
    }

    #[test]
    fn non_finite_inertia_is_one_issue_and_skips_the_shape_checks() {
        let mut doc = document();
        doc.mass_properties.inertia = InertiaDocument::diagonal(Real::NAN, 1.0, 1.0);
        let report = validate(&doc);
        assert!(has_code(&report, "inertia.non_finite"));
        assert!(!has_code(&report, "inertia.not_positive_definite"));
        assert!(!has_code(&report, "inertia.triangle_inequality"));
        assert!(!has_code(&report, "inertia.asymmetric"));
    }

    #[test]
    fn asymmetric_inertia_warns_with_relative_tolerance() {
        let mut doc = document();
        doc.mass_properties.inertia =
            InertiaDocument::full(1.0, 0.001, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0);
        let report = validate(&doc);
        let issue = finding(&report, "inertia.asymmetric");
        assert_eq!(issue.severity, Severity::Warning);
        assert!(issue.technical.is_some());
        assert!(issue.suggestion.is_some());
    }

    #[test]
    fn tiny_asymmetry_within_tolerance_is_not_reported() {
        let mut doc = document();
        doc.mass_properties.inertia =
            InertiaDocument::full(1.0, 1e-12, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0);
        assert!(!has_code(&validate(&doc), "inertia.asymmetric"));
    }

    #[test]
    fn negative_definite_inertia_uses_the_spec_message() {
        let mut doc = document();
        doc.mass_properties.inertia = InertiaDocument::diagonal(-1.0, -1.0, -1.0);
        let report = validate(&doc);
        let issue = finding(&report, "inertia.not_positive_definite");
        assert_eq!(issue.severity, Severity::Error);
        assert_eq!(
            issue.detail,
            "The supplied tensor is not positive definite. Check the mass properties export and body reference frame."
        );
    }

    #[test]
    fn indefinite_inertia_is_an_error() {
        let mut doc = document();
        doc.mass_properties.inertia = InertiaDocument::diagonal(1.0, -1.0, 2.0);
        assert!(has_code(&validate(&doc), "inertia.not_positive_definite"));
    }

    #[test]
    fn triangle_inequality_violation_is_an_error() {
        let mut doc = document();
        doc.mass_properties.inertia = InertiaDocument::diagonal(1.0, 1.0, 5.0);
        let report = validate(&doc);
        let issue = finding(&report, "inertia.triangle_inequality");
        assert_eq!(issue.severity, Severity::Error);
        assert!(issue.technical.as_deref().unwrap().contains("Ix + Iy"));
        assert!(!has_code(&report, "inertia.not_positive_definite"));
    }

    #[test]
    fn triangle_inequality_holds_for_a_slender_body() {
        let mut doc = document();
        doc.mass_properties.inertia = InertiaDocument::diagonal(0.1, 5.0, 5.0);
        assert!(!has_code(&validate(&doc), "inertia.triangle_inequality"));
    }

    #[test]
    fn default_frame_without_confirmation_warns() {
        let mut doc = document();
        doc.metadata.clear();
        let report = validate(&doc);
        let issue = finding(&report, "frame.unconfirmed");
        assert_eq!(issue.severity, Severity::Warning);
    }

    #[test]
    fn confirmed_default_frame_does_not_warn() {
        let report = validate(&document());
        assert!(!has_code(&report, "frame.unconfirmed"));
    }

    #[test]
    fn explicit_non_default_frame_does_not_warn() {
        let mut doc = document();
        doc.metadata.clear();
        doc.frame.world = WorldFrame::Ned;
        assert!(!has_code(&validate(&doc), "frame.unconfirmed"));

        doc.frame.world = WorldFrame::Enu;
        doc.frame.body = BodyFrame::ForwardLeftUp;
        assert!(!has_code(&validate(&doc), "frame.unconfirmed"));
    }

    #[test]
    fn frame_confirmation_accepts_a_string_marker() {
        let mut doc = document();
        doc.metadata.insert(
            "frame_confirmed".to_string(),
            serde_json::Value::String("yes".to_string()),
        );
        assert!(!has_code(&validate(&doc), "frame.unconfirmed"));
    }

    #[test]
    fn application_point_assumption_is_always_reported() {
        let mut doc = document();
        doc.metadata.clear();
        doc.mass_properties.mass = -1.0;
        let report = validate(&doc);
        assert_eq!(
            finding(&report, "frame.application_points_assumed").severity,
            Severity::Info
        );
    }

    #[test]
    fn mesh_without_a_scale_warns() {
        let mut doc = document();
        doc.mesh = Some(MeshDocument {
            reference: "rocket.glb".to_string(),
            ..Default::default()
        });
        assert_eq!(
            finding(&validate(&doc), "mesh.scale_unknown").severity,
            Severity::Warning
        );
    }

    #[test]
    fn mesh_with_a_positive_scale_is_quiet() {
        let mut doc = document();
        doc.mesh = Some(MeshDocument {
            reference: "rocket.glb".to_string(),
            scale: 0.001,
            ..Default::default()
        });
        assert!(!has_code(&validate(&doc), "mesh.scale_unknown"));
    }

    #[test]
    fn curve_that_is_not_monotonic_is_an_error() {
        let mut doc = document();
        doc.mass_properties.mass_curve = Some(MassCurveDocument {
            times: vec![0.0, 0.0],
            masses: vec![10.0, 9.0],
            ..Default::default()
        });
        assert_eq!(
            finding(&validate(&doc), "mass_curve.invalid").severity,
            Severity::Error
        );
    }

    #[test]
    fn curve_that_gains_mass_warns() {
        let mut doc = document();
        doc.mass_properties.mass_curve = Some(MassCurveDocument {
            times: vec![0.0, 1.0],
            masses: vec![10.0, 12.0],
            ..Default::default()
        });
        let report = validate(&doc);
        assert!(!has_code(&report, "mass_curve.invalid"));
        assert_eq!(
            finding(&report, "mass_curve.suspicious").severity,
            Severity::Warning
        );
    }

    #[test]
    fn a_burn_curve_is_not_suspicious() {
        let mut doc = document();
        doc.mass_properties.mass_curve = Some(MassCurveDocument {
            times: vec![0.0, 1.0, 2.0],
            masses: vec![10.0, 8.0, 6.0],
            ..Default::default()
        });
        assert!(!has_code(&validate(&doc), "mass_curve.suspicious"));
    }

    #[test]
    fn curve_with_mismatched_tracks_is_an_error() {
        let mut doc = document();
        doc.mass_properties.mass_curve = Some(MassCurveDocument {
            times: vec![0.0, 1.0],
            masses: vec![10.0, 9.0],
            centers_of_gravity: vec![Vec3Record::default()],
            ..Default::default()
        });
        assert!(has_code(&validate(&doc), "mass_curve.invalid"));
    }

    #[test]
    fn thrust_length_mismatch_is_an_error() {
        let mut doc = document();
        doc.thrust = Some(ThrustDocument {
            times: vec![0.0, 1.0],
            thrusts: vec![100.0],
            ..Default::default()
        });
        assert_eq!(
            finding(&validate(&doc), "thrust.profile_invalid").severity,
            Severity::Error
        );
    }

    #[test]
    fn thrust_times_must_increase() {
        let mut doc = document();
        doc.thrust = Some(ThrustDocument {
            times: vec![0.0, 1.0, 0.5],
            thrusts: vec![100.0, 80.0, 60.0],
            ..Default::default()
        });
        let report = validate(&doc);
        assert!(finding(&report, "thrust.profile_invalid")
            .detail
            .contains("increase strictly"));
    }

    #[test]
    fn thrust_burnout_before_ignition_is_an_error() {
        let mut doc = document();
        doc.thrust = Some(ThrustDocument {
            times: vec![5.0, 2.0],
            thrusts: vec![10.0, 8.0],
            ..Default::default()
        });
        let report = validate(&doc);
        assert!(report.issues.iter().any(|issue| {
            issue.code == "thrust.profile_invalid"
                && issue.detail.contains("earlier than the first")
        }));
    }

    #[test]
    fn thrust_before_zero_seconds_is_an_error() {
        let mut doc = document();
        doc.thrust = Some(ThrustDocument {
            times: vec![-1.0, 1.0],
            thrusts: vec![10.0, 8.0],
            ..Default::default()
        });
        assert!(finding(&validate(&doc), "thrust.profile_invalid")
            .title
            .contains("before the run"));
    }

    #[test]
    fn a_sane_thrust_profile_is_quiet() {
        let mut doc = document();
        doc.thrust = Some(ThrustDocument {
            times: vec![0.0, 1.0, 3.0],
            thrusts: vec![0.0, 500.0, 0.0],
            ..Default::default()
        });
        assert!(!has_code(&validate(&doc), "thrust.profile_invalid"));
    }

    #[test]
    fn an_empty_thrust_profile_is_quiet() {
        let mut doc = document();
        doc.thrust = Some(ThrustDocument::default());
        assert!(!has_code(&validate(&doc), "thrust.profile_invalid"));
    }

    #[test]
    fn non_positive_geometry_is_an_error() {
        let mut doc = document();
        doc.reference_geometry.reference_area = 0.0;
        assert_eq!(
            finding(&validate(&doc), "geometry.non_positive").severity,
            Severity::Error
        );
    }

    #[test]
    fn negative_reference_length_is_an_error() {
        let mut doc = document();
        doc.reference_geometry.reference_length = -0.2;
        assert!(has_code(&validate(&doc), "geometry.non_positive"));
    }

    #[test]
    fn aero_coefficients_are_not_validated_for_sign() {
        let mut doc = document();
        doc.aerodynamics = Some(AeroDocument {
            drag_coefficient_at_mach: vec![(0.0, -0.02)],
            ..Default::default()
        });
        let report = validate(&doc);
        assert!(!report.has_errors());
    }

    #[test]
    fn validate_document_can_be_used_incrementally() {
        let mut report = ValidationReport::new("");
        validate_document(&document(), &mut report);
        assert_eq!(report.summary(), "0 error(s), 0 warning(s), 2 note(s)");

        // A report that already carries a warning must not claim the model is fine.
        let mut fresh = ValidationReport::new("x");
        fresh.add_warning("w", "W", "detail");
        finalize_report(&mut fresh);
        assert!(!has_code(&fresh, "model.ok"));
    }

    #[test]
    fn a_drag_table_out_of_mach_order_is_an_error() {
        // Interpolation assumes ascending breakpoints. A table out of order would
        // read the wrong coefficient without failing, so it is rejected.
        let mut doc = document();
        doc.aerodynamics = Some(AeroDocument {
            drag_coefficient_at_mach: vec![(0.0, 0.30), (1.2, 0.55), (0.8, 0.42)],
            drag_only: true,
            ..Default::default()
        });
        let report = validate(&doc);
        assert!(has_code(&report, "aero.table_order"));
        assert!(report.has_errors());

        // Duplicate breakpoints are just as unusable.
        let mut duplicate = document();
        duplicate.aerodynamics = Some(AeroDocument {
            drag_coefficient_at_mach: vec![(0.5, 0.30), (0.5, 0.40)],
            drag_only: true,
            ..Default::default()
        });
        assert!(has_code(&validate(&duplicate), "aero.table_order"));
    }

    #[test]
    fn a_non_finite_aerodynamic_coefficient_is_an_error() {
        let mut doc = document();
        doc.aerodynamics = Some(AeroDocument {
            drag_coefficient_at_mach: vec![(0.0, f64::NAN)],
            drag_only: true,
            ..Default::default()
        });
        assert!(has_code(&validate(&doc), "aero.non_finite"));
    }

    #[test]
    fn an_aerodynamics_block_without_derivatives_is_a_note() {
        // The model claims not to be drag only and carries zeros everywhere, so
        // the run would compute the drag-only motion either way. That is worth
        // saying, but it is not an error.
        let mut doc = document();
        doc.aerodynamics = Some(AeroDocument {
            drag_coefficient_at_mach: vec![(0.0, 0.45)],
            drag_only: false,
            ..Default::default()
        });
        let report = validate(&doc);
        assert!(has_code(&report, "aero.no_derivatives"));
        assert!(!report.has_errors());

        // With one derivative present, the note goes away.
        let mut with_lift = document();
        with_lift.aerodynamics = Some(AeroDocument {
            drag_coefficient_at_mach: vec![(0.0, 0.45)],
            lift_slope: 2.0,
            drag_only: false,
            ..Default::default()
        });
        assert!(!has_code(&validate(&with_lift), "aero.no_derivatives"));
    }

    #[test]
    fn imported_model_matches_document_validation() {
        let doc = document();
        let mut from_document = validate(&doc);
        let imported = import_document(&doc, &ImportOptions::default()).unwrap();
        let from_import = validate_imported(&imported);

        from_document.issues.sort_by(|a, b| a.code.cmp(&b.code));
        let mut sorted_import = from_import.clone();
        sorted_import.issues.sort_by(|a, b| a.code.cmp(&b.code));
        assert_eq!(codes(&from_document), codes(&sorted_import));
        assert_eq!(from_import, imported.validation);
    }

    #[test]
    fn source_level_findings_survive_revalidation() {
        let mut doc = document();
        doc.metadata.clear();
        doc.mass_properties.inertia =
            InertiaDocument::full(0.9, 0.01, 0.0, 0.0, 0.9, 0.0, 0.0, 0.0, 0.1);
        let imported = import_document(&doc, &ImportOptions::default()).unwrap();
        let again = validate_imported(&imported);
        assert!(has_code(&again, "frame.unconfirmed"));
        assert!(has_code(&again, "inertia.asymmetric"));
        assert_eq!(again, imported.validation);
    }
}
