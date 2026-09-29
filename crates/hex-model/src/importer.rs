//! Import a model document and interpret it as SI-canonical data.
//!
//! Every value that carries a unit is converted here, once, through
//! [`QuantityKind::scale_for`], so nothing downstream ever needs a conversion
//! factor. A unit string that cannot be resolved is reported as a
//! `units.unknown` warning and the raw value is carried through unconverted:
//! silently guessing a factor would hide a wrong number, while a warning makes
//! it visible.
//!
//! An import never fails because the physics looks wrong. It produces an
//! [`ImportedModel`] whose `validation` report holds every finding, and the UI is
//! responsible for showing it.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::Path;

use hex_core::{
    Frame, InertiaTensor, MassCurve, MassProperties, QuantityKind, Quaternion, Real, UnitSystem,
    ValidationIssue, ValidationReport, Vec3, Vec3Record,
};

use crate::error::ModelError;
use crate::schema::{interp_table, AeroDocument, InertiaDocument, ModelDocument};
use crate::validator::{frame_confirmation_issue, inertia_asymmetry_issue, validate_imported};
use crate::versioning::{check_schema_version, migrate_document};

/// Reference geometry resolved to SI.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ReferenceGeometry {
    /// Aerodynamic reference area, m^2.
    pub reference_area: Real,
    /// Aerodynamic reference length, m.
    pub reference_length: Real,
    /// Body diameter, m. Zero when the body is not axisymmetric.
    pub body_diameter: Real,
}

/// A mesh reference resolved to SI.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshReference {
    /// Path to the mesh, exactly as written in the document.
    pub reference: String,
    /// Metres per mesh unit.
    pub scale: Real,
    /// Fixed orientation offset as a rotation vector, in radians.
    pub orientation: Vec3,
}

/// A thrust profile resolved to SI.
#[derive(Debug, Clone, PartialEq)]
pub struct ThrustModel {
    /// Application point in the body frame, m.
    pub application_point: Vec3,
    /// Unit thrust direction in the body frame.
    pub direction: Vec3,
    /// Fixed misalignment rotation, when the source declared one.
    pub misalignment: Option<Quaternion>,
    /// Sample times, s.
    pub times: Vec<Real>,
    /// Thrust magnitude at each sample time, N.
    pub thrusts: Vec<Real>,
    /// Constant engine moment about the centre of gravity, N*m.
    pub engine_moment: Vec3,
}

impl ThrustModel {
    /// Whether the profile can be interpolated.
    pub fn is_usable(&self) -> bool {
        self.times.len() == self.thrusts.len() && self.times.len() >= 2
    }

    /// Total impulse over the profile, N*s.
    ///
    /// Returns `None` when the profile is unusable or its times do not strictly
    /// increase, because a trapezoid rule over unordered samples is meaningless.
    pub fn total_impulse(&self) -> Option<Real> {
        if !self.is_usable() {
            return None;
        }
        let mut impulse = 0.0;
        for index in 1..self.times.len() {
            let dt = self.times[index] - self.times[index - 1];
            if dt <= 0.0 || !dt.is_finite() {
                return None;
            }
            impulse += 0.5 * (self.thrusts[index] + self.thrusts[index - 1]) * dt;
        }
        Some(impulse)
    }
}

/// Aerodynamic data as stored, kept dimensionless.
///
/// Coefficients and their slopes are ratios; only the reference geometry they are
/// multiplied by carries a unit, so nothing here is converted.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AeroModel {
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

impl AeroModel {
    /// Copy the dimensionless coefficients out of a document.
    pub fn from_document(doc: &AeroDocument) -> Self {
        Self {
            drag_coefficient_at_mach: doc.drag_coefficient_at_mach.clone(),
            lift_slope: doc.lift_slope,
            lift_zero: doc.lift_zero,
            side_slope: doc.side_slope,
            pitch_zero: doc.pitch_zero,
            pitch_slope: doc.pitch_slope,
            yaw_slope: doc.yaw_slope,
            pitch_damping: doc.pitch_damping,
            yaw_damping: doc.yaw_damping,
            roll_damping: doc.roll_damping,
            drag_only: doc.drag_only,
        }
    }

    /// Drag coefficient at a Mach number, clamped to the sampled range.
    pub fn drag_coefficient_at(&self, mach: Real) -> Option<Real> {
        interp_table(&self.drag_coefficient_at_mach, mach)
    }
}

/// How a document's numbers should be interpreted.
///
/// The defaults are SI with no overrides, so an import with
/// [`ImportOptions::default`] converts nothing unless the document declares a
/// different unit system in its metadata.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImportOptions {
    /// Unit system the source uses.
    ///
    /// This option wins over anything the document says, because it is what the
    /// user chose in the import dialog. Set it to [`UnitSystem::Unknown`] to let
    /// the document metadata decide, and to receive a warning when nothing does.
    pub unit_system: UnitSystem,
    /// Frame to interpret the model in, replacing the one in the document.
    pub frame: Option<Frame>,
    /// Unit string for mass, for example `"kg"`, `"g"`, `"lb"`, or `"slug"`.
    pub mass_unit: Option<String>,
    /// Unit string for lengths, for example `"m"`, `"cm"`, or `"ft"`.
    pub length_unit: Option<String>,
    /// Unit string for moments of inertia, for example `"kg*m^2"` or `"g*cm^2"`.
    pub inertia_unit: Option<String>,
    /// Metres per mesh unit, replacing the mesh scale in the document.
    pub mesh_scale_override: Option<Real>,
}

impl ImportOptions {
    /// Explicit SI import with no per-quantity overrides.
    pub fn si() -> Self {
        Self::default()
    }

    /// Imperial defaults: slugs, feet, slug*ft^2, pound-force.
    pub fn imperial() -> Self {
        Self {
            unit_system: UnitSystem::Imperial,
            ..Self::default()
        }
    }

    /// Metric with degrees for angles.
    pub fn metric_degrees() -> Self {
        Self {
            unit_system: UnitSystem::MetricDegrees,
            ..Self::default()
        }
    }
}

/// An interpreted model: SI-canonical data plus everything the import learned.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportedModel {
    /// Stable identity of the model.
    pub model_id: String,
    /// Author-chosen version of the model.
    pub model_version: String,
    /// Human-readable name.
    pub name: String,
    /// Optional longer description.
    pub description: Option<String>,
    /// Frames the interpreted quantities are expressed in.
    pub frame: Frame,
    /// Reference geometry in SI.
    pub reference_geometry: ReferenceGeometry,
    /// Mass, centre of gravity, and inertia, all SI.
    pub mass_properties: MassProperties,
    /// Mass curve in SI, when the document declared one.
    pub mass_curve: Option<MassCurve>,
    /// Mesh reference in SI, when the document declared one.
    pub mesh: Option<MeshReference>,
    /// Thrust profile in SI, when the document declared one.
    pub thrust: Option<ThrustModel>,
    /// Aerodynamic data, when the document declared any.
    pub aerodynamics: Option<AeroModel>,
    /// Every finding from the import and the model checks. The UI must show this.
    pub validation: ValidationReport,
    /// Unit system the import used.
    pub unit_system: UnitSystem,
    /// Stable hash of the source text, for change detection.
    pub source_hash: String,
}

impl ImportedModel {
    /// Total mass in kilograms.
    pub fn mass(&self) -> Real {
        self.mass_properties.mass
    }

    /// Centre of gravity in the body frame, metres.
    pub fn center_of_gravity(&self) -> Vec3 {
        self.mass_properties.center_of_gravity
    }

    /// Inertia about the centre of gravity, in the body frame.
    pub fn inertia(&self) -> InertiaTensor {
        self.mass_properties.inertia
    }

    /// Mass at a time, falling back to the constant mass when no curve is usable.
    pub fn mass_at(&self, time: Real) -> Real {
        match &self.mass_curve {
            Some(curve) => curve.mass_at(time).unwrap_or(self.mass_properties.mass),
            None => self.mass_properties.mass,
        }
    }

    /// Whether the import recorded no blocking finding.
    pub fn is_valid(&self) -> bool {
        !self.validation.has_errors()
    }

    /// Fail with [`ModelError::ValidationFailed`] when the model has errors.
    ///
    /// Use this at the point where the app decides to run a simulation, not at
    /// the point where it loads a file: the user has to see the report first.
    pub fn check_valid(&self) -> Result<(), ModelError> {
        if self.validation.has_errors() {
            return Err(ModelError::validation_failed(self.validation.clone()));
        }
        Ok(())
    }

    /// One-line description for a status strip.
    pub fn summary(&self) -> String {
        format!(
            "{} ({}) loaded as {}: {}",
            self.name,
            self.model_id,
            self.unit_system.label(),
            self.validation.summary()
        )
    }
}

/// Convert a parsed document into an SI-canonical model.
///
/// The document is hashed from its canonical JSON, because no source text exists
/// at this point. Use [`import_json`] or [`import_file`] to keep the hash tied to
/// the bytes on disk.
pub fn import_document(
    doc: &ModelDocument,
    options: &ImportOptions,
) -> Result<ImportedModel, ModelError> {
    let canonical = serde_json::to_string(doc)?;
    let source_hash = hash_source(&canonical);
    import_with_hash(doc, options, source_hash)
}

/// Parse and import a document from JSON text.
pub fn import_json(text: &str, options: &ImportOptions) -> Result<ImportedModel, ModelError> {
    let doc = ModelDocument::from_json(text)?;
    let source_hash = hash_source(text);
    import_with_hash(&doc, options, source_hash)
}

/// Read and import a document from disk.
pub fn import_file(path: &Path, options: &ImportOptions) -> Result<ImportedModel, ModelError> {
    let text = fs::read_to_string(path)?;
    import_json(&text, options)
}

/// Stable hash of the source text, used to notice that a model file changed.
///
/// Built from `DefaultHasher` so the crate stays dependency-free. The hash is
/// stable for a given Rust standard library, which is enough for change detection
/// inside one installation; it is not a cryptographic digest.
pub fn hash_source(text: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Map a free-text unit system label onto the supported systems.
pub fn unit_system_from_str(text: &str) -> Option<UnitSystem> {
    match text
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "si" | "si_units" | "metric" => Some(UnitSystem::Si),
        "imperial" | "us_customary" | "usc" | "ft_slug" => Some(UnitSystem::Imperial),
        "metric_degrees" | "metricdegrees" | "metric_deg" => Some(UnitSystem::MetricDegrees),
        "unknown" => Some(UnitSystem::Unknown),
        _ => None,
    }
}

fn import_with_hash(
    doc: &ModelDocument,
    options: &ImportOptions,
    source_hash: String,
) -> Result<ImportedModel, ModelError> {
    let mut doc = doc.clone();
    migrate_document(&mut doc);
    check_schema_version(&doc.schema_version)?;
    doc.check_required_fields()?;

    let unit_system = detect_unit_system(options, &doc);
    let frame = options.frame.unwrap_or_else(|| doc.frame());
    let converter = UnitConverter::new(options, unit_system, used_quantities(&doc));

    let reference_geometry = ReferenceGeometry {
        reference_area: converter.to_m2(doc.reference_geometry.reference_area),
        reference_length: converter.to_m(doc.reference_geometry.reference_length),
        body_diameter: converter.to_m(doc.reference_geometry.body_diameter),
    };

    let mass_properties = MassProperties::new(
        converter.to_kg(doc.mass_properties.mass),
        converter.to_m_vec(doc.mass_properties.center_of_gravity),
        converter.to_inertia(&doc.mass_properties.inertia),
    );

    let mass_curve = doc
        .mass_properties
        .mass_curve
        .as_ref()
        .map(|curve| converter.to_curve(curve));

    let mesh = doc.mesh.as_ref().map(|mesh| MeshReference {
        reference: mesh.reference.clone(),
        scale: options
            .mesh_scale_override
            .unwrap_or_else(|| converter.to_m(mesh.scale)),
        orientation: converter.to_rad_vec(mesh.orientation),
    });

    let thrust = doc.thrust.as_ref().map(|thrust| ThrustModel {
        application_point: converter.to_m_vec(thrust.application_point),
        direction: thrust.unit_direction(),
        misalignment: thrust.misalignment().map(|q| q.normalized()),
        times: thrust.times.iter().map(|t| converter.to_s(*t)).collect(),
        thrusts: thrust.thrusts.iter().map(|f| converter.to_n(*f)).collect(),
        engine_moment: converter.to_nm_vec(thrust.engine_moment),
    });

    let aerodynamics = doc.aerodynamics.as_ref().map(AeroModel::from_document);

    // Source-level findings are parked in the report the validator then replays,
    // so a later call to `validate_imported` reproduces this report exactly.
    let mut source_report = ValidationReport::new(doc.model_id.clone());
    source_report.extend(converter.into_issues());
    if let Some(issue) = inertia_asymmetry_issue(&doc.mass_properties.inertia) {
        source_report.push(issue);
    }
    if let Some(issue) = frame_confirmation_issue(&doc) {
        source_report.push(issue);
    }

    let mut model = ImportedModel {
        model_id: doc.model_id.clone(),
        model_version: doc.model_version.clone(),
        name: doc.name.clone(),
        description: doc.description.clone(),
        frame,
        reference_geometry,
        mass_properties,
        mass_curve,
        mesh,
        thrust,
        aerodynamics,
        validation: source_report,
        unit_system,
        source_hash,
    };
    model.validation = validate_imported(&model);
    Ok(model)
}

fn detect_unit_system(options: &ImportOptions, doc: &ModelDocument) -> UnitSystem {
    if options.unit_system != UnitSystem::Unknown {
        return options.unit_system;
    }
    doc.metadata
        .get("unit_system")
        .and_then(|value| value.as_str())
        .and_then(unit_system_from_str)
        .unwrap_or(UnitSystem::Unknown)
}

/// Which physical quantities the document actually uses.
///
/// Only used quantities produce an "assumed SI" warning when the unit system is
/// unknown, so a model without thrust is not nagged about force units.
#[derive(Debug, Clone, Copy, Default)]
struct UsedQuantities {
    force: bool,
    moment: bool,
    time: bool,
    angle: bool,
}

impl UsedQuantities {
    fn contains(self, kind: QuantityKind) -> bool {
        match kind {
            QuantityKind::Mass | QuantityKind::Position => true,
            QuantityKind::Force => self.force,
            QuantityKind::Moment => self.moment,
            QuantityKind::Time => self.time,
            QuantityKind::Angle => self.angle,
            _ => false,
        }
    }
}

fn used_quantities(doc: &ModelDocument) -> UsedQuantities {
    UsedQuantities {
        force: doc.thrust.is_some(),
        moment: doc.thrust.is_some(),
        time: doc.thrust.is_some() || doc.mass_properties.mass_curve.is_some(),
        angle: doc.mesh.is_some(),
    }
}

/// Resolves every unit in a document once and records what it could not resolve.
struct UnitConverter {
    unit_system: UnitSystem,
    used: UsedQuantities,
    issues: Vec<ValidationIssue>,
    mass: Real,
    length: Real,
    inertia: Real,
    force: Real,
    moment: Real,
    time: Real,
    angle: Real,
}

impl UnitConverter {
    fn new(options: &ImportOptions, unit_system: UnitSystem, used: UsedQuantities) -> Self {
        let mut converter = Self {
            unit_system,
            used,
            issues: Vec::new(),
            mass: 1.0,
            length: 1.0,
            inertia: 1.0,
            force: 1.0,
            moment: 1.0,
            time: 1.0,
            angle: 1.0,
        };
        converter.mass = converter.resolve_kind(
            QuantityKind::Mass,
            options.mass_unit.as_deref(),
            default_mass_unit(unit_system),
        );
        converter.length = converter.resolve_kind(
            QuantityKind::Position,
            options.length_unit.as_deref(),
            default_length_unit(unit_system),
        );
        converter.force =
            converter.resolve_kind(QuantityKind::Force, None, default_force_unit(unit_system));
        converter.moment =
            converter.resolve_kind(QuantityKind::Moment, None, default_moment_unit(unit_system));
        converter.time = converter.resolve_kind(QuantityKind::Time, None, "s");
        converter.angle =
            converter.resolve_kind(QuantityKind::Angle, None, default_angle_unit(unit_system));
        converter.inertia = converter.resolve_inertia(
            options.inertia_unit.as_deref(),
            default_inertia_unit(unit_system),
        );
        converter
    }

    fn resolve_kind(
        &mut self,
        kind: QuantityKind,
        explicit: Option<&str>,
        default_unit: &str,
    ) -> Real {
        let unit = explicit.unwrap_or(default_unit);
        match kind.scale_for(unit) {
            Some(scale) => {
                if explicit.is_none() && self.unit_system == UnitSystem::Unknown {
                    self.report_assumed(kind, unit);
                }
                scale.factor
            }
            None => {
                self.report_unknown(kind.label(), unit);
                1.0
            }
        }
    }

    fn resolve_inertia(&mut self, explicit: Option<&str>, default_unit: &str) -> Real {
        let unit = explicit.unwrap_or(default_unit);
        match inertia_unit_scale(unit) {
            Some(factor) => {
                if explicit.is_none() && self.unit_system == UnitSystem::Unknown {
                    self.report_assumed(QuantityKind::Mass, unit);
                }
                factor
            }
            None => {
                self.report_unknown("moment of inertia", unit);
                1.0
            }
        }
    }

    fn report_unknown(&mut self, quantity: &str, unit: &str) {
        self.issues.push(
            ValidationIssue::warning(
                "units.unknown",
                format!("Unrecognised unit for {quantity}"),
                format!(
                    "The unit \"{unit}\" given for {quantity} is not recognised, so the value was left exactly as written and converted with no factor. Treat the numbers as suspicious until the unit is corrected."
                ),
            )
            .with_context(quantity)
            .with_suggestion(
                "Set the correct unit in the import options, or fix the unit string in the source file.",
            )
            .with_technical(format!(
                "quantity = {quantity}, unit = {unit}, conversion factor = 1.0 (none applied)"
            )),
        );
    }

    fn report_assumed(&mut self, kind: QuantityKind, assumed: &str) {
        if !self.used.contains(kind) {
            return;
        }
        self.issues.push(
            ValidationIssue::warning(
                "units.unknown",
                format!("No unit system declared for {}", kind.label().to_ascii_lowercase()),
                format!(
                    "The source did not declare a unit system, so {} was assumed for {}. Check the import options if that is wrong.",
                    assumed,
                    kind.label().to_ascii_lowercase()
                ),
            )
            .with_context(kind.label())
            .with_suggestion("Set the source unit system in the import options.")
            .with_technical(format!(
                "quantity = {}, assumed unit = {assumed}",
                kind.label()
            )),
        );
    }

    fn into_issues(self) -> Vec<ValidationIssue> {
        self.issues
    }

    fn to_kg(&self, value: Real) -> Real {
        value * self.mass
    }

    fn to_m(&self, value: Real) -> Real {
        value * self.length
    }

    fn to_m2(&self, value: Real) -> Real {
        value * self.length * self.length
    }

    fn to_m_vec(&self, value: Vec3Record) -> Vec3 {
        value.to_vec3() * self.length
    }

    fn to_rad_vec(&self, value: Vec3Record) -> Vec3 {
        value.to_vec3() * self.angle
    }

    fn to_n(&self, value: Real) -> Real {
        value * self.force
    }

    fn to_nm_vec(&self, value: Vec3Record) -> Vec3 {
        value.to_vec3() * self.moment
    }

    fn to_s(&self, value: Real) -> Real {
        value * self.time
    }

    fn to_inertia(&self, inertia: &InertiaDocument) -> InertiaTensor {
        let tensor = inertia.to_tensor();
        let scale = self.inertia;
        InertiaTensor::new(
            tensor.ixx * scale,
            tensor.iyy * scale,
            tensor.izz * scale,
            tensor.ixy * scale,
            tensor.ixz * scale,
            tensor.iyz * scale,
        )
    }

    fn to_curve(&self, curve: &crate::schema::MassCurveDocument) -> MassCurve {
        MassCurve {
            times: curve.times.iter().map(|t| self.to_s(*t)).collect(),
            masses: curve.masses.iter().map(|m| self.to_kg(*m)).collect(),
            centers_of_gravity: curve
                .centers_of_gravity
                .iter()
                .map(|v| self.to_m_vec(*v))
                .collect(),
            inertias: curve.inertias.iter().map(|i| self.to_inertia(i)).collect(),
        }
    }
}

/// Scale from an inertia unit label to kg*m^2.
///
/// An inertia unit is a mass unit times a squared length unit. Each factor is
/// resolved through [`QuantityKind::scale_for`], so an inertia unit is never
/// converted by a hand-written constant that could drift from the table hex-core
/// owns. Both the separated form (`"slug*ft^2"`) and the fused form (`"kgm2"`)
/// are accepted; exponents other than mass^1 * length^2 are rejected.
fn inertia_unit_scale(unit: &str) -> Option<Real> {
    let normalised = unit
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '.'], "*");
    if normalised.is_empty() {
        return None;
    }
    let tokens: Vec<&str> = normalised.split('*').filter(|t| !t.is_empty()).collect();
    let mut mass: Option<Real> = None;
    let mut length: Option<Real> = None;

    for token in &tokens {
        if tokens.len() == 1 {
            let (mass_token, rest) = split_mass_length(token)?;
            let (length_token, exponent) = split_power(rest);
            if exponent != 2 {
                return None;
            }
            mass = Some(QuantityKind::Mass.scale_for(mass_token)?.factor);
            length = Some(
                QuantityKind::Position
                    .scale_for(length_token)?
                    .factor
                    .powi(exponent),
            );
            continue;
        }
        let (base, exponent) = split_power(token);
        if let Some(scale) = QuantityKind::Mass.scale_for(base) {
            if exponent != 1 || mass.is_some() {
                return None;
            }
            mass = Some(scale.factor);
        } else if let Some(scale) = QuantityKind::Position.scale_for(base) {
            if exponent != 2 || length.is_some() {
                return None;
            }
            length = Some(scale.factor.powi(exponent));
        } else {
            return None;
        }
    }

    Some(mass? * length?)
}

/// Split a fused token such as `"kgm2"` into its mass unit and the remainder.
fn split_mass_length(token: &str) -> Option<(&str, &str)> {
    for (index, _) in token.char_indices().skip(1) {
        let (prefix, rest) = token.split_at(index);
        if !rest.is_empty() && QuantityKind::Mass.scale_for(prefix).is_some() {
            return Some((prefix, rest));
        }
    }
    None
}

/// Split `"m^2"` or `"m2"` into the base unit and its exponent.
fn split_power(factor: &str) -> (&str, i32) {
    if let Some((base, exponent)) = factor.split_once('^') {
        return (base, exponent.parse::<i32>().unwrap_or(0));
    }
    let base_length = factor.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    if base_length == 0 || base_length == factor.len() {
        return (factor, 1);
    }
    let (base, digits) = factor.split_at(base_length);
    (base, digits.parse::<i32>().unwrap_or(1))
}

fn default_mass_unit(unit_system: UnitSystem) -> &'static str {
    match unit_system {
        UnitSystem::Imperial => "slug",
        _ => "kg",
    }
}

fn default_length_unit(unit_system: UnitSystem) -> &'static str {
    match unit_system {
        UnitSystem::Imperial => "ft",
        _ => "m",
    }
}

fn default_inertia_unit(unit_system: UnitSystem) -> &'static str {
    match unit_system {
        UnitSystem::Imperial => "slug*ft^2",
        _ => "kg*m^2",
    }
}

fn default_force_unit(unit_system: UnitSystem) -> &'static str {
    match unit_system {
        UnitSystem::Imperial => "lbf",
        _ => "N",
    }
}

fn default_moment_unit(unit_system: UnitSystem) -> &'static str {
    match unit_system {
        UnitSystem::Imperial => "lbf*ft",
        _ => "N*m",
    }
}

fn default_angle_unit(unit_system: UnitSystem) -> &'static str {
    match unit_system {
        UnitSystem::MetricDegrees => "deg",
        _ => "rad",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{MassCurveDocument, MeshDocument, ThrustDocument};
    use hex_core::Severity;

    fn document() -> ModelDocument {
        let mut doc = ModelDocument::minimal("rocket-1", "Sounding rocket", 10.0, [1.0, 2.0, 3.0]);
        doc.reference_geometry.reference_area = 0.05;
        doc.reference_geometry.reference_length = 0.2;
        doc.mass_properties.center_of_gravity = Vec3Record::new(0.1, 0.0, 0.0);
        doc
    }

    fn approx(a: Real, b: Real, tol: Real) -> bool {
        (a - b).abs() <= tol
    }

    fn unit_warnings(model: &ImportedModel) -> Vec<&ValidationIssue> {
        model
            .validation
            .issues
            .iter()
            .filter(|issue| issue.code == "units.unknown")
            .collect()
    }

    #[test]
    fn default_options_are_si_with_no_overrides() {
        let options = ImportOptions::default();
        assert_eq!(options.unit_system, UnitSystem::Si);
        assert!(options.frame.is_none());
        assert!(options.mass_unit.is_none());
        assert!(options.length_unit.is_none());
        assert!(options.inertia_unit.is_none());
        assert!(options.mesh_scale_override.is_none());
        assert_eq!(ImportOptions::si(), options);
        assert_eq!(ImportOptions::imperial().unit_system, UnitSystem::Imperial);
        assert_eq!(
            ImportOptions::metric_degrees().unit_system,
            UnitSystem::MetricDegrees
        );
    }

    #[test]
    fn si_import_converts_nothing() {
        let model = import_document(&document(), &ImportOptions::default()).unwrap();
        assert!(approx(model.mass(), 10.0, 1e-12));
        assert!(approx(model.reference_geometry.reference_area, 0.05, 1e-12));
        assert!(approx(
            model.reference_geometry.reference_length,
            0.2,
            1e-12
        ));
        assert!(approx(model.center_of_gravity().x, 0.1, 1e-12));
        assert!(approx(model.inertia().izz, 3.0, 1e-12));
        assert_eq!(model.unit_system, UnitSystem::Si);
        assert!(unit_warnings(&model).is_empty());
        assert!(!model.validation.has_errors());
    }

    #[test]
    fn header_fields_are_carried_over() {
        let mut doc = document();
        doc.model_version = "2.3".to_string();
        doc.description = Some("Test article".to_string());
        let model = import_document(&doc, &ImportOptions::default()).unwrap();
        assert_eq!(model.model_id, "rocket-1");
        assert_eq!(model.model_version, "2.3");
        assert_eq!(model.name, "Sounding rocket");
        assert_eq!(model.description.as_deref(), Some("Test article"));
    }

    #[test]
    fn unknown_mass_unit_warns_instead_of_guessing() {
        let options = ImportOptions {
            mass_unit: Some("stone".to_string()),
            ..ImportOptions::default()
        };
        let model = import_document(&document(), &options).unwrap();
        assert!(approx(model.mass(), 10.0, 1e-12));
        let warnings = unit_warnings(&model);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].severity, Severity::Warning);
        assert!(warnings[0].detail.contains("stone"));
        assert!(!model.validation.has_errors());
    }

    #[test]
    fn unknown_inertia_unit_warns_instead_of_guessing() {
        let options = ImportOptions {
            inertia_unit: Some("stone*ft^2".to_string()),
            ..ImportOptions::default()
        };
        let model = import_document(&document(), &options).unwrap();
        assert!(approx(model.inertia().ixx, 1.0, 1e-12));
        assert_eq!(unit_warnings(&model).len(), 1);
    }

    #[test]
    fn unknown_unit_error_variant_is_available_for_strict_callers() {
        let err = ModelError::unit_unknown("mass", "stone");
        assert!(!err.is_blocking());
        assert!(err.user_message().contains("stone"));
    }

    #[test]
    fn gram_mass_unit_converts() {
        let options = ImportOptions {
            mass_unit: Some("g".to_string()),
            ..ImportOptions::default()
        };
        let model = import_document(&document(), &options).unwrap();
        assert!(approx(model.mass(), 0.01, 1e-12));
        assert!(unit_warnings(&model).is_empty());
    }

    #[test]
    fn pound_mass_unit_converts() {
        let options = ImportOptions {
            mass_unit: Some("lb".to_string()),
            ..ImportOptions::default()
        };
        let model = import_document(&document(), &options).unwrap();
        assert!(approx(model.mass(), 4.535_923_7, 1e-9));
    }

    #[test]
    fn imperial_defaults_convert_mass_length_and_inertia() {
        let model = import_document(&document(), &ImportOptions::imperial()).unwrap();
        assert!(approx(model.mass(), 145.939_029_4, 1e-6));
        assert!(approx(
            model.reference_geometry.reference_length,
            0.06096,
            1e-12
        ));
        assert!(approx(
            model.reference_geometry.reference_area,
            0.05 * 0.3048 * 0.3048,
            1e-15
        ));
        assert!(approx(model.center_of_gravity().x, 0.03048, 1e-12));
        assert!(approx(model.inertia().izz, 3.0 * 1.355_817_948_331_4, 1e-8));
        assert!(unit_warnings(&model).is_empty());
    }

    #[test]
    fn centimetre_length_unit_converts() {
        let options = ImportOptions {
            length_unit: Some("cm".to_string()),
            ..ImportOptions::default()
        };
        let model = import_document(&document(), &options).unwrap();
        assert!(approx(
            model.reference_geometry.reference_length,
            0.002,
            1e-15
        ));
        assert!(approx(
            model.reference_geometry.reference_area,
            0.000_005,
            1e-18
        ));
        assert!(approx(model.center_of_gravity().x, 0.001, 1e-15));
    }

    #[test]
    fn foot_length_unit_converts() {
        let options = ImportOptions {
            length_unit: Some("ft".to_string()),
            ..ImportOptions::default()
        };
        let model = import_document(&document(), &options).unwrap();
        assert!(approx(
            model.reference_geometry.reference_length,
            0.06096,
            1e-12
        ));
    }

    #[test]
    fn inertia_units_accept_the_common_spellings() {
        assert!(approx(inertia_unit_scale("kg*m^2").unwrap(), 1.0, 1e-15));
        assert!(approx(inertia_unit_scale("kg m2").unwrap(), 1.0, 1e-15));
        assert!(approx(inertia_unit_scale("kgm2").unwrap(), 1.0, 1e-15));
        assert!(approx(inertia_unit_scale("g*cm^2").unwrap(), 1e-7, 1e-20));
        assert!(approx(inertia_unit_scale("gcm2").unwrap(), 1e-7, 1e-20));
        // slug*ft^2 agrees with the hex-core pound-force foot to nine decimals.
        assert!(approx(
            inertia_unit_scale("slug*ft^2").unwrap(),
            1.355_817_948_331_4,
            1e-9
        ));
        assert!(inertia_unit_scale("stone*ft^2").is_none());
        assert!(inertia_unit_scale("").is_none());
        assert!(inertia_unit_scale("kg").is_none());
    }

    #[test]
    fn inertia_unit_with_a_cubic_factor_is_rejected() {
        assert!(inertia_unit_scale("kg*ft^3").is_none());
    }

    #[test]
    fn inertia_unit_grams_centimetres_converts() {
        let options = ImportOptions {
            inertia_unit: Some("g*cm^2".to_string()),
            ..ImportOptions::default()
        };
        let model = import_document(&document(), &options).unwrap();
        assert!(approx(model.inertia().ixx, 1e-7, 1e-20));
    }

    #[test]
    fn metric_degrees_converts_mesh_orientation() {
        let mut doc = document();
        doc.mesh = Some(MeshDocument {
            reference: "rocket.glb".to_string(),
            scale: 0.001,
            orientation: Vec3Record::new(90.0, 0.0, 0.0),
        });
        let model = import_document(&doc, &ImportOptions::metric_degrees()).unwrap();
        let mesh = model.mesh.as_ref().unwrap();
        assert!(approx(
            mesh.orientation.x,
            std::f64::consts::FRAC_PI_2,
            1e-12
        ));
        assert!(approx(mesh.scale, 0.001, 1e-15));
    }

    #[test]
    fn radians_are_left_alone() {
        let mut doc = document();
        doc.mesh = Some(MeshDocument {
            reference: "rocket.glb".to_string(),
            scale: 1.0,
            orientation: Vec3Record::new(1.5, 0.0, 0.0),
        });
        let model = import_document(&doc, &ImportOptions::default()).unwrap();
        assert!(approx(
            model.mesh.as_ref().unwrap().orientation.x,
            1.5,
            1e-15
        ));
    }

    #[test]
    fn mesh_scale_override_wins_and_silences_the_warning() {
        let mut doc = document();
        doc.mesh = Some(MeshDocument {
            reference: "rocket.glb".to_string(),
            scale: 0.0,
            ..Default::default()
        });
        assert!(import_document(&doc, &ImportOptions::default())
            .unwrap()
            .validation
            .issues
            .iter()
            .any(|i| i.code == "mesh.scale_unknown"));

        let options = ImportOptions {
            mesh_scale_override: Some(0.001),
            ..ImportOptions::default()
        };
        let model = import_document(&doc, &options).unwrap();
        assert!(approx(model.mesh.as_ref().unwrap().scale, 0.001, 1e-15));
        assert!(!model
            .validation
            .issues
            .iter()
            .any(|i| i.code == "mesh.scale_unknown"));
    }

    #[test]
    fn mass_curve_is_converted_track_by_track() {
        let mut doc = document();
        doc.mass_properties.mass_curve = Some(MassCurveDocument {
            times: vec![0.0, 1.0],
            masses: vec![10.0, 6.0],
            centers_of_gravity: vec![Vec3Record::default(), Vec3Record::new(1.0, 0.0, 0.0)],
            inertias: vec![
                InertiaDocument::diagonal(1.0, 1.0, 1.0),
                InertiaDocument::diagonal(2.0, 2.0, 2.0),
            ],
        });
        let model = import_document(&doc, &ImportOptions::imperial()).unwrap();
        let curve = model.mass_curve.as_ref().unwrap();
        assert!(approx(curve.masses[0], 145.939_029_4, 1e-6));
        assert!(approx(curve.centers_of_gravity[1].x, 0.3048, 1e-12));
        assert!(approx(
            curve.inertias[1].ixx,
            2.0 * 1.355_817_948_331_4,
            1e-8
        ));
        assert!(curve.centers_of_gravity.len() == 2);
    }

    #[test]
    fn mass_at_uses_the_curve_and_falls_back_to_the_constant() {
        let mut doc = document();
        doc.mass_properties.mass_curve = Some(MassCurveDocument {
            times: vec![0.0, 1.0, 2.0],
            masses: vec![10.0, 8.0, 6.0],
            ..Default::default()
        });
        let model = import_document(&doc, &ImportOptions::default()).unwrap();
        assert!(approx(model.mass_at(0.5), 9.0, 1e-12));

        let plain = import_document(&document(), &ImportOptions::default()).unwrap();
        assert!(approx(plain.mass_at(999.0), 10.0, 1e-12));
    }

    #[test]
    fn thrust_profile_is_converted_to_si() {
        let mut doc = document();
        doc.thrust = Some(ThrustDocument {
            application_point: Vec3Record::new(1.0, 0.0, 0.0),
            direction: Vec3Record::new(0.0, 0.0, 10.0),
            misalignment_quaternion: Some([1.0, 0.0, 0.0, 0.0]),
            times: vec![0.0, 1.0],
            thrusts: vec![100.0, 50.0],
            engine_moment: Vec3Record::new(0.0, 1.0, 0.0),
        });
        let model = import_document(&doc, &ImportOptions::imperial()).unwrap();
        let thrust = model.thrust.as_ref().unwrap();
        assert!(approx(thrust.application_point.x, 0.3048, 1e-12));
        assert!(approx(thrust.direction.z, 1.0, 1e-12));
        assert!(approx(thrust.thrusts[0], 444.822_161_526_05, 1e-9));
        assert!(approx(thrust.engine_moment.y, 1.355_817_948_331_4, 1e-12));
        assert_eq!(thrust.misalignment.unwrap(), Quaternion::identity());
        assert!(thrust.is_usable());
        assert!(approx(
            thrust.total_impulse().unwrap(),
            0.5 * (444.822 + 222.411),
            1e-3
        ));
    }

    #[test]
    fn unusable_thrust_profiles_have_no_impulse() {
        let model = ThrustModel {
            application_point: Vec3::zeros(),
            direction: Vec3::x(),
            misalignment: None,
            times: vec![0.0, 0.0],
            thrusts: vec![1.0, 2.0],
            engine_moment: Vec3::zeros(),
        };
        // Paired but not strictly increasing in time, so no impulse can be formed.
        assert!(model.is_usable());
        assert!(model.total_impulse().is_none());

        let short = ThrustModel {
            times: vec![0.0],
            thrusts: vec![1.0],
            ..model
        };
        assert!(!short.is_usable());
        assert!(short.total_impulse().is_none());
    }

    #[test]
    fn aero_coefficients_are_passed_through_unchanged() {
        let mut doc = document();
        doc.aerodynamics = Some(AeroDocument {
            drag_coefficient_at_mach: vec![(0.0, 0.02), (1.0, 0.03)],
            lift_slope: 3.1,
            drag_only: true,
            ..Default::default()
        });
        let model = import_document(&doc, &ImportOptions::imperial()).unwrap();
        let aero = model.aerodynamics.as_ref().unwrap();
        assert!(approx(aero.lift_slope, 3.1, 1e-15));
        assert!(aero.drag_only);
        assert!(approx(aero.drag_coefficient_at(0.5).unwrap(), 0.025, 1e-12));
    }

    #[test]
    fn missing_optional_blocks_stay_none() {
        let model = import_document(&document(), &ImportOptions::default()).unwrap();
        assert!(model.mass_curve.is_none());
        assert!(model.mesh.is_none());
        assert!(model.thrust.is_none());
        assert!(model.aerodynamics.is_none());
    }

    #[test]
    fn import_options_frame_overrides_the_document() {
        let mut doc = document();
        doc.frame.world = hex_core::WorldFrame::Ned;
        let options = ImportOptions {
            frame: Some(Frame::new(
                hex_core::WorldFrame::Enu,
                hex_core::BodyFrame::ForwardLeftUp,
            )),
            ..ImportOptions::default()
        };
        let model = import_document(&doc, &options).unwrap();
        assert_eq!(model.frame.world, hex_core::WorldFrame::Enu);
        assert_eq!(model.frame.body, hex_core::BodyFrame::ForwardLeftUp);

        let from_document = import_document(&doc, &ImportOptions::default()).unwrap();
        assert_eq!(from_document.frame.world, hex_core::WorldFrame::Ned);
    }

    #[test]
    fn unknown_unit_system_warns_and_detection_survives() {
        let mut doc = document();
        doc.metadata.clear();
        let options = ImportOptions {
            unit_system: UnitSystem::Unknown,
            ..ImportOptions::default()
        };
        let model = import_document(&doc, &options).unwrap();
        assert_eq!(model.unit_system, UnitSystem::Unknown);
        assert_eq!(unit_warnings(&model).len(), 3);
        assert!(approx(model.mass(), 10.0, 1e-12));

        doc.metadata.insert(
            "unit_system".to_string(),
            serde_json::Value::String("imperial".to_string()),
        );
        let detected = import_document(&doc, &options).unwrap();
        assert_eq!(detected.unit_system, UnitSystem::Imperial);
        assert!(approx(detected.mass(), 145.939_029_4, 1e-6));
        assert!(unit_warnings(&detected).is_empty());
    }

    #[test]
    fn explicit_unit_system_wins_over_metadata() {
        let mut doc = document();
        doc.metadata.insert(
            "unit_system".to_string(),
            serde_json::Value::String("imperial".to_string()),
        );
        let model = import_document(&doc, &ImportOptions::default()).unwrap();
        assert_eq!(model.unit_system, UnitSystem::Si);
        assert!(approx(model.mass(), 10.0, 1e-12));
    }

    #[test]
    fn unit_system_labels_are_recognised_in_several_spellings() {
        assert_eq!(unit_system_from_str("SI"), Some(UnitSystem::Si));
        assert_eq!(
            unit_system_from_str(" us-customary "),
            Some(UnitSystem::Imperial)
        );
        assert_eq!(
            unit_system_from_str("metric degrees"),
            Some(UnitSystem::MetricDegrees)
        );
        assert_eq!(unit_system_from_str("klingon"), None);
    }

    #[test]
    fn source_hash_is_stable_and_content_addressed() {
        let text = document().to_json_pretty().unwrap();
        assert_eq!(hash_source(&text), hash_source(&text));
        assert_ne!(hash_source(&text), hash_source(&format!("{text} ")));

        let first = import_json(&text, &ImportOptions::default()).unwrap();
        let second = import_json(&text, &ImportOptions::default()).unwrap();
        assert_eq!(first.source_hash, second.source_hash);
        assert_eq!(first, second);

        let other = import_document(&document(), &ImportOptions::default()).unwrap();
        assert_eq!(
            other.source_hash,
            hash_source(&serde_json::to_string(&document()).unwrap())
        );
    }

    #[test]
    fn json_import_matches_document_import() {
        let doc = document();
        let text = doc.to_json_pretty().unwrap();
        let from_json = import_json(&text, &ImportOptions::default()).unwrap();
        let from_document = import_document(&doc, &ImportOptions::default()).unwrap();
        assert_eq!(from_json.model_id, from_document.model_id);
        assert_eq!(from_json.mass_properties, from_document.mass_properties);
        assert_eq!(from_json.validation, from_document.validation);
    }

    #[test]
    fn import_json_reports_malformed_text() {
        assert!(matches!(
            import_json("not json", &ImportOptions::default()),
            Err(ModelError::Json(_))
        ));
    }

    #[test]
    fn import_json_reports_a_missing_model_id() {
        assert!(matches!(
            import_json("{}", &ImportOptions::default()),
            Err(ModelError::MissingField { .. })
        ));
    }

    #[test]
    fn import_document_reports_a_missing_model_id() {
        let mut doc = document();
        doc.model_id = String::new();
        assert!(matches!(
            import_document(&doc, &ImportOptions::default()),
            Err(ModelError::MissingField { .. })
        ));
    }

    #[test]
    fn import_rejects_another_schema_major() {
        let mut doc = document();
        doc.schema_version = "2.0".to_string();
        match import_document(&doc, &ImportOptions::default()) {
            Err(ModelError::UnsupportedSchemaVersion { found, supported }) => {
                assert_eq!(found, "2.0");
                assert_eq!(supported, crate::versioning::SCHEMA_VERSION);
            }
            other => panic!("expected UnsupportedSchemaVersion, got {other:?}"),
        }
    }

    #[test]
    fn import_accepts_a_later_minor_schema() {
        let mut doc = document();
        doc.schema_version = "1.7".to_string();
        assert!(import_document(&doc, &ImportOptions::default()).is_ok());
    }

    #[test]
    fn import_normalises_a_missing_schema_version() {
        let mut doc = document();
        doc.schema_version = String::new();
        let model = import_document(&doc, &ImportOptions::default()).unwrap();
        assert_eq!(model.model_id, "rocket-1");
    }

    #[test]
    fn a_broken_model_still_imports_with_a_blocking_report() {
        let mut doc = document();
        doc.mass_properties.mass = -5.0;
        doc.mass_properties.inertia = InertiaDocument::diagonal(-1.0, -1.0, -1.0);
        let model = import_document(&doc, &ImportOptions::default()).unwrap();
        assert!(!model.is_valid());
        assert!(model.validation.has_errors());
        let err = model.check_valid().unwrap_err();
        assert!(err.is_blocking());
        assert_eq!(err.code(), "validation.failed");
    }

    #[test]
    fn check_valid_accepts_a_good_model() {
        let model = import_document(&document(), &ImportOptions::default()).unwrap();
        assert!(model.is_valid());
        assert!(model.check_valid().is_ok());
        assert!(model.summary().contains("Sounding rocket"));
    }

    #[test]
    fn import_file_reports_a_missing_file() {
        let path = std::env::temp_dir().join("hex-model-does-not-exist-0123456789.json");
        assert!(matches!(
            import_file(&path, &ImportOptions::default()),
            Err(ModelError::Io(_))
        ));
    }

    #[test]
    fn inertias_are_symmetrised_on_import() {
        let mut doc = document();
        doc.mass_properties.inertia =
            InertiaDocument::full(1.0, 0.2, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0);
        let model = import_document(&doc, &ImportOptions::default()).unwrap();
        assert!(approx(model.inertia().ixy, 0.1, 1e-15));
        assert!(model
            .validation
            .issues
            .iter()
            .any(|i| i.code == "inertia.asymmetric"));
    }
}
