//! Sensor calibration and the guided validation procedure.
//!
//! Calibration is stored, never hidden. Every applied correction is recorded so a
//! recorded session can be reinterpreted later without re-running the experiment.
//!
//! Nothing here invents a correction. A three-axis accelerometer calibration with
//! fewer than the required observations stays a partial calibration and is
//! reported as such, rather than being extrapolated into a full ellipsoid fit.

use hex_core::{Frame, Quaternion, Real, Vec3};
use serde::{Deserialize, Serialize};

use crate::mapping::{ChannelDestination, DeviceProfile, SensorFamily};

/// A scalar correction for one channel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChannelCalibration {
    /// Value subtracted before scaling.
    pub offset: Real,
    /// Multiplier applied after the offset.
    pub scale: Real,
    /// Whether this calibration has been established and should be applied.
    pub enabled: bool,
}

impl Default for ChannelCalibration {
    fn default() -> Self {
        Self::identity()
    }
}

impl ChannelCalibration {
    pub const fn identity() -> Self {
        Self {
            offset: 0.0,
            scale: 1.0,
            enabled: false,
        }
    }

    /// A calibration that is applied.
    pub fn new(offset: Real, scale: Real) -> Self {
        Self {
            offset,
            scale,
            enabled: true,
        }
    }

    pub fn apply(&self, raw: Real) -> Real {
        if self.enabled {
            (raw - self.offset) * self.scale
        } else {
            raw
        }
    }

    /// Whether the calibration does anything.
    pub fn is_effective(&self) -> bool {
        self.enabled && (self.offset.abs() > 0.0 || (self.scale - 1.0).abs() > 0.0)
    }

    pub fn label(&self) -> String {
        if !self.enabled {
            "not applied".to_string()
        } else {
            format!("offset {:+.6}, scale {:.6}", self.offset, self.scale)
        }
    }
}

/// A three-axis calibration, which may be partial.
///
/// A full accelerometer calibration needs six orientations, one per axis
/// direction, to solve for a bias and a per-axis gain. Fewer observations cannot
/// separate bias from gain, so the record states exactly how many were used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreeAxisCalibration {
    /// Bias vector in the sensor frame.
    pub offset: Vec3,
    /// Per-axis gain in the sensor frame.
    pub gain: Vec3,
    /// Number of observations the fit used.
    pub observations: usize,
    /// Whether the fit is complete enough to apply.
    pub complete: bool,
    /// Residual of the fit, when one was computed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub residual: Option<Real>,
    /// Note shown in the calibration panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Default for ThreeAxisCalibration {
    fn default() -> Self {
        Self::identity()
    }
}

impl ThreeAxisCalibration {
    /// Number of observations a complete six-position fit needs.
    pub const COMPLETE_OBSERVATIONS: usize = 6;

    pub fn identity() -> Self {
        Self {
            offset: Vec3::zeros(),
            gain: Vec3::new(1.0, 1.0, 1.0),
            observations: 0,
            complete: false,
            residual: None,
            note: None,
        }
    }

    /// Apply the calibration to a sensor-frame reading.
    pub fn apply(&self, raw: Vec3) -> Vec3 {
        if !self.complete {
            return raw;
        }
        Vec3::new(
            (raw.x - self.offset.x) * self.gain.x,
            (raw.y - self.offset.y) * self.gain.y,
            (raw.z - self.offset.z) * self.gain.z,
        )
    }

    /// Solve a six-position fit from averaged readings at the six axis
    /// orientations.
    ///
    /// Each observation is the average reading while the sensor's axis is aligned
    /// with, and then against, the reference direction. For an accelerometer the
    /// reference magnitude is `reference`, which is the local gravity magnitude.
    ///
    /// Returns an incomplete calibration when fewer than six observations are
    /// supplied, because a smaller set cannot separate bias from gain.
    pub fn from_six_positions(observations: &[Vec3], reference: Real) -> Self {
        if observations.len() < Self::COMPLETE_OBSERVATIONS {
            return Self {
                offset: Vec3::zeros(),
                gain: Vec3::new(1.0, 1.0, 1.0),
                observations: observations.len(),
                complete: false,
                residual: None,
                note: Some(format!(
                    "{} of {} observations. A fewer-than-six fit cannot separate bias from gain, so no correction is applied.",
                    observations.len(),
                    Self::COMPLETE_OBSERVATIONS
                )),
            };
        }

        // Pair up +axis and -axis observations per axis.
        let mut offset = Vec3::zeros();
        let mut gain = Vec3::new(1.0, 1.0, 1.0);
        let mut residual: Real = 0.0;

        for axis in 0..3 {
            let plus = observations[axis * 2];
            let minus = observations[axis * 2 + 1];
            let p = plus[axis];
            let m = minus[axis];
            let span = p - m;
            if span.abs() < 1e-12 {
                // A degenerate pair gives no information on this axis.
                return Self {
                    offset: Vec3::zeros(),
                    gain: Vec3::new(1.0, 1.0, 1.0),
                    observations: observations.len(),
                    complete: false,
                    residual: None,
                    note: Some(format!(
                        "The +{} and -{} observations are indistinguishable, so no gain can be fitted.",
                        axis,
                        axis
                    )),
                };
            }
            let g = 2.0 * reference / span;
            let b = 0.5 * (p + m);
            offset[axis] = b;
            gain[axis] = g;
            // Check that the fitted model reproduces both observations.
            residual = residual.max(((p - b) * g - reference).abs());
            residual = residual.max(((m - b) * g + reference).abs());
        }

        Self {
            offset,
            gain,
            observations: observations.len(),
            complete: true,
            residual: Some(residual),
            note: None,
        }
    }

    /// A bias-only calibration from stationary samples.
    ///
    /// Useful for a gyroscope, where the bias is the whole correction and the
    /// gain is taken as unity because no absolute reference is available.
    pub fn bias_only(samples: &[Vec3]) -> Self {
        if samples.is_empty() {
            return Self::identity();
        }
        let mut sum = Vec3::zeros();
        for s in samples {
            sum += *s;
        }
        let mean = sum / samples.len() as Real;
        Self {
            offset: mean,
            gain: Vec3::new(1.0, 1.0, 1.0),
            observations: samples.len(),
            complete: true,
            residual: None,
            note: Some(format!(
                "Bias only, from {} stationary samples. Gain is taken as unity.",
                samples.len()
            )),
        }
    }
}

/// The complete calibration set for a device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationSet {
    /// Accelerometer calibration.
    pub accelerometer: ThreeAxisCalibration,
    /// Gyroscope calibration.
    pub gyroscope: ThreeAxisCalibration,
    /// Magnetometer calibration, usually bias-only for the MVP.
    pub magnetometer: ThreeAxisCalibration,
    /// Per-channel scalar calibrations keyed by destination.
    pub channels: Vec<(ChannelDestination, ChannelCalibration)>,
    /// Reference gravity magnitude used for the accelerometer fit, m/s^2.
    pub reference_gravity: Real,
    /// Sensor-to-body rotation established during validation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensor_to_body: Option<Quaternion>,
    /// Whether the whole set has been saved.
    pub saved: bool,
    /// When the set was established, as an opaque timestamp string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub established_at: Option<String>,
}

impl Default for CalibrationSet {
    fn default() -> Self {
        Self {
            accelerometer: ThreeAxisCalibration::identity(),
            gyroscope: ThreeAxisCalibration::identity(),
            magnetometer: ThreeAxisCalibration::identity(),
            channels: Vec::new(),
            reference_gravity: hex_core::STANDARD_GRAVITY,
            sensor_to_body: None,
            saved: false,
            established_at: None,
        }
    }
}

impl CalibrationSet {
    /// A set with only a gyro bias, which is the minimum useful calibration.
    pub fn gyro_bias_only(bias: Vec3) -> Self {
        Self {
            gyroscope: ThreeAxisCalibration {
                offset: bias,
                gain: Vec3::new(1.0, 1.0, 1.0),
                observations: 1,
                complete: true,
                residual: None,
                note: Some("Bias-only gyroscope calibration".to_string()),
            },
            ..Self::default()
        }
    }

    /// Calibration for a family.
    pub fn family(&self, family: SensorFamily) -> &ThreeAxisCalibration {
        match family {
            SensorFamily::Accelerometer => &self.accelerometer,
            SensorFamily::Gyroscope => &self.gyroscope,
            SensorFamily::Magnetometer => &self.magnetometer,
        }
    }

    /// Mutable access to a family's calibration.
    pub fn family_mut(&mut self, family: SensorFamily) -> &mut ThreeAxisCalibration {
        match family {
            SensorFamily::Accelerometer => &mut self.accelerometer,
            SensorFamily::Gyroscope => &mut self.gyroscope,
            SensorFamily::Magnetometer => &mut self.magnetometer,
        }
    }

    /// Apply the family calibration to a sensor-frame vector.
    pub fn apply_family(&self, family: SensorFamily, raw: Vec3) -> Vec3 {
        self.family(family).apply(raw)
    }

    /// Apply every correction in order: family calibration, then the
    /// sensor-to-body rotation.
    pub fn correct(&self, family: SensorFamily, raw: Vec3) -> Vec3 {
        let calibrated = self.apply_family(family, raw);
        match self.sensor_to_body {
            Some(q) => q.rotate(calibrated),
            None => calibrated,
        }
    }

    /// Several parts of a calibration are optional, so the summary must say which
    /// ones were actually established.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (family, cal) in [
            (SensorFamily::Accelerometer, &self.accelerometer),
            (SensorFamily::Gyroscope, &self.gyroscope),
            (SensorFamily::Magnetometer, &self.magnetometer),
        ] {
            lines.push(format!(
                "{}: {}",
                family.label(),
                if cal.complete {
                    match cal.residual {
                        Some(r) => format!("{} observations, residual {:.3e}", cal.observations, r),
                        None => format!("{} observations", cal.observations),
                    }
                } else {
                    format!("not established ({} observations)", cal.observations)
                }
            ));
        }
        let effective = self
            .channels
            .iter()
            .filter(|(_, c)| c.is_effective())
            .count();
        lines.push(format!("Per-channel corrections applied: {}", effective));
        lines.push(match self.sensor_to_body {
            Some(_) => "Sensor-to-body rotation: established".to_string(),
            None => "Sensor-to-body rotation: not established, identity assumed".to_string(),
        });
        lines
    }

    /// Whether the calibration is good enough to trust accelerometer-derived
    /// attitude.
    pub fn is_usable_for_attitude(&self) -> bool {
        self.gyroscope.complete
    }
}

/// A step in the guided sensor validation checklist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationStep {
    pub id: String,
    pub title: String,
    /// What the user must physically do, when the step needs an action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instruction: Option<String>,
    pub status: CheckStatus,
    /// Measurement backing the status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured: Option<Real>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ValidationStep {
    pub fn new(id: &str, title: &str) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            instruction: None,
            status: CheckStatus::Pending,
            measured: None,
            expected: None,
            detail: None,
        }
    }

    pub fn with_instruction(mut self, instruction: impl Into<String>) -> Self {
        self.instruction = Some(instruction.into());
        self
    }

    pub fn pass(mut self, measured: Real, expected: impl Into<String>) -> Self {
        self.status = CheckStatus::Pass;
        self.measured = Some(measured);
        self.expected = Some(expected.into());
        self
    }

    pub fn fail(mut self, detail: impl Into<String>) -> Self {
        self.status = CheckStatus::Fail;
        self.detail = Some(detail.into());
        self
    }

    pub fn warn(mut self, detail: impl Into<String>) -> Self {
        self.status = CheckStatus::Warning;
        self.detail = Some(detail.into());
        self
    }

    pub fn skip(mut self, detail: impl Into<String>) -> Self {
        self.status = CheckStatus::Skipped;
        self.detail = Some(detail.into());
        self
    }
}

/// Outcome of one checklist step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Not yet evaluated.
    Pending,
    Pass,
    Warning,
    Fail,
    /// Deliberately not applicable.
    Skipped,
}

impl CheckStatus {
    pub fn label(self) -> &'static str {
        match self {
            CheckStatus::Pending => "Pending",
            CheckStatus::Pass => "Pass",
            CheckStatus::Warning => "Warning",
            CheckStatus::Fail => "Fail",
            CheckStatus::Skipped => "Skipped",
        }
    }

    /// Whether this step prevents recording.
    pub fn is_blocking(self) -> bool {
        matches!(self, CheckStatus::Fail | CheckStatus::Pending)
    }
}

/// The physical orientation the user is asked to place the board in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalOrientation {
    /// Rocket nose up, board axes aligned with the body.
    NoseUp,
    /// Rocket nose down.
    NoseDown,
    /// Rocket lying on its side.
    OnSide,
    /// Device stationary on the bench, any orientation.
    Stationary,
}

impl PhysicalOrientation {
    pub fn label(self) -> &'static str {
        match self {
            PhysicalOrientation::NoseUp => "Rocket nose upward",
            PhysicalOrientation::NoseDown => "Rocket nose downward",
            PhysicalOrientation::OnSide => "Rocket lying on its side",
            PhysicalOrientation::Stationary => "Device stationary",
        }
    }

    /// What the accelerometer should read in the body frame for this pose.
    ///
    /// The body frame is X forward, Y right, Z down, so gravity appears along
    /// body Z when the vehicle is upright.
    pub fn expected_specific_force(self) -> Vec3 {
        match self {
            PhysicalOrientation::NoseUp => Vec3::new(0.0, 0.0, -1.0),
            PhysicalOrientation::NoseDown => Vec3::new(0.0, 0.0, 1.0),
            PhysicalOrientation::OnSide => Vec3::new(0.0, -1.0, 0.0),
            PhysicalOrientation::Stationary => Vec3::new(0.0, 0.0, 1.0),
        }
    }

    /// The instruction shown to the user.
    pub fn instruction(self) -> &'static str {
        match self {
            PhysicalOrientation::NoseUp => {
                "Stand the vehicle or board upright with the nose pointing up, and hold it still."
            }
            PhysicalOrientation::NoseDown => "Point the nose straight down, and hold it still.",
            PhysicalOrientation::OnSide => {
                "Lay the vehicle on its side so body Y points up, and hold it still."
            }
            PhysicalOrientation::Stationary => "Leave the device completely still on the bench.",
        }
    }
}

/// Live observations the checklist evaluates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationInput {
    /// Whether any packet has arrived at all.
    pub received_any_packet: bool,
    /// Observed packet rate, hertz.
    pub observed_rate_hz: Real,
    /// Configured packet rate, hertz.
    pub expected_rate_hz: Option<Real>,
    /// Fraction of the expected rate that counts as stable.
    pub rate_stability_tolerance: Real,
    /// Whether device timestamps are strictly increasing.
    pub timestamps_increasing: bool,
    /// Fraction of packets whose timestamps did not increase.
    pub timestamp_failure_ratio: Real,
    /// Averaged accelerometer reading in the body frame, m/s^2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accelerometer_mean: Option<Vec3>,
    /// Averaged gyroscope reading in the body frame, rad/s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gyroscope_mean: Option<Vec3>,
    /// Per-axis accelerometer ranges, m/s^2.
    pub accelerometer_ranges: Vec<Real>,
    /// Per-axis gyroscope ranges, rad/s.
    pub gyroscope_ranges: Vec<Real>,
    /// Channels that never changed.
    pub constant_channels: Vec<String>,
    /// Channels that hit their plausible limit.
    pub saturated_channels: Vec<String>,
    /// Pose the user declared for the accelerometer check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_orientation: Option<PhysicalOrientation>,
    /// Reference gravity magnitude, m/s^2.
    pub reference_gravity: Real,
    /// Whether a calibration has been saved.
    pub calibration_saved: bool,
}

impl Default for ValidationInput {
    fn default() -> Self {
        Self {
            received_any_packet: false,
            observed_rate_hz: 0.0,
            expected_rate_hz: None,
            rate_stability_tolerance: 0.2,
            timestamps_increasing: false,
            timestamp_failure_ratio: 0.0,
            accelerometer_mean: None,
            gyroscope_mean: None,
            accelerometer_ranges: Vec::new(),
            gyroscope_ranges: Vec::new(),
            constant_channels: Vec::new(),
            saturated_channels: Vec::new(),
            declared_orientation: None,
            reference_gravity: hex_core::STANDARD_GRAVITY,
            calibration_saved: false,
        }
    }
}

/// The result of running the checklist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SensorValidation {
    pub steps: Vec<ValidationStep>,
    /// Whether recording may start.
    pub ready: bool,
    /// Whether any step failed.
    pub has_failure: bool,
    /// Whether any step needs attention without blocking.
    pub has_warning: bool,
}

impl SensorValidation {
    /// A summary line for the connection panel.
    pub fn summary(&self) -> String {
        let passed = self
            .steps
            .iter()
            .filter(|s| s.status == CheckStatus::Pass)
            .count();
        let failed = self
            .steps
            .iter()
            .filter(|s| s.status == CheckStatus::Fail)
            .count();
        let warned = self
            .steps
            .iter()
            .filter(|s| s.status == CheckStatus::Warning)
            .count();
        format!(
            "{} passed, {} warnings, {} failed, of {} checks",
            passed,
            warned,
            failed,
            self.steps.len()
        )
    }

    /// The step with a given identifier.
    pub fn step(&self, id: &str) -> Option<&ValidationStep> {
        self.steps.iter().find(|s| s.id == id)
    }
}

/// Run the guided validation checklist.
///
/// Every check produces a step, including the ones that could not be evaluated,
/// so the panel always shows the same list and a user can see what is missing
/// rather than guessing.
pub fn run_validation(input: &ValidationInput) -> SensorValidation {
    let mut steps = Vec::new();

    // 1. The device responds.
    steps.push(if input.received_any_packet {
        ValidationStep::new("device.responds", "Device responds").pass(1.0, "at least one packet")
    } else {
        ValidationStep::new("device.responds", "Device responds")
            .with_instruction("Check the cable, the selected port, and the baud rate.")
            .fail("No packet has been received on this port.")
    });

    // 2. Packet rate is stable.
    steps.push(match input.expected_rate_hz {
        Some(expected) if expected > 0.0 => {
            let ratio = input.observed_rate_hz / expected;
            let low = 1.0 - input.rate_stability_tolerance;
            let high = 1.0 + input.rate_stability_tolerance;
            if ratio >= low && ratio <= high {
                ValidationStep::new("rate.stable", "Packet rate is stable").pass(
                    input.observed_rate_hz,
                    format!("{:.1} to {:.1} Hz", expected * low, expected * high),
                )
            } else {
                ValidationStep::new("rate.stable", "Packet rate is stable")
                    .with_instruction(
                        "Check that the device is not blocked by a long loop, and that the baud rate is correct.",
                    )
                    .warn(format!(
                        "Observed {:.1} Hz against a configured {:.1} Hz.",
                        input.observed_rate_hz, expected
                    ))
            }
        }
        _ => ValidationStep::new("rate.stable", "Packet rate is stable")
            .skip("No expected rate is configured, so stability cannot be judged."),
    });

    // 3. Timestamps increase.
    steps.push(if input.timestamps_increasing {
        ValidationStep::new("time.increasing", "Timestamps increase").pass(
            1.0 - input.timestamp_failure_ratio,
            "strictly increasing",
        )
    } else if input.received_any_packet {
        ValidationStep::new("time.increasing", "Timestamps increase")
            .with_instruction(
                "A non-increasing timestamp is usually a counter that wrapped or a device clock reset.",
            )
            .fail(format!(
                "{:.1} percent of timestamps did not increase.",
                100.0 * input.timestamp_failure_ratio
            ))
    } else {
        ValidationStep::new("time.increasing", "Timestamps increase")
            .fail("No timestamps have been received.")
    });

    // 4 and 5. Accelerometer and gyroscope ranges.
    steps.push(range_step(
        "accel.range",
        "Accelerometer within its expected range",
        &input.accelerometer_ranges,
        &[-160.0, 160.0],
        "m/s^2",
    ));
    steps.push(range_step(
        "gyro.range",
        "Gyroscope within its expected range",
        &input.gyroscope_ranges,
        &[-35.0, 35.0],
        "rad/s",
    ));

    // 6. No channel is permanently zero.
    steps.push(if input.constant_channels.is_empty() {
        ValidationStep::new("channel.not_constant", "No channel is stuck")
            .pass(0.0, "every mapped channel changed")
    } else {
        ValidationStep::new("channel.not_constant", "No channel is stuck").warn(format!(
            "These channels never changed: {}.",
            input.constant_channels.join(", ")
        ))
    });

    // 7. No channel is saturated.
    steps.push(if input.saturated_channels.is_empty() {
        ValidationStep::new("channel.not_saturated", "No channel is saturated")
            .pass(0.0, "no channel at its limit")
    } else {
        ValidationStep::new("channel.not_saturated", "No channel is saturated")
            .with_instruction("Reduce the stimulus or increase the sensor range.")
            .warn(format!(
                "These channels reached a plausible limit: {}.",
                input.saturated_channels.join(", ")
            ))
    });

    // 8. Axis mapping confirmed.
    steps.push(
        ValidationStep::new("axes.confirmed", "Axis mapping confirmed")
            .with_instruction(
                "Compare the preview against the physical pose. A swapped or inverted axis is the most common cause of an attitude that looks wrong.",
            )
            .skip("Confirm the mapping in the preview table before recording."),
    );

    // 9. Static gravity direction is plausible.
    steps.push(gravity_direction_step(input));

    // 10. Calibration is saved.
    steps.push(if input.calibration_saved {
        ValidationStep::new("calibration.saved", "Calibration is saved").pass(1.0, "saved")
    } else {
        ValidationStep::new("calibration.saved", "Calibration is saved")
            .with_instruction(
                "Run the stationary bias check and the six-position accelerometer check, then save.",
            )
            .warn("No calibration is saved, so raw values will be used.")
    });

    let has_failure = steps.iter().any(|s| s.status == CheckStatus::Fail);
    let has_warning = steps.iter().any(|s| s.status == CheckStatus::Warning);
    let ready = !has_failure;

    SensorValidation {
        steps,
        ready,
        has_failure,
        has_warning,
    }
}

fn range_step(
    id: &str,
    title: &str,
    observed: &[Real],
    limits: &[Real; 2],
    unit: &str,
) -> ValidationStep {
    if observed.is_empty() {
        return ValidationStep::new(id, title).skip("No samples have been collected yet.");
    }
    let peak = observed.iter().fold(0.0 as Real, |a, v| a.max(v.abs()));
    if peak >= limits[1] {
        ValidationStep::new(id, title).warn(format!(
            "The largest magnitude was {:.3} {}, at or above the plausible limit of {:.1} {}.",
            peak, unit, limits[1], unit
        ))
    } else {
        ValidationStep::new(id, title).pass(peak, format!("below {:.1} {}", limits[1], unit))
    }
}

fn gravity_direction_step(input: &ValidationInput) -> ValidationStep {
    let Some(orientation) = input.declared_orientation else {
        return ValidationStep::new("gravity.direction", "Static gravity direction is plausible")
            .with_instruction(
                "Choose the pose the device is in, then hold it still so the average can be compared.",
            )
            .skip("No pose has been declared.");
    };
    let Some(mean) = input.accelerometer_mean else {
        return ValidationStep::new("gravity.direction", "Static gravity direction is plausible")
            .with_instruction(orientation.instruction())
            .skip("No stationary samples have been collected for this pose.");
    };

    let magnitude = mean.norm();
    let expected_magnitude = input.reference_gravity;
    let magnitude_error = (magnitude - expected_magnitude).abs() / expected_magnitude;

    // The specific force sensed while static is opposite to gravity, so compare
    // against the negated expectation from the pose table.
    // An accelerometer measures specific force, so the static reading points
    // along the pose table entry directly. Negating it would compare against
    // gravity instead of against the measurement.
    let expected_direction = orientation.expected_specific_force();
    let direction_error = if magnitude > 1e-6 {
        let observed = mean / magnitude;
        let cosine = observed.dot(&expected_direction).clamp(-1.0, 1.0);
        cosine.acos().to_degrees()
    } else {
        180.0
    };

    if magnitude_error > 0.15 {
        ValidationStep::new("gravity.direction", "Static gravity direction is plausible")
            .with_instruction(
                "Check that the accelerometer unit is correct. A reading in g instead of m/s^2 is the usual cause.",
            )
            .fail(format!(
                "The magnitude was {:.3} m/s^2 against an expected {:.3} m/s^2, a {:.1} percent difference. Check the unit and the calibration.",
                magnitude,
                expected_magnitude,
                100.0 * magnitude_error
            ))
    } else if direction_error > 25.0 {
        ValidationStep::new("gravity.direction", "Static gravity direction is plausible")
            .with_instruction(
                "If the pose is correct, an axis is swapped or inverted. Recheck the axis mapping and sign columns.",
            )
            .fail(format!(
                "The gravity direction is {:.1} degrees from what the declared pose predicts ({}). An axis is probably swapped or inverted.",
                direction_error,
                orientation.label()
            ))
    } else if direction_error > 10.0 {
        ValidationStep::new("gravity.direction", "Static gravity direction is plausible")
            .warn(format!(
                "The gravity direction is {:.1} degrees from the expected direction, which suggests a small mounting tilt or a partial calibration.",
                direction_error
            ))
    } else {
        ValidationStep::new("gravity.direction", "Static gravity direction is plausible").pass(
            direction_error,
            "within 10 degrees of the expected direction",
        )
    }
}

/// Average a set of sensor-frame readings, used by the calibration procedure.
pub fn average(samples: &[Vec3]) -> Vec3 {
    if samples.is_empty() {
        return Vec3::zeros();
    }
    let mut sum = Vec3::zeros();
    for s in samples {
        sum += *s;
    }
    sum / samples.len() as Real
}

/// Report which channels a calibration set will actually affect.
pub fn calibration_impact(profile: &DeviceProfile, calibration: &CalibrationSet) -> Vec<String> {
    let mut lines = Vec::new();
    for family in [
        SensorFamily::Accelerometer,
        SensorFamily::Gyroscope,
        SensorFamily::Magnetometer,
    ] {
        if !profile.has_complete_family(family) {
            continue;
        }
        let cal = calibration.family(family);
        lines.push(format!(
            "{}: {}",
            family.label(),
            if cal.complete {
                "a correction will be applied"
            } else {
                "raw values will be used"
            }
        ));
    }
    if profile.board_rotation.is_some() || calibration.sensor_to_body.is_some() {
        lines.push(
            "A sensor-to-body rotation is configured, so every vector is rotated before use."
                .to_string(),
        );
    }
    lines
}

/// Check that the declared frame of a profile matches the frame a project uses.
pub fn frame_matches(profile_frame: Frame, project_frame: Frame) -> bool {
    profile_frame == project_frame
}

/// The destinations a calibration set covers.
pub fn calibrated_destinations(set: &CalibrationSet) -> Vec<ChannelDestination> {
    set.channels
        .iter()
        .filter(|(_, c)| c.is_effective())
        .map(|(d, _)| *d)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::DeviceProfile;

    #[test]
    fn channel_calibration_identity_does_nothing() {
        let c = ChannelCalibration::identity();
        assert!((c.apply(5.0) - 5.0).abs() < 1e-15);
        assert!(!c.is_effective());
        assert_eq!(c.label(), "not applied");
    }

    #[test]
    fn channel_calibration_applies_offset_then_scale() {
        let c = ChannelCalibration::new(1.0, 2.0);
        assert!((c.apply(3.0) - 4.0).abs() < 1e-15);
        assert!(c.is_effective());
        assert!(c.label().contains("offset"));
    }

    #[test]
    fn disabled_channel_calibration_is_ignored() {
        let c = ChannelCalibration {
            enabled: false,
            ..ChannelCalibration::new(5.0, 3.0)
        };
        assert!((c.apply(1.0) - 1.0).abs() < 1e-15);
        assert!(!c.is_effective());
    }

    #[test]
    fn six_position_fit_recovers_a_known_bias_and_gain() {
        // True model: reading = true / gain + bias.
        let bias = Vec3::new(0.25, -0.4, 0.1);
        let gain = Vec3::new(1.02, 0.98, 1.05);
        let g = 9.80665;
        let mut observations = Vec::new();
        for axis in 0..3 {
            let mut plus = Vec3::zeros();
            plus[axis] = g / gain[axis] + bias[axis];
            let mut minus = Vec3::zeros();
            minus[axis] = -g / gain[axis] + bias[axis];
            observations.push(plus);
            observations.push(minus);
        }
        let cal = ThreeAxisCalibration::from_six_positions(&observations, g);
        assert!(cal.complete);
        assert_eq!(cal.observations, 6);
        // Applying the calibration must recover +/- g on the driven axis. The
        // other axes carry the residual bias, which is expected because this
        // observation had no stimulus on them.
        for axis in 0..3 {
            let corrected = cal.apply(observations[axis * 2]);
            assert!((corrected[axis] - g).abs() < 1e-9, "axis {} plus", axis);
            let corrected_neg = cal.apply(observations[axis * 2 + 1]);
            assert!(
                (corrected_neg[axis] + g).abs() < 1e-9,
                "axis {} minus",
                axis
            );
        }
        assert!(cal.residual.unwrap() < 1e-9);
    }

    #[test]
    fn incomplete_six_position_fit_is_not_applied() {
        let g = 9.80665;
        let observations = vec![
            Vec3::new(0.0, 0.0, g),
            Vec3::new(0.0, 0.0, -g),
            Vec3::new(g, 0.0, 0.0),
        ];
        let cal = ThreeAxisCalibration::from_six_positions(&observations, g);
        assert!(!cal.complete);
        assert_eq!(cal.observations, 3);
        assert!(cal
            .note
            .as_ref()
            .unwrap()
            .contains("cannot separate bias from gain"));
        // An incomplete fit must be the identity.
        let raw = Vec3::new(1.0, 2.0, 3.0);
        assert!((cal.apply(raw) - raw).norm() < 1e-15);
    }

    #[test]
    fn degenerate_six_position_pair_is_rejected() {
        let g = 9.80665;
        let observations = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(g, 0.0, 0.0),
            Vec3::new(-g, 0.0, 0.0),
            Vec3::new(0.0, g, 0.0),
            Vec3::new(0.0, -g, 0.0),
        ];
        let cal = ThreeAxisCalibration::from_six_positions(&observations, g);
        assert!(!cal.complete);
        assert!(cal.note.as_ref().unwrap().contains("indistinguishable"));
    }

    #[test]
    fn bias_only_calibration_averages_the_samples() {
        let samples = [Vec3::new(0.1, 0.2, 0.3), Vec3::new(0.3, 0.2, 0.1)];
        let cal = ThreeAxisCalibration::bias_only(&samples);
        assert!(cal.complete);
        assert!((cal.offset - Vec3::new(0.2, 0.2, 0.2)).norm() < 1e-12);
        assert!((cal.gain - Vec3::new(1.0, 1.0, 1.0)).norm() < 1e-15);
        assert!(cal.note.as_ref().unwrap().contains("Bias only"));
    }

    #[test]
    fn bias_only_with_no_samples_is_the_identity() {
        let cal = ThreeAxisCalibration::bias_only(&[]);
        assert!(!cal.complete);
        assert!((cal.offset.norm()).abs() < 1e-15);
    }

    #[test]
    fn calibration_set_corrects_then_rotates() {
        let set = CalibrationSet {
            gyroscope: ThreeAxisCalibration {
                offset: Vec3::new(0.1, 0.0, 0.0),
                gain: Vec3::new(1.0, 1.0, 1.0),
                observations: 1,
                complete: true,
                residual: None,
                note: None,
            },
            sensor_to_body: Some(Quaternion::from_axis_angle(
                Vec3::z(),
                std::f64::consts::FRAC_PI_2,
            )),
            ..CalibrationSet::default()
        };
        let out = set.correct(SensorFamily::Gyroscope, Vec3::new(0.1, 1.0, 0.0));
        // Bias removed first, giving body (0, 1, 0). A +90 degree rotation about
        // Z then maps body X to body Y, so (0, 1, 0) becomes (-1, 0, 0).
        assert!((out.x + 1.0).abs() < 1e-12, "got {:?}", out);
        assert!(out.y.abs() < 1e-12);
    }

    #[test]
    fn calibration_set_summary_states_what_is_missing() {
        let set = CalibrationSet::gyro_bias_only(Vec3::new(0.01, 0.0, 0.0));
        let summary = set.summary();
        assert!(summary
            .iter()
            .any(|l| l.contains("Accelerometer: not established")));
        assert!(summary
            .iter()
            .any(|l| l.contains("Gyroscope: 1 observations")));
        assert!(summary
            .iter()
            .any(|l| l.contains("Sensor-to-body rotation: not established")));
        assert!(set.is_usable_for_attitude());
    }

    #[test]
    fn physical_orientation_expectations_are_opposite_to_gravity() {
        // Body Z points down, so an upright vehicle senses specific force along
        // -Z from the pose table, and the accelerometer mean should be opposite.
        assert_eq!(
            PhysicalOrientation::NoseUp.expected_specific_force(),
            Vec3::new(0.0, 0.0, -1.0)
        );
        assert!(!PhysicalOrientation::NoseUp.instruction().is_empty());
        assert!(PhysicalOrientation::OnSide.instruction().contains("side"));
    }

    #[test]
    fn validation_passes_on_a_good_device() {
        let input = ValidationInput {
            received_any_packet: true,
            observed_rate_hz: 100.0,
            expected_rate_hz: Some(100.0),
            timestamps_increasing: true,
            accelerometer_mean: Some(Vec3::new(0.0, 0.0, -9.81)),
            gyroscope_mean: Some(Vec3::zeros()),
            accelerometer_ranges: vec![0.1, 0.1, 0.2],
            gyroscope_ranges: vec![0.01, 0.01, 0.01],
            declared_orientation: Some(PhysicalOrientation::NoseUp),
            calibration_saved: true,
            ..Default::default()
        };
        let result = run_validation(&input);
        assert!(!result.has_failure, "{:#?}", result.steps);
        assert!(result.ready);
        assert!(result.summary().contains("passed"));
        assert_eq!(
            result.step("device.responds").unwrap().status,
            CheckStatus::Pass
        );
    }

    #[test]
    fn validation_fails_when_the_device_is_silent() {
        let result = run_validation(&ValidationInput::default());
        assert!(result.has_failure);
        assert!(!result.ready);
        assert_eq!(
            result.step("device.responds").unwrap().status,
            CheckStatus::Fail
        );
        assert_eq!(
            result.step("time.increasing").unwrap().status,
            CheckStatus::Fail
        );
    }

    #[test]
    fn validation_warns_on_a_wrong_packet_rate() {
        let input = ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            observed_rate_hz: 30.0,
            expected_rate_hz: Some(100.0),
            ..Default::default()
        };
        let result = run_validation(&input);
        assert_eq!(
            result.step("rate.stable").unwrap().status,
            CheckStatus::Warning
        );
        assert!(!result.has_failure);
    }

    #[test]
    fn validation_skips_the_rate_check_without_an_expected_rate() {
        let input = ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            ..Default::default()
        };
        let result = run_validation(&input);
        assert_eq!(
            result.step("rate.stable").unwrap().status,
            CheckStatus::Skipped
        );
    }

    #[test]
    fn validation_detects_a_stuck_channel() {
        let input = ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            constant_channels: vec!["gz".to_string()],
            ..Default::default()
        };
        let result = run_validation(&input);
        let step = result.step("channel.not_constant").unwrap();
        assert_eq!(step.status, CheckStatus::Warning);
        assert!(step.detail.as_ref().unwrap().contains("gz"));
    }

    #[test]
    fn validation_detects_saturation() {
        let input = ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            saturated_channels: vec!["ax".to_string()],
            accelerometer_ranges: vec![170.0, 1.0, 1.0],
            ..Default::default()
        };
        let result = run_validation(&input);
        assert_eq!(
            result.step("channel.not_saturated").unwrap().status,
            CheckStatus::Warning
        );
        assert_eq!(
            result.step("accel.range").unwrap().status,
            CheckStatus::Warning
        );
    }

    #[test]
    fn validation_fails_a_wrong_gravity_magnitude() {
        // A reading in g instead of m/s^2.
        let input = ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            accelerometer_mean: Some(Vec3::new(0.0, 0.0, -1.0)),
            declared_orientation: Some(PhysicalOrientation::NoseUp),
            ..Default::default()
        };
        let result = run_validation(&input);
        let step = result.step("gravity.direction").unwrap();
        assert_eq!(step.status, CheckStatus::Fail);
        assert!(step.detail.as_ref().unwrap().contains("unit"));
    }

    #[test]
    fn validation_fails_a_swapped_axis() {
        // Nose up should read -Z, but the raw data reads +Y.
        let input = ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            accelerometer_mean: Some(Vec3::new(0.0, -9.81, 0.0)),
            declared_orientation: Some(PhysicalOrientation::NoseUp),
            ..Default::default()
        };
        let result = run_validation(&input);
        let step = result.step("gravity.direction").unwrap();
        assert_eq!(step.status, CheckStatus::Fail);
        assert!(step
            .detail
            .as_ref()
            .unwrap()
            .contains("swapped or inverted"));
    }

    #[test]
    fn validation_accepts_a_small_mounting_tilt() {
        let tilt = 6.0f64.to_radians();
        // The accelerometer measures specific force, so the pose table entry is
        // the expected measurement direction. Rotate it by a small mounting tilt.
        let expected = PhysicalOrientation::NoseUp.expected_specific_force();
        let mean = Quaternion::from_axis_angle(Vec3::x(), tilt).rotate(expected)
            * hex_core::STANDARD_GRAVITY;
        let input = ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            accelerometer_mean: Some(mean),
            declared_orientation: Some(PhysicalOrientation::NoseUp),
            ..Default::default()
        };
        let result = run_validation(&input);
        assert_eq!(
            result.step("gravity.direction").unwrap().status,
            CheckStatus::Pass
        );
    }

    #[test]
    fn validation_skips_the_gravity_check_without_a_pose() {
        let input = ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            accelerometer_mean: Some(Vec3::new(0.0, 0.0, -9.81)),
            ..Default::default()
        };
        let result = run_validation(&input);
        assert_eq!(
            result.step("gravity.direction").unwrap().status,
            CheckStatus::Skipped
        );
    }

    #[test]
    fn axis_confirmation_is_always_manual() {
        let result = run_validation(&ValidationInput {
            received_any_packet: true,
            timestamps_increasing: true,
            ..Default::default()
        });
        assert_eq!(
            result.step("axes.confirmed").unwrap().status,
            CheckStatus::Skipped
        );
    }

    #[test]
    fn calibration_impact_only_mentions_complete_families() {
        let profile = DeviceProfile::example_csv_profile();
        let set = CalibrationSet::default();
        let lines = calibration_impact(&profile, &set);
        assert!(lines.iter().any(|l| l.contains("Accelerometer")));
        assert!(lines.iter().any(|l| l.contains("Gyroscope")));
        assert!(!lines.iter().any(|l| l.contains("Magnetometer")));
    }

    #[test]
    fn frame_matching_is_exact() {
        let a = Frame::default();
        assert!(frame_matches(a, a));
        let b = Frame::new(
            hex_core::WorldFrame::Ned,
            hex_core::BodyFrame::ForwardLeftUp,
        );
        assert!(!frame_matches(a, b));
    }

    #[test]
    fn average_of_no_samples_is_zero() {
        assert!(average(&[]).norm() < 1e-15);
        let a = average(&[Vec3::new(1.0, 2.0, 3.0), Vec3::new(3.0, 4.0, 5.0)]);
        assert!((a - Vec3::new(2.0, 3.0, 4.0)).norm() < 1e-12);
    }

    #[test]
    fn calibrated_destinations_lists_effective_channels() {
        let set = CalibrationSet {
            channels: vec![
                (
                    ChannelDestination::Barometer,
                    ChannelCalibration::new(10.0, 1.0),
                ),
                (ChannelDestination::Motor, ChannelCalibration::identity()),
            ],
            ..CalibrationSet::default()
        };
        let dests = calibrated_destinations(&set);
        assert_eq!(dests, vec![ChannelDestination::Barometer]);
    }

    #[test]
    fn check_status_blocking_rules() {
        assert!(CheckStatus::Fail.is_blocking());
        assert!(CheckStatus::Pending.is_blocking());
        assert!(!CheckStatus::Pass.is_blocking());
        assert!(!CheckStatus::Warning.is_blocking());
        assert!(!CheckStatus::Skipped.is_blocking());
        assert_eq!(CheckStatus::Warning.label(), "Warning");
    }

    #[test]
    fn validation_step_builders_set_the_right_status() {
        assert_eq!(
            ValidationStep::new("a", "A").pass(1.0, "x").status,
            CheckStatus::Pass
        );
        assert_eq!(
            ValidationStep::new("a", "A").fail("x").status,
            CheckStatus::Fail
        );
        assert_eq!(
            ValidationStep::new("a", "A").warn("x").status,
            CheckStatus::Warning
        );
        assert_eq!(
            ValidationStep::new("a", "A").skip("x").status,
            CheckStatus::Skipped
        );
        assert!(ValidationStep::new("a", "A")
            .with_instruction("do it")
            .instruction
            .is_some());
    }
}
