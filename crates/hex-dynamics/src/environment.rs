//! Environment models: gravity, atmosphere, wind, ground, and launch rail.
//!
//! Every model here is explicit about what it assumes, because a simulation
//! result is only as trustworthy as the environment it was run in. Gravity is
//! uniform by default and must be switched to a central model deliberately. The
//! atmosphere is a real layered standard atmosphere, not a single exponential
//! fit, and the UI labels it as a standard-day model rather than measured air.

use hex_core::{Real, Vec3, EARTH_MU, EARTH_RADIUS, STANDARD_GRAVITY};
use serde::{Deserialize, Serialize};

/// The gravity model used for a run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum GravityModel {
    /// A uniform local field of the given magnitude, pointing along the world
    /// down axis. This is the default and is accurate enough near the ground.
    Uniform { magnitude: Real },
    /// Uniform gravity plus a linear decrease with altitude at the standard
    /// lapse rate. A middle ground for high-altitude flights.
    LinearLapse {
        sea_level: Real,
        lapse_per_metre: Real,
    },
    /// Central inverse-square gravity about a spherical Earth.
    ///
    /// The world frame origin is treated as a point on the surface, so the
    /// geocentric radius is `EARTH_RADIUS + altitude`.
    Central { mu: Real, earth_radius: Real },
}

impl Default for GravityModel {
    fn default() -> Self {
        GravityModel::Uniform {
            magnitude: STANDARD_GRAVITY,
        }
    }
}

impl GravityModel {
    /// Standard uniform gravity.
    pub fn standard() -> Self {
        GravityModel::default()
    }

    /// Uniform gravity with an explicit magnitude.
    pub fn uniform(magnitude: Real) -> Self {
        GravityModel::Uniform { magnitude }
    }

    /// Spherical Earth central gravity with the standard gravitational
    /// parameter and mean radius.
    pub fn central_earth() -> Self {
        GravityModel::Central {
            mu: EARTH_MU,
            earth_radius: EARTH_RADIUS,
        }
    }

    /// Acceleration vector in the world frame at the given position.
    ///
    /// `world_down` is the unit vector of the world frame that points toward the
    /// Earth, and `position` is measured from the launch point in the world
    /// frame.
    pub fn acceleration(&self, position: Vec3, world_down: Vec3) -> Vec3 {
        match *self {
            GravityModel::Uniform { magnitude } => world_down * magnitude,
            GravityModel::LinearLapse {
                sea_level,
                lapse_per_metre,
            } => {
                // Altitude is the component of position along world up.
                let altitude = -position.dot(&world_down);
                let magnitude = (sea_level + lapse_per_metre * altitude).max(0.0);
                world_down * magnitude
            }
            GravityModel::Central { mu, earth_radius } => {
                // Radial vector from the Earth centre to the vehicle.
                let up = -world_down;
                let r = up * earth_radius + position;
                let range = r.norm();
                if range < 1.0 {
                    return world_down * STANDARD_GRAVITY;
                }
                // g = -mu * r / |r|^3, written as an inward unit vector scaled.
                -r * (mu / (range * range * range))
            }
        }
    }

    /// Human-readable label shown in the run configuration.
    pub fn label(&self) -> String {
        match *self {
            GravityModel::Uniform { magnitude } => {
                format!("Uniform, {:.5} m/s^2", magnitude)
            }
            GravityModel::LinearLapse {
                sea_level,
                lapse_per_metre,
            } => format!(
                "Uniform with altitude lapse, {:.5} m/s^2 at {:.3e} per m",
                sea_level, lapse_per_metre
            ),
            GravityModel::Central { mu, earth_radius } => format!(
                "Central inverse square, mu = {:.6e} m^3/s^2, R = {:.0} m",
                mu, earth_radius
            ),
        }
    }

    /// Nominal sea-level magnitude, used when initialising displays.
    pub fn nominal_magnitude(&self) -> Real {
        match *self {
            GravityModel::Uniform { magnitude } => magnitude,
            GravityModel::LinearLapse { sea_level, .. } => sea_level,
            GravityModel::Central { mu, earth_radius } => mu / (earth_radius * earth_radius),
        }
    }

    /// Whether this model varies with position.
    pub fn is_position_dependent(&self) -> bool {
        !matches!(self, GravityModel::Uniform { .. })
    }
}

/// A layer of the standard atmosphere.
#[derive(Debug, Clone, Copy, PartialEq)]
struct AtmosphereLayer {
    /// Base geopotential altitude of the layer, m.
    base_altitude: Real,
    /// Temperature at the base, K.
    base_temperature: Real,
    /// Temperature gradient, K per metre. Zero for an isothermal layer.
    lapse_rate: Real,
}

/// The 1976 US Standard Atmosphere, defined up to 86 km.
///
/// This is a *standard day* model. It is not a weather model and the UI must say
/// so; a hot day or a low-pressure day changes drag noticeably.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StandardAtmosphere {
    /// Temperature offset applied to every layer, in K. Zero is the standard day.
    pub temperature_offset: Real,
    /// Pressure multiplier applied at sea level. One is the standard day.
    pub pressure_scale: Real,
    /// Sea-level pressure used, in Pa.
    pub sea_level_pressure: Real,
}

impl Default for StandardAtmosphere {
    fn default() -> Self {
        Self {
            temperature_offset: 0.0,
            pressure_scale: 1.0,
            sea_level_pressure: 101_325.0,
        }
    }
}

/// Geopotential layer boundaries and lapse rates of the 1976 standard atmosphere.
const LAYERS: [AtmosphereLayer; 7] = [
    AtmosphereLayer {
        base_altitude: 0.0,
        base_temperature: 288.15,
        lapse_rate: -0.0065,
    },
    AtmosphereLayer {
        base_altitude: 11_000.0,
        base_temperature: 216.65,
        lapse_rate: 0.0,
    },
    AtmosphereLayer {
        base_altitude: 20_000.0,
        base_temperature: 216.65,
        lapse_rate: 0.001,
    },
    AtmosphereLayer {
        base_altitude: 32_000.0,
        base_temperature: 228.65,
        lapse_rate: 0.0028,
    },
    AtmosphereLayer {
        base_altitude: 47_000.0,
        base_temperature: 270.65,
        lapse_rate: 0.0,
    },
    AtmosphereLayer {
        base_altitude: 51_000.0,
        base_temperature: 270.65,
        lapse_rate: -0.0028,
    },
    AtmosphereLayer {
        base_altitude: 71_000.0,
        base_temperature: 214.65,
        lapse_rate: -0.002,
    },
];

/// Top of the modelled atmosphere, m.
pub const ATMOSPHERE_TOP: Real = 84_852.0;

/// Universal gas constant for dry air, J/(kg K).
pub const R_AIR: Real = 287.052_874;

/// Ratio of specific heats for air.
pub const GAMMA_AIR: Real = 1.4;

/// Sea-level speed of sound for the standard atmosphere, m/s.
pub const SEA_LEVEL_SOUND_SPEED: Real = 340.294;

impl StandardAtmosphere {
    /// Atmosphere at a standard sea-level day.
    pub fn standard_day() -> Self {
        Self::default()
    }

    /// Temperature in kelvin at geometric altitude `altitude`, metres.
    ///
    /// The geopotential correction is applied, so a high-altitude result is not
    /// biased by the variation of gravity with height.
    pub fn temperature(&self, altitude: Real) -> Real {
        let geopotential = geopotential_altitude(altitude);
        if geopotential <= 0.0 {
            return LAYERS[0].base_temperature + self.temperature_offset;
        }
        if geopotential >= ATMOSPHERE_TOP {
            return 186.87 + self.temperature_offset;
        }
        let layer = self.layer_for(geopotential);
        let t = layer.base_temperature + layer.lapse_rate * (geopotential - layer.base_altitude);
        (t + self.temperature_offset).max(1.0)
    }

    /// The layer containing a geopotential altitude.
    ///
    /// A layer spans from its base altitude inclusive to the next layer's base
    /// exclusive, which is the ISA convention. A sample sitting exactly on a
    /// boundary therefore belongs to the layer above it, so the lapse rate does
    /// not leak a fractional step across the boundary.
    fn layer_for(&self, geopotential: Real) -> AtmosphereLayer {
        for (i, layer) in LAYERS.iter().enumerate() {
            let top = LAYERS
                .get(i + 1)
                .map(|l| l.base_altitude)
                .unwrap_or(Real::INFINITY);
            if geopotential >= layer.base_altitude && geopotential < top {
                return *layer;
            }
        }
        LAYERS[LAYERS.len() - 1]
    }

    /// Pressure in pascals at geometric altitude `altitude`, metres.
    pub fn pressure(&self, altitude: Real) -> Real {
        let geopotential = geopotential_altitude(altitude);
        let mut pressure = self.sea_level_pressure * self.pressure_scale;

        for (i, layer) in LAYERS.iter().enumerate() {
            let next_base = LAYERS
                .get(i + 1)
                .map(|l| l.base_altitude)
                .unwrap_or(ATMOSPHERE_TOP);
            if geopotential <= layer.base_altitude {
                break;
            }
            // Layer base conditions, using the supplied temperature offset so
            // pressure and density stay consistent with `temperature`.
            let base_t = layer.base_temperature + self.temperature_offset;
            let top = geopotential.min(next_base);
            let dh = top - layer.base_altitude;
            if dh <= 0.0 {
                continue;
            }
            if layer.lapse_rate.abs() < 1e-12 {
                pressure *= (-STANDARD_GRAVITY * dh / (R_AIR * base_t)).exp();
            } else {
                let t_top = base_t + layer.lapse_rate * dh;
                if t_top <= 0.0 {
                    return 0.0;
                }
                pressure *= (base_t / t_top).powf(STANDARD_GRAVITY / (R_AIR * layer.lapse_rate));
            }
            if geopotential <= next_base {
                break;
            }
        }

        // Above the modelled range, decay to a negligible but finite value.
        if geopotential > ATMOSPHERE_TOP {
            let excess = geopotential - ATMOSPHERE_TOP;
            pressure *= (-excess / 7000.0).exp();
        }
        pressure.max(0.0)
    }

    /// Density in kilograms per cubic metre at geometric altitude `altitude`.
    pub fn density(&self, altitude: Real) -> Real {
        let p = self.pressure(altitude);
        let t = self.temperature(altitude);
        if t <= 0.0 {
            return 0.0;
        }
        p / (R_AIR * t)
    }

    /// Local speed of sound in metres per second.
    pub fn speed_of_sound(&self, altitude: Real) -> Real {
        (GAMMA_AIR * R_AIR * self.temperature(altitude)).sqrt()
    }

    /// Dynamic viscosity from Sutherland's law, Pa s.
    pub fn dynamic_viscosity(&self, altitude: Real) -> Real {
        let t = self.temperature(altitude);
        let sutherland = 110.4;
        let mu0 = 1.716e-5;
        let t0 = 273.15;
        mu0 * (t / t0).powf(1.5) * (t0 + sutherland) / (t + sutherland)
    }

    /// All the air properties at once, which is what the force models need.
    pub fn sample(&self, altitude: Real) -> AirProperties {
        AirProperties {
            altitude,
            temperature: self.temperature(altitude),
            pressure: self.pressure(altitude),
            density: self.density(altitude),
            speed_of_sound: self.speed_of_sound(altitude),
            dynamic_viscosity: self.dynamic_viscosity(altitude),
        }
    }

    /// A short label for the run metadata.
    pub fn label(&self) -> String {
        if self.temperature_offset.abs() < 1e-9 && (self.pressure_scale - 1.0).abs() < 1e-9 {
            "US Standard Atmosphere 1976 (standard day)".to_string()
        } else {
            format!(
                "US Standard Atmosphere 1976, {:+.1} K offset, pressure x{:.4}",
                self.temperature_offset, self.pressure_scale
            )
        }
    }
}

/// Air properties at one altitude.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AirProperties {
    /// Geometric altitude in metres.
    pub altitude: Real,
    /// Temperature in kelvin.
    pub temperature: Real,
    /// Pressure in pascals.
    pub pressure: Real,
    /// Density in kilograms per cubic metre.
    pub density: Real,
    /// Speed of sound in metres per second.
    pub speed_of_sound: Real,
    /// Dynamic viscosity in pascal seconds.
    pub dynamic_viscosity: Real,
}

/// Convert geometric altitude to geopotential altitude, metres.
pub fn geopotential_altitude(altitude: Real) -> Real {
    // h = R * z / (R + z)
    EARTH_RADIUS * altitude / (EARTH_RADIUS + altitude)
}

/// An atmosphere model choice.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum AtmosphereModel {
    /// Treat the air as absent. Useful for vacuum or drag-free verification.
    Vacuum,
    /// The layered standard atmosphere.
    Standard(StandardAtmosphere),
    /// A constant density and temperature, for a deliberately simplified run.
    Constant {
        density: Real,
        temperature: Real,
        speed_of_sound: Real,
    },
}

impl Default for AtmosphereModel {
    fn default() -> Self {
        AtmosphereModel::Standard(StandardAtmosphere::default())
    }
}

impl AtmosphereModel {
    pub fn standard_day() -> Self {
        AtmosphereModel::default()
    }

    pub fn is_vacuum(&self) -> bool {
        matches!(self, AtmosphereModel::Vacuum)
    }

    /// Sample the air at an altitude.
    pub fn sample(&self, altitude: Real) -> AirProperties {
        match *self {
            AtmosphereModel::Vacuum => AirProperties {
                altitude,
                temperature: 0.0,
                pressure: 0.0,
                density: 0.0,
                speed_of_sound: 1.0,
                dynamic_viscosity: 0.0,
            },
            AtmosphereModel::Standard(a) => a.sample(altitude),
            AtmosphereModel::Constant {
                density,
                temperature,
                speed_of_sound,
            } => AirProperties {
                altitude,
                temperature,
                pressure: density * R_AIR * temperature,
                density,
                speed_of_sound: speed_of_sound.max(1e-6),
                dynamic_viscosity: 1.8e-5,
            },
        }
    }

    pub fn label(&self) -> String {
        match self {
            AtmosphereModel::Vacuum => "Vacuum, no aerodynamic forces".to_string(),
            AtmosphereModel::Standard(a) => a.label(),
            AtmosphereModel::Constant {
                density,
                temperature,
                ..
            } => format!(
                "Constant air, density {:.4} kg/m^3, temperature {:.2} K",
                density, temperature
            ),
        }
    }
}

/// A wind field expressed in the world frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum WindModel {
    /// Still air.
    #[default]
    Calm,
    /// A constant wind vector in the world frame, metres per second.
    Constant { velocity: Vec3 },
    /// A constant direction whose speed follows a power law with altitude.
    ///
    /// `speed = reference_speed * (altitude / reference_altitude) ^ exponent`,
    /// clamped below `reference_altitude` so the wind does not blow up at the
    /// pad. This is the common engineering approximation, not a measured profile.
    PowerLaw {
        /// Horizontal wind direction as a unit vector in the world frame.
        direction: Vec3,
        reference_speed: Real,
        reference_altitude: Real,
        exponent: Real,
        /// Vertical component as a fraction of the horizontal speed.
        vertical_fraction: Real,
    },
    /// A tabulated profile keyed on altitude.
    Table {
        /// Altitudes in metres, strictly increasing.
        altitudes: Vec<Real>,
        /// Wind vectors in the world frame at each altitude.
        velocities: Vec<Vec3>,
    },
}

impl WindModel {
    /// A constant horizontal wind of the given speed and direction.
    pub fn constant(speed: Real, direction: Vec3) -> Self {
        let d = if direction.norm() > 1e-12 {
            hex_core::normalized(direction)
        } else {
            Vec3::x()
        };
        WindModel::Constant {
            velocity: d * speed,
        }
    }

    /// Wind velocity in the world frame at the given altitude.
    ///
    /// `world_up` is the unit up axis of the world frame.
    pub fn velocity(&self, altitude: Real, world_up: Vec3) -> Vec3 {
        match *self {
            WindModel::Calm => Vec3::zeros(),
            WindModel::Constant { velocity } => velocity,
            WindModel::PowerLaw {
                direction,
                reference_speed,
                reference_altitude,
                exponent,
                vertical_fraction,
            } => {
                let d = if direction.norm() > 1e-12 {
                    hex_core::normalized(direction)
                } else {
                    Vec3::x()
                };
                let reference_altitude = reference_altitude.max(1.0);
                let clamped = altitude.max(reference_altitude * 0.05);
                let speed = reference_speed * (clamped / reference_altitude).powf(exponent);
                d * speed + world_up * (speed * vertical_fraction)
            }
            WindModel::Table {
                ref altitudes,
                ref velocities,
            } => {
                let n = altitudes.len();
                if n == 0 || n != velocities.len() {
                    return Vec3::zeros();
                }
                if altitude <= altitudes[0] {
                    return velocities[0];
                }
                if altitude >= altitudes[n - 1] {
                    return velocities[n - 1];
                }
                let idx = match altitudes.partition_point(|a| *a <= altitude) {
                    0 => 0,
                    i if i >= n => n - 2,
                    i => i - 1,
                };
                let span = altitudes[idx + 1] - altitudes[idx];
                let f = if span <= 0.0 {
                    0.0
                } else {
                    (altitude - altitudes[idx]) / span
                };
                velocities[idx] * (1.0 - f) + velocities[idx + 1] * f
            }
        }
    }

    /// Nominal wind speed used in the run summary, in m/s.
    pub fn nominal_speed(&self) -> Real {
        match *self {
            WindModel::Calm => 0.0,
            WindModel::Constant { velocity } => velocity.norm(),
            WindModel::PowerLaw {
                reference_speed, ..
            } => reference_speed.abs(),
            WindModel::Table { ref velocities, .. } => {
                velocities.iter().map(|v| v.norm()).fold(0.0, Real::max)
            }
        }
    }

    pub fn label(&self) -> String {
        match self {
            WindModel::Calm => "Calm".to_string(),
            WindModel::Constant { velocity } => {
                format!("Constant wind {:.2} m/s", velocity.norm())
            }
            WindModel::PowerLaw {
                reference_speed,
                exponent,
                ..
            } => format!(
                "Power law wind, {:.2} m/s reference, exponent {:.2}",
                reference_speed, exponent
            ),
            WindModel::Table { altitudes, .. } => {
                format!("Tabulated wind, {} levels", altitudes.len())
            }
        }
    }
}

/// A launch rail or tube constraint.
///
/// While the constraint is active the vehicle's rotation and its motion
/// perpendicular to the rail are held, and only translation along the rail is
/// integrated. The constraint releases after the rail length is travelled.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LaunchRail {
    /// Unit direction of the rail in the world frame, pointing away from the pad.
    pub direction: Vec3,
    /// Rail length in metres, measured along the rail.
    pub length: Real,
    /// Whether the constraint is enabled for this run.
    pub enabled: bool,
}

impl LaunchRail {
    /// A vertical rail of the given length.
    pub fn vertical(length: Real, world_up: Vec3) -> Self {
        Self {
            direction: hex_core::normalized(world_up),
            length,
            enabled: true,
        }
    }

    /// A rail tilted by `elevation` radians from horizontal and `azimuth`
    /// radians from the world X axis, with `world_up` as the vertical axis.
    pub fn tilted(
        elevation: Real,
        azimuth: Real,
        length: Real,
        world_up: Vec3,
        world_north: Vec3,
    ) -> Self {
        // Build an orthonormal horizontal pair from the supplied axes.
        let east = world_north.cross(&world_up);
        let east = if east.norm() > 1e-9 {
            hex_core::normalized(east)
        } else {
            Vec3::x()
        };
        let north = world_up.cross(&hex_core::normalized(east));
        let horizontal = north * azimuth.cos() + east * azimuth.sin();
        let direction = horizontal * elevation.cos() + world_up * elevation.sin();
        Self {
            direction,
            length,
            enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            direction: Vec3::z(),
            length: 0.0,
            enabled: false,
        }
    }

    /// Unit direction, normalised defensively.
    pub fn unit_direction(&self) -> Vec3 {
        if self.direction.norm() > 1e-12 {
            hex_core::normalized(self.direction)
        } else {
            Vec3::z()
        }
    }

    /// Distance travelled along the rail from the launch point.
    pub fn distance_along(&self, position: Vec3, launch_point: Vec3) -> Real {
        (position - launch_point).dot(&self.unit_direction())
    }

    /// Whether the vehicle has travelled far enough to leave the rail.
    pub fn has_released(&self, position: Vec3, launch_point: Vec3) -> bool {
        !self.enabled || self.distance_along(position, launch_point) >= self.length
    }

    pub fn label(&self) -> String {
        if !self.enabled {
            return "No launch constraint".to_string();
        }
        let d = self.unit_direction();
        let elevation = d.dot(&Vec3::z()).clamp(-1.0, 1.0).asin().to_degrees();
        format!(
            "Launch rail, {:.2} m, elevation {:.1} deg from world Z",
            self.length, elevation
        )
    }
}

/// The ground plane and the events tied to it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GroundPlane {
    /// Whether the ground plane is active for events and collision.
    pub enabled: bool,
    /// Local terrain elevation relative to the modelled sea-level datum, metres.
    pub elevation: Real,
}

impl Default for GroundPlane {
    fn default() -> Self {
        Self {
            enabled: true,
            elevation: 0.0,
        }
    }
}

/// The complete environment for a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Environment {
    pub gravity: GravityModel,
    pub atmosphere: AtmosphereModel,
    pub wind: WindModel,
    pub rail: LaunchRail,
    pub ground: GroundPlane,
    /// Air density source the user selected, recorded for reproducibility.
    pub air_density_source: AirDensitySource,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            gravity: GravityModel::default(),
            atmosphere: AtmosphereModel::default(),
            wind: WindModel::default(),
            rail: LaunchRail::disabled(),
            ground: GroundPlane::default(),
            air_density_source: AirDensitySource::StandardAtmosphere,
        }
    }
}

impl Environment {
    /// A vacuum environment with uniform gravity, used by verification tests.
    pub fn vacuum_uniform_gravity(magnitude: Real) -> Self {
        Self {
            gravity: GravityModel::uniform(magnitude),
            atmosphere: AtmosphereModel::Vacuum,
            wind: WindModel::Calm,
            rail: LaunchRail::disabled(),
            ground: GroundPlane {
                enabled: false,
                elevation: 0.0,
            },
            air_density_source: AirDensitySource::Vacuum,
        }
    }

    /// Air properties at a world-frame position along the supplied up axis.
    pub fn air_at(&self, position: Vec3, world_up: Vec3) -> AirProperties {
        let altitude = position.dot(&world_up) + self.ground.elevation;
        self.atmosphere.sample(altitude)
    }

    /// Wind velocity at a world-frame position.
    pub fn wind_at(&self, position: Vec3, world_up: Vec3) -> Vec3 {
        let altitude = position.dot(&world_up) + self.ground.elevation;
        self.wind.velocity(altitude, world_up)
    }

    /// A multi-line description for the run metadata panel.
    pub fn describe(&self) -> Vec<String> {
        vec![
            format!("Gravity: {}", self.gravity.label()),
            format!("Atmosphere: {}", self.atmosphere.label()),
            format!("Wind: {}", self.wind.label()),
            format!("Constraint: {}", self.rail.label()),
            format!(
                "Ground: {}",
                if self.ground.enabled {
                    format!("plane at {:.2} m", self.ground.elevation)
                } else {
                    "disabled".to_string()
                }
            ),
        ]
    }
}

/// Where the air density used by the drag model came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirDensitySource {
    /// The layered standard atmosphere.
    #[default]
    StandardAtmosphere,
    /// A constant density supplied by the user.
    Constant,
    /// No air at all.
    Vacuum,
}

impl AirDensitySource {
    pub fn label(self) -> &'static str {
        match self {
            AirDensitySource::StandardAtmosphere => "Standard atmosphere",
            AirDensitySource::Constant => "Constant density",
            AirDensitySource::Vacuum => "Vacuum",
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
    fn uniform_gravity_points_down() {
        let g = GravityModel::uniform(9.81);
        let down = Vec3::new(0.0, 0.0, -1.0);
        assert!((g.acceleration(Vec3::zeros(), down) - Vec3::new(0.0, 0.0, -9.81)).norm() < 1e-12);
        assert!(!g.is_position_dependent());
    }

    #[test]
    fn linear_lapse_reduces_gravity_with_altitude() {
        let g = GravityModel::LinearLapse {
            sea_level: 9.80665,
            lapse_per_metre: -3.086e-6,
        };
        let down = Vec3::new(0.0, 0.0, -1.0);
        let ground = g.acceleration(Vec3::zeros(), down).norm();
        let high = g.acceleration(Vec3::new(0.0, 0.0, 100_000.0), down).norm();
        assert!(high < ground);
        assert!(approx(ground - high, 0.3086, 1e-3));
    }

    #[test]
    fn central_gravity_matches_surface_value() {
        let g = GravityModel::central_earth();
        let down = Vec3::new(0.0, 0.0, -1.0);
        let a = g.acceleration(Vec3::zeros(), down);
        assert!((a.norm() - 9.82).abs() < 0.02, "got {}", a.norm());
        assert!((a - down * a.norm()).norm() < 1e-9);
    }

    #[test]
    fn central_gravity_falls_off_with_altitude() {
        let g = GravityModel::central_earth();
        let down = Vec3::new(0.0, 0.0, -1.0);
        let surface = g.acceleration(Vec3::zeros(), down).norm();
        let orbit = g.acceleration(Vec3::new(0.0, 0.0, 400_000.0), down).norm();
        assert!(orbit < surface);
        let ratio = orbit / surface;
        let expected = (EARTH_RADIUS / (EARTH_RADIUS + 400_000.0)).powi(2);
        assert!((ratio - expected).abs() < 1e-6);
    }

    #[test]
    fn standard_atmosphere_sea_level_values() {
        let a = StandardAtmosphere::default();
        assert!(approx(a.temperature(0.0), 288.15, 1e-9));
        assert!(approx(a.pressure(0.0), 101_325.0, 1e-6));
        assert!(approx(a.density(0.0), 1.225, 0.002));
        assert!(approx(a.speed_of_sound(0.0), 340.294, 0.01));
    }

    #[test]
    fn standard_atmosphere_tropopause_is_isothermal() {
        let a = StandardAtmosphere::default();

        // The ISA layer table is defined on geopotential altitude, so the
        // 216.65 K tropopause sits at geopotential 11 km, which is a geometric
        // altitude slightly above 11 km.
        let geometric_for_11km_geopotential = EARTH_RADIUS * 11_000.0 / (EARTH_RADIUS - 11_000.0);
        assert!(approx(
            a.temperature(geometric_for_11km_geopotential),
            216.65,
            0.01
        ));

        // Inside the isothermal layer the temperature is flat.
        let t12 = a.temperature(12_000.0);
        let t15 = a.temperature(15_000.0);
        assert!(approx(t12, 216.65, 0.01), "t(12 km) = {}", t12);
        assert!(approx(t12, t15, 0.01));

        // The tropospheric lapse rate is 6.5 K per km of geopotential.
        let t10 = a.temperature(10_000.0);
        assert!(approx(t10, 223.15, 0.2), "t(10 km) = {}", t10);
    }

    #[test]
    fn standard_atmosphere_known_pressure_ratios() {
        let a = StandardAtmosphere::default();
        // The standard atmosphere gives about 26.4 kPa at 10 km.
        let p10 = a.pressure(10_000.0);
        assert!((p10 - 26_436.0).abs() < 120.0, "p(10km) = {}", p10);
        // And about 5.47 kPa at 20 km.
        let p20 = a.pressure(20_000.0);
        assert!((p20 - 5_474.9).abs() < 90.0, "p(20km) = {}", p20);
    }

    #[test]
    fn density_decreases_monotonically() {
        let a = StandardAtmosphere::default();
        let mut last = a.density(0.0);
        let mut altitude = 0.0;
        while altitude < 80_000.0 {
            altitude += 1000.0;
            let d = a.density(altitude);
            assert!(d <= last + 1e-12, "density rose at {}", altitude);
            last = d;
        }
    }

    #[test]
    fn temperature_offset_shifts_the_profile() {
        let base = StandardAtmosphere::default();
        let hot = StandardAtmosphere {
            temperature_offset: 15.0,
            ..Default::default()
        };
        assert!(approx(
            hot.temperature(0.0) - base.temperature(0.0),
            15.0,
            1e-9
        ));
        // Hotter air at the same pressure is less dense.
        assert!(hot.density(0.0) < base.density(0.0));
    }

    #[test]
    fn vacuum_has_no_air() {
        let a = AtmosphereModel::Vacuum;
        let s = a.sample(1000.0);
        assert_eq!(s.density, 0.0);
        assert_eq!(s.pressure, 0.0);
        assert!(a.is_vacuum());
    }

    #[test]
    fn constant_atmosphere_is_self_consistent() {
        let a = AtmosphereModel::Constant {
            density: 1.0,
            temperature: 300.0,
            speed_of_sound: 340.0,
        };
        let s = a.sample(9999.0);
        assert_eq!(s.density, 1.0);
        assert!(approx(s.pressure, 1.0 * R_AIR * 300.0, 1e-6));
    }

    #[test]
    fn calm_wind_is_zero() {
        let w = WindModel::Calm;
        assert_eq!(w.velocity(100.0, Vec3::z()), Vec3::zeros());
        assert_eq!(w.nominal_speed(), 0.0);
    }

    #[test]
    fn constant_wind_is_independent_of_altitude() {
        let w = WindModel::constant(7.5, Vec3::x());
        let a = w.velocity(0.0, Vec3::z());
        let b = w.velocity(10_000.0, Vec3::z());
        assert!((a - b).norm() < 1e-15);
        assert!(approx(a.norm(), 7.5, 1e-12));
    }

    #[test]
    fn power_law_wind_grows_with_altitude() {
        let w = WindModel::PowerLaw {
            direction: Vec3::x(),
            reference_speed: 5.0,
            reference_altitude: 10.0,
            exponent: 0.14,
            vertical_fraction: 0.0,
        };
        let low = w.velocity(10.0, Vec3::z()).norm();
        let high = w.velocity(100.0, Vec3::z()).norm();
        assert!(approx(low, 5.0, 1e-9));
        assert!(high > low);
        assert!(approx(high, 5.0 * 10f64.powf(0.14), 1e-9));
        // Below the reference altitude the speed is clamped, not diverging.
        assert!(w.velocity(0.1, Vec3::z()).norm().is_finite());
    }

    #[test]
    fn tabulated_wind_interpolates() {
        let w = WindModel::Table {
            altitudes: vec![0.0, 1000.0],
            velocities: vec![Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)],
        };
        let mid = w.velocity(500.0, Vec3::z());
        assert!((mid - Vec3::new(5.0, 0.0, 0.0)).norm() < 1e-12);
        assert!(approx(w.nominal_speed(), 10.0, 1e-12));
        // Outside the table the end values hold.
        assert_eq!(w.velocity(-100.0, Vec3::z()), Vec3::zeros());
    }

    #[test]
    fn vertical_rail_releases_after_its_length() {
        let rail = LaunchRail::vertical(5.0, Vec3::z());
        let pad = Vec3::zeros();
        assert!(!rail.has_released(Vec3::new(0.0, 0.0, 4.9), pad));
        assert!(rail.has_released(Vec3::new(0.0, 0.0, 5.0), pad));
        assert!(approx(
            rail.distance_along(Vec3::new(0.0, 0.0, 3.0), pad),
            3.0,
            1e-12
        ));
    }

    #[test]
    fn disabled_rail_reports_released_immediately() {
        let rail = LaunchRail::disabled();
        assert!(rail.has_released(Vec3::zeros(), Vec3::zeros()));
        assert!(rail.label().contains("No launch"));
    }

    #[test]
    fn tilted_rail_has_the_requested_elevation() {
        let rail = LaunchRail::tilted(0.0, 0.0, 3.0, Vec3::z(), Vec3::y());
        // Zero elevation is horizontal.
        assert!(rail.unit_direction().z.abs() < 1e-9);

        let vertical =
            LaunchRail::tilted(std::f64::consts::FRAC_PI_2, 0.0, 3.0, Vec3::z(), Vec3::y());
        assert!((vertical.unit_direction() - Vec3::z()).norm() < 1e-9);

        let forty_five =
            LaunchRail::tilted(std::f64::consts::FRAC_PI_4, 0.0, 3.0, Vec3::z(), Vec3::y());
        assert!(approx(
            forty_five.unit_direction().z,
            std::f64::consts::FRAC_1_SQRT_2,
            1e-9
        ));
    }

    #[test]
    fn environment_air_uses_ground_elevation() {
        let mut env = Environment::default();
        env.ground.elevation = 1500.0;
        let air = env.air_at(Vec3::zeros(), Vec3::z());
        let reference = StandardAtmosphere::default().sample(1500.0);
        assert!((air.pressure - reference.pressure).abs() < 1e-9);
        assert!((air.density - reference.density).abs() < 1e-12);
    }

    #[test]
    fn environment_describe_lists_every_component() {
        let lines = Environment::default().describe();
        assert_eq!(lines.len(), 5);
        assert!(lines[0].contains("Gravity"));
        assert!(lines[1].contains("Atmosphere"));
    }

    #[test]
    fn vacuum_environment_has_zero_density() {
        let env = Environment::vacuum_uniform_gravity(9.81);
        assert_eq!(env.air_at(Vec3::zeros(), Vec3::z()).density, 0.0);
        assert!(!env.ground.enabled);
    }

    #[test]
    fn geopotential_correction_is_small_at_low_altitude() {
        let h = geopotential_altitude(1000.0);
        assert!(h < 1000.0);
        assert!((h - 999.843).abs() < 0.01);
    }
}
