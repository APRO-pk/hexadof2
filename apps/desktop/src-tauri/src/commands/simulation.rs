//! Simulation commands: validation, running, cancellation, and result access.

use std::sync::{Arc, Mutex};

use hex_core::ValidationReport;
use hex_dynamics::{
    ChannelSelector, Environment, ForceConfig, GravityModel, InitialConditions, MassSpec,
    RunConfig, RunOutcome, RunSummary, SimulationMode, SolverKind, StateSample,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::error::{missing, CommandError};
use crate::events;
use crate::state::{emit, emit_notice, emit_progress, ActiveSimulation, AppState, StoredRun};

/// A scenario description supplied by the Dynamics screen.
///
/// Every field is optional so the frontend can send a partial edit, and the
/// defaults come from the application settings rather than from a hidden constant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioRequest {
    /// Scenario name.
    pub name: Option<String>,
    /// Scenario description.
    pub description: Option<String>,
    /// `six_dof` or `three_dof`.
    pub mode: Option<String>,
    /// Start time in seconds.
    pub start_time: Option<f64>,
    /// End time in seconds.
    pub end_time: Option<f64>,
    /// Output sampling interval in seconds.
    pub output_interval: Option<f64>,
    /// Integration step for a fixed-step run, seconds.
    pub step: Option<f64>,
    /// Relative tolerance, which selects the adaptive solver.
    pub relative_tolerance: Option<f64>,
    /// Absolute tolerance.
    pub absolute_tolerance: Option<f64>,
    /// Initial position in the world frame, metres.
    pub position: Option<[f64; 3]>,
    /// Initial velocity in the world frame, metres per second.
    pub velocity: Option<[f64; 3]>,
    /// Initial attitude as roll, pitch, yaw in radians.
    pub euler: Option<[f64; 3]>,
    /// Initial angular velocity in the body frame, radians per second.
    pub angular_velocity: Option<[f64; 3]>,
    /// Initial mass in kilograms.
    pub mass: Option<f64>,
    /// Inertia about the centre of gravity, as `[ixx, iyy, izz, ixy, ixz, iyz]`.
    pub inertia: Option<[f64; 6]>,
    /// Uniform gravity magnitude, metres per second squared.
    pub gravity: Option<f64>,
    /// Whether the atmosphere is the standard atmosphere.
    pub standard_atmosphere: Option<bool>,
    /// Constant air density, kilograms per cubic metre, when not standard.
    pub air_density: Option<f64>,
    /// Constant wind speed, metres per second.
    pub wind_speed: Option<f64>,
    /// Wind direction as a world-frame unit vector.
    pub wind_direction: Option<[f64; 3]>,
    /// Launch rail length in metres. Zero disables the constraint.
    pub rail_length: Option<f64>,
    /// Whether gravity is applied.
    pub gravity_enabled: Option<bool>,
    /// Thrust magnitude in newtons.
    pub thrust: Option<f64>,
    /// Burn duration in seconds.
    pub burn_time: Option<f64>,
    /// Thrust application point in the body frame, metres.
    pub thrust_application_point: Option<[f64; 3]>,
    /// Drag coefficient for the built-in estimate. Ignored when the run uses the
    /// coefficients an imported model declares.
    pub drag_coefficient: Option<f64>,
    /// Which aerodynamic model the run should use.
    ///
    /// * `auto`: the imported model's coefficients when it declares them, and the
    ///   built-in estimate otherwise. This is the default.
    /// * `model`: the imported model's coefficients, and a blocking error when it
    ///   declares none.
    /// * `estimate`: the built-in drag-only estimate, using `drag_coefficient`.
    /// * `estimate_with_lift`: the built-in estimate plus lift and moments.
    /// * `none`: aerodynamics off.
    pub aero_source: Option<String>,
    /// Lift slope per radian, for the built-in estimate with lift.
    pub lift_slope: Option<f64>,
    /// Pitch damping derivative, for the built-in estimate with lift.
    pub pitch_damping: Option<f64>,
    /// Constant body-frame control moment, newton metres.
    pub control_moment: Option<[f64; 3]>,
    /// Control moment duration in seconds.
    pub control_duration: Option<f64>,
    /// A landing burn that ignites on the stopping-distance condition.
    ///
    /// This is what makes a self landing flight possible: the engine lights when
    /// the vehicle is low enough and falling fast enough that its own
    /// thrust-to-weight can still stop it at the ground. It is a trigger, not a
    /// guidance law, so nothing steers.
    pub landing_burn: Option<LandingBurnRequest>,
    /// Ground elevation in metres.
    pub ground_elevation: Option<f64>,
    /// Whether the run is saved into the project automatically.
    pub save: Option<bool>,
}

/// A landing burn, as the interface configures it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandingBurnRequest {
    /// Thrust magnitude while the engine is lit, newtons.
    pub thrust: f64,
    /// Deceleration held back from the stopping distance, metres per second
    /// squared. Larger lights the engine earlier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deceleration_margin: Option<f64>,
    /// The burn is refused above this height, metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum_ignition_altitude: Option<f64>,
    /// Descent rate below which the burn is not lit, metres per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_descent_rate: Option<f64>,
}

impl Default for ScenarioRequest {
    fn default() -> Self {
        Self {
            name: Some("Run".to_string()),
            description: None,
            mode: None,
            start_time: None,
            end_time: Some(5.0),
            output_interval: Some(0.01),
            step: Some(0.0005),
            relative_tolerance: None,
            absolute_tolerance: None,
            position: None,
            velocity: None,
            // A -90 degree pitch points the thrust axis up the world Z axis, so the
            // default scenario is a vertical flight rather than a horizontal slide.
            euler: Some([0.0, -std::f64::consts::FRAC_PI_2, 0.0]),
            angular_velocity: None,
            mass: None,
            inertia: None,
            gravity: None,
            standard_atmosphere: Some(true),
            air_density: None,
            wind_speed: None,
            wind_direction: None,
            rail_length: None,
            gravity_enabled: Some(true),
            thrust: Some(120.0),
            burn_time: Some(1.5),
            thrust_application_point: None,
            drag_coefficient: Some(0.45),
            aero_source: Some("auto".to_string()),
            lift_slope: None,
            pitch_damping: None,
            control_moment: None,
            control_duration: None,
            landing_burn: None,
            ground_elevation: None,
            save: Some(false),
        }
    }
}

fn vec3(value: Option<[f64; 3]>, fallback: [f64; 3]) -> hex_core::Vec3 {
    let v = value.unwrap_or(fallback);
    hex_core::Vec3::new(v[0], v[1], v[2])
}

fn inertia_of(
    value: Option<[f64; 6]>,
    fallback: hex_core::InertiaTensor,
) -> hex_core::InertiaTensor {
    match value {
        Some(v) => hex_core::InertiaTensor::new(v[0], v[1], v[2], v[3], v[4], v[5]),
        None => fallback,
    }
}

/// Which aerodynamic model a scenario asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AeroChoice {
    /// The imported model's coefficients when it declares them, otherwise the
    /// built-in drag-only estimate.
    Auto,
    /// The imported model's coefficients, required.
    Model,
    /// The built-in drag-only estimate.
    Estimate,
    /// The built-in estimate with lift and moments.
    EstimateWithLift,
    /// No aerodynamics.
    None,
}

impl AeroChoice {
    /// The choice a scenario request describes.
    pub fn from_request(request: &ScenarioRequest) -> Self {
        match request.aero_source.as_deref().map(str::trim) {
            Some("none") | Some("off") | Some("disabled") => AeroChoice::None,
            Some("model") | Some("imported") | Some("imported_model") => AeroChoice::Model,
            Some("estimate") | Some("drag_only") | Some("estimated") => AeroChoice::Estimate,
            Some("estimate_with_lift") | Some("full") | Some("lift") => {
                AeroChoice::EstimateWithLift
            }
            // `auto` and anything unrecognised fall back to the model when it has
            // coefficients, which is the least surprising behaviour.
            _ => AeroChoice::Auto,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            AeroChoice::Auto => "auto",
            AeroChoice::Model => "model",
            AeroChoice::Estimate => "estimate",
            AeroChoice::EstimateWithLift => "estimate_with_lift",
            AeroChoice::None => "none",
        }
    }

    /// The choices a user can pick, in the order the interface lists them.
    pub fn all() -> [AeroChoice; 5] {
        [
            AeroChoice::Auto,
            AeroChoice::Model,
            AeroChoice::Estimate,
            AeroChoice::EstimateWithLift,
            AeroChoice::None,
        ]
    }
}

/// Convert the aerodynamic block of an imported model into the dynamics crate's
/// coefficient model.
///
/// The document declares one roll derivative, which is the rate derivative; the
/// sideslip roll derivative has no counterpart and is left at zero rather than
/// being given a guessed value.
pub fn aero_from_model(
    aero: &hex_model::AeroModel,
    fallback_drag_coefficient: Option<f64>,
) -> hex_dynamics::AeroModel {
    let drag = if aero.drag_coefficient_at_mach.is_empty() {
        hex_dynamics::AeroCoefficientTable::constant(fallback_drag_coefficient.unwrap_or(0.0))
    } else {
        let mach: Vec<f64> = aero
            .drag_coefficient_at_mach
            .iter()
            .map(|(mach, _)| *mach)
            .collect();
        let values: Vec<f64> = aero
            .drag_coefficient_at_mach
            .iter()
            .map(|(_, coefficient)| *coefficient)
            .collect();
        hex_dynamics::AeroCoefficientTable::from_samples(mach, values)
    };

    let derivatives = hex_dynamics::AeroDerivatives {
        lift_slope: aero.lift_slope,
        lift_zero: aero.lift_zero,
        side_slope: aero.side_slope,
        pitch_zero: aero.pitch_zero,
        pitch_slope: aero.pitch_slope,
        yaw_slope: aero.yaw_slope,
        roll_slope: 0.0,
        pitch_damping: aero.pitch_damping,
        yaw_damping: aero.yaw_damping,
        roll_damping: aero.roll_damping,
    };

    // The coefficients were declared rather than estimated, so they carry the
    // tighter uncertainty even though they are still only coefficients.
    hex_dynamics::AeroModel::from_table(drag, derivatives, aero.drag_only, 0.1)
}

/// The aerodynamic model a scenario describes.
pub fn build_aero(
    request: &ScenarioRequest,
    model: Option<&hex_model::ImportedModel>,
) -> Option<hex_dynamics::AeroModel> {
    let declared = model.and_then(|m| m.aerodynamics.as_ref());
    let choice = AeroChoice::from_request(request);

    let from_model = || declared.map(|aero| aero_from_model(aero, request.drag_coefficient));

    match choice {
        AeroChoice::None => None,
        AeroChoice::Model => from_model(),
        AeroChoice::Estimate => request
            .drag_coefficient
            .map(hex_dynamics::AeroModel::drag_only_estimate),
        AeroChoice::EstimateWithLift => request.drag_coefficient.map(|cd| {
            hex_dynamics::AeroModel::estimate_with_lift(
                cd,
                hex_dynamics::AeroDerivatives {
                    lift_slope: request.lift_slope.unwrap_or(2.0),
                    pitch_slope: -1.2,
                    pitch_damping: request.pitch_damping.unwrap_or(-8.0),
                    yaw_slope: -0.8,
                    roll_slope: -0.1,
                    ..hex_dynamics::AeroDerivatives::default()
                },
            )
        }),
        AeroChoice::Auto => from_model().or_else(|| {
            request
                .drag_coefficient
                .map(hex_dynamics::AeroModel::drag_only_estimate)
        }),
    }
}

/// Build a run configuration from a scenario request and the current settings.
pub fn build_config(
    request: &ScenarioRequest,
    settings: &hex_project::Settings,
    model: Option<&hex_model::ImportedModel>,
) -> RunConfig {
    let defaults = RunConfig::default();

    // The imported model supplies mass properties and geometry when it is present,
    // so a user does not retype numbers the model already declares.
    let (mass, inertia, geometry_area, geometry_length, thrust_point) = match model {
        Some(m) => (
            m.mass(),
            m.inertia(),
            m.reference_geometry.reference_area,
            m.reference_geometry.reference_length,
            m.thrust.as_ref().map(|t| {
                let p = t.application_point;
                [p.x, p.y, p.z]
            }),
        ),
        None => (
            defaults.initial.mass,
            hex_core::InertiaTensor::default(),
            0.01,
            1.0,
            None,
        ),
    };

    let mass_value = request.mass.unwrap_or(mass);
    let inertia_value = inertia_of(request.inertia, inertia);

    let mode = match request.mode.as_deref() {
        Some("three_dof") | Some("3dof") | Some("point_mass") => SimulationMode::ThreeDof,
        _ => SimulationMode::SixDof,
    };

    let solver = match (request.relative_tolerance, request.step) {
        (Some(rtol), _) => SolverKind::Rk45 {
            relative_tolerance: rtol.max(1e-14),
            absolute_tolerance: request.absolute_tolerance.unwrap_or(rtol * 1e-3).max(1e-16),
            maximum_step: 0.05,
            minimum_step: 1e-9,
            initial_step: 1e-4,
        },
        (None, Some(step)) => SolverKind::fixed(step),
        (None, None) => settings.dynamics.solver,
    };

    let mut environment = Environment {
        gravity: GravityModel::uniform(
            request
                .gravity
                .unwrap_or_else(|| settings.dynamics.gravity.nominal_magnitude()),
        ),
        ..Environment::default()
    };
    if request.standard_atmosphere == Some(false) {
        let density = request.air_density.unwrap_or(1.225);
        environment.atmosphere = hex_dynamics::AtmosphereModel::Constant {
            density,
            temperature: 288.15,
            speed_of_sound: 340.294,
        };
        environment.air_density_source = hex_dynamics::AirDensitySource::Constant;
    }
    if let Some(speed) = request.wind_speed {
        if speed.abs() > 1e-9 {
            environment.wind = hex_dynamics::WindModel::constant(
                speed,
                vec3(request.wind_direction, [1.0, 0.0, 0.0]),
            );
        }
    }
    if let Some(length) = request.rail_length {
        if length > 0.0 {
            environment.rail = hex_dynamics::LaunchRail::vertical(length, hex_core::Vec3::z());
        }
    }
    if let Some(elevation) = request.ground_elevation {
        environment.ground.elevation = elevation;
    }

    let mut thrust = request.thrust.map(|magnitude| {
        let mut profile = hex_dynamics::ThrustProfile::constant(
            magnitude.max(0.0),
            request.burn_time.unwrap_or(1.5).max(0.0),
        );
        profile.application_point = vec3(
            request.thrust_application_point,
            thrust_point.unwrap_or([0.0; 3]),
        );
        profile
    });
    if let Some(point) = request.thrust_application_point {
        if let Some(profile) = thrust.as_mut() {
            profile.application_point = vec3(Some(point), [0.0; 3]);
        }
    }

    let aero = build_aero(request, model);
    let _ = (geometry_area, geometry_length);

    let control = request.control_moment.map(|moment| {
        hex_dynamics::ControlMoment::constant(
            hex_core::Vec3::new(moment[0], moment[1], moment[2]),
            request.control_duration.unwrap_or(1.0).max(0.0),
        )
    });

    let landing_burn = request.landing_burn.as_ref().map(|burn| {
        let mut provider = hex_dynamics::LandingBurn::constant(burn.thrust.max(0.0));
        provider.deceleration_margin = burn.deceleration_margin.unwrap_or(1.0).max(0.0);
        provider.maximum_ignition_altitude =
            burn.maximum_ignition_altitude.unwrap_or(500.0).max(0.0);
        provider.minimum_descent_rate = burn.minimum_descent_rate.unwrap_or(0.2).max(0.0);
        // The landing engine is the same engine, so it pushes along the same axis
        // through the same application point as the ascent burn.
        provider.application_point = vec3(
            request.thrust_application_point,
            thrust_point.unwrap_or([0.0; 3]),
        );
        provider
    });

    let initial = InitialConditions {
        time: request.start_time.unwrap_or(0.0),
        mass: mass_value,
        ..InitialConditions::at_rest(mass_value)
    }
    .with_position(vec3(request.position, [0.0; 3]))
    .with_velocity(vec3(request.velocity, [0.0; 3]))
    .with_euler(
        request.euler.map(|e| e[0]).unwrap_or(0.0),
        request.euler.map(|e| e[1]).unwrap_or(0.0),
        request.euler.map(|e| e[2]).unwrap_or(0.0),
    )
    .with_angular_velocity(vec3(request.angular_velocity, [0.0; 3]));

    RunConfig {
        name: request.name.clone().unwrap_or_else(|| "Run".to_string()),
        description: request.description.clone().unwrap_or_default(),
        mode,
        start_time: request.start_time.unwrap_or(0.0),
        end_time: request.end_time.unwrap_or(5.0),
        output_interval: request.output_interval.unwrap_or(0.01),
        solver,
        normalization: settings.dynamics.normalization,
        initial,
        mass: MassSpec::constant(mass_value, inertia_value),
        environment,
        forces: ForceConfig {
            gravity: request.gravity_enabled.unwrap_or(true),
            thrust,
            landing_burn,
            aero,
            control,
            ..ForceConfig::default()
        },
        thresholds: settings.dynamics.thresholds(),
        ..RunConfig::default()
    }
}

/// Validation of a scenario before it runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioValidation {
    /// Whether the run may start.
    pub can_run: bool,
    /// Blocking problems.
    pub errors: Vec<String>,
    /// Non-blocking findings.
    pub warnings: Vec<String>,
    /// Expected number of output samples.
    pub expected_samples: usize,
    /// Solver description.
    pub solver: String,
    /// Mass model description.
    pub mass_model: String,
    /// Environment description, one line per component.
    pub environment: Vec<String>,
    /// Force and moment model description.
    pub forces: Vec<String>,
    /// Active force provider names.
    pub active_providers: Vec<String>,
    /// The validation report, so the UI can render issues with severity and fix
    /// suggestions rather than plain strings.
    pub report: ValidationReport,
}

/// Check the aerodynamic choice against what the run was actually given.
///
/// A scenario can ask for the imported model's coefficients and the model can
/// declare none, which has to be a blocking error rather than a run that quietly
/// falls back to an estimate the user did not ask for.
fn check_aero_choice(
    request: &ScenarioRequest,
    model: Option<&hex_model::ImportedModel>,
    aero: Option<&hex_dynamics::AeroModel>,
    report: &mut ValidationReport,
) {
    let choice = AeroChoice::from_request(request);
    let declared = model.and_then(|m| m.aerodynamics.as_ref());

    match (choice, aero, declared) {
        (AeroChoice::Model, None, _) => report.push(
            hex_core::ValidationIssue::error(
                "aero.no_coefficients",
                "The imported model declares no aerodynamic coefficients",
                "The scenario asks for the imported model's coefficients, and the model has no aerodynamics block to take them from.",
            )
            .with_context("aerodynamics")
            .with_suggestion(
                "Choose the built-in estimate, or add an aerodynamics block to the model and import it again.",
            ),
        ),
        (AeroChoice::Auto, Some(a), Some(_)) if a.source == hex_dynamics::AeroSource::ImportedModel => {
            report.push(
                hex_core::ValidationIssue::info(
                    "aero.using_model",
                    "Using the imported model's aerodynamic coefficients",
                    format!(
                        "The aerodynamics block of the imported model supplies the coefficients. {}",
                        a.fidelity_label()
                    ),
                )
                .with_context("aerodynamics")
                .with_technical(format!(
                    "drag table entries = {}, drag only = {}, source = {}",
                    a.drag.values.len(),
                    a.drag_only,
                    a.source.label()
                )),
            );
        }
        (AeroChoice::Estimate | AeroChoice::EstimateWithLift | AeroChoice::Auto, Some(a), declared) => {
            if declared.is_some() && choice != AeroChoice::Auto {
                report.push(hex_core::ValidationIssue::warning(
                    "aero.model_ignored",
                    "The imported model's aerodynamic coefficients are not being used",
                    "The scenario selects a built-in estimate, so the aerodynamics block of the imported model is ignored.",
                )
                .with_context("aerodynamics")
                .with_suggestion(
                    "Select the imported model coefficients to use what the model declares.",
                ));
            }
            let _ = a;
        }
        (AeroChoice::None, _, _) => report.push(hex_core::ValidationIssue::warning(
            "aero.disabled",
            "Aerodynamics are disabled",
            "The run has no aerodynamic model, so no drag, lift, or aerodynamic moment acts on the vehicle. This is only appropriate for a vacuum or a verification case.",
        )
        .with_context("aerodynamics")),
        (_, None, None) => report.push(hex_core::ValidationIssue::warning(
            "aero.none_available",
            "No aerodynamic model is available",
            "Neither the scenario nor the imported model supplies aerodynamic coefficients, so the run has no aerodynamics.",
        )
        .with_context("aerodynamics")
        .with_suggestion(
            "Enter a drag coefficient, or import a model that declares an aerodynamics block.",
        )),
        _ => {}
    }
}

fn validate_config(
    config: &RunConfig,
    request: &ScenarioRequest,
    model: Option<&hex_model::ImportedModel>,
) -> ScenarioValidation {
    let mut report = ValidationReport::new(config.name.clone());
    for problem in config.validate() {
        report.push(
            hex_core::ValidationIssue::error(
                "scenario.invalid",
                "The scenario is not usable",
                problem,
            )
            .with_suggestion("Correct the highlighted value in the Dynamics configuration."),
        );
    }
    check_aero_choice(request, model, config.forces.aero.as_ref(), &mut report);
    check_landing_burn(request, config, &mut report);
    report.recompute();

    let errors: Vec<String> = report.errors().map(|i| i.detail.clone()).collect();
    let warnings: Vec<String> = report
        .issues
        .iter()
        .filter(|i| i.severity == hex_core::Severity::Warning)
        .map(|i| i.detail.clone())
        .collect();

    ScenarioValidation {
        can_run: report.can_proceed(),
        errors,
        warnings,
        expected_samples: config.expected_samples(),
        solver: config.solver.label(),
        mass_model: config.mass.label(),
        environment: config.environment.describe(),
        forces: config.forces.describe(),
        active_providers: {
            let chain = config.forces.to_chain();
            chain.active_names()
        },
        report,
    }
}

/// Check the landing burn against the vehicle it has to stop.
///
/// A landing burn that cannot out-thrust the vehicle's own weight will never
/// light, because the stopping distance does not exist. That has to be an error
/// rather than a run that quietly falls to the ground.
fn check_landing_burn(
    request: &ScenarioRequest,
    config: &RunConfig,
    report: &mut ValidationReport,
) {
    let Some(burn) = config.forces.landing_burn.as_ref() else {
        return;
    };
    if burn.thrust <= 0.0 {
        report.push(
            hex_core::ValidationIssue::error(
                "landing.thrust",
                "The landing burn has no thrust",
                "A landing burn of zero newtons can never stop the vehicle.".to_string(),
            )
            .with_suggestion("Set a landing thrust greater than the vehicle's weight."),
        );
        return;
    }
    let mass = config.initial.mass.max(1e-9);
    let gravity = config.environment.gravity.nominal_magnitude();
    if !burn.can_stop(mass, gravity) {
        report.push(
            hex_core::ValidationIssue::error(
                "landing.cannot_stop",
                "The landing burn cannot stop this vehicle",
                format!(
                    "{:.1} N on {:.3} kg leaves {:.2} m/s^2 of net deceleration once gravity and the {:.2} m/s^2 margin are taken out, so the stopping distance is never reached and the burn will not light.",
                    burn.thrust,
                    mass,
                    burn.thrust / mass - gravity - burn.deceleration_margin,
                    burn.deceleration_margin
                ),
            )
            .with_suggestion("Raise the landing thrust, lower the mass, or lower the deceleration margin."),
        );
    }
    if let Some(thrust) = request.thrust {
        if thrust > 0.0 && burn.thrust > thrust {
            report.push(
                hex_core::ValidationIssue::warning(
                    "landing.more_than_ascent",
                    "The landing burn is stronger than the ascent burn",
                    format!(
                        "The landing burn is {:.1} N against an ascent burn of {:.1} N. Nothing forbids it, but it is unusual enough to be worth confirming.",
                        burn.thrust, thrust
                    ),
                )
                .with_suggestion("Confirm the two thrust values were not swapped."),
            );
        }
    }
    report.push(hex_core::ValidationIssue::info(
        "landing.trigger",
        "The landing burn ignites on a stopping-distance trigger",
        "The engine lights when the vehicle is descending, low, and close enough to the ground to stop at its own thrust-to-weight. It is a trigger, not a guidance law: nothing steers, and the vehicle has to already be upright.",
    ));
}

/// Validate a scenario without running it.
#[tauri::command]
pub fn simulation_validate(
    app: AppHandle,
    state: State<'_, AppState>,
    scenario: ScenarioRequest,
) -> Result<ScenarioValidation, CommandError> {
    let (settings, model) = state.with(|s| (s.settings.clone(), s.models.last().cloned()))?;
    let config = build_config(&scenario, &settings, model.as_ref());
    let validation = validate_config(&config, &scenario, model.as_ref());

    emit(
        &app,
        events::channel::VALIDATION_RESULT,
        events::ValidationResultEvent {
            subject: config.name.clone(),
            status: validation.report.status.label().to_string(),
            errors: validation.report.error_count(),
            warnings: validation.report.warning_count(),
        },
    );
    if !validation.can_run {
        for error in &validation.errors {
            emit_notice(
                &app,
                events::NoticeEvent::error("Scenario cannot run", error.clone()),
            );
        }
    }
    Ok(validation)
}

/// A run summary plus the samples needed to plot and replay it, in a form the
/// frontend can consume directly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResultView {
    /// Directory name, when the run was saved.
    pub artifact_name: Option<String>,
    /// The recorded summary.
    pub summary: RunSummary,
    /// Times in seconds.
    pub times: Vec<f64>,
    /// One column per requested channel, in the order requested.
    pub columns: Vec<Vec<f64>>,
    /// Channel names matching `columns`.
    pub channel_names: Vec<String>,
    /// Channel units matching `columns`.
    pub channel_units: Vec<String>,
    /// Events, as codes and times plus the full records.
    pub events: Vec<hex_dynamics::FlightEvent>,
    /// Warnings raised by the run.
    pub warnings: Vec<hex_dynamics::RunWarning>,
    /// Every channel this run can be plotted with.
    pub available_channels: Vec<String>,
}

/// The channels requested for a result view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelRequest {
    /// Channel identifiers from `available_channels`.
    pub channels: Vec<String>,
    /// Maximum samples to return, decimated evenly when the history is longer.
    pub maximum_samples: Option<usize>,
}

impl Default for ChannelRequest {
    fn default() -> Self {
        Self {
            channels: Vec::new(),
            maximum_samples: Some(4000),
        }
    }
}

/// Resolve a channel identifier into a selector.
pub fn selector_from_name(name: &str) -> Option<ChannelSelector> {
    use ChannelSelector::*;
    Some(match name {
        "time" => Time,
        "position_x" => PositionX,
        "position_y" => PositionY,
        "position_z" => PositionZ,
        "altitude" => Altitude,
        "velocity_x" => VelocityX,
        "velocity_y" => VelocityY,
        "velocity_z" => VelocityZ,
        "speed" => Speed,
        "vertical_velocity" => VerticalVelocity,
        "acceleration_world_x" | "acceleration_x" => AccelerationWorldX,
        "acceleration_world_y" | "acceleration_y" => AccelerationWorldY,
        "acceleration_world_z" | "acceleration_z" => AccelerationWorldZ,
        "acceleration" | "acceleration_magnitude" => AccelerationMagnitude,
        "quaternion_w" => QuaternionW,
        "quaternion_x" => QuaternionX,
        "quaternion_y" => QuaternionY,
        "quaternion_z" => QuaternionZ,
        "roll" => Roll,
        "pitch" => Pitch,
        "yaw" => Yaw,
        "angular_rate_x" => AngularRateX,
        "angular_rate_y" => AngularRateY,
        "angular_rate_z" => AngularRateZ,
        "angular_rate" | "angular_rate_magnitude" => AngularRateMagnitude,
        "mass" => Mass,
        "force_body_x" | "force_x" => ForceBodyX,
        "force_body_y" | "force_y" => ForceBodyY,
        "force_body_z" | "force_z" => ForceBodyZ,
        "force" | "force_magnitude" => ForceMagnitude,
        "moment_body_x" | "moment_x" => MomentBodyX,
        "moment_body_y" | "moment_y" => MomentBodyY,
        "moment_body_z" | "moment_z" => MomentBodyZ,
        "moment" | "moment_magnitude" => MomentMagnitude,
        "dynamic_pressure" => DynamicPressure,
        "angle_of_attack" => AngleOfAttack,
        "sideslip" => Sideslip,
        "air_density" => AirDensity,
        "mach" => Mach,
        _ => return None,
    })
}

/// The default channel set a new results view opens with.
pub fn default_channel_names() -> Vec<String> {
    [
        "time",
        "altitude",
        "vertical_velocity",
        "speed",
        "acceleration",
        "roll",
        "pitch",
        "yaw",
        "angular_rate_x",
        "angular_rate_y",
        "angular_rate_z",
        "force",
        "moment",
        "angle_of_attack",
        "sideslip",
        "dynamic_pressure",
        "mass",
        "position_x",
        "position_y",
        "position_z",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Every channel name a run can be plotted with.
pub fn all_channel_names() -> Vec<String> {
    // The plot panel and the comparison plots use one vocabulary, so this is the
    // single list both read.
    viewer_channel_names()
}

/// The channel vocabulary the plot panels use.
pub fn viewer_channel_names() -> Vec<String> {
    [
        "time",
        "position_x",
        "position_y",
        "position_z",
        "altitude",
        "velocity_x",
        "velocity_y",
        "velocity_z",
        "speed",
        "vertical_velocity",
        "acceleration_x",
        "acceleration_y",
        "acceleration_z",
        "acceleration",
        "quaternion_w",
        "quaternion_x",
        "quaternion_y",
        "quaternion_z",
        "roll",
        "pitch",
        "yaw",
        "angular_rate_x",
        "angular_rate_y",
        "angular_rate_z",
        "angular_rate",
        "mass",
        "force_x",
        "force_y",
        "force_z",
        "force",
        "moment_x",
        "moment_y",
        "moment_z",
        "moment",
        "dynamic_pressure",
        "angle_of_attack",
        "sideslip",
        "air_density",
        "mach",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Build a result view from an outcome.
pub fn view_from_outcome(
    outcome: &RunOutcome,
    artifact_name: Option<String>,
    request: &ChannelRequest,
) -> RunResultView {
    let names: Vec<String> = if request.channels.is_empty() {
        default_channel_names()
    } else {
        request.channels.clone()
    };
    let selectors: Vec<ChannelSelector> =
        names.iter().filter_map(|n| selector_from_name(n)).collect();
    let resolved_names: Vec<String> = names
        .iter()
        .filter(|n| selector_from_name(n).is_some())
        .cloned()
        .collect();

    // Decimate evenly rather than truncating, so a long history still shows its
    // whole shape. The recording itself is untouched.
    let limit = request.maximum_samples.unwrap_or(4000).max(2);
    let stride = (outcome.samples.len() / limit).max(1);
    let mut times = Vec::new();
    let mut columns: Vec<Vec<f64>> = vec![Vec::new(); selectors.len()];
    for (i, sample) in outcome.samples.iter().enumerate() {
        if i % stride != 0 && i + 1 != outcome.samples.len() {
            continue;
        }
        times.push(sample.time);
        for (c, selector) in selectors.iter().enumerate() {
            columns[c].push(selector.value(sample));
        }
    }

    RunResultView {
        artifact_name,
        summary: outcome.summary.clone(),
        times,
        columns,
        channel_names: resolved_names,
        channel_units: selectors.iter().map(|s| s.unit().to_string()).collect(),
        events: outcome.events.clone(),
        warnings: outcome.warnings.clone(),
        available_channels: all_channel_names(),
    }
}

/// Run a simulation, reporting progress through events.
///
/// The integration runs on a blocking thread so a multi-second run never stalls
/// the interface, and the outcome is stored in the application state on the way
/// back through the async runtime.
#[tauri::command]
pub async fn simulation_run(
    app: AppHandle,
    scenario: ScenarioRequest,
    channels: Option<ChannelRequest>,
) -> Result<RunResultView, CommandError> {
    let state = app.state::<AppState>();
    let (settings, model, project) = state.with(|s| {
        (
            s.settings.clone(),
            s.models.last().cloned(),
            s.project.clone(),
        )
    })?;

    // Refuse to start a second run rather than interleaving two integrations.
    let already_running = state.with(|s| s.active_simulation.is_some())?;
    if already_running {
        return Err(missing(
            "simulation.already_running",
            "A simulation is already running",
            "Only one run may be active at a time.",
            "Wait for the current run to finish, or stop it.",
        ));
    }

    let config = build_config(&scenario, &settings, model.as_ref());
    let validation = validate_config(&config, &scenario, model.as_ref());
    if !validation.can_run {
        return Err(CommandError::new(
            "simulation.invalid",
            "The scenario cannot run",
            validation.errors.join("; "),
            "Correct the highlighted values and validate again.",
        ));
    }

    let cancel = hex_dynamics::CancellationToken::new();
    let progress_slot: Arc<Mutex<Option<hex_dynamics::RunProgress>>> = Arc::new(Mutex::new(None));
    state.with(|s| {
        s.active_simulation = Some(ActiveSimulation {
            cancel: cancel.clone(),
            progress: progress_slot.clone(),
        });
        Ok::<(), CommandError>(())
    })??;

    let app_for_run = app.clone();
    let config_for_run = config.clone();
    let progress_for_run = progress_slot.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let built = hex_dynamics::build_run(config_for_run);
        hex_dynamics::run_built(&built, &cancel, |progress| {
            if let Ok(mut slot) = progress_for_run.lock() {
                *slot = Some(progress);
            }
            emit_progress(&app_for_run, &progress);
        })
    })
    .await
    .map_err(|e| {
        CommandError::new(
            "simulation.join",
            "The simulation task failed to complete",
            e.to_string(),
            "Report this as a bug with the diagnostic bundle.",
        )
    })?;

    // Clear the active job before reporting, so the UI is never left locked.
    state.with(|s| {
        s.active_simulation = None;
        Ok::<(), CommandError>(())
    })??;

    let mut artifact_name = None;
    if scenario.save.unwrap_or(false) {
        if let Some(project) = project.as_ref() {
            match hex_project::save_run(
                project,
                &config.name,
                &outcome.summary,
                &outcome.samples,
                &outcome.events,
                &config,
            ) {
                Ok(artifact) => {
                    artifact_name = Some(artifact.name.clone());
                    emit_notice(
                        &app,
                        events::NoticeEvent::info(
                            "Run saved",
                            format!("Saved as {}.", artifact.name),
                        ),
                    );
                }
                Err(e) => emit_notice(
                    &app,
                    events::NoticeEvent::warning("The run could not be saved", e.user_message()),
                ),
            }
        } else {
            emit_notice(
                &app,
                events::NoticeEvent::warning(
                    "The run was not saved",
                    "No project is open, so the result exists only in this session.",
                ),
            );
        }
    }

    for warning in &outcome.warnings {
        emit(
            &app,
            if warning.blocking {
                events::channel::SIMULATION_FAILED
            } else {
                events::channel::SIMULATION_WARNING
            },
            events::SimulationNoticeEvent {
                code: warning.code.clone(),
                title: warning.title.clone(),
                detail: warning.detail.clone(),
                blocking: warning.blocking,
            },
        );
    }

    emit(
        &app,
        events::channel::SIMULATION_COMPLETED,
        events::SimulationCompletedEvent {
            run_name: artifact_name.clone(),
            status: outcome.status.label().to_string(),
            samples: outcome.samples.len(),
            warnings: outcome.warnings.len(),
            events: outcome.events.len(),
        },
    );

    let view = view_from_outcome(
        &outcome,
        artifact_name.clone(),
        &channels.unwrap_or_default(),
    );
    state.with(|s| {
        s.last_run = Some(StoredRun {
            outcome,
            artifact_name,
        });
        Ok::<(), CommandError>(())
    })??;
    Ok(view)
}

/// Cancel the running simulation.
#[tauri::command]
pub fn simulation_cancel(state: State<'_, AppState>) -> Result<bool, CommandError> {
    state.with(|s| match s.active_simulation.as_ref() {
        Some(active) => {
            active.cancel.cancel();
            true
        }
        None => false,
    })
}

/// The latest progress report, for a fallback poll.
#[tauri::command]
pub fn simulation_progress(
    state: State<'_, AppState>,
) -> Result<Option<events::SimulationProgressEvent>, CommandError> {
    state.with(|s| {
        s.active_simulation
            .as_ref()
            .and_then(|a| a.progress.lock().ok().and_then(|p| *p))
            .map(|p| events::SimulationProgressEvent {
                time: p.time,
                fraction: p.fraction,
                samples: p.samples,
                accepted_steps: p.accepted_steps,
                rejected_steps: p.rejected_steps,
                events: p.events,
            })
    })
}

/// Whether a simulation is running.
#[tauri::command]
pub fn simulation_is_running(state: State<'_, AppState>) -> Result<bool, CommandError> {
    state.with(|s| s.active_simulation.is_some())
}

/// Read the stored result again with a different channel selection.
#[tauri::command]
pub fn simulation_load_results(
    state: State<'_, AppState>,
    channels: Option<ChannelRequest>,
) -> Result<RunResultView, CommandError> {
    state.with(|s| {
        let run = s.last_run.as_ref().ok_or_else(|| {
            missing(
                "simulation.no_result",
                "No simulation result is loaded",
                "There is no run in this session to read.",
                "Run a simulation, or open a saved run from the Projects screen.",
            )
        })?;
        Ok(view_from_outcome(
            &run.outcome,
            run.artifact_name.clone(),
            &channels.unwrap_or_default(),
        ))
    })?
}

/// List the runs saved in the open project.
#[tauri::command]
pub fn simulation_list_saved(
    state: State<'_, AppState>,
) -> Result<Vec<SavedRunSummary>, CommandError> {
    let project = state.require_project()?;
    Ok(hex_project::list_runs(&project)
        .into_iter()
        .map(|run| SavedRunSummary {
            name: run.name,
            label: run.summary.name.clone(),
            status: run.summary.status.label().to_string(),
            samples: run.summary.sample_count,
            duration: run.summary.duration,
            events: run.summary.event_count,
            warnings: run.summary.warnings.len(),
            solver: run.summary.solver.clone(),
        })
        .collect())
}

/// A saved run as the run list presents it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedRunSummary {
    /// Directory name.
    pub name: String,
    /// Run name from the summary.
    pub label: String,
    /// Status label.
    pub status: String,
    /// Samples in the history.
    pub samples: usize,
    /// Simulated duration in seconds.
    pub duration: f64,
    /// Events recorded.
    pub events: usize,
    /// Warnings recorded.
    pub warnings: usize,
    /// Solver description.
    pub solver: String,
}

/// Load a saved run back into the session.
#[tauri::command]
pub fn simulation_open_saved(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    channels: Option<ChannelRequest>,
) -> Result<RunResultView, CommandError> {
    let project = state.require_project()?;
    let artifact = hex_project::find_run(&project, &name)?;
    let history = artifact.load_history()?;
    let events = artifact.load_events()?;

    if let Some(warning) = history.integrity_warning() {
        emit_notice(
            &app,
            events::NoticeEvent::warning("History integrity", warning),
        );
    }

    let samples = history.to_samples();
    let summary = artifact.summary.clone();
    let outcome = RunOutcome {
        status: summary.status,
        samples,
        events,
        summary,
        warnings: artifact.summary.warnings.clone(),
        final_diagnostics: Default::default(),
        final_state: Default::default(),
    };
    let view = view_from_outcome(&outcome, Some(name), &channels.unwrap_or_default());
    state.with(|s| {
        s.last_run = Some(StoredRun {
            outcome,
            artifact_name: view.artifact_name.clone(),
        });
        Ok::<(), CommandError>(())
    })??;
    Ok(view)
}

/// Add a user marker to the loaded run.
#[tauri::command]
pub fn simulation_add_marker(
    app: AppHandle,
    state: State<'_, AppState>,
    time: f64,
    label: String,
) -> Result<Vec<hex_dynamics::FlightEvent>, CommandError> {
    let (project, artifact_name) = state.with(|s| {
        (
            s.project.clone(),
            s.last_run.as_ref().and_then(|r| r.artifact_name.clone()),
        )
    })?;
    if !time.is_finite() {
        return Err(crate::error::invalid(
            "The marker time must be a finite number.",
            "Type a time in seconds.",
        ));
    }
    let events = state.with(|s| {
        let run = s.last_run.as_mut().ok_or_else(|| {
            missing(
                "simulation.no_result",
                "No simulation result is loaded",
                "There is nothing to mark.",
                "Run a simulation or open a saved run first.",
            )
        })?;
        run.outcome.add_marker(time, label.clone());
        Ok::<_, CommandError>(run.outcome.events.clone())
    })??;

    if let (Some(project), Some(name)) = (project, artifact_name) {
        if let Ok(run) = hex_project::find_run(&project, &name) {
            if let Err(e) = hex_project::add_marker(&run, time, &label) {
                emit_notice(
                    &app,
                    events::NoticeEvent::warning("Marker not persisted", e.user_message()),
                );
            }
        }
    }
    Ok(events)
}

/// The events of the loaded run, for the timeline.
#[tauri::command]
pub fn simulation_events(
    state: State<'_, AppState>,
) -> Result<Vec<hex_dynamics::FlightEvent>, CommandError> {
    state.with(|s| {
        s.last_run
            .as_ref()
            .map(|r| r.outcome.events.clone())
            .ok_or_else(|| {
                missing(
                    "simulation.no_result",
                    "No simulation result is loaded",
                    "There is no timeline to show.",
                    "Run a simulation or open a saved run first.",
                )
            })
    })?
}

/// Export the loaded run history as CSV into the project reports directory.
#[tauri::command]
pub fn simulation_export_csv(
    state: State<'_, AppState>,
    name: String,
) -> Result<String, CommandError> {
    let project = state.require_project()?;
    let artifact = hex_project::find_run(&project, &name)?;
    let directory = project.reports_dir().join("exports");
    std::fs::create_dir_all(&directory)
        .map_err(|e| hex_project::ProjectError::io(&directory, e))?;
    let destination = hex_project::io::unique_file(
        &directory,
        &hex_project::io::slugify(&artifact.summary.name, "run"),
        "csv",
    );
    let rows = hex_project::export_history_csv(&artifact, &destination)?;
    Ok(format!("{} ({} rows)", destination.to_string_lossy(), rows))
}

/// The state samples of the loaded run, for the replay viewport.
#[tauri::command]
pub fn simulation_replay_data(
    state: State<'_, AppState>,
    maximum_samples: Option<usize>,
) -> Result<ReplayData, CommandError> {
    state.with(|s| {
        let run = s.last_run.as_ref().ok_or_else(|| {
            missing(
                "simulation.no_result",
                "No simulation result is loaded",
                "There is nothing to replay.",
                "Run a simulation or open a saved run first.",
            )
        })?;
        let limit = maximum_samples.unwrap_or(2000).max(2);
        let stride = (run.outcome.samples.len() / limit).max(1);
        let mut times = Vec::new();
        let mut positions = Vec::new();
        let mut quaternions = Vec::new();
        let mut velocities = Vec::new();
        let mut angular_rates = Vec::new();
        for (i, sample) in run.outcome.samples.iter().enumerate() {
            if i % stride != 0 && i + 1 != run.outcome.samples.len() {
                continue;
            }
            times.push(sample.time);
            positions.push(sample.position);
            quaternions.push(sample.attitude);
            velocities.push(sample.velocity);
            angular_rates.push(sample.angular_velocity);
        }
        Ok(ReplayData {
            times,
            positions,
            quaternions,
            velocities,
            angular_rates,
            events: run.outcome.events.clone(),
            source: "simulation".to_string(),
            has_position: true,
        })
    })?
}

/// The motion data a replay viewport needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayData {
    /// Times in seconds.
    pub times: Vec<f64>,
    /// Positions in the world frame, metres.
    pub positions: Vec<[f64; 3]>,
    /// Body-to-world quaternions, scalar first.
    pub quaternions: Vec<[f64; 4]>,
    /// Velocities in the world frame, metres per second.
    pub velocities: Vec<[f64; 3]>,
    /// Angular velocities in the body frame, radians per second.
    pub angular_rates: Vec<[f64; 3]>,
    /// Events on the timeline.
    pub events: Vec<hex_dynamics::FlightEvent>,
    /// Where the motion came from.
    pub source: String,
    /// Whether the data includes absolute position rather than orientation only.
    pub has_position: bool,
}

impl ReplayData {
    /// Number of frames.
    pub fn len(&self) -> usize {
        self.times.len()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
}

/// Build replay data from a slice of samples.
pub fn replay_from_samples(
    samples: &[StateSample],
    events: Vec<hex_dynamics::FlightEvent>,
    source: &str,
    maximum_samples: usize,
) -> ReplayData {
    let limit = maximum_samples.max(2);
    let stride = (samples.len() / limit).max(1);
    let mut times = Vec::new();
    let mut positions = Vec::new();
    let mut quaternions = Vec::new();
    let mut velocities = Vec::new();
    let mut angular_rates = Vec::new();
    for (i, sample) in samples.iter().enumerate() {
        if i % stride != 0 && i + 1 != samples.len() {
            continue;
        }
        times.push(sample.time);
        positions.push(sample.position);
        quaternions.push(sample.attitude);
        velocities.push(sample.velocity);
        angular_rates.push(sample.angular_velocity);
    }
    ReplayData {
        times,
        positions,
        quaternions,
        velocities,
        angular_rates,
        events,
        source: source.to_string(),
        has_position: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_project::Settings;

    #[test]
    fn channel_names_resolve_to_selectors() {
        for name in all_channel_names() {
            assert!(
                selector_from_name(&name).is_some(),
                "{} does not resolve",
                name
            );
        }
        assert!(selector_from_name("nonsense").is_none());
        assert_eq!(
            selector_from_name("altitude"),
            Some(ChannelSelector::Altitude)
        );
        assert_eq!(
            selector_from_name("force_x"),
            Some(ChannelSelector::ForceBodyX)
        );
    }

    #[test]
    fn every_default_channel_resolves() {
        for name in default_channel_names() {
            assert!(selector_from_name(&name).is_some(), "{}", name);
        }
        assert!(all_channel_names().len() >= default_channel_names().len());
    }

    #[test]
    fn a_default_scenario_validates_and_runs() {
        let settings = Settings::default();
        let request = ScenarioRequest::default();
        let config = build_config(&request, &settings, None);
        let validation = validate_config(&config, &request, None);
        assert!(validation.can_run, "{:?}", validation.errors);
        assert!(validation.expected_samples > 100);
        assert!(validation.solver.contains("RK4") || validation.solver.contains("RK45"));
        assert!(validation.mass_model.contains("kg"));
        assert_eq!(validation.environment.len(), 5);
        assert!(validation.active_providers.contains(&"gravity".to_string()));
        assert!(validation.active_providers.contains(&"thrust".to_string()));
        assert!(validation
            .active_providers
            .contains(&"aerodynamics".to_string()));
    }

    #[test]
    fn an_invalid_scenario_is_refused() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            end_time: Some(1.0),
            start_time: Some(5.0),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        let validation = validate_config(&config, &request, None);
        assert!(!validation.can_run);
        assert!(!validation.errors.is_empty());
        assert!(validation.report.has_errors());
    }

    #[test]
    fn a_zero_output_interval_is_refused() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            output_interval: Some(0.0),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        assert!(!validate_config(&config, &request, None).can_run);
    }

    #[test]
    fn tolerance_selects_the_adaptive_solver() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            relative_tolerance: Some(1e-9),
            absolute_tolerance: Some(1e-12),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        assert!(config.solver.is_adaptive());
        assert!(validate_config(&config, &request, None)
            .solver
            .contains("Dormand"));
    }

    #[test]
    fn the_imported_model_supplies_mass_properties() {
        let settings = Settings::default();
        let text = crate::commands::model::model_example().unwrap();
        let model = hex_model::import_json(&text, &hex_model::ImportOptions::default()).unwrap();
        let request = ScenarioRequest {
            mass: None,
            inertia: None,
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, Some(&model));
        assert!((config.initial.mass - model.mass()).abs() < 1e-12);
        match &config.mass {
            MassSpec::Constant { inertia, .. } => {
                assert!((inertia.ixx - model.inertia().ixx).abs() < 1e-12);
            }
            other => panic!("expected a constant mass, got {:?}", other),
        }
    }

    #[test]
    fn a_rail_and_wind_are_honoured() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            rail_length: Some(3.0),
            wind_speed: Some(5.0),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        assert!(config.environment.rail.enabled);
        assert!((config.environment.rail.length - 3.0).abs() < 1e-12);
        assert!((config.environment.wind.nominal_speed() - 5.0).abs() < 1e-9);
    }

    #[test]
    fn three_dof_mode_is_selected_by_name() {
        let settings = Settings::default();
        for name in ["three_dof", "3dof", "point_mass"] {
            let request = ScenarioRequest {
                mode: Some(name.to_string()),
                ..ScenarioRequest::default()
            };
            let config = build_config(&request, &settings, None);
            assert_eq!(config.mode, SimulationMode::ThreeDof, "{}", name);
        }
        let request = ScenarioRequest {
            mode: Some("six_dof".to_string()),
            ..ScenarioRequest::default()
        };
        assert_eq!(
            build_config(&request, &settings, None).mode,
            SimulationMode::SixDof
        );
    }

    #[test]
    fn the_built_in_estimate_with_lift_adds_lift_and_damping() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            aero_source: Some("estimate_with_lift".to_string()),
            lift_slope: Some(3.0),
            pitch_damping: Some(-12.0),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        let aero = config.forces.aero.as_ref().unwrap();
        assert!(!aero.drag_only);
        assert!((aero.lift_slope - 3.0).abs() < 1e-12);
        assert!((aero.pitch_damping + 12.0).abs() < 1e-12);
        assert!(aero.has_lift_or_moments());
        assert_eq!(aero.source, hex_dynamics::AeroSource::Estimate);
        assert_eq!(
            aero.fidelity_label(),
            "Simplified: coefficient model with lift and moments, constant drag coefficient"
        );
    }

    #[test]
    fn the_default_scenario_uses_the_built_in_drag_estimate() {
        let settings = Settings::default();
        let request = ScenarioRequest::default();
        let config = build_config(&request, &settings, None);
        let aero = config.forces.aero.as_ref().expect("an aerodynamic model");
        assert!(aero.drag_only);
        assert_eq!(aero.source, hex_dynamics::AeroSource::Estimate);
        assert!(!aero.has_mach_dependent_drag());
    }

    #[test]
    fn a_scenario_can_turn_aerodynamics_off() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            aero_source: Some("none".to_string()),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        assert!(config.forces.aero.is_none());
    }

    #[test]
    fn a_model_that_declares_coefficients_supplies_them_by_default() {
        let settings = Settings::default();
        let model = model_with_aero(false, vec![(0.0, 0.35), (0.8, 0.42), (1.5, 0.6)]);
        let request = ScenarioRequest::default();
        let config = build_config(&request, &settings, Some(&model));
        let aero = config.forces.aero.as_ref().expect("an aerodynamic model");

        // The model's table is used, not the scenario's constant estimate.
        assert_eq!(aero.source, hex_dynamics::AeroSource::ImportedModel);
        assert!(aero.has_mach_dependent_drag());
        assert_eq!(aero.drag.mach.len(), 3);
        assert!((aero.drag.at_mach(0.8) - 0.42).abs() < 1e-12);
        assert!((aero.drag.at_mach(0.4) - 0.385).abs() < 1e-12);
        assert!(!aero.drag_only);
        assert!(aero.has_lift_or_moments());

        // And the run says where the numbers came from.
        let validation = validate_config(&config, &request, Some(&model));
        assert!(validation
            .report
            .issues
            .iter()
            .any(|i| i.code == "aero.using_model"));
        assert!(validation
            .forces
            .iter()
            .any(|line| line.contains("coefficients from imported model")));
    }

    #[test]
    fn asking_for_model_coefficients_without_them_is_a_blocking_error() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            aero_source: Some("model".to_string()),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        let validation = validate_config(&config, &request, None);
        assert!(!validation.can_run);
        assert!(validation
            .report
            .issues
            .iter()
            .any(|i| i.code == "aero.no_coefficients" && i.severity.is_blocking()));
    }

    #[test]
    fn a_drag_only_model_keeps_its_honest_label() {
        let settings = Settings::default();
        let model = model_with_aero(true, vec![(0.0, 0.35), (1.0, 0.5)]);
        let request = ScenarioRequest::default();
        let config = build_config(&request, &settings, Some(&model));
        let aero = config.forces.aero.as_ref().expect("an aerodynamic model");
        assert!(aero.drag_only);
        assert_eq!(aero.source, hex_dynamics::AeroSource::ImportedModel);
        assert_eq!(
            aero.fidelity_label(),
            "Simplified: drag only, tabulated against Mach"
        );
    }

    #[test]
    fn choosing_an_estimate_says_the_model_coefficients_were_ignored() {
        let settings = Settings::default();
        let model = model_with_aero(false, vec![(0.0, 0.35)]);
        let request = ScenarioRequest {
            aero_source: Some("estimate".to_string()),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, Some(&model));
        let validation = validate_config(&config, &request, Some(&model));
        assert!(validation.can_run);
        assert!(validation
            .report
            .issues
            .iter()
            .any(|i| i.code == "aero.model_ignored"));
    }

    /// An imported model that declares an aerodynamics block.
    fn model_with_aero(drag_only: bool, table: Vec<(f64, f64)>) -> hex_model::ImportedModel {
        let mut document =
            hex_model::ModelDocument::minimal("test.aero", "Aero test", 4.5, [0.09, 0.09, 0.012]);
        document.aerodynamics = Some(hex_model::AeroDocument {
            drag_coefficient_at_mach: table,
            lift_slope: 2.4,
            pitch_slope: -1.8,
            pitch_damping: -9.0,
            yaw_slope: -0.9,
            roll_damping: -0.2,
            drag_only,
            ..hex_model::AeroDocument::default()
        });
        hex_model::import_json(
            &document.to_json_pretty().expect("the document serialises"),
            &hex_model::ImportOptions::default(),
        )
        .expect("the model imports")
    }

    #[test]
    fn a_constant_control_moment_is_configured() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            control_moment: Some([0.0, 1.5, 0.0]),
            control_duration: Some(0.5),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        let control = config.forces.control.as_ref().unwrap();
        assert!(control.enabled);
        assert!((control.moment_at(0.25).y - 1.5).abs() < 1e-12);
        assert!(control.moment_at(1.0).norm() < 1e-12);
    }

    #[test]
    fn a_view_is_built_from_a_real_run() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            end_time: Some(0.2),
            output_interval: Some(0.005),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        let outcome = hex_dynamics::run_simulation(config);
        assert!(outcome.status.is_usable(), "{:?}", outcome.status);

        let view = view_from_outcome(&outcome, None, &ChannelRequest::default());
        assert_eq!(view.times.len(), view.columns[0].len());
        assert_eq!(view.channel_names.len(), view.columns.len());
        assert_eq!(view.channel_units.len(), view.columns.len());
        assert!(!view.available_channels.is_empty());
        // Altitude must be monotonic upward at the start of a boosted run.
        let altitude_index = view
            .channel_names
            .iter()
            .position(|n| n == "altitude")
            .expect("altitude channel");
        assert!(view.columns[altitude_index].last().unwrap() > &0.0);
    }

    #[test]
    fn a_view_decimates_a_long_history_without_truncating_it() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            end_time: Some(2.0),
            output_interval: Some(0.001),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        let outcome = hex_dynamics::run_simulation(config);
        let total = outcome.samples.len();
        assert!(total > 500);

        let view = view_from_outcome(
            &outcome,
            None,
            &ChannelRequest {
                channels: vec!["time".to_string()],
                maximum_samples: Some(100),
            },
        );
        assert!(view.times.len() <= 200, "got {}", view.times.len());
        assert!(view.times.len() > 50);
        // The last sample is always kept, so the plot reaches the end.
        let last_time = outcome.samples.last().unwrap().time;
        assert!((view.times.last().unwrap() - last_time).abs() < 1e-12);
    }

    #[test]
    fn an_unknown_channel_is_dropped_rather_than_returning_zeros() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            end_time: Some(0.05),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        let outcome = hex_dynamics::run_simulation(config);
        let view = view_from_outcome(
            &outcome,
            None,
            &ChannelRequest {
                channels: vec!["time".to_string(), "nonsense".to_string()],
                maximum_samples: Some(10),
            },
        );
        assert_eq!(view.channel_names, vec!["time".to_string()]);
        assert_eq!(view.columns.len(), 1);
    }

    #[test]
    fn replay_data_matches_the_samples() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            end_time: Some(0.2),
            output_interval: Some(0.005),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        let outcome = hex_dynamics::run_simulation(config);
        let replay =
            replay_from_samples(&outcome.samples, outcome.events.clone(), "simulation", 100);
        assert!(!replay.is_empty());
        assert_eq!(replay.times.len(), replay.positions.len());
        assert_eq!(replay.times.len(), replay.quaternions.len());
        assert_eq!(replay.source, "simulation");
        assert!(replay.has_position);
        assert!((replay.quaternions[0][0] - outcome.samples[0].attitude[0]).abs() < 1e-15);
    }

    #[test]
    fn a_static_air_environment_is_configured() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            standard_atmosphere: Some(false),
            air_density: Some(0.9),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        match &config.environment.atmosphere {
            hex_dynamics::AtmosphereModel::Constant { density, .. } => {
                assert!((density - 0.9).abs() < 1e-12);
            }
            other => panic!("expected a constant atmosphere, got {:?}", other),
        }
        assert_eq!(
            config.environment.air_density_source,
            hex_dynamics::AirDensitySource::Constant
        );
    }

    #[test]
    fn gravity_can_be_disabled_for_a_verification_run() {
        let settings = Settings::default();
        let request = ScenarioRequest {
            gravity_enabled: Some(false),
            ..ScenarioRequest::default()
        };
        let config = build_config(&request, &settings, None);
        assert!(!config.forces.gravity);
        let validation = validate_config(&config, &request, None);
        assert!(!validation.active_providers.contains(&"gravity".to_string()));
    }
}
