//! HexaDOF core math and conventions.
//!
//! This crate is deliberately free of any dependency on the frontend, the
//! telemetry transport, or the renderer. It defines the shared vocabulary that
//! every other HexaDOF crate builds on:
//!
//! * SI-first [`units`] with UI-boundary conversion helpers.
//! * Explicit coordinate [`frames`].
//! * Quaternion and rotation conventions, tested against known rotations.
//! * Timestamp handling for device and host clocks.
//! * A structured [`validation`] report type used across the workspace.
//!
//! # Conventions
//!
//! * Quaternions are scalar first: `q = [w, x, y, z]`.
//! * `q_bi` maps **body** vectors into the **inertial** frame:
//!   `v_i = q_bi * v_b`.
//! * Angular velocity is expressed in the **body** frame.
//! * All internal quantities are SI.

pub mod frames;
pub mod mass;
pub mod quaternion;
pub mod timestamps;
pub mod units;
pub mod validation;
pub mod vectors;

pub use frames::{AxisMapping, BodyFrame, Frame, FrameTransform, SensorAxes, WorldFrame};
pub use mass::{InertiaTensor, MassCurve, MassProperties};
pub use quaternion::{Quaternion, UnitQuaternion};
pub use timestamps::{
    analyze_clock, ClockAnalysis, DeviceClock, Gap, RolloverCorrection, TimeSource, Timebase,
    Timestamp, TimestampUnit,
};
pub use units::{
    Angle, AngularRate, Length, Mass, Pressure, Quantity, QuantityKind, Temperature, UnitScale,
    UnitSystem, Velocity,
};
pub use validation::{Severity, ValidationIssue, ValidationReport, ValidationStatus};
pub use vectors::{normalized, normalized_or, Mat3, Vec3, Vec3Record};

/// Scalar type used throughout the numerical core.
pub type Real = f64;

/// Standard gravity, m/s^2.
pub const STANDARD_GRAVITY: Real = 9.80665;

/// Mean Earth radius, m.
pub const EARTH_RADIUS: Real = 6_371_000.0;

/// Earth gravitational parameter, m^3/s^2.
pub const EARTH_MU: Real = 3.986_004_418e14;

/// Default tolerance for symmetric-tensor and orthogonality comparisons.
pub const DEFAULT_TOLERANCE: Real = 1e-9;

/// Absolute tolerance used when testing near-zero quantities.
pub const DEFAULT_ABS_TOLERANCE: Real = 1e-12;

/// Application version string, embedded into run metadata.
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
