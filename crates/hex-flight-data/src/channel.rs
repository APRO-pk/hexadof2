//! The normalised channel model the rest of the application reads.
//!
//! A [`Channel`] is the only representation of a measured series that downstream
//! code sees. It carries raw samples together with everything needed to turn
//! them into SI values in body axes: a [`UnitScale`], an axis sign, the declared
//! sensor axis convention, and the source frame. Import produces channels;
//! analysis, estimation, and the UI read them. Nothing downstream re-parses a
//! file, so a unit or axis mistake is fixed in exactly one place.

use serde::{Deserialize, Serialize};

use hex_core::units::UnitConfidence;
use hex_core::{Frame, QuantityKind, Real, SensorAxes, UnitScale};

/// Stable identifier for a channel inside one session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ChannelId(pub String);

impl ChannelId {
    /// Wrap any name-like value as an identifier.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Borrow the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when the identifier carries no characters.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for ChannelId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for ChannelId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl std::fmt::Display for ChannelId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The physical role a channel plays in a flight log.
///
/// The discriminants are the on-disk role codes used by the HexaDOF binary log,
/// so reordering the variants would break existing files. Append new roles at
/// the end instead.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Default,
)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum ChannelRole {
    /// The sample time axis.
    Time = 0,
    /// Body X acceleration.
    AccelX = 1,
    /// Body Y acceleration.
    AccelY = 2,
    /// Body Z acceleration.
    AccelZ = 3,
    /// Body X angular rate.
    GyroX = 4,
    /// Body Y angular rate.
    GyroY = 5,
    /// Body Z angular rate.
    GyroZ = 6,
    /// Body X magnetic field.
    MagX = 7,
    /// Body Y magnetic field.
    MagY = 8,
    /// Body Z magnetic field.
    MagZ = 9,
    /// Static pressure.
    BaroPressure = 10,
    /// Barometer die temperature.
    BaroTemperature = 11,
    /// Geodetic latitude.
    GnssLatitude = 12,
    /// Geodetic longitude.
    GnssLongitude = 13,
    /// Geodetic altitude above the ellipsoid.
    GnssAltitude = 14,
    /// Ground speed.
    GnssSpeed = 15,
    /// Attitude quaternion scalar part.
    QuaternionW = 16,
    /// Attitude quaternion X part.
    QuaternionX = 17,
    /// Attitude quaternion Y part.
    QuaternionY = 18,
    /// Attitude quaternion Z part.
    QuaternionZ = 19,
    /// Motor or ESC throttle command.
    MotorThrottle = 20,
    /// Control surface deflection.
    ControlSurface = 21,
    /// Bus or pack voltage.
    Voltage = 22,
    /// Bus or pack current.
    Current = 23,
    /// Generic temperature.
    Temperature = 24,
    /// A channel with no known physical meaning that is still carried through.
    #[default]
    Raw = 25,
    /// A channel the user explicitly excluded from analysis.
    Ignored = 26,
}

impl ChannelRole {
    /// The physical quantity this role carries, used for unit conversion.
    pub fn quantity_kind(&self) -> QuantityKind {
        match self {
            Self::Time => QuantityKind::Time,
            Self::AccelX | Self::AccelY | Self::AccelZ => QuantityKind::Acceleration,
            Self::GyroX | Self::GyroY | Self::GyroZ => QuantityKind::AngularRate,
            Self::MagX | Self::MagY | Self::MagZ => QuantityKind::MagneticField,
            Self::BaroPressure => QuantityKind::Pressure,
            Self::BaroTemperature | Self::Temperature => QuantityKind::Temperature,
            Self::GnssLatitude | Self::GnssLongitude => QuantityKind::Angle,
            Self::GnssAltitude => QuantityKind::Position,
            Self::GnssSpeed => QuantityKind::Velocity,
            Self::QuaternionW | Self::QuaternionX | Self::QuaternionY | Self::QuaternionZ => {
                QuantityKind::Attitude
            }
            Self::MotorThrottle => QuantityKind::Ratio,
            Self::ControlSurface => QuantityKind::Angle,
            Self::Voltage => QuantityKind::Voltage,
            Self::Current => QuantityKind::Current,
            Self::Raw | Self::Ignored => QuantityKind::Raw,
        }
    }

    /// Human-readable name shown in the mapping table.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::AccelX => "Acceleration X",
            Self::AccelY => "Acceleration Y",
            Self::AccelZ => "Acceleration Z",
            Self::GyroX => "Angular rate X",
            Self::GyroY => "Angular rate Y",
            Self::GyroZ => "Angular rate Z",
            Self::MagX => "Magnetic field X",
            Self::MagY => "Magnetic field Y",
            Self::MagZ => "Magnetic field Z",
            Self::BaroPressure => "Barometric pressure",
            Self::BaroTemperature => "Barometer temperature",
            Self::GnssLatitude => "Latitude",
            Self::GnssLongitude => "Longitude",
            Self::GnssAltitude => "Altitude",
            Self::GnssSpeed => "Ground speed",
            Self::QuaternionW => "Attitude scalar",
            Self::QuaternionX => "Attitude X",
            Self::QuaternionY => "Attitude Y",
            Self::QuaternionZ => "Attitude Z",
            Self::MotorThrottle => "Motor throttle",
            Self::ControlSurface => "Control surface",
            Self::Voltage => "Voltage",
            Self::Current => "Current",
            Self::Temperature => "Temperature",
            Self::Raw => "Raw",
            Self::Ignored => "Ignored",
        }
    }

    /// The SI unit a corrected value of this role is expressed in.
    pub fn unit(&self) -> &'static str {
        match self {
            Self::Time => "s",
            Self::AccelX | Self::AccelY | Self::AccelZ => "m/s^2",
            Self::GyroX | Self::GyroY | Self::GyroZ => "rad/s",
            Self::MagX | Self::MagY | Self::MagZ => "T",
            Self::BaroPressure => "Pa",
            Self::BaroTemperature | Self::Temperature => "K",
            Self::GnssLatitude | Self::GnssLongitude => "deg",
            Self::GnssAltitude => "m",
            Self::GnssSpeed => "m/s",
            Self::QuaternionW
            | Self::QuaternionX
            | Self::QuaternionY
            | Self::QuaternionZ
            | Self::MotorThrottle => "1",
            Self::ControlSurface => "rad",
            Self::Voltage => "V",
            Self::Current => "A",
            Self::Raw => "1",
            Self::Ignored => "",
        }
    }

    /// Stable dotted machine code, for example `accel.x`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::AccelX => "accel.x",
            Self::AccelY => "accel.y",
            Self::AccelZ => "accel.z",
            Self::GyroX => "gyro.x",
            Self::GyroY => "gyro.y",
            Self::GyroZ => "gyro.z",
            Self::MagX => "mag.x",
            Self::MagY => "mag.y",
            Self::MagZ => "mag.z",
            Self::BaroPressure => "baro.pressure",
            Self::BaroTemperature => "baro.temperature",
            Self::GnssLatitude => "gnss.latitude",
            Self::GnssLongitude => "gnss.longitude",
            Self::GnssAltitude => "gnss.altitude",
            Self::GnssSpeed => "gnss.speed",
            Self::QuaternionW => "quaternion.w",
            Self::QuaternionX => "quaternion.x",
            Self::QuaternionY => "quaternion.y",
            Self::QuaternionZ => "quaternion.z",
            Self::MotorThrottle => "motor.throttle",
            Self::ControlSurface => "control.surface",
            Self::Voltage => "power.voltage",
            Self::Current => "power.current",
            Self::Temperature => "temperature",
            Self::Raw => "raw",
            Self::Ignored => "ignored",
        }
    }

    /// Component index for the three-axis roles, in X, Y, Z order.
    ///
    /// Latitude, longitude, and altitude are treated as a local position triple
    /// so a GNSS fix can be moved through the same axis mapper as a sensor.
    pub fn axis_index(&self) -> Option<usize> {
        match self {
            Self::AccelX | Self::GyroX | Self::MagX | Self::GnssLatitude => Some(0),
            Self::AccelY | Self::GyroY | Self::MagY | Self::GnssLongitude => Some(1),
            Self::AccelZ | Self::GyroZ | Self::MagZ | Self::GnssAltitude => Some(2),
            _ => None,
        }
    }

    /// The sensor or subsystem family this role belongs to.
    pub fn family(&self) -> Option<ChannelFamily> {
        match self {
            Self::AccelX | Self::AccelY | Self::AccelZ => Some(ChannelFamily::Accelerometer),
            Self::GyroX | Self::GyroY | Self::GyroZ => Some(ChannelFamily::Gyroscope),
            Self::MagX | Self::MagY | Self::MagZ => Some(ChannelFamily::Magnetometer),
            Self::BaroPressure | Self::BaroTemperature => Some(ChannelFamily::Barometer),
            Self::GnssLatitude | Self::GnssLongitude | Self::GnssAltitude | Self::GnssSpeed => {
                Some(ChannelFamily::Gnss)
            }
            Self::QuaternionW | Self::QuaternionX | Self::QuaternionY | Self::QuaternionZ => {
                Some(ChannelFamily::Orientation)
            }
            Self::MotorThrottle => Some(ChannelFamily::Motor),
            Self::Voltage | Self::Current => Some(ChannelFamily::Power),
            _ => None,
        }
    }

    /// The three roles of a family in X, Y, Z order, when the family has axes.
    pub fn axis_triple(family: ChannelFamily) -> Option<[ChannelRole; 3]> {
        match family {
            ChannelFamily::Accelerometer => Some([Self::AccelX, Self::AccelY, Self::AccelZ]),
            ChannelFamily::Gyroscope => Some([Self::GyroX, Self::GyroY, Self::GyroZ]),
            ChannelFamily::Magnetometer => Some([Self::MagX, Self::MagY, Self::MagZ]),
            ChannelFamily::Gnss => {
                Some([Self::GnssLatitude, Self::GnssLongitude, Self::GnssAltitude])
            }
            _ => None,
        }
    }

    /// Numeric role code written into a binary log channel descriptor.
    pub fn role_code(&self) -> u8 {
        *self as u8
    }

    /// Recover a role from a binary log role code. Unknown codes become
    /// [`ChannelRole::Raw`] so a newer log still imports readably.
    pub fn from_role_code(code: u8) -> Self {
        match code {
            0 => Self::Time,
            1 => Self::AccelX,
            2 => Self::AccelY,
            3 => Self::AccelZ,
            4 => Self::GyroX,
            5 => Self::GyroY,
            6 => Self::GyroZ,
            7 => Self::MagX,
            8 => Self::MagY,
            9 => Self::MagZ,
            10 => Self::BaroPressure,
            11 => Self::BaroTemperature,
            12 => Self::GnssLatitude,
            13 => Self::GnssLongitude,
            14 => Self::GnssAltitude,
            15 => Self::GnssSpeed,
            16 => Self::QuaternionW,
            17 => Self::QuaternionX,
            18 => Self::QuaternionY,
            19 => Self::QuaternionZ,
            20 => Self::MotorThrottle,
            21 => Self::ControlSurface,
            22 => Self::Voltage,
            23 => Self::Current,
            24 => Self::Temperature,
            26 => Self::Ignored,
            _ => Self::Raw,
        }
    }

    /// A plausible physical range for this role, used to flag sensor clipping.
    ///
    /// Ranges are deliberately generous: they exist to catch stuck-at-rail
    /// sensors, not to reject aggressive flight data.
    pub fn plausible_range(&self) -> Option<(Real, Real)> {
        match self {
            Self::AccelX | Self::AccelY | Self::AccelZ => Some((-160.0, 160.0)),
            Self::GyroX | Self::GyroY | Self::GyroZ => Some((-35.0, 35.0)),
            Self::MagX | Self::MagY | Self::MagZ => Some((-1.0e-3, 1.0e-3)),
            Self::BaroPressure => Some((0.0, 120_000.0)),
            Self::BaroTemperature | Self::Temperature => Some((150.0, 400.0)),
            Self::Voltage => Some((0.0, 60.0)),
            Self::Current => Some((-200.0, 200.0)),
            Self::MotorThrottle => Some((0.0, 1.0)),
            Self::GnssSpeed => Some((0.0, 400.0)),
            Self::GnssAltitude => Some((-500.0, 30_000.0)),
            Self::ControlSurface => Some((-1.6, 1.6)),
            _ => None,
        }
    }
}

/// The subsystem a channel belongs to, used to group the channel list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelFamily {
    /// Three-axis accelerometer.
    Accelerometer,
    /// Three-axis rate gyroscope.
    Gyroscope,
    /// Three-axis magnetometer.
    Magnetometer,
    /// Static pressure and its temperature.
    Barometer,
    /// Satellite navigation fix.
    Gnss,
    /// Attitude estimate.
    Orientation,
    /// Motor, ESC, or propulsion command.
    Motor,
    /// Electrical power.
    Power,
    /// Any family that has no dedicated grouping.
    Other,
}

impl ChannelFamily {
    /// Human-readable name shown as a group heading.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Accelerometer => "Accelerometer",
            Self::Gyroscope => "Gyroscope",
            Self::Magnetometer => "Magnetometer",
            Self::Barometer => "Barometer",
            Self::Gnss => "GNSS",
            Self::Orientation => "Orientation",
            Self::Motor => "Motor",
            Self::Power => "Power",
            Self::Other => "Other",
        }
    }
}

/// One normalised measurement series.
///
/// `values` holds the samples exactly as the source produced them. Call
/// [`Channel::corrected_values`] to get SI values with the sign applied; the
/// separation keeps the raw log reproducible while the rest of the app works in
/// SI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Channel {
    /// Name as it appears in the source file.
    pub name: String,
    /// Physical role assigned during mapping.
    pub role: ChannelRole,
    /// Unit label declared by the source, kept for display.
    pub unit: String,
    /// Conversion from the declared unit to SI.
    pub scale: UnitScale,
    /// Axis sign, always exactly `1.0` or `-1.0`.
    pub sign: Real,
    /// Sensor axis convention declared for this channel.
    pub frame: SensorAxes,
    /// World and body frame the values are referred to, when the source said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_frame: Option<Frame>,
    /// Samples exactly as read from the source.
    pub values: Vec<Real>,
    /// How the unit was established.
    pub unit_confidence: UnitConfidence,
    /// Free-text note carried from the import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Channel {
    /// Build a channel with the role's default unit and an identity scale.
    ///
    /// `sign` is snapped to exactly `1.0` or `-1.0`, because a partial sign is
    /// always a mapping bug rather than an intent.
    pub fn new(name: impl Into<String>, role: ChannelRole, values: Vec<Real>) -> Self {
        Self {
            name: name.into(),
            role,
            unit: role.unit().to_string(),
            scale: UnitScale::IDENTITY,
            sign: snap_sign(1.0),
            frame: SensorAxes::Xyz,
            source_frame: None,
            values,
            unit_confidence: UnitConfidence::Assumed,
            notes: None,
        }
    }

    /// Numeric series with the unit scale and the axis sign applied.
    pub fn corrected_values(&self) -> Vec<Real> {
        self.values
            .iter()
            .map(|raw| self.scale.apply(*raw) * self.sign)
            .collect()
    }

    /// Number of samples, including ones that are not finite.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True when the channel holds no samples.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Statistics over the corrected values, using the role's plausible range to
    /// decide whether the series is clipped.
    pub fn statistics(&self) -> ChannelStatistics {
        self.statistics_within(self.role.plausible_range())
    }

    /// Statistics over the corrected values with an explicit plausible range.
    pub fn statistics_within(&self, plausible_range: Option<(Real, Real)>) -> ChannelStatistics {
        let count = self.values.len();
        let mut nan_count = 0usize;
        let mut infinite_count = 0usize;
        let mut finite_count = 0usize;
        let mut min = Real::INFINITY;
        let mut max = Real::NEG_INFINITY;
        let mut sum = 0.0;

        for value in self.corrected_values() {
            if value.is_nan() {
                nan_count += 1;
            } else if value.is_infinite() {
                infinite_count += 1;
            } else {
                finite_count += 1;
                sum += value;
                if value < min {
                    min = value;
                }
                if value > max {
                    max = value;
                }
            }
        }

        let (min, max) = if finite_count == 0 {
            (0.0, 0.0)
        } else {
            (min, max)
        };
        let mean = if finite_count == 0 {
            0.0
        } else {
            sum / finite_count as Real
        };

        let mut squared = 0.0;
        for value in self.corrected_values() {
            if value.is_finite() {
                let d = value - mean;
                squared += d * d;
            }
        }
        let standard_deviation = if finite_count == 0 {
            0.0
        } else {
            (squared / finite_count as Real).sqrt()
        };

        let (saturated_low, saturated_high) = match plausible_range {
            Some((low, high)) if finite_count > 0 => (min <= low, max >= high),
            _ => (false, false),
        };

        ChannelStatistics {
            count,
            finite_count,
            nan_count,
            infinite_count,
            min,
            max,
            mean,
            standard_deviation,
            constant: finite_count > 0 && min == max,
            saturated_low,
            saturated_high,
        }
    }

    /// True when the series can carry a computation.
    ///
    /// A channel needs at least two finite samples that are not all identical;
    /// an empty, all-NaN, single-sample, or stuck series would silently poison
    /// any estimate built on it.
    pub fn is_usable(&self) -> bool {
        let stats = self.statistics();
        stats.finite_count >= 2 && !stats.constant
    }
}

/// Snap an arbitrary sign to exactly `+1.0` or `-1.0`.
pub fn snap_sign(sign: Real) -> Real {
    if sign < 0.0 {
        -1.0
    } else {
        1.0
    }
}

/// Distribution summary for one channel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChannelStatistics {
    /// Total sample count, including non-finite values.
    pub count: usize,
    /// Samples that are neither NaN nor infinite.
    pub finite_count: usize,
    /// Samples that are NaN.
    pub nan_count: usize,
    /// Samples that are infinite.
    pub infinite_count: usize,
    /// Smallest finite value, or zero when there is none.
    pub min: Real,
    /// Largest finite value, or zero when there is none.
    pub max: Real,
    /// Arithmetic mean of the finite values.
    pub mean: Real,
    /// Population standard deviation of the finite values.
    pub standard_deviation: Real,
    /// True when every finite value is identical.
    pub constant: bool,
    /// True when the smallest value reached the bottom of the plausible range.
    pub saturated_low: bool,
    /// True when the largest value reached the top of the plausible range.
    pub saturated_high: bool,
}

impl ChannelStatistics {
    /// Fraction of samples that are finite, in `[0, 1]`.
    pub fn finite_fraction(&self) -> Real {
        if self.count == 0 {
            0.0
        } else {
            self.finite_count as Real / self.count as Real
        }
    }

    /// Peak-to-peak spread of the finite values.
    pub fn range(&self) -> Real {
        self.max - self.min
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(role: ChannelRole, values: Vec<Real>) -> Channel {
        Channel::new("test", role, values)
    }

    #[test]
    fn role_codes_round_trip() {
        for role in [
            ChannelRole::Time,
            ChannelRole::AccelX,
            ChannelRole::BaroPressure,
            ChannelRole::QuaternionZ,
            ChannelRole::Ignored,
            ChannelRole::Raw,
        ] {
            assert_eq!(ChannelRole::from_role_code(role.role_code()), role);
        }
        assert_eq!(ChannelRole::from_role_code(200), ChannelRole::Raw);
    }

    #[test]
    fn role_codes_are_stable_discriminants() {
        assert_eq!(ChannelRole::Time.role_code(), 0);
        assert_eq!(ChannelRole::AccelX.role_code(), 1);
        assert_eq!(ChannelRole::GyroZ.role_code(), 6);
        assert_eq!(ChannelRole::Ignored.role_code(), 26);
    }

    #[test]
    fn axis_index_covers_the_three_axis_families() {
        assert_eq!(ChannelRole::AccelY.axis_index(), Some(1));
        assert_eq!(ChannelRole::GyroZ.axis_index(), Some(2));
        assert_eq!(ChannelRole::MagX.axis_index(), Some(0));
        assert_eq!(ChannelRole::GnssAltitude.axis_index(), Some(2));
        assert_eq!(ChannelRole::Voltage.axis_index(), None);
        assert_eq!(ChannelRole::Time.axis_index(), None);
    }

    #[test]
    fn family_grouping_matches_the_role() {
        assert_eq!(
            ChannelRole::AccelZ.family(),
            Some(ChannelFamily::Accelerometer)
        );
        assert_eq!(ChannelRole::GnssSpeed.family(), Some(ChannelFamily::Gnss));
        assert_eq!(ChannelRole::Time.family(), None);
        assert_eq!(
            ChannelRole::axis_triple(ChannelFamily::Gyroscope),
            Some([ChannelRole::GyroX, ChannelRole::GyroY, ChannelRole::GyroZ])
        );
        assert_eq!(ChannelRole::axis_triple(ChannelFamily::Power), None);
    }

    #[test]
    fn quantity_kinds_follow_the_role() {
        assert_eq!(
            ChannelRole::AccelX.quantity_kind(),
            QuantityKind::Acceleration
        );
        assert_eq!(
            ChannelRole::GyroY.quantity_kind(),
            QuantityKind::AngularRate
        );
        assert_eq!(
            ChannelRole::BaroPressure.quantity_kind(),
            QuantityKind::Pressure
        );
        assert_eq!(ChannelRole::Raw.quantity_kind(), QuantityKind::Raw);
    }

    #[test]
    fn labels_units_and_codes_are_never_empty_for_known_roles() {
        for role in [
            ChannelRole::Time,
            ChannelRole::AccelX,
            ChannelRole::BaroPressure,
            ChannelRole::GnssSpeed,
            ChannelRole::Temperature,
            ChannelRole::Raw,
        ] {
            assert!(!role.label().is_empty());
            assert!(!role.unit().is_empty());
            assert!(!role.code().is_empty());
        }
        assert_eq!(ChannelRole::AccelX.unit(), "m/s^2");
        assert_eq!(ChannelRole::AccelX.code(), "accel.x");
    }

    #[test]
    fn corrected_values_apply_scale_then_sign() {
        let mut c = channel(ChannelRole::AccelX, vec![1.0, 2.0, 3.0]);
        c.scale = UnitScale::new(9.80665, 0.0);
        c.sign = -1.0;
        let corrected = c.corrected_values();
        assert!((corrected[0] + 9.80665).abs() < 1e-12);
        assert!((corrected[2] + 3.0 * 9.80665).abs() < 1e-12);
    }

    #[test]
    fn sign_is_snapped_to_unit_magnitude() {
        assert_eq!(snap_sign(0.2), 1.0);
        assert_eq!(snap_sign(-0.2), -1.0);
        assert_eq!(snap_sign(1.0), 1.0);
        assert_eq!(snap_sign(-1.0), -1.0);
    }

    #[test]
    fn statistics_describe_a_clean_series() {
        let c = channel(ChannelRole::Raw, vec![1.0, 2.0, 3.0, 4.0]);
        let s = c.statistics();
        assert_eq!(s.count, 4);
        assert_eq!(s.finite_count, 4);
        assert_eq!(s.nan_count, 0);
        assert_eq!(s.min, 1.0);
        assert_eq!(s.max, 4.0);
        assert!((s.mean - 2.5).abs() < 1e-12);
        assert!((s.standard_deviation - 1.118_033_988_749_895).abs() < 1e-12);
        assert!(!s.constant);
        assert!((s.finite_fraction() - 1.0).abs() < 1e-12);
        assert!((s.range() - 3.0).abs() < 1e-12);
    }

    #[test]
    fn statistics_count_non_finite_values_without_failing() {
        let c = channel(
            ChannelRole::Raw,
            vec![1.0, Real::NAN, Real::INFINITY, 3.0, Real::NAN],
        );
        let s = c.statistics();
        assert_eq!(s.count, 5);
        assert_eq!(s.finite_count, 2);
        assert_eq!(s.nan_count, 2);
        assert_eq!(s.infinite_count, 1);
        assert_eq!(s.min, 1.0);
        assert_eq!(s.max, 3.0);
    }

    #[test]
    fn statistics_detect_constant_and_clipped_series() {
        let c = channel(ChannelRole::AccelX, vec![-160.0, -160.0, -160.0]);
        let s = c.statistics();
        assert!(s.constant);
        assert!(s.saturated_low);
        assert!(!s.saturated_high);

        let c = channel(ChannelRole::AccelY, vec![160.0, 10.0]);
        let s = c.statistics();
        assert!(!s.constant);
        assert!(s.saturated_high);
        assert!(!s.saturated_low);
    }

    #[test]
    fn statistics_without_range_never_report_clipping() {
        let c = channel(ChannelRole::Raw, vec![1.0e9, -1.0e9]);
        let s = c.statistics_within(None);
        assert!(!s.saturated_low);
        assert!(!s.saturated_high);
    }

    #[test]
    fn empty_channel_statistics_are_neutral() {
        let c = channel(ChannelRole::Raw, vec![]);
        let s = c.statistics();
        assert_eq!(s.count, 0);
        assert_eq!(s.finite_count, 0);
        assert_eq!(s.mean, 0.0);
        assert_eq!(s.standard_deviation, 0.0);
        assert!(!s.constant);
        assert_eq!(s.finite_fraction(), 0.0);
        assert!(!c.is_usable());
        assert!(c.is_empty());
    }

    #[test]
    fn usability_rejects_stuck_and_too_short_series() {
        assert!(channel(ChannelRole::Raw, vec![1.0, 2.0]).is_usable());
        assert!(!channel(ChannelRole::Raw, vec![1.0, 1.0, 1.0]).is_usable());
        assert!(!channel(ChannelRole::Raw, vec![1.0]).is_usable());
        assert!(!channel(ChannelRole::Raw, vec![Real::NAN, Real::NAN]).is_usable());
    }

    #[test]
    fn channel_id_is_usable_as_a_map_key() {
        let mut map = std::collections::HashMap::new();
        map.insert(ChannelId::from("time"), 1);
        assert_eq!(map.get(&ChannelId::new("time")), Some(&1));
        assert_eq!(ChannelId::from("accel_x").to_string(), "accel_x");
        assert!(ChannelId::new("").is_empty());
    }
}
