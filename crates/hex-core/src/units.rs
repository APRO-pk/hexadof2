//! SI-first unit handling.
//!
//! HexaDOF stores every quantity in SI. These types exist to make the unit of a
//! value explicit at the API boundary and to centralise the display
//! conversions the UI performs. No internal computation should need a
//! conversion factor.

use serde::{Deserialize, Serialize};

use crate::{Real, STANDARD_GRAVITY};

/// A length in metres.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Length(pub Real);

impl Length {
    pub const fn from_m(v: Real) -> Self {
        Self(v)
    }
    pub const fn from_km(v: Real) -> Self {
        Self(v * 1000.0)
    }
    pub const fn from_ft(v: Real) -> Self {
        Self(v * 0.3048)
    }
    pub const fn m(self) -> Real {
        self.0
    }
    pub const fn km(self) -> Real {
        self.0 / 1000.0
    }
    pub const fn ft(self) -> Real {
        self.0 / 0.3048
    }
}

/// A velocity in metres per second.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Velocity(pub Real);

impl Velocity {
    pub const fn from_mps(v: Real) -> Self {
        Self(v)
    }
    pub const fn from_kmh(v: Real) -> Self {
        Self(v / 3.6)
    }
    pub const fn from_knots(v: Real) -> Self {
        Self(v * 0.514_444_444_444_444_4)
    }
    pub const fn mps(self) -> Real {
        self.0
    }
    pub const fn kmh(self) -> Real {
        self.0 * 3.6
    }
    pub const fn knots(self) -> Real {
        self.0 / 0.514_444_444_444_444_4
    }
    pub const fn mach(self, speed_of_sound: Real) -> Real {
        self.0 / speed_of_sound
    }
}

/// An angular rate in radians per second.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AngularRate(pub Real);

impl AngularRate {
    pub const fn from_rad_s(v: Real) -> Self {
        Self(v)
    }
    pub const fn from_deg_s(v: Real) -> Self {
        Self(v.to_radians())
    }
    /// Interpret a value in revolutions per minute.
    pub fn from_rpm(v: Real) -> Self {
        Self(v * std::f64::consts::TAU / 60.0)
    }
    pub const fn rad_s(self) -> Real {
        self.0
    }
    pub const fn deg_s(self) -> Real {
        self.0.to_degrees()
    }
    pub fn rpm(self) -> Real {
        self.0 * 60.0 / std::f64::consts::TAU
    }
}

/// An angle in radians.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Angle(pub Real);

impl Angle {
    pub const fn from_rad(v: Real) -> Self {
        Self(v)
    }
    pub const fn from_deg(v: Real) -> Self {
        Self(v.to_radians())
    }
    pub const fn rad(self) -> Real {
        self.0
    }
    pub const fn deg(self) -> Real {
        self.0.to_degrees()
    }
    /// Wrap into the half-open interval `(-pi, pi]`.
    pub fn wrapped(self) -> Self {
        let two_pi = std::f64::consts::TAU;
        let mut v = (self.0 + std::f64::consts::PI).rem_euclid(two_pi) - std::f64::consts::PI;
        if v <= -std::f64::consts::PI {
            v += two_pi;
        }
        Self(v)
    }
}

/// A mass in kilograms.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Mass(pub Real);

impl Mass {
    pub const fn from_kg(v: Real) -> Self {
        Self(v)
    }
    pub const fn from_g(v: Real) -> Self {
        Self(v / 1000.0)
    }
    pub const fn kg(self) -> Real {
        self.0
    }
    pub const fn g(self) -> Real {
        self.0 * 1000.0
    }
    /// Weight under standard gravity, in newtons.
    pub const fn weight_standard(self) -> Real {
        self.0 * STANDARD_GRAVITY
    }
}

/// A pressure in pascals.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Pressure(pub Real);

impl Pressure {
    /// Standard sea-level pressure, Pa.
    pub const SEA_LEVEL: Real = 101_325.0;
    pub const fn from_pa(v: Real) -> Self {
        Self(v)
    }
    pub const fn from_hpa(v: Real) -> Self {
        Self(v * 100.0)
    }
    pub const fn from_bar(v: Real) -> Self {
        Self(v * 100_000.0)
    }
    pub const fn from_psi(v: Real) -> Self {
        Self(v * 6_894.757_293_168)
    }
    pub const fn pa(self) -> Real {
        self.0
    }
    pub const fn hpa(self) -> Real {
        self.0 / 100.0
    }
    pub const fn bar(self) -> Real {
        self.0 / 100_000.0
    }
    pub const fn psi(self) -> Real {
        self.0 / 6_894.757_293_168
    }
}

/// A temperature in kelvin.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Temperature(pub Real);

impl Temperature {
    /// Standard sea-level temperature, K.
    pub const SEA_LEVEL: Real = 288.15;
    pub const fn from_k(v: Real) -> Self {
        Self(v)
    }
    pub const fn from_c(v: Real) -> Self {
        Self(v + 273.15)
    }
    pub const fn k(self) -> Real {
        self.0
    }
    pub const fn c(self) -> Real {
        self.0 - 273.15
    }
}

/// The unit system a project or imported artefact declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitSystem {
    /// Metres, kilograms, seconds, radians, newtons, pascals, kelvin.
    #[default]
    Si,
    /// Feet, slugs, seconds, radians, pound-force, pounds per square foot.
    Imperial,
    /// Metres, kilograms, seconds, degrees, newtons.
    MetricDegrees,
    /// The unit system could not be determined from the source file.
    Unknown,
}

impl UnitSystem {
    pub fn label(self) -> &'static str {
        match self {
            UnitSystem::Si => "SI (m, kg, s, rad)",
            UnitSystem::Imperial => "Imperial (ft, slug, s, rad)",
            UnitSystem::MetricDegrees => "Metric (m, kg, s, deg)",
            UnitSystem::Unknown => "Unknown",
        }
    }
}

/// Whether a quantity's unit was declared by the source or assumed by HexaDOF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitConfidence {
    /// The source file declared the unit explicitly.
    Declared,
    /// HexaDOF inferred the unit from the project defaults.
    Assumed,
    /// No plausible unit could be established.
    Unknown,
}

/// A value paired with its declared unit, used at import boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Quantity {
    /// Value expressed in SI base units after conversion.
    pub si_value: Real,
    /// Human-readable unit label as declared by the source.
    pub declared_unit: String,
    /// How the unit was established.
    pub confidence: UnitConfidence,
}

impl Quantity {
    pub fn declared(si_value: Real, unit: impl Into<String>) -> Self {
        Self {
            si_value,
            declared_unit: unit.into(),
            confidence: UnitConfidence::Declared,
        }
    }

    pub fn assumed(si_value: Real, unit: impl Into<String>) -> Self {
        Self {
            si_value,
            declared_unit: unit.into(),
            confidence: UnitConfidence::Assumed,
        }
    }

    pub fn unknown(si_value: Real) -> Self {
        Self {
            si_value,
            declared_unit: String::new(),
            confidence: UnitConfidence::Unknown,
        }
    }

    /// True when the unit is trustworthy enough to compute with silently.
    pub fn is_trustworthy(&self) -> bool {
        self.confidence == UnitConfidence::Declared
    }
}

/// Factors required to interpret a channel expressed in an arbitrary unit.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct UnitScale {
    /// Multiplier applied to the raw value.
    pub factor: Real,
    /// Offset added after scaling. Used for temperatures.
    pub offset: Real,
}

impl UnitScale {
    pub const IDENTITY: UnitScale = UnitScale {
        factor: 1.0,
        offset: 0.0,
    };

    pub const fn new(factor: Real, offset: Real) -> Self {
        Self { factor, offset }
    }

    pub fn apply(&self, raw: Real) -> Real {
        raw * self.factor + self.offset
    }

    pub fn invert(&self, si: Real) -> Real {
        (si - self.offset) / self.factor
    }
}

impl Default for UnitScale {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// Canonical physical quantity a telemetry or flight channel carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuantityKind {
    Acceleration,
    AngularRate,
    MagneticField,
    Pressure,
    Temperature,
    Position,
    Velocity,
    Attitude,
    Angle,
    Force,
    Moment,
    Mass,
    Density,
    Voltage,
    Current,
    Ratio,
    Time,
    Count,
    Raw,
}

impl QuantityKind {
    /// The SI unit HexaDOF uses for this quantity.
    pub fn si_unit(self) -> &'static str {
        match self {
            QuantityKind::Acceleration => "m/s^2",
            QuantityKind::AngularRate => "rad/s",
            QuantityKind::MagneticField => "T",
            QuantityKind::Pressure => "Pa",
            QuantityKind::Temperature => "K",
            QuantityKind::Position => "m",
            QuantityKind::Velocity => "m/s",
            QuantityKind::Attitude => "quaternion",
            QuantityKind::Angle => "rad",
            QuantityKind::Force => "N",
            QuantityKind::Moment => "N*m",
            QuantityKind::Mass => "kg",
            QuantityKind::Density => "kg/m^3",
            QuantityKind::Voltage => "V",
            QuantityKind::Current => "A",
            QuantityKind::Ratio => "1",
            QuantityKind::Time => "s",
            QuantityKind::Count => "count",
            QuantityKind::Raw => "raw",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            QuantityKind::Acceleration => "Acceleration",
            QuantityKind::AngularRate => "Angular rate",
            QuantityKind::MagneticField => "Magnetic field",
            QuantityKind::Pressure => "Pressure",
            QuantityKind::Temperature => "Temperature",
            QuantityKind::Position => "Position",
            QuantityKind::Velocity => "Velocity",
            QuantityKind::Attitude => "Attitude",
            QuantityKind::Angle => "Angle",
            QuantityKind::Force => "Force",
            QuantityKind::Moment => "Moment",
            QuantityKind::Mass => "Mass",
            QuantityKind::Density => "Density",
            QuantityKind::Voltage => "Voltage",
            QuantityKind::Current => "Current",
            QuantityKind::Ratio => "Ratio",
            QuantityKind::Time => "Time",
            QuantityKind::Count => "Count",
            QuantityKind::Raw => "Raw",
        }
    }

    /// Resolve a free-text unit label into a scale that produces SI values.
    ///
    /// Returns `None` when the unit is not recognised; callers must surface that
    /// as a validation issue rather than guessing.
    pub fn scale_for(self, unit: &str) -> Option<UnitScale> {
        let u = unit.trim().to_ascii_lowercase().replace(' ', "");
        let s = |f: Real| Some(UnitScale::new(f, 0.0));
        match self {
            QuantityKind::Acceleration => match u.as_str() {
                "m/s^2" | "m/s2" | "m/s²" | "mps2" | "ms-2" => s(1.0),
                "g" | "g0" | "gees" => s(STANDARD_GRAVITY),
                "ft/s^2" | "ft/s2" | "ft/s²" => s(0.3048),
                _ => None,
            },
            QuantityKind::AngularRate => match u.as_str() {
                "rad/s" | "rads" | "rad/sec" => s(1.0),
                "deg/s" | "dps" | "deg/sec" => s(std::f64::consts::PI / 180.0),
                "rpm" => s(std::f64::consts::TAU / 60.0),
                _ => None,
            },
            QuantityKind::MagneticField => match u.as_str() {
                "t" | "tesla" => s(1.0),
                "mt" | "millitesla" => s(1e-3),
                "ut" | "microtesla" | "µt" => s(1e-6),
                "gauss" | "gs" => s(1e-4),
                _ => None,
            },
            QuantityKind::Pressure => match u.as_str() {
                "pa" | "pascal" => s(1.0),
                "hpa" | "mbar" | "millibar" => s(100.0),
                "kpa" => s(1000.0),
                "bar" => s(100_000.0),
                "psi" => s(6_894.757_293_168),
                "atm" => s(101_325.0),
                _ => None,
            },
            QuantityKind::Temperature => match u.as_str() {
                "k" | "kelvin" => s(1.0),
                "c" | "°c" | "celsius" | "degc" => Some(UnitScale::new(1.0, 273.15)),
                "f" | "°f" | "fahrenheit" | "degf" => {
                    Some(UnitScale::new(5.0 / 9.0, 255.372_222_222_222_2))
                }
                _ => None,
            },
            QuantityKind::Position => match u.as_str() {
                "m" | "meter" | "metre" | "meters" | "metres" => s(1.0),
                "km" => s(1000.0),
                "cm" => s(0.01),
                "mm" => s(0.001),
                "ft" | "feet" => s(0.3048),
                "mi" | "mile" | "miles" => s(1609.344),
                _ => None,
            },
            QuantityKind::Velocity => match u.as_str() {
                "m/s" | "mps" | "m/s^1" => s(1.0),
                "km/h" | "kmh" | "kph" => s(1.0 / 3.6),
                "kt" | "kts" | "knot" | "knots" => s(0.514_444_444_444_444_4),
                "ft/s" | "fps" => s(0.3048),
                "mph" => s(0.44704),
                _ => None,
            },
            QuantityKind::Angle => match u.as_str() {
                "rad" | "radian" | "radians" => s(1.0),
                "deg" | "°" | "degree" | "degrees" => s(std::f64::consts::PI / 180.0),
                _ => None,
            },
            QuantityKind::Force => match u.as_str() {
                "n" | "newton" | "newtons" => s(1.0),
                "kn" => s(1000.0),
                "lbf" | "lb" | "pound" | "pounds" => s(4.448_221_615_260_5),
                "kgf" => s(9.80665),
                _ => None,
            },
            QuantityKind::Moment => match u.as_str() {
                "n*m" | "nm" | "n-m" | "newtonmeter" => s(1.0),
                "lbf*ft" | "lbfft" | "ft*lbf" => s(1.355_817_948_331_4),
                _ => None,
            },
            QuantityKind::Mass => match u.as_str() {
                "kg" | "kilogram" | "kilograms" => s(1.0),
                "g" | "gram" | "grams" => s(0.001),
                "lb" | "lbs" | "pound" | "pounds" => s(0.453_592_37),
                "slug" | "slugs" => s(14.593_902_94),
                _ => None,
            },
            QuantityKind::Density => match u.as_str() {
                "kg/m^3" | "kg/m3" | "kg/m³" => s(1.0),
                "slug/ft^3" | "slug/ft³" => s(515.378_818_393_537),
                _ => None,
            },
            QuantityKind::Voltage => match u.as_str() {
                "v" | "volt" | "volts" => s(1.0),
                "mv" => s(0.001),
                _ => None,
            },
            QuantityKind::Current => match u.as_str() {
                "a" | "amp" | "amps" | "ampere" => s(1.0),
                "ma" => s(0.001),
                _ => None,
            },
            QuantityKind::Ratio => match u.as_str() {
                "1" | "" | "-" | "unitless" | "none" => s(1.0),
                "%" | "percent" => s(0.01),
                _ => None,
            },
            QuantityKind::Time => match u.as_str() {
                "s" | "sec" | "secs" | "second" | "seconds" => s(1.0),
                "ms" => s(0.001),
                "us" | "µs" => s(1e-6),
                "min" | "minutes" => s(60.0),
                _ => None,
            },
            QuantityKind::Attitude | QuantityKind::Count | QuantityKind::Raw => match u.as_str() {
                "1" | "" | "-" | "unitless" | "none" => s(1.0),
                _ => None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: Real, b: Real, tol: Real) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn length_round_trips() {
        let l = Length::from_km(1.5);
        assert!(approx(l.m(), 1500.0, 1e-9));
        assert!(approx(l.ft(), 4_921.259_842_519_685, 1e-6));
        assert!(approx(Length::from_ft(1000.0).m(), 304.8, 1e-9));
    }

    #[test]
    fn velocity_conversions() {
        let v = Velocity::from_kmh(36.0);
        assert!(approx(v.mps(), 10.0, 1e-12));
        assert!(approx(v.kmh(), 36.0, 1e-12));
        assert!(approx(v.mach(340.0), 10.0 / 340.0, 1e-12));
    }

    #[test]
    fn angular_rate_conversions() {
        let r = AngularRate::from_deg_s(90.0);
        assert!(approx(r.rad_s(), std::f64::consts::FRAC_PI_2, 1e-12));
        assert!(approx(
            AngularRate::from_rpm(60.0).rad_s(),
            std::f64::consts::TAU,
            1e-12
        ));
    }

    #[test]
    fn pressure_conversions() {
        assert!(approx(Pressure::from_bar(1.01325).pa(), 101_325.0, 1e-6));
        assert!(approx(Pressure::from_hpa(1013.25).pa(), 101_325.0, 1e-6));
    }

    #[test]
    fn unit_aliases_use_the_real_symbols_a_logger_writes() {
        // A header such as "temp (degC)" or "wz (deg/s)" is common, and so is the
        // proper symbol. Both must resolve, which they only do if the alias table
        // holds the same characters the file does.
        let scale = QuantityKind::Temperature
            .scale_for("°C")
            .expect("the degree sign is a temperature alias");
        assert!(approx(scale.factor, 1.0, 1e-12));
        assert!(approx(scale.offset, 273.15, 1e-12));
        assert!(QuantityKind::Temperature.scale_for("°F").is_some());

        let degrees = QuantityKind::Angle
            .scale_for("°")
            .expect("a bare degree sign is an angle alias");
        assert!(approx(degrees.factor, std::f64::consts::PI / 180.0, 1e-15));

        assert!(QuantityKind::Acceleration.scale_for("m/s²").is_some());
        assert!(QuantityKind::Density.scale_for("kg/m³").is_some());
        assert!(QuantityKind::MagneticField.scale_for("µT").is_some());
        assert!(QuantityKind::Time.scale_for("µs").is_some());

        // The spelled out forms keep working, so a logger without the symbols is
        // not disadvantaged.
        assert!(QuantityKind::Temperature.scale_for("degC").is_some());
        assert!(QuantityKind::Time.scale_for("us").is_some());
    }

    #[test]
    fn angle_wrapping() {
        let a = Angle::from_deg(370.0).wrapped();
        assert!(approx(a.deg(), 10.0, 1e-9));
        let b = Angle::from_deg(-190.0).wrapped();
        assert!(approx(b.deg(), 170.0, 1e-9));
        let c = Angle::from_deg(180.0).wrapped();
        assert!(approx(c.deg(), 180.0, 1e-9));
    }

    #[test]
    fn temperature_offsets() {
        assert!(approx(Temperature::from_c(20.0).k(), 293.15, 1e-12));
        assert!(approx(Temperature::from_k(273.15).c(), 0.0, 1e-12));
    }

    #[test]
    fn scale_lookup_is_explicit_about_unknown_units() {
        assert!(QuantityKind::Acceleration
            .scale_for("furlongs/fortnight")
            .is_none());
        let g = QuantityKind::Acceleration.scale_for("g").unwrap();
        assert!(approx(g.apply(1.0), STANDARD_GRAVITY, 1e-12));

        let c = QuantityKind::Temperature.scale_for("C").unwrap();
        assert!(approx(c.apply(0.0), 273.15, 1e-12));

        let dps = QuantityKind::AngularRate.scale_for("deg/s").unwrap();
        assert!(approx(dps.apply(180.0), std::f64::consts::PI, 1e-12));
    }

    #[test]
    fn unit_scale_inverts() {
        let scale = UnitScale::new(3.0, 5.0);
        let si = scale.apply(7.0);
        assert!(approx(si, 26.0, 1e-12));
        assert!(approx(scale.invert(si), 7.0, 1e-12));
    }
}
