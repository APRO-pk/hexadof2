//! Channel mapping: packet fields to physical channels, with units, sign, axes,
//! and sensor-to-body rotation.
//!
//! A mapping is the contract between a device's wire format and the rest of the
//! application. The UI must show every column of it, because a wrong sign or a
//! missing axis swap produces plausible-looking data that is simply wrong.

use hex_core::{AxisMapping, Frame, QuantityKind, Real, SensorAxes, UnitScale, Vec3};
use serde::{Deserialize, Serialize};

/// What a packet field is being used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelDestination {
    /// The time column.
    Time,
    /// Accelerometer X.
    AccelX,
    AccelY,
    AccelZ,
    /// Gyroscope X.
    GyroX,
    GyroY,
    GyroZ,
    /// Magnetometer X.
    MagX,
    MagY,
    MagZ,
    /// Barometric pressure.
    Barometer,
    /// Air or board temperature.
    Temperature,
    /// GNSS latitude in degrees.
    GnssLatitude,
    GnssLongitude,
    /// GNSS altitude in metres.
    GnssAltitude,
    /// GNSS ground speed in metres per second.
    GnssSpeed,
    /// An orientation quaternion component supplied by the device.
    QuaternionW,
    QuaternionX,
    QuaternionY,
    QuaternionZ,
    /// Motor throttle or thrust command.
    Motor,
    /// A control surface position.
    ControlSurface,
    /// Battery or supply voltage.
    Voltage,
    /// Current draw.
    Current,
    /// A packet sequence counter.
    Sequence,
    /// Received but not used.
    Unused,
}

impl ChannelDestination {
    /// Every destination, in the order the mapping table should present them.
    pub fn all() -> &'static [ChannelDestination] {
        use ChannelDestination::*;
        &[
            Time,
            Sequence,
            AccelX,
            AccelY,
            AccelZ,
            GyroX,
            GyroY,
            GyroZ,
            MagX,
            MagY,
            MagZ,
            Barometer,
            Temperature,
            GnssLatitude,
            GnssLongitude,
            GnssAltitude,
            GnssSpeed,
            QuaternionW,
            QuaternionX,
            QuaternionY,
            QuaternionZ,
            Motor,
            ControlSurface,
            Voltage,
            Current,
            Unused,
        ]
    }

    pub fn label(self) -> &'static str {
        use ChannelDestination::*;
        match self {
            Time => "Time",
            Sequence => "Packet sequence",
            AccelX => "Accelerometer X",
            AccelY => "Accelerometer Y",
            AccelZ => "Accelerometer Z",
            GyroX => "Gyroscope X",
            GyroY => "Gyroscope Y",
            GyroZ => "Gyroscope Z",
            MagX => "Magnetometer X",
            MagY => "Magnetometer Y",
            MagZ => "Magnetometer Z",
            Barometer => "Barometric pressure",
            Temperature => "Temperature",
            GnssLatitude => "GNSS latitude",
            GnssLongitude => "GNSS longitude",
            GnssAltitude => "GNSS altitude",
            GnssSpeed => "GNSS speed",
            QuaternionW => "Quaternion W",
            QuaternionX => "Quaternion X",
            QuaternionY => "Quaternion Y",
            QuaternionZ => "Quaternion Z",
            Motor => "Motor command",
            ControlSurface => "Control surface",
            Voltage => "Voltage",
            Current => "Current",
            Unused => "Not used",
        }
    }

    /// The physical quantity this destination carries, which determines the
    /// units the mapping must convert from.
    pub fn quantity_kind(self) -> QuantityKind {
        use ChannelDestination::*;
        match self {
            Time => QuantityKind::Time,
            Sequence => QuantityKind::Count,
            AccelX | AccelY | AccelZ => QuantityKind::Acceleration,
            GyroX | GyroY | GyroZ => QuantityKind::AngularRate,
            MagX | MagY | MagZ => QuantityKind::MagneticField,
            Barometer => QuantityKind::Pressure,
            Temperature => QuantityKind::Temperature,
            GnssLatitude | GnssLongitude => QuantityKind::Angle,
            GnssAltitude => QuantityKind::Position,
            GnssSpeed => QuantityKind::Velocity,
            QuaternionW | QuaternionX | QuaternionY | QuaternionZ => QuantityKind::Attitude,
            Motor | ControlSurface => QuantityKind::Ratio,
            Voltage => QuantityKind::Voltage,
            Current => QuantityKind::Current,
            Unused => QuantityKind::Raw,
        }
    }

    /// The SI unit the destination expects after conversion.
    pub fn si_unit(self) -> &'static str {
        self.quantity_kind().si_unit()
    }

    /// Which sensor an axis belongs to, if it is an axis of a three-axis sensor.
    pub fn sensor_family(self) -> Option<SensorFamily> {
        use ChannelDestination::*;
        match self {
            AccelX | AccelY | AccelZ => Some(SensorFamily::Accelerometer),
            GyroX | GyroY | GyroZ => Some(SensorFamily::Gyroscope),
            MagX | MagY | MagZ => Some(SensorFamily::Magnetometer),
            _ => None,
        }
    }

    /// Axis index within its family.
    pub fn axis_index(self) -> Option<usize> {
        use ChannelDestination::*;
        match self {
            AccelX | GyroX | MagX => Some(0),
            AccelY | GyroY | MagY => Some(1),
            AccelZ | GyroZ | MagZ => Some(2),
            _ => None,
        }
    }

    /// Whether the destination carries data the physics can use directly.
    pub fn is_physical(self) -> bool {
        !matches!(
            self,
            ChannelDestination::Unused | ChannelDestination::Sequence
        )
    }

    pub fn code(self) -> &'static str {
        use ChannelDestination::*;
        match self {
            Time => "time",
            Sequence => "sequence",
            AccelX => "accel_x",
            AccelY => "accel_y",
            AccelZ => "accel_z",
            GyroX => "gyro_x",
            GyroY => "gyro_y",
            GyroZ => "gyro_z",
            MagX => "mag_x",
            MagY => "mag_y",
            MagZ => "mag_z",
            Barometer => "barometer",
            Temperature => "temperature",
            GnssLatitude => "gnss_latitude",
            GnssLongitude => "gnss_longitude",
            GnssAltitude => "gnss_altitude",
            GnssSpeed => "gnss_speed",
            QuaternionW => "quaternion_w",
            QuaternionX => "quaternion_x",
            QuaternionY => "quaternion_y",
            QuaternionZ => "quaternion_z",
            Motor => "motor",
            ControlSurface => "control_surface",
            Voltage => "voltage",
            Current => "current",
            Unused => "unused",
        }
    }
}

/// The three-axis sensors a mapping can group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorFamily {
    Accelerometer,
    Gyroscope,
    Magnetometer,
}

impl SensorFamily {
    pub fn label(self) -> &'static str {
        match self {
            SensorFamily::Accelerometer => "Accelerometer",
            SensorFamily::Gyroscope => "Gyroscope",
            SensorFamily::Magnetometer => "Magnetometer",
        }
    }

    /// The three destinations of this family, in body axis order.
    pub fn destinations(self) -> [ChannelDestination; 3] {
        use ChannelDestination::*;
        match self {
            SensorFamily::Accelerometer => [AccelX, AccelY, AccelZ],
            SensorFamily::Gyroscope => [GyroX, GyroY, GyroZ],
            SensorFamily::Magnetometer => [MagX, MagY, MagZ],
        }
    }
}

/// One row of the channel mapping table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelMapping {
    /// Field name as it appears in the packet.
    pub packet_field: String,
    /// Index of the field in the packet payload, when the format is positional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_index: Option<usize>,
    /// What this field is used for.
    pub destination: ChannelDestination,
    /// Unit as the device reports it.
    pub source_unit: String,
    /// Sign applied after conversion. Must be exactly 1 or -1.
    pub sign: Real,
    /// Sensor axis permutation and mounting rotation for this sensor family.
    #[serde(default)]
    pub sensor_axes: SensorAxes,
    /// Optional explicit axis mapping that overrides `sensor_axes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis_mapping: Option<AxisMapping>,
    /// Whether this row is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// A note shown in the mapping table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn default_true() -> bool {
    true
}

impl ChannelMapping {
    /// A mapping row for a field, with the unit taken from the destination's SI
    /// unit so a device that already reports SI needs no conversion.
    pub fn new(packet_field: impl Into<String>, destination: ChannelDestination) -> Self {
        Self {
            packet_field: packet_field.into(),
            field_index: None,
            destination,
            source_unit: destination.si_unit().to_string(),
            sign: 1.0,
            sensor_axes: SensorAxes::default(),
            axis_mapping: None,
            enabled: true,
            note: None,
        }
    }

    /// A mapping row with an explicit source unit.
    pub fn with_unit(
        packet_field: impl Into<String>,
        destination: ChannelDestination,
        source_unit: impl Into<String>,
    ) -> Self {
        Self {
            source_unit: source_unit.into(),
            ..Self::new(packet_field, destination)
        }
    }

    /// Set the source unit, keeping the destination's SI unit as the target.
    pub fn unit(mut self, source_unit: impl Into<String>) -> Self {
        self.source_unit = source_unit.into();
        self
    }

    pub fn with_sign(mut self, sign: Real) -> Self {
        self.sign = if sign < 0.0 { -1.0 } else { 1.0 };
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn with_field_index(mut self, index: usize) -> Self {
        self.field_index = Some(index);
        self
    }

    /// The unit scale implied by `source_unit`.
    ///
    /// Returns `None` when the unit is not recognised, which the validator turns
    /// into a blocking problem rather than assuming SI.
    pub fn unit_scale(&self) -> Option<UnitScale> {
        self.destination
            .quantity_kind()
            .scale_for(&self.source_unit)
    }

    /// Scale and sign together, as applied to a raw value.
    pub fn value_scale(&self) -> Option<UnitScale> {
        self.unit_scale()
            .map(|s| UnitScale::new(s.factor * self.sign, s.offset))
    }

    /// The axis mapping this row uses, resolving the named convention when no
    /// explicit mapping is present.
    pub fn resolved_axis_mapping(&self) -> AxisMapping {
        self.axis_mapping
            .unwrap_or_else(|| AxisMapping::from_sensor_axes(self.sensor_axes))
    }

    /// Whether this row is a usable, enabled physical channel.
    pub fn is_active(&self) -> bool {
        self.enabled && self.destination.is_physical()
    }
}

/// A complete device profile.
///
/// Profiles are reusable across sessions, which is the point: a user should
/// configure a board once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceProfile {
    /// Stable profile identifier.
    pub id: String,
    /// Display name, for example "Arduino Nano 33 BLE, bench rig".
    pub name: String,
    /// Optional device identity from the USB descriptor, used to auto-select.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_identity: Option<String>,
    pub serial: crate::serial::SerialSettings,
    pub format: crate::protocol::PacketFormat,
    pub mappings: Vec<ChannelMapping>,
    /// Body and world frame conventions this profile assumes.
    pub frame: Frame,
    /// Sensor-to-body rotation for the whole board, applied to every axis family
    /// unless a row overrides it.
    #[serde(default)]
    pub board_rotation: Option<hex_core::Quaternion>,
    /// Mounting offset from the vehicle centre of gravity to the board, in the
    /// body frame, metres. Used to correct accelerometer readings.
    #[serde(default)]
    pub mounting_offset: [Real; 3],
    /// Sampling rate the device is expected to run at, hertz.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_rate_hz: Option<Real>,
    /// Timestamp source for the profile.
    pub timestamp_source: hex_core::TimeSource,
    /// Orientation estimator settings.
    pub estimator: EstimatorSettings,
    /// Free-form notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self {
            id: "profile.default".to_string(),
            name: "New device profile".to_string(),
            device_identity: None,
            serial: crate::serial::SerialSettings::default(),
            format: crate::protocol::PacketFormat::default(),
            mappings: Vec::new(),
            frame: Frame::default(),
            board_rotation: None,
            mounting_offset: [0.0; 3],
            expected_rate_hz: None,
            timestamp_source: hex_core::TimeSource::Device,
            estimator: EstimatorSettings::default(),
            notes: None,
        }
    }
}

impl DeviceProfile {
    /// A profile for a CSV device printing `timestamp,ax,ay,az,gx,gy,gz,baro`
    /// in SI units, which is the layout the product specification uses as its
    /// example.
    pub fn example_csv_profile() -> Self {
        use ChannelDestination::*;
        let names = ["timestamp", "ax", "ay", "az", "gx", "gy", "gz", "baro"];
        let destinations = [Time, AccelX, AccelY, AccelZ, GyroX, GyroY, GyroZ, Barometer];
        let mut mappings = Vec::new();
        for (i, (name, dest)) in names.iter().zip(destinations.iter()).enumerate() {
            mappings.push(
                ChannelMapping::new(*name, *dest)
                    .with_field_index(i)
                    .unit(dest.si_unit()),
            );
        }
        Self {
            id: "profile.example.csv".to_string(),
            name: "Example CSV device".to_string(),
            format: crate::protocol::PacketFormat::csv(
                names.iter().map(|s| s.to_string()).collect(),
                hex_core::TimestampUnit::Seconds,
            ),
            expected_rate_hz: Some(100.0),
            mappings,
            ..Self::default()
        }
    }

    /// A profile for the reference binary firmware.
    pub fn example_binary_profile(field_count: usize) -> Self {
        use ChannelDestination::*;
        let spec = crate::protocol::BinaryFrameSpec::hexadof_default(field_count);
        let destinations = [
            AccelX,
            AccelY,
            AccelZ,
            GyroX,
            GyroY,
            GyroZ,
            MagX,
            MagY,
            MagZ,
            Barometer,
            Temperature,
        ];
        let mappings = spec
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let dest = destinations.get(i).copied().unwrap_or(Unused);
                ChannelMapping::new(f.name.clone(), dest)
                    .with_field_index(i)
                    .unit(dest.si_unit())
            })
            .collect();
        Self {
            id: "profile.example.binary".to_string(),
            name: "Example framed binary device".to_string(),
            format: crate::protocol::PacketFormat::BinaryFramed(spec),
            mappings,
            // A 61-byte frame at 200 Hz needs about 12 kB per second, which
            // 115200 baud cannot carry, so the example profile raises the rate.
            serial: crate::serial::SerialSettings::at_baud(460_800),
            expected_rate_hz: Some(200.0),
            ..Self::default()
        }
    }

    /// The mapping row for a destination, if any.
    pub fn mapping_for(&self, destination: ChannelDestination) -> Option<&ChannelMapping> {
        self.mappings
            .iter()
            .find(|m| m.destination == destination && m.enabled)
    }

    /// Destinations this profile can populate.
    pub fn populated_destinations(&self) -> Vec<ChannelDestination> {
        self.mappings
            .iter()
            .filter(|m| m.is_active())
            .map(|m| m.destination)
            .collect()
    }

    /// The axis mapping for a sensor family, taken from the first enabled row of
    /// that family.
    pub fn family_axis_mapping(&self, family: SensorFamily) -> AxisMapping {
        let explicit = self
            .mappings
            .iter()
            .find(|m| m.destination.sensor_family() == Some(family) && m.enabled)
            .map(|m| m.resolved_axis_mapping());
        match (explicit, self.board_rotation) {
            (Some(mut m), Some(q)) => {
                m.mounting = Some(match m.mounting {
                    Some(inner) => q * inner,
                    None => q,
                });
                m
            }
            (Some(m), None) => m,
            (None, Some(q)) => AxisMapping {
                mounting: Some(q),
                ..AxisMapping::identity()
            },
            (None, None) => AxisMapping::identity(),
        }
    }

    /// Whether the three axes of a family are all mapped, so the sensor can be
    /// used as a vector.
    pub fn has_complete_family(&self, family: SensorFamily) -> bool {
        family
            .destinations()
            .iter()
            .all(|d| self.mapping_for(*d).is_some())
    }

    /// The expected sample interval implied by `expected_rate_hz`.
    pub fn nominal_dt(&self) -> Option<Real> {
        self.expected_rate_hz.filter(|r| *r > 0.0).map(|r| 1.0 / r)
    }

    /// Problems that must be fixed before a session can start.
    ///
    /// Returns one `MappingProblem` per issue, so the connection panel can list
    /// them rather than showing a single opaque error.
    pub fn validate(&self) -> Vec<MappingProblem> {
        let mut problems = Vec::new();

        // A framed binary protocol carries its own device timestamp in the frame
        // header, so it satisfies the time requirement without a mapped field.
        let time_from_frame_header = matches!(
            &self.format,
            crate::protocol::PacketFormat::BinaryFramed(spec) if spec.device_timestamp.is_some()
        );
        if self.mapping_for(ChannelDestination::Time).is_none() && !time_from_frame_header {
            problems.push(MappingProblem::blocking(
                "mapping.time_missing",
                "No time column is mapped",
                "Live data needs a timestamp. Map one packet field to Time, or let HexaDOF stamp each packet with the host clock.",
                "Select a field and set its destination to Time.",
            ));
        }

        for m in self.mappings.iter().filter(|m| m.enabled) {
            if m.unit_scale().is_none() {
                problems.push(MappingProblem::blocking(
                    "mapping.unit_unknown",
                    format!("Unknown unit for {}", m.packet_field),
                    format!(
                        "The unit \"{}\" is not recognised for {}. HexaDOF will not guess, because a wrong unit silently corrupts every downstream result.",
                        m.source_unit,
                        m.destination.label()
                    ),
                    format!(
                        "Set the unit to one of the {} units, for example {}.",
                        m.destination.label().to_lowercase(),
                        m.destination.si_unit()
                    ),
                ));
            }
            if m.sign != 1.0 && m.sign != -1.0 {
                problems.push(MappingProblem::blocking(
                    "mapping.sign_invalid",
                    format!("Invalid sign for {}", m.packet_field),
                    "The sign must be exactly +1 or -1.".to_string(),
                    "Set the sign to +1 or -1.",
                ));
            }
            let axis = m.resolved_axis_mapping();
            if !axis.is_valid() {
                problems.push(MappingProblem::blocking(
                    "mapping.axis_invalid",
                    format!("Invalid axis mapping for {}", m.packet_field),
                    "The axis permutation repeats or omits a sensor axis.".to_string(),
                    "Choose a permutation that uses each sensor axis exactly once.",
                ));
            }
        }

        for family in [
            SensorFamily::Accelerometer,
            SensorFamily::Gyroscope,
            SensorFamily::Magnetometer,
        ] {
            let mapped = family
                .destinations()
                .iter()
                .filter(|d| self.mapping_for(**d).is_some())
                .count();
            if mapped > 0 && mapped < 3 {
                problems.push(MappingProblem::advisory(
                    "mapping.family_incomplete",
                    format!("{} is only partly mapped", family.label()),
                    format!(
                        "{} of 3 axes are mapped. Attitude estimation needs all three axes of a sensor.",
                        mapped
                    ),
                    "Map the remaining axes, or set them to Not used so the omission is explicit.",
                ));
            }
        }

        if let Some(rate) = self.expected_rate_hz {
            if rate <= 0.0 {
                problems.push(MappingProblem::blocking(
                    "mapping.rate_invalid",
                    "Expected sample rate must be positive",
                    "A zero or negative rate cannot be scheduled.".to_string(),
                    "Enter the rate the device actually runs at, in hertz.",
                ));
            }
            let payload = match &self.format {
                crate::protocol::PacketFormat::CsvLine { .. } => 40,
                crate::protocol::PacketFormat::BinaryFramed(b) => b.frame_width(),
            };
            let required = rate * payload as Real;
            let available = self.serial.maximum_bytes_per_second();
            if required > available {
                problems.push(MappingProblem::advisory(
                    "mapping.rate_exceeds_line",
                    "Configured rate may not fit the serial line",
                    format!(
                        "{:.0} packets per second at about {} bytes each needs roughly {:.0} bytes per second, but {} baud carries about {:.0}.",
                        rate, payload, required, self.serial.baud_rate, available
                    ),
                    "Reduce the packet rate or the payload size, or raise the baud rate.",
                ));
            }
        }

        problems
    }

    /// Whether the profile can start a session.
    pub fn is_usable(&self) -> bool {
        !self.validate().iter().any(|p| p.blocking)
    }

    /// Rows for the mapping table, in the order the table should show them.
    pub fn table_rows(&self) -> Vec<MappingRow> {
        self.mappings
            .iter()
            .map(|m| MappingRow {
                packet_field: m.packet_field.clone(),
                destination: m.destination,
                source_unit: m.source_unit.clone(),
                si_unit: m.destination.si_unit(),
                sign: m.sign,
                sensor_axes: m.sensor_axes,
                enabled: m.enabled,
                unit_known: m.unit_scale().is_some(),
            })
            .collect()
    }

    /// A preview of what a raw packet becomes after the mapping.
    ///
    /// The UI shows this beside the mapping table so a sign or axis mistake is
    /// visible before a session starts.
    pub fn preview(&self, packet: &crate::protocol::DecodedPacket) -> Vec<MappedValue> {
        let mut out = Vec::new();
        for m in self.mappings.iter().filter(|m| m.enabled) {
            let raw = match m.field_index {
                Some(i) => packet.values.get(i).copied(),
                None => packet.value(&m.packet_field),
            };
            let Some(raw) = raw else { continue };
            let converted = m.value_scale().map(|s| s.apply(raw));
            out.push(MappedValue {
                packet_field: m.packet_field.clone(),
                destination: m.destination,
                raw,
                converted,
                source_unit: m.source_unit.clone(),
                si_unit: m.destination.si_unit(),
                usable: converted.map(|v| v.is_finite()).unwrap_or(false),
            });
        }
        out
    }
}

/// A row of the mapping table as the UI renders it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingRow {
    pub packet_field: String,
    pub destination: ChannelDestination,
    pub source_unit: String,
    pub si_unit: &'static str,
    pub sign: Real,
    pub sensor_axes: SensorAxes,
    pub enabled: bool,
    /// Whether the unit was recognised. An unknown unit is highlighted because
    /// the value cannot be converted.
    pub unit_known: bool,
}

/// One field after the mapping has been applied.
///
/// This is a UI preview value and is not persisted, so it deliberately has no
/// serde representation.
#[derive(Debug, Clone, PartialEq)]
pub struct MappedValue {
    pub raw: Real,
    pub converted: Option<Real>,
    pub usable: bool,
    pub packet_field: String,
    pub destination: ChannelDestination,
    pub source_unit: String,
    pub si_unit: &'static str,
}

/// A problem with a device profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingProblem {
    pub code: String,
    pub title: String,
    pub detail: String,
    pub suggestion: String,
    /// Whether this blocks starting a session.
    pub blocking: bool,
}

impl MappingProblem {
    pub fn blocking(
        code: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            title: title.into(),
            detail: detail.into(),
            suggestion: suggestion.into(),
            blocking: true,
        }
    }

    pub fn advisory(
        code: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            title: title.into(),
            detail: detail.into(),
            suggestion: suggestion.into(),
            blocking: false,
        }
    }
}

/// Settings for the orientation estimator.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EstimatorSettings {
    /// Which estimator to run.
    pub mode: EstimatorMode,
    /// Whether accelerometer correction is allowed at all.
    pub accelerometer_correction: bool,
    /// Maximum non-gravitational acceleration, in m/s^2, at which the
    /// accelerometer is still trusted as a gravity reference.
    pub accelerometer_trust_limit: Real,
    /// Complementary filter weight on the gyroscope, between 0 and 1. One is
    /// pure gyro integration.
    pub gyro_weight: Real,
    /// Whether to zero the gyro bias from the first stationary samples.
    pub estimate_gyro_bias: bool,
    /// Stationary samples used for the bias estimate.
    pub bias_samples: usize,
    /// Freeze the displayed attitude when the estimator health is poor.
    pub freeze_on_unhealthy: bool,
}

impl Default for EstimatorSettings {
    fn default() -> Self {
        Self {
            mode: EstimatorMode::GyroPropagation,
            accelerometer_correction: false,
            accelerometer_trust_limit: 1.0,
            gyro_weight: 0.98,
            estimate_gyro_bias: true,
            bias_samples: 200,
            freeze_on_unhealthy: true,
        }
    }
}

impl EstimatorSettings {
    /// Settings that trust the accelerometer only while nearly stationary.
    pub fn conservative_complementary() -> Self {
        Self {
            mode: EstimatorMode::Complementary,
            accelerometer_correction: true,
            accelerometer_trust_limit: 0.5,
            gyro_weight: 0.98,
            ..Self::default()
        }
    }

    /// Validate the settings, returning descriptions of each problem.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !(0.0..=1.0).contains(&self.gyro_weight) {
            problems.push("the gyroscope weight must lie between 0 and 1".to_string());
        }
        if self.accelerometer_trust_limit <= 0.0 {
            problems.push("the accelerometer trust limit must be positive".to_string());
        }
        if self.estimate_gyro_bias && self.bias_samples == 0 {
            problems.push("a bias estimate needs at least one sample".to_string());
        }
        if self.mode == EstimatorMode::Complementary && !self.accelerometer_correction {
            problems.push(
                "a complementary filter needs accelerometer correction, otherwise it is pure gyro integration"
                    .to_string(),
            );
        }
        problems
    }

    /// A one-line description for the health panel.
    pub fn label(&self) -> String {
        match self.mode {
            EstimatorMode::GyroPropagation => {
                "Gyro propagation only. Orientation will drift; it is not an absolute reference."
                    .to_string()
            }
            EstimatorMode::Complementary => format!(
                "Complementary filter, gyro weight {:.2}, accelerometer trusted below {:.1} m/s^2",
                self.gyro_weight, self.accelerometer_trust_limit
            ),
            EstimatorMode::DeviceSupplied => {
                "Orientation supplied by the device. HexaDOF does not filter it.".to_string()
            }
        }
    }
}

/// Which estimator to run on live data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimatorMode {
    /// Integrate the gyroscope only. Simple, and expected to drift.
    GyroPropagation,
    /// Fuse the gyroscope with a gated accelerometer reference.
    Complementary,
    /// Use a quaternion the device already computed.
    DeviceSupplied,
}

impl EstimatorMode {
    pub fn label(self) -> &'static str {
        match self {
            EstimatorMode::GyroPropagation => "Gyro propagation",
            EstimatorMode::Complementary => "Complementary filter",
            EstimatorMode::DeviceSupplied => "Device supplied",
        }
    }

    /// Whether this mode is an absolute orientation reference.
    ///
    /// None of the MVP modes are. A gyro integration drifts and a complementary
    /// filter is still relative without a magnetometer, so the UI must not claim
    /// otherwise.
    pub fn is_absolute_reference(self) -> bool {
        false
    }
}

/// Apply a signed axis mapping to a three-axis sensor reading.
pub fn apply_axis_mapping(mapping: &AxisMapping, reading: Vec3) -> Vec3 {
    mapping.sensor_to_body(reading)
}

/// The destination of every packet field, in a form the packet inspector shows.
pub fn describe_mapping(mappings: &[ChannelMapping]) -> Vec<String> {
    mappings
        .iter()
        .filter(|m| m.enabled)
        .map(|m| {
            format!(
                "{} -> {} ({} -> {}, sign {:+.0}{})",
                m.packet_field,
                m.destination.label(),
                m.source_unit,
                m.destination.si_unit(),
                m.sign,
                match m.unit_scale() {
                    Some(_) => "",
                    None => ", UNIT UNKNOWN",
                }
            )
        })
        .collect()
}

/// Convert a raw field value into SI using a mapping row.
pub fn convert_value(mapping: &ChannelMapping, raw: Real) -> Option<Real> {
    mapping.value_scale().map(|s| s.apply(raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        encode_frame, BinaryFrameSpec, DecodedPacket, PacketDecoder, PacketFormat,
    };
    use hex_core::TimestampUnit;

    #[test]
    fn every_destination_has_a_label_unit_and_code() {
        for d in ChannelDestination::all() {
            assert!(!d.label().is_empty(), "{:?}", d);
            assert!(!d.si_unit().is_empty(), "{:?}", d);
            assert!(!d.code().is_empty(), "{:?}", d);
        }
    }

    #[test]
    fn axis_destinations_report_family_and_index() {
        assert_eq!(
            ChannelDestination::AccelY.sensor_family(),
            Some(SensorFamily::Accelerometer)
        );
        assert_eq!(ChannelDestination::GyroZ.axis_index(), Some(2));
        assert_eq!(ChannelDestination::Barometer.axis_index(), None);
        assert_eq!(ChannelDestination::Time.sensor_family(), None);
    }

    #[test]
    fn destination_quantity_kinds_are_physical() {
        assert_eq!(
            ChannelDestination::AccelX.quantity_kind(),
            QuantityKind::Acceleration
        );
        assert_eq!(
            ChannelDestination::GyroX.quantity_kind(),
            QuantityKind::AngularRate
        );
        assert_eq!(
            ChannelDestination::Barometer.quantity_kind(),
            QuantityKind::Pressure
        );
        assert_eq!(
            ChannelDestination::GnssLatitude.quantity_kind(),
            QuantityKind::Angle
        );
        assert_eq!(
            ChannelDestination::Unused.quantity_kind(),
            QuantityKind::Raw
        );
        assert!(!ChannelDestination::Unused.is_physical());
    }

    #[test]
    fn unit_scale_converts_the_common_device_units() {
        let g = ChannelMapping::with_unit("ax", ChannelDestination::AccelX, "g");
        let scale = g.unit_scale().unwrap();
        assert!((scale.apply(1.0) - hex_core::STANDARD_GRAVITY).abs() < 1e-12);

        let dps = ChannelMapping::with_unit("gx", ChannelDestination::GyroX, "deg/s");
        assert!((dps.unit_scale().unwrap().apply(180.0) - std::f64::consts::PI).abs() < 1e-12);

        let hpa = ChannelMapping::with_unit("baro", ChannelDestination::Barometer, "hPa");
        assert!((hpa.unit_scale().unwrap().apply(1013.25) - 101_325.0).abs() < 1e-6);
    }

    #[test]
    fn sign_is_applied_after_the_unit_scale() {
        let m = ChannelMapping::with_unit("gz", ChannelDestination::GyroZ, "deg/s").with_sign(-1.0);
        let v = convert_value(&m, 90.0).unwrap();
        assert!((v + std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }

    #[test]
    fn sign_is_clamped_to_unit_magnitude() {
        let m = ChannelMapping::new("ax", ChannelDestination::AccelX).with_sign(42.0);
        assert!((m.sign - 1.0).abs() < 1e-15);
        let m = m.with_sign(-0.1);
        assert!((m.sign + 1.0).abs() < 1e-15);
    }

    #[test]
    fn unknown_unit_reports_none_rather_than_guessing() {
        let m = ChannelMapping::with_unit("ax", ChannelDestination::AccelX, "furlongs");
        assert!(m.unit_scale().is_none());
        assert!(convert_value(&m, 1.0).is_none());
    }

    #[test]
    fn example_csv_profile_is_valid_and_populates_the_expected_channels() {
        let p = DeviceProfile::example_csv_profile();
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        assert!(p.is_usable());
        let dests = p.populated_destinations();
        assert!(dests.contains(&ChannelDestination::Time));
        assert!(dests.contains(&ChannelDestination::AccelX));
        assert!(dests.contains(&ChannelDestination::GyroZ));
        assert!(dests.contains(&ChannelDestination::Barometer));
        assert!(p.has_complete_family(SensorFamily::Accelerometer));
        assert!(p.has_complete_family(SensorFamily::Gyroscope));
        assert!(!p.has_complete_family(SensorFamily::Magnetometer));
    }

    #[test]
    fn example_binary_profile_is_valid() {
        let p = DeviceProfile::example_binary_profile(11);
        assert!(p.validate().is_empty(), "{:?}", p.validate());
        assert!(p.has_complete_family(SensorFamily::Magnetometer));
    }

    #[test]
    fn profile_without_a_time_column_is_blocking() {
        let mut p = DeviceProfile::example_csv_profile();
        p.mappings
            .retain(|m| m.destination != ChannelDestination::Time);
        let problems = p.validate();
        assert!(problems.iter().any(|x| x.code == "mapping.time_missing"));
        assert!(!p.is_usable());
    }

    #[test]
    fn profile_with_an_unknown_unit_is_blocking() {
        let mut p = DeviceProfile::example_csv_profile();
        p.mappings[1].source_unit = "bananas".to_string();
        let problems = p.validate();
        let problem = problems
            .iter()
            .find(|x| x.code == "mapping.unit_unknown")
            .expect("unit problem");
        assert!(problem.blocking);
        assert!(problem.detail.contains("bananas"));
        assert!(!p.is_usable());
    }

    #[test]
    fn partly_mapped_sensor_family_is_an_advisory_not_a_blocker() {
        let mut p = DeviceProfile::example_csv_profile();
        p.mappings
            .retain(|m| m.destination != ChannelDestination::AccelZ);
        let problems = p.validate();
        let problem = problems
            .iter()
            .find(|x| x.code == "mapping.family_incomplete")
            .expect("family problem");
        assert!(!problem.blocking);
        assert!(problem.title.contains("Accelerometer"));
    }

    #[test]
    fn impossible_packet_rate_is_reported() {
        let mut p = DeviceProfile::example_csv_profile();
        p.serial.baud_rate = 9600;
        p.expected_rate_hz = Some(10_000.0);
        let problems = p.validate();
        assert!(problems
            .iter()
            .any(|x| x.code == "mapping.rate_exceeds_line"));
    }

    #[test]
    fn invalid_axis_mapping_is_blocking() {
        let mut p = DeviceProfile::example_csv_profile();
        p.mappings[1].axis_mapping = Some(AxisMapping {
            body_to_sensor: [0, 0, 2],
            sign: [1.0, 1.0, 1.0],
            mounting: None,
        });
        let problems = p.validate();
        assert!(problems.iter().any(|x| x.code == "mapping.axis_invalid"));
    }

    #[test]
    fn table_rows_report_unit_confidence() {
        let mut p = DeviceProfile::example_csv_profile();
        p.mappings[2].source_unit = "??".to_string();
        let rows = p.table_rows();
        let row = rows
            .iter()
            .find(|r| r.packet_field == "ay")
            .expect("ay row");
        assert!(!row.unit_known);
        assert_eq!(row.si_unit, "m/s^2");
    }

    #[test]
    fn preview_shows_raw_and_converted_values() {
        let profile = DeviceProfile::example_csv_profile();
        let mut decoder = PacketDecoder::new(profile.format.clone());
        let out = decoder.push(b"1.5,0.0,0.0,9.81,0.0,0.0,0.0,101325\n");
        assert_eq!(out.packets.len(), 1);
        let preview = profile.preview(&out.packets[0]);
        assert_eq!(preview.len(), 8);
        let baro = preview
            .iter()
            .find(|v| v.destination == ChannelDestination::Barometer)
            .unwrap();
        assert!((baro.raw - 101_325.0).abs() < 1e-9);
        assert!((baro.converted.unwrap() - 101_325.0).abs() < 1e-9);
        assert!(baro.usable);
    }

    #[test]
    fn preview_reports_a_bad_unit_as_unusable() {
        let mut profile = DeviceProfile::example_csv_profile();
        profile.mappings[1].source_unit = "gigawatts".to_string();
        let mut decoder = PacketDecoder::new(profile.format.clone());
        let out = decoder.push(b"0.0,1.0,0.0,9.81,0.0,0.0,0.0,101325\n");
        let preview = profile.preview(&out.packets[0]);
        let ax = preview
            .iter()
            .find(|v| v.destination == ChannelDestination::AccelX)
            .unwrap();
        assert!(ax.converted.is_none());
        assert!(!ax.usable);
    }

    #[test]
    fn family_axis_mapping_combines_the_row_and_the_board_rotation() {
        let mut p = DeviceProfile::example_csv_profile();
        p.board_rotation = Some(hex_core::Quaternion::from_axis_angle(Vec3::z(), 1.0));
        let m = p.family_axis_mapping(SensorFamily::Accelerometer);
        assert!(m.mounting.is_some());
        let out = m.sensor_to_body(Vec3::new(1.0, 0.0, 0.0));
        // A 1 radian rotation about Z maps X toward Y.
        assert!(out.y > 0.8);
    }

    #[test]
    fn estimator_settings_validation() {
        assert!(EstimatorSettings::default().validate().is_empty());
        let bad = EstimatorSettings {
            gyro_weight: 1.5,
            ..Default::default()
        };
        assert!(!bad.validate().is_empty());
        let bad = EstimatorSettings {
            mode: EstimatorMode::Complementary,
            accelerometer_correction: false,
            ..Default::default()
        };
        assert!(bad.validate().iter().any(|p| p.contains("complementary")));
        assert!(EstimatorSettings::conservative_complementary()
            .validate()
            .is_empty());
    }

    #[test]
    fn no_estimator_mode_claims_to_be_absolute() {
        for mode in [
            EstimatorMode::GyroPropagation,
            EstimatorMode::Complementary,
            EstimatorMode::DeviceSupplied,
        ] {
            assert!(!mode.is_absolute_reference());
            assert!(!mode.label().is_empty());
        }
        assert!(EstimatorSettings::default().label().contains("drift"));
    }

    #[test]
    fn nominal_dt_comes_from_the_expected_rate() {
        let mut p = DeviceProfile::default();
        assert!(p.nominal_dt().is_none());
        p.expected_rate_hz = Some(200.0);
        assert!((p.nominal_dt().unwrap() - 0.005).abs() < 1e-12);
        p.expected_rate_hz = Some(0.0);
        assert!(p.nominal_dt().is_none());
    }

    #[test]
    fn describe_mapping_flags_unknown_units() {
        let mut p = DeviceProfile::example_csv_profile();
        p.mappings[0].source_unit = "zorks".to_string();
        let lines = describe_mapping(&p.mappings);
        assert!(lines.iter().any(|l| l.contains("UNIT UNKNOWN")));
        assert!(lines.iter().any(|l| l.contains("Accelerometer X")));
    }

    #[test]
    fn mapping_problem_constructors_set_blocking() {
        let b = MappingProblem::blocking("a", "b", "c", "d");
        assert!(b.blocking);
        let a = MappingProblem::advisory("a", "b", "c", "d");
        assert!(!a.blocking);
    }

    #[test]
    fn binary_profile_previews_a_real_frame() {
        let profile = DeviceProfile::example_binary_profile(3);
        let spec = match &profile.format {
            PacketFormat::BinaryFramed(s) => s.clone(),
            _ => panic!("expected a binary format"),
        };
        let frame = encode_frame(&spec, 1, 100_000, &[1.0, 2.0, 3.0]).unwrap();
        let mut decoder = PacketDecoder::new(profile.format.clone());
        let out = decoder.push(&frame);
        assert_eq!(out.packets.len(), 1);
        let preview = profile.preview(&out.packets[0]);
        assert_eq!(preview.len(), 3);
        assert!(preview.iter().all(|v| v.usable));
    }

    #[test]
    fn csv_profile_accepts_a_whitespace_separated_timestamp() {
        let profile = DeviceProfile::example_csv_profile();
        let mut decoder = PacketDecoder::new(PacketFormat::csv(vec![], TimestampUnit::Seconds));
        let out = decoder.push(b"  0.25 , 1.0 , 0.0 , 9.81 , 0.0 , 0.0 , 0.0 , 101325 \n");
        assert_eq!(out.packets.len(), 1);
        let preview = profile.preview(&out.packets[0]);
        let time = preview
            .iter()
            .find(|v| v.destination == ChannelDestination::Time)
            .unwrap();
        assert!((time.converted.unwrap() - 0.25).abs() < 1e-12);
    }

    #[test]
    fn default_profile_has_a_serial_default_and_no_mappings() {
        let p = DeviceProfile::default();
        assert_eq!(p.serial.baud_rate, 115_200);
        assert!(p.mappings.is_empty());
        assert!(!p.is_usable());
    }

    #[test]
    fn sensor_family_destinations_are_ordered() {
        assert_eq!(
            SensorFamily::Gyroscope.destinations(),
            [
                ChannelDestination::GyroX,
                ChannelDestination::GyroY,
                ChannelDestination::GyroZ
            ]
        );
        assert_eq!(SensorFamily::Accelerometer.label(), "Accelerometer");
    }

    #[test]
    fn decoded_packet_without_field_names_still_maps_by_index() {
        let mut p = DeviceProfile::example_csv_profile();
        for (i, m) in p.mappings.iter_mut().enumerate() {
            m.field_index = Some(i);
        }
        let packet = DecodedPacket {
            device_time: Some(0.0),
            host_time: None,
            message_type: None,
            sequence: None,
            values: vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0],
            field_names: vec![],
            frame_bytes: 40,
            raw_payload: vec![],
        };
        let preview = p.preview(&packet);
        assert_eq!(preview.len(), 8);
        assert!((preview[7].raw - 7.0).abs() < 1e-12);
    }

    #[test]
    fn apply_axis_mapping_delegates_to_the_axis_mapping() {
        let m = AxisMapping::from_sensor_axes(SensorAxes::XyzNegZ);
        let out = apply_axis_mapping(&m, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(out, Vec3::new(1.0, 2.0, -3.0));
    }

    #[test]
    fn binary_spec_width_is_used_for_the_rate_check() {
        let profile = DeviceProfile::example_binary_profile(11);
        let spec = match &profile.format {
            PacketFormat::BinaryFramed(s) => s.clone(),
            _ => unreachable!(),
        };
        let expected = spec.frame_width();
        assert!(expected > 13);
        assert_eq!(BinaryFrameSpec::hexadof_default(11).frame_width(), expected);
    }
}
