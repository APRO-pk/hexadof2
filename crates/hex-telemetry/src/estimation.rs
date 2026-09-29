//! Orientation estimation from live or recorded sensor data.
//!
//! The MVP estimators are deliberately modest and are labelled as such:
//!
//! * [`EstimatorMode::GyroPropagation`] integrates the gyroscope. It drifts, and
//!   neither the code nor the UI may call the result an absolute attitude.
//! * [`EstimatorMode::Complementary`] blends the gyro integration with an
//!   accelerometer gravity reference, but only while the measured specific force
//!   is close enough to gravity that the reference is meaningful. In boosted
//!   flight that gate closes and the estimate reverts to gyro propagation.
//! * [`EstimatorMode::DeviceSupplied`] passes through a quaternion the device
//!   computed. HexaDOF does not filter it and does not vouch for it.
//!
//! # Health
//!
//! An estimate is only useful if the user knows when to distrust it, so the
//! estimator reports a health state alongside every attitude.

use hex_core::{Quaternion, Real, Vec3};
use serde::{Deserialize, Serialize};

use crate::mapping::{EstimatorMode, EstimatorSettings};

/// Which estimator produced an attitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttitudeSource {
    /// Integrated gyroscope only.
    GyroIntegrated,
    /// Gyroscope blended with a gated accelerometer reference.
    ComplementaryFilter,
    /// Supplied by the device.
    DeviceSupplied,
    /// Replayed from a flight log.
    Replayed,
    /// Produced by a simulation.
    Simulated,
    /// No attitude is available.
    Unavailable,
}

impl AttitudeSource {
    /// The label shown on the digital rocket, so a filtered estimate is never
    /// presented as measured truth.
    pub fn label(self) -> &'static str {
        match self {
            AttitudeSource::GyroIntegrated => "GYRO INTEGRATION (DRIFTS)",
            AttitudeSource::ComplementaryFilter => "COMPLEMENTARY FILTER ESTIMATE",
            AttitudeSource::DeviceSupplied => "DEVICE SUPPLIED",
            AttitudeSource::Replayed => "REPLAYED FROM LOG",
            AttitudeSource::Simulated => "SIMULATED",
            AttitudeSource::Unavailable => "NO ATTITUDE AVAILABLE",
        }
    }

    /// Whether the source is measured truth, which none of the MVP sources are.
    pub fn is_absolute_truth(self) -> bool {
        false
    }

    pub fn code(self) -> &'static str {
        match self {
            AttitudeSource::GyroIntegrated => "gyro_integrated",
            AttitudeSource::ComplementaryFilter => "complementary_filter",
            AttitudeSource::DeviceSupplied => "device_supplied",
            AttitudeSource::Replayed => "replayed",
            AttitudeSource::Simulated => "simulated",
            AttitudeSource::Unavailable => "unavailable",
        }
    }
}

/// How much the estimator should be trusted right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimatorHealth {
    /// Correction is available and the estimate is behaving.
    Good,
    /// Correction is unavailable, so the estimate is drifting.
    Degraded,
    /// The estimate should not be used for anything quantitative.
    Poor,
    /// The estimate is frozen at its last good value.
    Frozen,
}

impl EstimatorHealth {
    pub fn label(self) -> &'static str {
        match self {
            EstimatorHealth::Good => "Good",
            EstimatorHealth::Degraded => "Degraded",
            EstimatorHealth::Poor => "Poor",
            EstimatorHealth::Frozen => "Frozen",
        }
    }

    /// A sentence the UI shows beside the health indicator.
    pub fn explanation(self) -> &'static str {
        match self {
            EstimatorHealth::Good => "Correction is available and the innovation is small.",
            EstimatorHealth::Degraded => {
                "Correction is unavailable, so the attitude is integrating gyroscope drift."
            }
            EstimatorHealth::Poor => {
                "The estimate is unreliable. Treat the displayed attitude as indicative only."
            }
            EstimatorHealth::Frozen => "The estimate has been frozen at its last healthy value.",
        }
    }

    pub fn is_usable(self) -> bool {
        matches!(self, EstimatorHealth::Good | EstimatorHealth::Degraded)
    }
}

/// A full estimator report for one sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstimatorState {
    /// Estimated body-to-world attitude.
    pub attitude: Quaternion,
    /// Which estimator produced it.
    pub source: AttitudeSource,
    pub health: EstimatorHealth,
    /// Current gyroscope bias estimate, rad/s.
    pub gyro_bias: Vec3,
    /// Strength of the accelerometer correction applied this sample, between 0
    /// and 1.
    pub correction_strength: Real,
    /// Angular difference between the gyro prediction and the accelerometer
    /// reference, in radians. `None` when no correction was attempted.
    pub innovation: Option<Real>,
    /// Magnitude of the measured specific force, m/s^2.
    pub specific_force: Real,
    /// Whether the accelerometer gate is open.
    pub correction_available: bool,
    /// Age of the most recent usable accelerometer reference, seconds.
    pub reference_age: Real,
    /// Estimated drift rate, rad/s, from the bias estimate.
    pub drift_rate: Real,
    /// Whether any sensor reported saturation.
    pub saturated: bool,
    /// A short status line for the health panel.
    pub note: String,
}

impl Default for EstimatorState {
    fn default() -> Self {
        Self {
            attitude: Quaternion::identity(),
            source: AttitudeSource::Unavailable,
            health: EstimatorHealth::Poor,
            gyro_bias: Vec3::zeros(),
            correction_strength: 0.0,
            innovation: None,
            specific_force: 0.0,
            correction_available: false,
            reference_age: Real::INFINITY,
            drift_rate: 0.0,
            saturated: false,
            note: "No data yet".to_string(),
        }
    }
}

impl EstimatorState {
    /// The label to display over the digital rocket.
    pub fn attitude_source_label(&self) -> &'static str {
        self.source.label()
    }

    /// Whether the displayed attitude should be treated as indicative only.
    pub fn is_indicative_only(&self) -> bool {
        !self.health.is_usable()
    }
}

/// A sample fed into the estimator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstimatorInput {
    /// Time in seconds.
    pub time: Real,
    /// Time step since the previous sample, seconds.
    pub dt: Real,
    /// Gyroscope reading in the body frame, rad/s, after calibration.
    pub gyro_body: Vec3,
    /// Accelerometer reading in the body frame, m/s^2, after calibration.
    pub accel_body: Vec3,
    /// Magnetometer reading in the body frame, after calibration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mag_body: Option<Vec3>,
    /// A quaternion supplied by the device, when the format carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_quaternion: Option<Quaternion>,
    /// Whether the accelerometer is clipped on any axis.
    #[serde(default)]
    pub accelerometer_saturated: bool,
    /// Whether the gyroscope is clipped on any axis.
    #[serde(default)]
    pub gyroscope_saturated: bool,
}

impl Default for EstimatorInput {
    fn default() -> Self {
        Self {
            time: 0.0,
            dt: 0.0,
            gyro_body: Vec3::zeros(),
            accel_body: Vec3::zeros(),
            mag_body: None,
            device_quaternion: None,
            accelerometer_saturated: false,
            gyroscope_saturated: false,
        }
    }
}

impl EstimatorInput {
    /// A stationary sample with gravity along -Z in the body frame.
    pub fn stationary(time: Real, dt: Real) -> Self {
        Self {
            time,
            dt,
            gyro_body: Vec3::zeros(),
            accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
            ..Self::default()
        }
    }
}

/// Gyro bias estimator and attitude estimator.
#[derive(Debug, Clone)]
pub struct AttitudeEstimator {
    settings: EstimatorSettings,
    attitude: Quaternion,
    gyro_bias: Vec3,
    /// Samples accumulated for the stationary bias estimate.
    bias_samples: Vec<Vec3>,
    bias_established: bool,
    correction_strength: Real,
    /// Innovation from the most recent correction, in radians.
    pub innovation: Option<Real>,
    reference_age: Real,
    correction_available: bool,
    /// Number of consecutive samples with no usable accelerometer reference.
    starved_samples: u64,
    frozen: bool,
    source: AttitudeSource,
    reference_gravity: Real,
    /// Unit world up axis, used by the accelerometer reference.
    world_up: Vec3,
}

impl AttitudeEstimator {
    /// Build an estimator for a mode.
    pub fn new(settings: EstimatorSettings) -> Self {
        let source = match settings.mode {
            EstimatorMode::GyroPropagation => AttitudeSource::GyroIntegrated,
            EstimatorMode::Complementary => AttitudeSource::ComplementaryFilter,
            EstimatorMode::DeviceSupplied => AttitudeSource::DeviceSupplied,
        };
        Self {
            settings,
            attitude: Quaternion::identity(),
            gyro_bias: Vec3::zeros(),
            bias_samples: Vec::new(),
            bias_established: !settings.estimate_gyro_bias,
            correction_strength: 0.0,
            innovation: None,
            reference_age: Real::INFINITY,
            correction_available: false,
            starved_samples: 0,
            frozen: false,
            source,
            reference_gravity: hex_core::STANDARD_GRAVITY,
            world_up: Vec3::z(),
        }
    }

    /// Set the initial attitude, for example from a known launch pose.
    pub fn with_initial_attitude(mut self, attitude: Quaternion) -> Self {
        self.attitude = attitude.normalized();
        self
    }

    /// Set the unit world up axis used by the accelerometer gravity reference.
    ///
    /// The default is world +Z, which is correct for the ENU world frame. A
    /// project that selects NED must set this to world -Z, otherwise the
    /// complementary filter corrects toward the wrong direction.
    pub fn with_world_up(mut self, world_up: Vec3) -> Self {
        let n = world_up.norm();
        if n > 1e-9 {
            self.world_up = world_up / n;
        }
        self
    }

    /// Set the reference gravity magnitude used by the accelerometer gate.
    pub fn with_reference_gravity(mut self, gravity: Real) -> Self {
        if gravity > 1e-6 {
            self.reference_gravity = gravity;
        }
        self
    }

    /// Pre-load a known gyro bias, skipping the stationary estimate.
    pub fn with_gyro_bias(mut self, bias: Vec3) -> Self {
        self.gyro_bias = bias;
        self.bias_established = true;
        self
    }

    pub fn settings(&self) -> &EstimatorSettings {
        &self.settings
    }

    pub fn attitude(&self) -> Quaternion {
        self.attitude
    }

    pub fn gyro_bias(&self) -> Vec3 {
        self.gyro_bias
    }

    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Manually freeze the estimate at its current value.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Resume updating after a freeze.
    pub fn unfreeze(&mut self) {
        self.frozen = false;
    }

    /// Process one sample and return the new report.
    pub fn update(&mut self, input: &EstimatorInput) -> EstimatorState {
        if self.settings.mode == EstimatorMode::DeviceSupplied {
            return self.update_device_supplied(input);
        }

        // Accumulate the stationary bias estimate before anything else, because
        // a bias applied later would leave the early samples uncorrected.
        if !self.bias_established {
            if self.is_stationary(input) {
                self.bias_samples.push(input.gyro_body);
                if self.bias_samples.len() >= self.settings.bias_samples {
                    self.finish_bias_estimate();
                }
            } else if !self.bias_samples.is_empty() {
                // Motion started before enough samples arrived. Use what is
                // available rather than discarding it, and say so.
                self.finish_bias_estimate();
            }
            // While the bias is being established, report the raw integration
            // with a note so the user is not misled by an uncalibrated attitude.
            let rate = input.gyro_body;
            self.propagate(rate, input.dt);
            return self.state(
                input,
                EstimatorHealth::Degraded,
                false,
                0.0,
                None,
                format!(
                    "Establishing gyro bias from stationary samples ({}/{})",
                    self.bias_samples.len(),
                    self.settings.bias_samples
                ),
            );
        }

        let rate = input.gyro_body - self.gyro_bias;

        if self.frozen {
            return self.state(
                input,
                EstimatorHealth::Frozen,
                false,
                0.0,
                None,
                "Estimate frozen at the last healthy value".to_string(),
            );
        }

        self.propagate(rate, input.dt);

        // Judge whether the accelerometer can be used as a gravity reference.
        let specific_force = input.accel_body.norm();
        let gate_open = self.settings.accelerometer_correction
            && self.settings.mode == EstimatorMode::Complementary
            && !input.accelerometer_saturated
            && specific_force > 1e-6
            && (specific_force - self.reference_gravity).abs()
                <= self.settings.accelerometer_trust_limit;

        self.correction_available = gate_open;
        if gate_open {
            self.reference_age = 0.0;
            self.starved_samples = 0;
        } else {
            self.reference_age += input.dt.max(0.0);
            self.starved_samples += 1;
        }

        let mut innovation = None;
        if gate_open {
            // The accelerometer measures specific force, which for a static
            // accelerometer points opposite to gravity. So the measured body
            // direction of world up is the negated specific force.
            let measured_up_body = (-input.accel_body).normalized_safe();
            // The estimate's own prediction of body up follows from rotating the
            // world up axis into the body frame with the current attitude.
            let predicted_up_body = self.attitude.rotate_inverse(self.world_up);
            let error_angle = predicted_up_body
                .dot(&measured_up_body)
                .clamp(-1.0, 1.0)
                .acos();
            innovation = Some(error_angle);

            // The correction is the smallest body-frame rotation that takes the
            // measurement onto the prediction. Using the opposite order flips
            // the sign and drives the estimate away from the reference.
            let axis = measured_up_body.cross(&predicted_up_body);
            let strength = (1.0 - self.settings.gyro_weight).clamp(0.0, 1.0);
            self.correction_strength = strength;
            if axis.norm() > 1e-12 && strength > 0.0 {
                let q_error =
                    Quaternion::from_axis_angle(axis.normalized_safe(), error_angle * strength);
                self.attitude = (self.attitude * q_error).normalized();
            }
        } else {
            self.correction_strength = 0.0;
        }

        let health = self.judge_health(input, gate_open);
        let note = self.describe_health(health, gate_open, innovation);

        let state = self.state(
            input,
            health,
            gate_open,
            self.correction_strength,
            innovation,
            note,
        );
        if self.settings.freeze_on_unhealthy && health == EstimatorHealth::Poor {
            self.frozen = true;
        }
        state
    }

    fn update_device_supplied(&mut self, input: &EstimatorInput) -> EstimatorState {
        match input.device_quaternion {
            Some(q) => {
                self.attitude = q.normalized();
                self.source = AttitudeSource::DeviceSupplied;
                self.state(
                    input,
                    EstimatorHealth::Degraded,
                    false,
                    0.0,
                    None,
                    "Attitude supplied by the device. HexaDOF does not filter it or verify it."
                        .to_string(),
                )
            }
            None => {
                self.source = AttitudeSource::Unavailable;
                self.state(
                    input,
                    EstimatorHealth::Poor,
                    false,
                    0.0,
                    None,
                    "The device profile selects device-supplied attitude but no quaternion is mapped."
                        .to_string(),
                )
            }
        }
    }

    fn is_stationary(&self, input: &EstimatorInput) -> bool {
        if input.gyroscope_saturated {
            return false;
        }
        let rate = input.gyro_body.norm();
        let force = input.accel_body.norm();
        rate < 0.05 && (force - self.reference_gravity).abs() < 0.5
    }

    fn finish_bias_estimate(&mut self) {
        if self.bias_samples.is_empty() {
            self.bias_established = true;
            return;
        }
        let mut sum = Vec3::zeros();
        for s in &self.bias_samples {
            sum += *s;
        }
        self.gyro_bias = sum / self.bias_samples.len() as Real;
        self.bias_established = true;
        self.bias_samples.clear();
    }

    /// Integrate the body rate over one step, normalising afterwards.
    fn propagate(&mut self, rate: Vec3, dt: Real) {
        if !dt.is_finite() || dt <= 0.0 || rate.norm() < 1e-15 {
            return;
        }
        let q_dot = self.attitude.derivative_body_rate(rate);
        self.attitude = (self.attitude + q_dot * dt).normalized();
    }

    fn judge_health(&self, input: &EstimatorInput, gate_open: bool) -> EstimatorHealth {
        if input.gyroscope_saturated {
            return EstimatorHealth::Poor;
        }
        if gate_open {
            return EstimatorHealth::Good;
        }
        // Without a gravity reference the estimate drifts at the bias rate. A
        // large bias, or a long stretch without correction, is genuinely poor
        // rather than merely degraded.
        let drift = self.gyro_bias.norm();
        if drift > 0.05 || self.starved_samples > 20_000 {
            return EstimatorHealth::Poor;
        }
        EstimatorHealth::Degraded
    }

    fn describe_health(
        &self,
        health: EstimatorHealth,
        gate_open: bool,
        innovation: Option<Real>,
    ) -> String {
        match health {
            EstimatorHealth::Good => match innovation {
                Some(i) => format!(
                    "Correction active, innovation {:.2} degrees",
                    i.to_degrees()
                ),
                None => "Correction active".to_string(),
            },
            EstimatorHealth::Degraded => {
                if gate_open {
                    "Correction active but the estimate is relative".to_string()
                } else {
                    format!(
                        "No gravity reference for {:.1} s, so the attitude is integrating a gyro bias of {:.4} rad/s",
                        self.reference_age, self.gyro_bias.norm()
                    )
                }
            }
            EstimatorHealth::Poor => {
                if input_saturated_gyro(self) {
                    "A gyroscope axis is saturated, so the rate integral is invalid".to_string()
                } else {
                    format!(
                        "Gyro bias is {:.4} rad/s with no correction for {:.1} s. The displayed attitude is indicative only.",
                        self.gyro_bias.norm(),
                        self.reference_age
                    )
                }
            }
            EstimatorHealth::Frozen => "Estimate frozen".to_string(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn state(
        &self,
        input: &EstimatorInput,
        health: EstimatorHealth,
        correction_available: bool,
        correction_strength: Real,
        innovation: Option<Real>,
        note: String,
    ) -> EstimatorState {
        EstimatorState {
            attitude: self.attitude,
            source: self.source,
            health,
            gyro_bias: self.gyro_bias,
            correction_strength,
            innovation,
            specific_force: input.accel_body.norm(),
            correction_available,
            reference_age: self.reference_age,
            drift_rate: self.gyro_bias.norm(),
            saturated: input.gyroscope_saturated || input.accelerometer_saturated,
            note,
        }
    }
}

fn input_saturated_gyro(_estimator: &AttitudeEstimator) -> bool {
    false
}

/// Helper so a zero vector never produces a NaN direction.
trait NormalizedSafe {
    fn normalized_safe(self) -> Vec3;
}

impl NormalizedSafe for Vec3 {
    fn normalized_safe(self) -> Vec3 {
        let n = self.norm();
        if n > 1e-12 && n.is_finite() {
            self / n
        } else {
            Vec3::z()
        }
    }
}

/// Estimate a gyro bias from the first stationary samples of a series.
///
/// Returns `None` when too few stationary samples were found, rather than
/// returning a bias computed from moving data.
pub fn estimate_gyro_bias(
    samples: &[EstimatorInput],
    required: usize,
    rate_threshold: Real,
    gravity: Real,
    gravity_tolerance: Real,
) -> Option<Vec3> {
    let mut sum = Vec3::zeros();
    let mut count = 0usize;
    for s in samples {
        if s.gyro_body.norm() < rate_threshold
            && (s.accel_body.norm() - gravity).abs() < gravity_tolerance
        {
            sum += s.gyro_body;
            count += 1;
            if count >= required {
                break;
            }
        }
    }
    if count < required {
        None
    } else {
        Some(sum / count as Real)
    }
}

/// Run a whole series through an estimator, returning one report per sample.
pub fn estimate_series(
    settings: EstimatorSettings,
    samples: &[EstimatorInput],
) -> Vec<EstimatorState> {
    let mut estimator = AttitudeEstimator::new(settings);
    samples.iter().map(|s| estimator.update(s)).collect()
}

/// The quaternion at each sample, for the replay and live display.
pub fn estimate_attitudes(
    settings: EstimatorSettings,
    samples: &[EstimatorInput],
) -> Vec<Quaternion> {
    estimate_series(settings, samples)
        .into_iter()
        .map(|s| s.attitude)
        .collect()
}

/// Compare an estimated attitude history with a reference, in radians.
pub fn attitude_error_series(estimated: &[Quaternion], reference: &[Quaternion]) -> Vec<Real> {
    estimated
        .iter()
        .zip(reference.iter())
        .map(|(a, b)| a.angular_distance(b))
        .collect()
}

/// Root-mean-square attitude error, in radians.
pub fn rms_attitude_error(errors: &[Real]) -> Option<Real> {
    if errors.is_empty() {
        return None;
    }
    let sum: Real = errors.iter().map(|e| e * e).sum();
    Some((sum / errors.len() as Real).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{EstimatorMode, EstimatorSettings};

    fn gyro_only() -> AttitudeEstimator {
        AttitudeEstimator::new(EstimatorSettings {
            mode: EstimatorMode::GyroPropagation,
            estimate_gyro_bias: false,
            ..Default::default()
        })
    }

    #[test]
    fn identity_propagation_with_no_rate_holds_attitude() {
        let mut e = gyro_only();
        let state = e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.01,
            ..Default::default()
        });
        assert!(state.attitude.angular_distance(&Quaternion::identity()) < 1e-15);
        assert_eq!(state.source, AttitudeSource::GyroIntegrated);
    }

    #[test]
    fn gyro_propagation_matches_the_analytic_rotation() {
        let mut e = gyro_only();
        let rate = Vec3::new(0.0, 0.0, 1.0);
        let dt = 0.001;
        let steps = 1000;
        for i in 0..steps {
            e.update(&EstimatorInput {
                time: i as Real * dt,
                dt,
                gyro_body: rate,
                accel_body: Vec3::zeros(),
                ..Default::default()
            });
        }
        // One second at 1 rad/s about body Z is a 1 radian rotation.
        let expected = Quaternion::from_axis_angle(Vec3::z(), 1.0);
        assert!(
            e.attitude().angular_distance(&expected) < 1e-6,
            "error {}",
            e.attitude().angular_distance(&expected)
        );
    }

    #[test]
    fn gyro_only_is_never_reported_as_absolute() {
        let state = EstimatorState {
            source: AttitudeSource::GyroIntegrated,
            ..Default::default()
        };
        assert!(!state.source.is_absolute_truth());
        assert!(state.attitude_source_label().contains("DRIFTS"));
        for source in [
            AttitudeSource::GyroIntegrated,
            AttitudeSource::ComplementaryFilter,
            AttitudeSource::DeviceSupplied,
            AttitudeSource::Replayed,
            AttitudeSource::Simulated,
        ] {
            assert!(!source.is_absolute_truth());
            assert!(!source.label().is_empty());
            assert!(!source.code().is_empty());
        }
    }

    #[test]
    fn gyro_bias_is_estimated_from_stationary_samples() {
        let mut e = AttitudeEstimator::new(EstimatorSettings {
            mode: EstimatorMode::GyroPropagation,
            estimate_gyro_bias: true,
            bias_samples: 10,
            ..Default::default()
        });
        let mut last = EstimatorState::default();
        for i in 0..12 {
            last = e.update(&EstimatorInput {
                time: i as Real * 0.01,
                dt: 0.01,
                gyro_body: Vec3::new(0.02, -0.01, 0.005),
                accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
                ..Default::default()
            });
        }
        assert!((e.gyro_bias().x - 0.02).abs() < 1e-12);
        assert!(last.note.contains("Correction") || last.note.contains("bias"));
    }

    #[test]
    fn bias_estimate_stops_once_motion_starts() {
        let mut e = AttitudeEstimator::new(EstimatorSettings {
            mode: EstimatorMode::GyroPropagation,
            estimate_gyro_bias: true,
            bias_samples: 1000,
            ..Default::default()
        });
        // Three stationary samples, then a violent motion sample.
        for i in 0..3 {
            e.update(&EstimatorInput {
                time: i as Real * 0.01,
                dt: 0.01,
                gyro_body: Vec3::new(0.02, 0.0, 0.0),
                accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
                ..Default::default()
            });
        }
        e.update(&EstimatorInput {
            time: 0.03,
            dt: 0.01,
            gyro_body: Vec3::new(0.02, 0.0, 0.0),
            accel_body: Vec3::new(0.0, 0.0, -40.0),
            ..Default::default()
        });
        // The bias is from the stationary samples only, not diluted by the
        // moving one.
        assert!((e.gyro_bias().x - 0.02).abs() < 1e-12);
        assert!(e.gyro_bias().iter().all(|v| v.is_finite()));
    }

    #[test]
    fn complementary_filter_pulls_toward_the_accelerometer_reference() {
        // Start tilted, hold the board flat, and watch the estimate converge.
        let mut e = AttitudeEstimator::new(EstimatorSettings {
            estimate_gyro_bias: false,
            ..EstimatorSettings::conservative_complementary()
        })
        .with_initial_attitude(Quaternion::from_axis_angle(Vec3::y(), 0.3));
        let mut last = EstimatorState::default();
        for i in 0..2000 {
            last = e.update(&EstimatorInput {
                time: i as Real * 0.005,
                dt: 0.005,
                gyro_body: Vec3::zeros(),
                accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
                ..Default::default()
            });
        }
        assert!(last.correction_available, "the gate should be open");
        assert!(last.innovation.unwrap() < 0.01);
        // The attitude must have rotated back toward level.
        assert!(e.attitude().angular_distance(&Quaternion::identity()) < 1e-6);
    }

    #[test]
    fn complementary_filter_gate_closes_under_boost() {
        let mut e = AttitudeEstimator::new(EstimatorSettings {
            estimate_gyro_bias: false,
            ..EstimatorSettings::conservative_complementary()
        });
        let state = e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.01,
            gyro_body: Vec3::zeros(),
            // 40 m/s^2 of specific force, far from gravity, as during boost.
            accel_body: Vec3::new(0.0, 0.0, -40.0),
            ..Default::default()
        });
        assert!(!state.correction_available);
        assert_eq!(state.correction_strength, 0.0);
        assert!(state.note.contains("No gravity reference"));
    }

    #[test]
    fn health_escalates_with_a_large_bias_and_no_correction() {
        let mut e = AttitudeEstimator::new(EstimatorSettings {
            mode: EstimatorMode::GyroPropagation,
            estimate_gyro_bias: false,
            ..Default::default()
        })
        .with_gyro_bias(Vec3::new(0.5, 0.0, 0.0));
        let state = e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.01,
            gyro_body: Vec3::zeros(),
            accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
            ..Default::default()
        });
        assert_eq!(state.health, EstimatorHealth::Poor);
        assert!(state.is_indicative_only());
        assert!(state.note.contains("indicative only"));
    }

    #[test]
    fn health_reports_poor_on_gyro_saturation() {
        let mut e = gyro_only();
        let state = e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.01,
            gyro_body: Vec3::new(30.0, 0.0, 0.0),
            gyroscope_saturated: true,
            ..Default::default()
        });
        assert_eq!(state.health, EstimatorHealth::Poor);
        assert!(state.saturated);
    }

    #[test]
    fn unhealthy_estimate_freezes_when_configured() {
        let settings = EstimatorSettings {
            mode: EstimatorMode::GyroPropagation,
            estimate_gyro_bias: false,
            freeze_on_unhealthy: true,
            ..Default::default()
        };
        let mut e = AttitudeEstimator::new(settings).with_gyro_bias(Vec3::new(0.5, 0.0, 0.0));
        e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.01,
            gyro_body: Vec3::zeros(),
            accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
            ..Default::default()
        });
        assert!(e.is_frozen());
        let frozen_attitude = e.attitude();
        // A large rate must no longer move the attitude.
        let state = e.update(&EstimatorInput {
            time: 0.01,
            dt: 0.01,
            gyro_body: Vec3::new(10.0, 0.0, 0.0),
            ..Default::default()
        });
        assert_eq!(state.health, EstimatorHealth::Frozen);
        assert!(e.attitude().angular_distance(&frozen_attitude) < 1e-15);

        e.unfreeze();
        assert!(!e.is_frozen());
    }

    #[test]
    fn device_supplied_mode_passes_the_quaternion_through() {
        let mut e = AttitudeEstimator::new(EstimatorSettings {
            mode: EstimatorMode::DeviceSupplied,
            ..Default::default()
        });
        let q = Quaternion::from_axis_angle(Vec3::x(), 0.7);
        let state = e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.01,
            device_quaternion: Some(q),
            ..Default::default()
        });
        assert_eq!(state.source, AttitudeSource::DeviceSupplied);
        assert!(state.attitude.angular_distance(&q) < 1e-12);
        assert!(state.note.contains("does not filter"));
    }

    #[test]
    fn device_supplied_mode_without_a_quaternion_is_unavailable() {
        let mut e = AttitudeEstimator::new(EstimatorSettings {
            mode: EstimatorMode::DeviceSupplied,
            ..Default::default()
        });
        let state = e.update(&EstimatorInput::default());
        assert_eq!(state.source, AttitudeSource::Unavailable);
        assert_eq!(state.health, EstimatorHealth::Poor);
        assert!(state.note.contains("no quaternion is mapped"));
    }

    #[test]
    fn zero_or_negative_dt_does_not_change_the_attitude() {
        let mut e = gyro_only();
        let before = e.attitude();
        e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.0,
            gyro_body: Vec3::new(1.0, 1.0, 1.0),
            ..Default::default()
        });
        assert!(e.attitude().angular_distance(&before) < 1e-15);
        e.update(&EstimatorInput {
            time: 0.0,
            dt: -0.5,
            gyro_body: Vec3::new(1.0, 1.0, 1.0),
            ..Default::default()
        });
        assert!(e.attitude().angular_distance(&before) < 1e-15);
    }

    #[test]
    fn normalised_safe_handles_a_zero_vector() {
        assert_eq!(Vec3::zeros().normalized_safe(), Vec3::z());
        let v = Vec3::new(3.0, 4.0, 0.0).normalized_safe();
        assert!((v.norm() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn series_estimation_returns_one_report_per_sample() {
        let samples: Vec<EstimatorInput> = (0..50)
            .map(|i| EstimatorInput {
                time: i as Real * 0.01,
                dt: 0.01,
                gyro_body: Vec3::new(0.0, 0.0, 0.5),
                accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
                ..Default::default()
            })
            .collect();
        let settings = EstimatorSettings {
            mode: EstimatorMode::GyroPropagation,
            estimate_gyro_bias: false,
            ..Default::default()
        };
        let reports = estimate_series(settings, &samples);
        assert_eq!(reports.len(), 50);
        let attitudes = estimate_attitudes(settings, &samples);
        assert_eq!(attitudes.len(), 50);
        // A constant 0.5 rad/s over 0.5 s is a 0.25 radian rotation.
        let expected = Quaternion::from_axis_angle(Vec3::z(), 0.25);
        assert!(attitudes.last().unwrap().angular_distance(&expected) < 1e-4);
    }

    #[test]
    fn standalone_bias_estimate_needs_enough_stationary_samples() {
        let samples: Vec<EstimatorInput> = (0..5)
            .map(|i| EstimatorInput {
                time: i as Real,
                dt: 0.01,
                gyro_body: Vec3::new(0.02, 0.0, 0.0),
                accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
                ..Default::default()
            })
            .collect();
        let bias = estimate_gyro_bias(&samples, 5, 0.05, hex_core::STANDARD_GRAVITY, 0.5).unwrap();
        assert!((bias.x - 0.02).abs() < 1e-12);
        assert!(estimate_gyro_bias(&samples, 50, 0.05, hex_core::STANDARD_GRAVITY, 0.5).is_none());
    }

    #[test]
    fn bias_estimate_skips_moving_samples() {
        let mut samples = Vec::new();
        for i in 0..4 {
            samples.push(EstimatorInput {
                time: i as Real,
                dt: 0.01,
                gyro_body: Vec3::new(5.0, 0.0, 0.0),
                accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
                ..Default::default()
            });
        }
        for i in 4..8 {
            samples.push(EstimatorInput {
                time: i as Real,
                dt: 0.01,
                gyro_body: Vec3::new(0.01, 0.0, 0.0),
                accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
                ..Default::default()
            });
        }
        let bias = estimate_gyro_bias(&samples, 4, 0.05, hex_core::STANDARD_GRAVITY, 0.5).unwrap();
        assert!((bias.x - 0.01).abs() < 1e-12, "got {:?}", bias);
    }

    #[test]
    fn attitude_error_series_and_rms() {
        let a = vec![Quaternion::identity(), Quaternion::identity()];
        let b = vec![
            Quaternion::identity(),
            Quaternion::from_axis_angle(Vec3::z(), 0.1),
        ];
        let errors = attitude_error_series(&a, &b);
        assert_eq!(errors.len(), 2);
        assert!(errors[0] < 1e-15);
        assert!((errors[1] - 0.1).abs() < 1e-12);
        let rms = rms_attitude_error(&errors).unwrap();
        assert!(rms > 0.0 && rms < 0.1);
        assert!(rms_attitude_error(&[]).is_none());
    }

    #[test]
    fn health_labels_and_explanations_are_present() {
        for health in [
            EstimatorHealth::Good,
            EstimatorHealth::Degraded,
            EstimatorHealth::Poor,
            EstimatorHealth::Frozen,
        ] {
            assert!(!health.label().is_empty());
            assert!(!health.explanation().is_empty());
        }
        assert!(EstimatorHealth::Good.is_usable());
        assert!(EstimatorHealth::Degraded.is_usable());
        assert!(!EstimatorHealth::Poor.is_usable());
        assert!(!EstimatorHealth::Frozen.is_usable());
    }

    #[test]
    fn stationary_input_helper_is_stationary() {
        let s = EstimatorInput::stationary(1.0, 0.01);
        assert!((s.accel_body.norm() - hex_core::STANDARD_GRAVITY).abs() < 1e-9);
        assert!(s.gyro_body.norm() < 1e-15);
    }

    #[test]
    fn estimator_builders_configure_the_instance() {
        let e = AttitudeEstimator::new(EstimatorSettings::default())
            .with_initial_attitude(Quaternion::from_axis_angle(Vec3::x(), 0.5))
            .with_reference_gravity(9.81)
            .with_gyro_bias(Vec3::new(0.01, 0.0, 0.0));
        assert!((e.gyro_bias().x - 0.01).abs() < 1e-12);
        assert_eq!(e.settings().mode, EstimatorSettings::default().mode);
        // A non-positive reference gravity must not replace the default.
        let e = AttitudeEstimator::new(EstimatorSettings::default()).with_reference_gravity(0.0);
        assert!(e.attitude().norm() > 0.9);
    }

    #[test]
    fn manual_freeze_and_unfreeze() {
        let mut e = gyro_only();
        let before = e.attitude();
        e.freeze();
        e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.1,
            gyro_body: Vec3::new(5.0, 0.0, 0.0),
            ..Default::default()
        });
        assert!(e.attitude().angular_distance(&before) < 1e-15);
        e.unfreeze();
        e.update(&EstimatorInput {
            time: 0.1,
            dt: 0.1,
            gyro_body: Vec3::new(5.0, 0.0, 0.0),
            ..Default::default()
        });
        assert!(e.attitude().angular_distance(&before) > 0.1);
    }

    #[test]
    fn complementary_settings_without_accelerometer_correction_behave_as_gyro_only() {
        let mut e = AttitudeEstimator::new(EstimatorSettings {
            mode: EstimatorMode::Complementary,
            accelerometer_correction: false,
            estimate_gyro_bias: false,
            ..Default::default()
        });
        let state = e.update(&EstimatorInput {
            time: 0.0,
            dt: 0.01,
            accel_body: Vec3::new(0.0, 0.0, -hex_core::STANDARD_GRAVITY),
            ..Default::default()
        });
        assert!(!state.correction_available);
    }
}
