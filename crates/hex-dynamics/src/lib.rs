//! HexaDOF rigid-body and point-mass flight dynamics.
//!
//! This crate is the numerical core. It has no dependency on the desktop shell,
//! on any file format, or on the renderer. It consumes a [`VehicleModel`] and an
//! [`Environment`], and produces a state history plus the diagnostics needed to
//! judge whether that history is trustworthy.
//!
//! # Conventions
//!
//! * SI units throughout.
//! * Quaternions are scalar first and map body vectors into the world frame.
//! * Angular velocity is expressed in the body frame.
//! * The state is a flat 14-element array documented in [`state`].
//!
//! # Verification
//!
//! The test suite covers the verification cases required by the specification:
//! free translation, uniform gravity, constant torque, torque-free asymmetric
//! body, quaternion rotation, force at an offset, frame transforms, and a
//! versioned regression scenario.

pub mod environment;
pub mod equations;
pub mod events;
pub mod forces;
pub mod integrators;
pub mod regression;
pub mod runner;
pub mod state;
pub mod vehicle;

pub use environment::{
    AirDensitySource, AirProperties, AtmosphereModel, Environment, GravityModel, GroundPlane,
    LaunchRail, StandardAtmosphere, WindModel,
};
pub use equations::{
    BoundDynamics, DynamicsDiagnostics, ProviderChain, RigidBodyDynamics, StateHealth,
};
pub use events::{
    detect_apogee_from_state, find_apogee, find_ground_impact, find_max_dynamic_pressure,
    first_of_kind, sort_events, time_between, EventDetector, EventKind, EventObservation,
    EventRules, FlightEvent,
};
pub use forces::{
    accumulate, AeroCoefficientTable, AeroDerivatives, AeroForce, AeroModel, AeroSource,
    ControlForce, ControlMoment, ExternalLoad, ForceAccumulator, ForceContext, ForceContribution,
    ForceProvider, ForceSource, GravityForce, LandingBurn, ThrustForce, ThrustProfile,
};
pub use integrators::{
    acceleration_from_force, state_from_array, state_to_array, Derivative, ErrorWeights,
    NormalizationPolicy, Rk4, Rk45, SolverKind, SolverStatistics, StepOutcome,
};
pub use regression::regression_scenarios;
pub use runner::{
    build_run, build_run_with_vehicle, default_charts, index_at_time, run_built, run_simulation,
    run_simulation_with_progress, CancellationToken, ChannelSelector, ForceConfig,
    InitialConditions, MassSpec, RunConfig, RunOutcome, RunProgress, RunSummary, RunWarning,
    SimulationResult, SimulationStatus, WarningThresholds,
};
pub use state::{RigidBodyState, SimulationMode, StateSample, STATE_LEN};
pub use vehicle::{
    ConstantMass, CurveMass, MassFlowProfile, MassProvider, ReferenceGeometry, Vehicle,
    VehicleModel,
};

/// Re-export of the core math types, so a consumer needs only one dependency.
pub use hex_core as core;

/// A one-line capability description for the diagnostics bundle.
pub fn capability_summary() -> String {
    format!(
        "Dynamics: {} simulation modes, {} state channels, {} solver families, {} event rules",
        SimulationMode::SixDof.label().len() + SimulationMode::ThreeDof.label().len(),
        runner::all_channel_names().len(),
        2,
        7
    )
}
