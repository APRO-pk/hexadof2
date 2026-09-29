//! Derived channels and the state-estimation inputs built from them.
//!
//! Everything here is computed from normalised channels and is labelled with its
//! [`Provenance`], so the UI can warn the user when a plotted signal was not
//! measured. Integrated quantities drift, and a channel that looks like a
//! measurement but is actually an integral must be visibly marked.

use serde::{Deserialize, Serialize};

use hex_core::{Quaternion, Real, Vec3};

use crate::channel::ChannelRole;

/// Temperature lapse rate of the ISA troposphere, K/m.
pub const ISA_LAPSE_RATE: Real = 0.0065;

/// Specific gas constant of dry air, J/(kg*K).
pub const ISA_GAS_CONSTANT: Real = 287.052_87;

/// Standard sea-level pressure, Pa.
pub const ISA_SEA_LEVEL_PRESSURE: Real = 101_325.0;

/// Standard sea-level temperature, K.
pub const ISA_SEA_LEVEL_TEMPERATURE: Real = 288.15;

/// Where a signal came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    /// Read directly from a sensor in the log.
    Measured,
    /// Produced by integrating a measured signal, so it drifts.
    DerivedByIntegration,
    /// Produced by a filter such as a complementary or Kalman filter.
    DerivedByFilter,
    /// Produced by an estimator that is not a simple filter.
    Estimated,
    /// Produced by a simulation rather than a flight.
    Simulated,
}

impl Provenance {
    /// Human-readable label shown next to the channel name.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Measured => "Measured",
            Self::DerivedByIntegration => "Derived by integration",
            Self::DerivedByFilter => "Derived by filter",
            Self::Estimated => "Estimated",
            Self::Simulated => "Simulated",
        }
    }

    /// True when the value was not measured directly.
    pub fn is_synthetic(&self) -> bool {
        !matches!(self, Self::Measured)
    }
}

/// Text shown when an integrated channel is selected.
pub const INTEGRATION_DRIFT_WARNING: &str =
    "This channel was produced by integrating measured data. Integration accumulates sensor bias and noise, so the absolute value drifts with time even when the flight is correct. Use the shape of the curve, not its absolute value.";

/// One scalar derived series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DerivedChannel {
    /// Display name.
    pub name: String,
    /// Role the series plays, used for unit and range handling.
    pub role: ChannelRole,
    /// SI unit of the values.
    pub unit: String,
    /// The computed samples.
    pub values: Vec<Real>,
    /// How the series was obtained.
    pub provenance: Provenance,
    /// Note explaining the computation, shown as a tooltip.
    pub notes: String,
}

impl DerivedChannel {
    /// Build a derived channel, taking the unit from the role unless overridden.
    pub fn new(
        name: impl Into<String>,
        role: ChannelRole,
        values: Vec<Real>,
        provenance: Provenance,
    ) -> Self {
        Self {
            name: name.into(),
            role,
            unit: role.unit().to_string(),
            values,
            provenance,
            notes: String::new(),
        }
    }

    /// The warning the UI must show, when there is one.
    pub fn warning(&self) -> Option<String> {
        match self.provenance {
            Provenance::DerivedByIntegration => Some(INTEGRATION_DRIFT_WARNING.to_string()),
            _ => None,
        }
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when the series holds no samples.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// One three-vector derived series.
#[derive(Debug, Clone, PartialEq)]
pub struct DerivedVectorChannel {
    /// Display name.
    pub name: String,
    /// Role the series plays.
    pub role: ChannelRole,
    /// SI unit of each component.
    pub unit: String,
    /// The computed vectors.
    pub values: Vec<Vec3>,
    /// How the series was obtained.
    pub provenance: Provenance,
    /// Note explaining the computation.
    pub notes: String,
}

impl DerivedVectorChannel {
    /// Build a derived vector channel.
    pub fn new(
        name: impl Into<String>,
        role: ChannelRole,
        values: Vec<Vec3>,
        provenance: Provenance,
    ) -> Self {
        Self {
            name: name.into(),
            role,
            unit: role.unit().to_string(),
            values,
            provenance,
            notes: String::new(),
        }
    }

    /// The warning the UI must show, when there is one.
    pub fn warning(&self) -> Option<String> {
        match self.provenance {
            Provenance::DerivedByIntegration => Some(INTEGRATION_DRIFT_WARNING.to_string()),
            _ => None,
        }
    }

    /// Number of samples.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when the series holds no samples.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Flatten the vectors into one column per component, in X, Y, Z order.
    pub fn components(&self) -> [Vec<Real>; 3] {
        let mut out = [
            Vec::with_capacity(self.values.len()),
            Vec::with_capacity(self.values.len()),
            Vec::with_capacity(self.values.len()),
        ];
        for value in &self.values {
            out[0].push(value.x);
            out[1].push(value.y);
            out[2].push(value.z);
        }
        out
    }
}

/// Pressure altitude from the International Standard Atmosphere troposphere.
///
/// Uses the standard lapse rate of [`ISA_LAPSE_RATE`] and standard gravity, so
/// the result is a pressure altitude rather than a GPS altitude. Non-finite or
/// non-positive pressures produce `NaN`, because no altitude is meaningful there.
pub fn derive_barometric_altitude(
    pressure_pa: &[Real],
    sea_level_pressure_pa: Real,
    sea_level_temperature_k: Real,
) -> Vec<Real> {
    let exponent = ISA_GAS_CONSTANT * ISA_LAPSE_RATE / hex_core::STANDARD_GRAVITY;
    let scale = if ISA_LAPSE_RATE > 0.0 {
        sea_level_temperature_k / ISA_LAPSE_RATE
    } else {
        0.0
    };
    pressure_pa
        .iter()
        .map(|pressure| {
            if !pressure.is_finite() || *pressure <= 0.0 || sea_level_pressure_pa <= 0.0 {
                return Real::NAN;
            }
            let ratio = pressure / sea_level_pressure_pa;
            scale * (1.0 - ratio.powf(exponent))
        })
        .collect()
}

/// Assemble three scalar columns into one vector series.
///
/// The output is as long as the shortest input, so a partially populated axis
/// triple cannot produce a vector with a missing component.
pub fn derive_vector_channels(axis_0: &[Real], axis_1: &[Real], axis_2: &[Real]) -> Vec<Vec3> {
    let count = axis_0.len().min(axis_1.len()).min(axis_2.len());
    (0..count)
        .map(|index| Vec3::new(axis_0[index], axis_1[index], axis_2[index]))
        .collect()
}

/// Integrate acceleration into velocity with the trapezoidal rule.
///
/// `accel_world` is the acceleration acting on the body, expressed in the world
/// frame. `gravity_world` is removed before integration, so the result is the
/// kinematic response to the non-gravitational part only. The returned series
/// carries [`Provenance::DerivedByIntegration`], because the result drifts.
pub fn integrate_velocity(
    times: &[Real],
    accel_world: &[Vec3],
    initial_velocity: Vec3,
    gravity_world: Vec3,
) -> DerivedVectorChannel {
    let count = times.len().min(accel_world.len());
    let mut values = Vec::with_capacity(count);
    let mut velocity = initial_velocity;
    if count > 0 {
        values.push(velocity);
    }
    for index in 1..count {
        let dt = times[index] - times[index - 1];
        if dt.is_finite() {
            let a0 = accel_world[index - 1] - gravity_world;
            let a1 = accel_world[index] - gravity_world;
            velocity += (a0 + a1) * (0.5 * dt);
        }
        values.push(velocity);
    }

    let mut channel = DerivedVectorChannel::new(
        "velocity",
        ChannelRole::GnssSpeed,
        values,
        Provenance::DerivedByIntegration,
    );
    channel.unit = "m/s".to_string();
    channel.notes =
        "Trapezoidal integration of the acceleration with the gravity vector removed.".to_string();
    channel
}

/// Velocity-only view of [`integrate_velocity`].
pub fn integrate_velocity_values(
    times: &[Real],
    accel_world: &[Vec3],
    initial_velocity: Vec3,
    gravity_world: Vec3,
) -> Vec<Vec3> {
    integrate_velocity(times, accel_world, initial_velocity, gravity_world).values
}

/// Integrate velocity into position with the trapezoidal rule.
///
/// The returned series carries [`Provenance::DerivedByIntegration`], because
/// integrating twice amplifies drift dramatically.
pub fn integrate_position(
    times: &[Real],
    velocity_world: &[Vec3],
    initial_position: Vec3,
) -> DerivedVectorChannel {
    let count = times.len().min(velocity_world.len());
    let mut values = Vec::with_capacity(count);
    let mut position = initial_position;
    if count > 0 {
        values.push(position);
    }
    for index in 1..count {
        let dt = times[index] - times[index - 1];
        if dt.is_finite() {
            position += (velocity_world[index - 1] + velocity_world[index]) * (0.5 * dt);
        }
        values.push(position);
    }

    let mut channel = DerivedVectorChannel::new(
        "position",
        ChannelRole::GnssAltitude,
        values,
        Provenance::DerivedByIntegration,
    );
    channel.unit = "m".to_string();
    channel.notes = "Trapezoidal integration of the velocity.".to_string();
    channel
}

/// Position-only view of [`integrate_position`].
pub fn integrate_position_values(
    times: &[Real],
    velocity_world: &[Vec3],
    initial_position: Vec3,
) -> Vec<Vec3> {
    integrate_position(times, velocity_world, initial_position).values
}

/// One classical Runge-Kutta step of the quaternion kinematics.
///
/// The core convention applies: scalar first, body-to-world attitude, and
/// `q_dot = 0.5 * q * [0, w_body]`.
fn rk4_step(q: Quaternion, omega_body: Vec3, dt: Real) -> Quaternion {
    let k1 = q.derivative_body_rate(omega_body);
    let k2 = (q + k1 * (0.5 * dt)).derivative_body_rate(omega_body);
    let k3 = (q + k2 * (0.5 * dt)).derivative_body_rate(omega_body);
    let k4 = (q + k3 * dt).derivative_body_rate(omega_body);
    (q + (k1 + k2 * 2.0 + k3 * 2.0 + k4) * (dt / 6.0)).normalized()
}

/// Propagate attitude from body rates with a fixed-step integrator.
///
/// `gyro_body` holds the body-frame rate for each interval starting at the
/// matching sample in `times`; `bias` is subtracted first. The rate in force at
/// sample `i` is applied over `times[i+1] - times[i]`, and one attitude is
/// returned per sample, so the result is as long as the shorter input. Every
/// step is normalised, and a non-finite rate is treated as zero so one corrupt
/// sample cannot destroy the whole attitude history.
pub fn propagate_gyro_attitude(
    times: &[Real],
    gyro_body: &[Vec3],
    initial: Quaternion,
    bias: Vec3,
) -> Vec<Quaternion> {
    let count = times.len().min(gyro_body.len());
    let mut out = Vec::with_capacity(count);
    if count == 0 {
        return out;
    }
    let mut q = initial.normalized();
    out.push(q);
    for index in 0..count - 1 {
        let dt = times[index + 1] - times[index];
        let rate_is_finite = gyro_body[index]
            .iter()
            .all(|component| component.is_finite());
        if dt.is_finite() && dt > 0.0 && rate_is_finite {
            q = rk4_step(q, gyro_body[index] - bias, dt);
        }
        out.push(q);
    }
    out
}

/// Average the first `stationary_count` rate samples as the gyro bias.
///
/// A stationary window is the simplest bias estimate and the one a user can
/// verify by eye. A zero or oversized count falls back to the whole series.
pub fn estimate_gyro_bias(gyro_body: &[Vec3], stationary_count: usize) -> Vec3 {
    if gyro_body.is_empty() {
        return Vec3::zeros();
    }
    let count = if stationary_count == 0 {
        gyro_body.len()
    } else {
        stationary_count.min(gyro_body.len())
    };
    let mut sum = Vec3::zeros();
    let mut used = 0usize;
    for value in gyro_body.iter().take(count) {
        if value.iter().all(|component| component.is_finite()) {
            sum += value;
            used += 1;
        }
    }
    if used == 0 {
        Vec3::zeros()
    } else {
        sum / used as Real
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::FRAC_PI_2;

    fn approx(a: Real, b: Real, tolerance: Real) -> bool {
        (a - b).abs() <= tolerance
    }

    fn ramp(count: usize, dt: Real) -> Vec<Real> {
        (0..count).map(|i| i as Real * dt).collect()
    }

    #[test]
    fn barometric_altitude_is_zero_at_sea_level_pressure() {
        let altitude = derive_barometric_altitude(
            &[ISA_SEA_LEVEL_PRESSURE],
            ISA_SEA_LEVEL_PRESSURE,
            ISA_SEA_LEVEL_TEMPERATURE,
        );
        assert!(altitude[0].abs() < 1e-9);
    }

    #[test]
    fn barometric_altitude_matches_a_hand_computed_value() {
        // 900 hPa in the ISA troposphere, worked by hand from
        // h = (T0 / L) * (1 - (p / p0) ^ (R * L / g)).
        let altitude = derive_barometric_altitude(&[90_000.0], 101_325.0, 288.15);
        assert!(
            approx(altitude[0], 988.5, 0.5),
            "expected about 988.5 m, got {}",
            altitude[0]
        );
    }

    #[test]
    fn barometric_altitude_matches_the_isa_table_at_500_hpa() {
        let altitude = derive_barometric_altitude(&[50_000.0], 101_325.0, 288.15);
        assert!(
            approx(altitude[0], 5574.0, 10.0),
            "expected about 5574 m, got {}",
            altitude[0]
        );
    }

    #[test]
    fn barometric_altitude_rejects_invalid_pressure() {
        let values = derive_barometric_altitude(
            &[0.0, -1.0, Real::NAN],
            ISA_SEA_LEVEL_PRESSURE,
            ISA_SEA_LEVEL_TEMPERATURE,
        );
        assert!(values.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn vector_channels_zip_to_the_shortest_axis() {
        let vectors = derive_vector_channels(&[1.0, 2.0, 3.0], &[4.0, 5.0], &[6.0, 7.0, 8.0]);
        assert_eq!(vectors.len(), 2);
        assert!((vectors[1] - Vec3::new(2.0, 5.0, 7.0)).norm() < 1e-15);
        assert!(derive_vector_channels(&[], &[], &[]).is_empty());
    }

    #[test]
    fn trapezoidal_velocity_integrates_constant_acceleration_exactly() {
        let dt = 0.01;
        let count = 101;
        let times = ramp(count, dt);
        let accel = vec![Vec3::new(2.0, -1.0, 0.5); count];
        let channel = integrate_velocity(&times, &accel, Vec3::zeros(), Vec3::zeros());
        assert_eq!(channel.len(), count);
        assert_eq!(channel.provenance, Provenance::DerivedByIntegration);
        let expected = Vec3::new(2.0, -1.0, 0.5) * (100.0 * dt);
        assert!((channel.values[100] - expected).norm() < 1e-12);
        // Halfway through, the analytic answer is half the final velocity.
        assert!((channel.values[50] - expected * 0.5).norm() < 1e-12);
    }

    #[test]
    fn velocity_integration_removes_the_gravity_vector() {
        let times = ramp(2, 1.0);
        let accel = vec![Vec3::zeros(); 2];
        let gravity = Vec3::new(0.0, 0.0, -9.80665);
        let channel = integrate_velocity(&times, &accel, Vec3::zeros(), gravity);
        // Removing a downward gravity vector leaves an upward net acceleration.
        assert!(approx(channel.values[1].z, 9.80665, 1e-9));
        assert!(approx(channel.values[1].x, 0.0, 1e-12));

        let zero_gravity = integrate_velocity(&times, &accel, Vec3::zeros(), Vec3::zeros());
        assert!(zero_gravity.values[1].norm() < 1e-15);
    }

    #[test]
    fn trapezoidal_position_integrates_constant_velocity_exactly() {
        let dt = 0.02;
        let count = 51;
        let times = ramp(count, dt);
        let velocity = vec![Vec3::new(1.0, 2.0, -3.0); count];
        let start = Vec3::new(10.0, -5.0, 1.0);
        let channel = integrate_position(&times, &velocity, start);
        assert_eq!(channel.len(), count);
        let expected = start + Vec3::new(1.0, 2.0, -3.0) * (50.0 * dt);
        assert!((channel.values[50] - expected).norm() < 1e-12);
        assert_eq!(channel.unit, "m");
    }

    #[test]
    fn integration_handles_empty_and_mismatched_inputs() {
        let empty = integrate_velocity(&[], &[], Vec3::zeros(), Vec3::zeros());
        assert!(empty.is_empty());
        let times = ramp(5, 0.1);
        let accel = vec![Vec3::new(1.0, 0.0, 0.0); 2];
        let channel = integrate_velocity(&times, &accel, Vec3::zeros(), Vec3::zeros());
        assert_eq!(channel.len(), 2);
    }

    #[test]
    fn integration_provenance_carries_the_drift_warning() {
        let channel = integrate_position(&ramp(3, 0.1), &[Vec3::zeros(); 3], Vec3::zeros());
        let warning = channel.warning().expect("integration warning");
        assert!(warning.contains("integrating measured data"));
        assert_eq!(channel.provenance.label(), "Derived by integration");
        assert!(channel.provenance.is_synthetic());

        let measured = DerivedChannel::new(
            "altitude",
            ChannelRole::GnssAltitude,
            vec![1.0, 2.0],
            Provenance::Measured,
        );
        assert!(measured.warning().is_none());
        assert!(!measured.provenance.is_synthetic());
        assert_eq!(measured.unit, "m");
        assert_eq!(measured.len(), 2);
    }

    #[test]
    fn gyro_attitude_constant_body_rate_matches_the_analytic_quaternion() {
        let dt = 0.001;
        let count = 1001;
        let times = ramp(count, dt);
        let rate = Vec3::new(1.3, 0.0, 0.0);
        let rates = vec![rate; count];
        let initial = Quaternion::from_axis_angle(Vec3::y(), 0.4);
        let attitude = propagate_gyro_attitude(&times, &rates, initial, Vec3::zeros());
        assert_eq!(attitude.len(), count);
        let expected = initial * Quaternion::from_rotation_vector(rate * (1000.0 * dt));
        assert!(
            attitude[count - 1].angular_distance(&expected) < 1e-9,
            "distance {}",
            attitude[count - 1].angular_distance(&expected)
        );
        // Every quaternion stays normalised.
        assert!(attitude.iter().all(|q| approx(q.norm(), 1.0, 1e-12)));
    }

    #[test]
    fn gyro_attitude_is_unchanged_by_a_zero_rate() {
        let times = ramp(5, 0.01);
        let rates = vec![Vec3::zeros(); 5];
        let initial = Quaternion::from_axis_angle(Vec3::z(), FRAC_PI_2);
        let attitude = propagate_gyro_attitude(&times, &rates, initial, Vec3::zeros());
        assert!(attitude
            .iter()
            .all(|q| q.angular_distance(&initial) < 1e-12));
    }

    #[test]
    fn subtracting_the_bias_removes_a_constant_offset() {
        let dt = 0.001;
        let count = 501;
        let times = ramp(count, dt);
        let bias = Vec3::new(0.02, -0.03, 0.01);
        let rates = vec![bias; count];
        let initial = Quaternion::identity();
        let attitude = propagate_gyro_attitude(&times, &rates, initial, bias);
        assert!(attitude
            .iter()
            .all(|q| q.angular_distance(&initial) < 1e-12));
    }

    #[test]
    fn non_finite_rates_do_not_destroy_the_attitude() {
        let times = ramp(3, 0.01);
        let mut rates = vec![Vec3::zeros(); 3];
        rates[1] = Vec3::new(Real::NAN, 0.0, 0.0);
        let initial = Quaternion::identity();
        let attitude = propagate_gyro_attitude(&times, &rates, initial, Vec3::zeros());
        assert_eq!(attitude.len(), 3);
        assert!(attitude.iter().all(|q| q.is_finite()));
    }

    #[test]
    fn gyro_bias_averages_the_stationary_window() {
        let mut rates = vec![Vec3::new(0.01, -0.02, 0.03); 10];
        rates.extend(vec![Vec3::new(5.0, 5.0, 5.0); 10]);
        let bias = estimate_gyro_bias(&rates, 10);
        assert!((bias - Vec3::new(0.01, -0.02, 0.03)).norm() < 1e-12);

        // Asking for more samples than exist uses the whole series.
        let bias = estimate_gyro_bias(&rates, 100);
        assert!((bias.x - 2.505).abs() < 1e-12);

        assert!(estimate_gyro_bias(&[], 5).norm() < 1e-15);
    }

    #[test]
    fn vector_channel_components_split_the_axes() {
        let channel = DerivedVectorChannel::new(
            "accel",
            ChannelRole::AccelX,
            vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)],
            Provenance::Measured,
        );
        let components = channel.components();
        assert_eq!(components[0], vec![1.0, 4.0]);
        assert_eq!(components[1], vec![2.0, 5.0]);
        assert_eq!(components[2], vec![3.0, 6.0]);
        assert_eq!(channel.unit, "m/s^2");
        assert!(channel.warning().is_none());
    }
}
