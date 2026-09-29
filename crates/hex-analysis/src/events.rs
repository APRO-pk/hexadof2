//! Automatic event detection in recorded flight data.
//!
//! These detectors work on an already-recorded history, so replay and comparison
//! can find events without re-running the simulation. They are independent of
//! [`hex_dynamics::events`], which detects events while integrating.
//!
//! # Honesty about inference
//!
//! A detected event is an inference from one channel, and it carries the
//! confidence that channel supports. A barometric apogee and an accelerometer
//! apogee are different claims, and a report must not present them as equally
//! certain.

use hex_core::{Real, Vec3};
use serde::{Deserialize, Serialize};

/// How an event was inferred.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventEvidence {
    /// Read directly from a channel that measures the quantity.
    Direct,
    /// Inferred from a change of sign in a derived quantity.
    SignChange,
    /// Inferred from a peak in a channel.
    Peak,
    /// Inferred from an integral of another channel.
    Integrated,
    /// Entered by the user.
    Manual,
}

impl EventEvidence {
    pub fn label(self) -> &'static str {
        match self {
            EventEvidence::Direct => "directly measured",
            EventEvidence::SignChange => "inferred from a sign change",
            EventEvidence::Peak => "inferred from a peak",
            EventEvidence::Integrated => "inferred by integration",
            EventEvidence::Manual => "added by the user",
        }
    }

    /// Confidence in the inference, between 0 and 1.
    ///
    /// An integration is the weakest evidence because the result drifts, so it is
    /// deliberately rated lower than a direct measurement.
    pub fn confidence(self) -> Real {
        match self {
            EventEvidence::Direct => 1.0,
            EventEvidence::Peak => 0.9,
            EventEvidence::SignChange => 0.8,
            EventEvidence::Integrated => 0.4,
            EventEvidence::Manual => 1.0,
        }
    }
}

/// A detected event in recorded data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetectedEvent {
    /// Event code, matching the simulation event codes where possible.
    pub code: String,
    /// Label for the timeline.
    pub label: String,
    /// Time in seconds on the series timebase.
    pub time: Real,
    /// Channel that produced the event.
    pub channel: String,
    /// Value of that channel at the event.
    pub value: Real,
    pub evidence: EventEvidence,
    /// Whether the user added the event.
    pub user_added: bool,
    /// Note about the reliability of this detection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl DetectedEvent {
    pub fn new(
        code: impl Into<String>,
        label: impl Into<String>,
        time: Real,
        channel: impl Into<String>,
        value: Real,
        evidence: EventEvidence,
    ) -> Self {
        Self {
            code: code.into(),
            label: label.into(),
            time,
            channel: channel.into(),
            value,
            evidence,
            user_added: false,
            note: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// A user marker.
    pub fn marker(time: Real, label: impl Into<String>) -> Self {
        Self {
            code: "custom".to_string(),
            label: label.into(),
            time,
            channel: "user".to_string(),
            value: 0.0,
            evidence: EventEvidence::Manual,
            user_added: true,
            note: None,
        }
    }

    /// How much this detection should be trusted.
    pub fn confidence(&self) -> Real {
        self.evidence.confidence()
    }

    /// Whether the event is reliable enough to use for alignment.
    pub fn is_reliable_for_alignment(&self) -> bool {
        self.confidence() >= 0.7
    }

    pub fn one_line(&self) -> String {
        format!(
            "{:>9.3} s  {:<26} {} ({})",
            self.time,
            self.label,
            self.channel,
            self.evidence.label()
        )
    }
}

/// A sampled channel the detectors read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    /// Channel name.
    pub name: String,
    /// Time values in seconds, increasing.
    pub times: Vec<Real>,
    /// Sample values, aligned with `times`.
    pub values: Vec<Real>,
    /// Unit label for the report.
    pub unit: String,
}

impl Signal {
    pub fn new(
        name: impl Into<String>,
        times: Vec<Real>,
        values: Vec<Real>,
        unit: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            times,
            values,
            unit: unit.into(),
        }
    }

    /// Whether the signal can be used.
    pub fn is_usable(&self) -> bool {
        self.times.len() == self.values.len() && self.times.len() >= 3
    }

    /// Peak value and the time at which it occurred.
    pub fn peak(&self) -> Option<(Real, Real)> {
        if !self.is_usable() {
            return None;
        }
        let mut best = 0usize;
        for (i, v) in self.values.iter().enumerate() {
            if v.is_finite() && (self.values[best].is_nan() || *v > self.values[best]) {
                best = i;
            }
        }
        Some((self.values[best], self.times[best]))
    }

    /// First time the series exceeds a threshold, with linear interpolation.
    pub fn first_crossing_above(&self, threshold: Real) -> Option<(Real, Real)> {
        if !self.is_usable() {
            return None;
        }
        for i in 1..self.times.len() {
            let a = self.values[i - 1];
            let b = self.values[i];
            if !a.is_finite() || !b.is_finite() {
                continue;
            }
            if a <= threshold && b > threshold && b > a {
                let f = (threshold - a) / (b - a);
                let t = self.times[i - 1] + f * (self.times[i] - self.times[i - 1]);
                return Some((t, threshold));
            }
        }
        None
    }

    /// First time the series falls to or below a threshold.
    pub fn first_crossing_below(&self, threshold: Real) -> Option<(Real, Real)> {
        if !self.is_usable() {
            return None;
        }
        for i in 1..self.times.len() {
            let a = self.values[i - 1];
            let b = self.values[i];
            if !a.is_finite() || !b.is_finite() {
                continue;
            }
            if a > threshold && b <= threshold && b < a {
                let f = (a - threshold) / (a - b);
                let t = self.times[i - 1] + f * (self.times[i] - self.times[i - 1]);
                return Some((t, threshold));
            }
        }
        None
    }

    /// First downward zero crossing, with interpolation.
    pub fn descending_zero_crossing(&self) -> Option<(Real, Real)> {
        if !self.is_usable() {
            return None;
        }
        for i in 1..self.times.len() {
            let a = self.values[i - 1];
            let b = self.values[i];
            if !a.is_finite() || !b.is_finite() {
                continue;
            }
            if a > 0.0 && b <= 0.0 && b < a {
                let f = a / (a - b);
                let t = self.times[i - 1] + f * (self.times[i] - self.times[i - 1]);
                return Some((t, 0.0));
            }
        }
        None
    }

    /// Interpolated value at a time, or `None` outside the recorded range.
    pub fn at(&self, time: Real) -> Option<Real> {
        if !self.is_usable() {
            return None;
        }
        if time < self.times[0] || time > self.times[self.times.len() - 1] {
            return None;
        }
        let idx = match self.times.partition_point(|t| *t <= time) {
            0 => 0,
            i if i >= self.times.len() => self.times.len() - 1,
            i => i - 1,
        };
        if idx + 1 >= self.times.len() {
            return Some(self.values[self.times.len() - 1]);
        }
        let span = self.times[idx + 1] - self.times[idx];
        if span <= 0.0 {
            return Some(self.values[idx]);
        }
        let f = (time - self.times[idx]) / span;
        Some(self.values[idx] * (1.0 - f) + self.values[idx + 1] * f)
    }

    /// Trapezoidal integral of the channel over its own timebase.
    ///
    /// Used for a velocity inferred from acceleration, which is why the result is
    /// always tagged as integrated evidence.
    pub fn integrate(&self, initial: Real) -> Vec<Real> {
        let n = self.times.len().min(self.values.len());
        let mut out = Vec::with_capacity(n);
        if n == 0 {
            return out;
        }
        let mut acc = initial;
        out.push(acc);
        for i in 1..n {
            let dt = self.times[i] - self.times[i - 1];
            let a = self.values[i - 1];
            let b = self.values[i];
            if dt.is_finite() && dt > 0.0 && a.is_finite() && b.is_finite() {
                acc += 0.5 * (a + b) * dt;
            }
            out.push(acc);
        }
        out
    }

    /// A signal of the magnitude of a three-component set.
    pub fn magnitude(
        name: impl Into<String>,
        times: Vec<Real>,
        components: &[Vec3],
        unit: impl Into<String>,
    ) -> Self {
        let values = components.iter().map(|v| v.norm()).collect();
        Self::new(name, times, values, unit)
    }
}

/// Detection options.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EventDetectionOptions {
    /// Thrust magnitude above which the motor is considered lit, in newtons.
    pub ignition_thrust_threshold: Real,
    /// Dynamic pressure below which a peak search is not considered, in pascals.
    pub minimum_dynamic_pressure: Real,
    /// Altitude below which a reversal is treated as settling rather than apogee,
    /// in metres.
    pub minimum_apogee_altitude: Real,
    /// Vertical speed magnitude above which a crossing counts, in m/s.
    pub apogee_velocity_threshold: Real,
    /// Whether to infer lift-off from acceleration when no rail channel exists.
    pub infer_liftoff_from_acceleration: bool,
    /// Specific force above the static value, in m/s^2, that marks lift-off.
    pub liftoff_specific_force_margin: Real,
}

impl Default for EventDetectionOptions {
    fn default() -> Self {
        Self {
            ignition_thrust_threshold: 1.0,
            minimum_dynamic_pressure: 50.0,
            minimum_apogee_altitude: 1.0,
            apogee_velocity_threshold: 0.5,
            infer_liftoff_from_acceleration: false,
            liftoff_specific_force_margin: 2.0,
        }
    }
}

/// The channels the detector reads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FlightSignals {
    /// Barometric or GNSS altitude in metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub altitude: Option<Signal>,
    /// Vertical velocity in metres per second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical_velocity: Option<Signal>,
    /// Total specific force magnitude in metres per second squared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specific_force: Option<Signal>,
    /// Dynamic pressure in pascals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_pressure: Option<Signal>,
    /// Thrust or motor command.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thrust: Option<Signal>,
    /// Pressure in pascals, used for a pressure-altitude fallback.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure: Option<Signal>,
}

impl FlightSignals {
    /// Which quantities are available.
    pub fn available(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.altitude.is_some() {
            out.push("altitude");
        }
        if self.vertical_velocity.is_some() {
            out.push("vertical velocity");
        }
        if self.specific_force.is_some() {
            out.push("specific force");
        }
        if self.dynamic_pressure.is_some() {
            out.push("dynamic pressure");
        }
        if self.thrust.is_some() {
            out.push("thrust");
        }
        if self.pressure.is_some() {
            out.push("pressure");
        }
        out
    }

    /// Whether enough channels are present to detect anything.
    pub fn is_usable(&self) -> bool {
        self.altitude.is_some() || self.vertical_velocity.is_some() || self.specific_force.is_some()
    }
}

/// Detect the standard flight events from recorded signals.
///
/// Every detection records which channel produced it and how strong the evidence
/// is. Events that cannot be detected are simply absent, rather than being
/// reported at a guessed time.
pub fn detect_events(
    signals: &FlightSignals,
    options: &EventDetectionOptions,
) -> Vec<DetectedEvent> {
    let mut events = Vec::new();

    // Motor burnout and ignition from the thrust channel.
    if let Some(thrust) = &signals.thrust {
        if thrust.is_usable() {
            if let Some((time, value)) =
                thrust.first_crossing_above(options.ignition_thrust_threshold)
            {
                events.push(DetectedEvent::new(
                    "ignition",
                    "Ignition",
                    time,
                    thrust.name.clone(),
                    value,
                    EventEvidence::Direct,
                ));
            }
            // Burnout is the first return to zero after the peak.
            if let Some((peak, _)) = thrust.peak() {
                if peak > options.ignition_thrust_threshold {
                    let after_peak = Signal::new(
                        thrust.name.clone(),
                        thrust.times.clone(),
                        thrust.values.clone(),
                        thrust.unit.clone(),
                    );
                    if let Some((time, value)) =
                        after_peak.first_crossing_below(options.ignition_thrust_threshold)
                    {
                        events.push(DetectedEvent::new(
                            "burnout",
                            "Motor burnout",
                            time,
                            thrust.name.clone(),
                            value,
                            EventEvidence::Direct,
                        ));
                    }
                }
            }
        }
    }

    // Lift-off from vertical velocity, or from specific force as a fallback.
    if let Some(vz) = &signals.vertical_velocity {
        if vz.is_usable() {
            if let Some((time, value)) = vz.first_crossing_above(0.5) {
                events.push(DetectedEvent::new(
                    "lift_off",
                    "Lift-off",
                    time,
                    vz.name.clone(),
                    value,
                    EventEvidence::SignChange,
                ));
            }
        }
    } else if options.infer_liftoff_from_acceleration {
        if let Some(sf) = &signals.specific_force {
            if sf.is_usable() {
                let threshold = hex_core::STANDARD_GRAVITY + options.liftoff_specific_force_margin;
                if let Some((time, value)) = sf.first_crossing_above(threshold) {
                    events.push(
                        DetectedEvent::new(
                            "lift_off",
                            "Lift-off",
                            time,
                            sf.name.clone(),
                            value,
                            EventEvidence::SignChange,
                        )
                        .with_note(
                            "Inferred from specific force because no vertical velocity channel is available. A rail exit or a handling bump can produce the same signature.",
                        ),
                    );
                }
            }
        }
    }

    // Maximum dynamic pressure.
    if let Some(q) = &signals.dynamic_pressure {
        if q.is_usable() {
            if let Some((peak, time)) = q.peak() {
                if peak >= options.minimum_dynamic_pressure {
                    events.push(DetectedEvent::new(
                        "max_q",
                        "Maximum dynamic pressure",
                        time,
                        q.name.clone(),
                        peak,
                        EventEvidence::Peak,
                    ));
                }
            }
        }
    }

    // Apogee from altitude, preferring a vertical velocity sign change because it
    // is not affected by a barometer's lag.
    let mut apogee_from_velocity = false;
    if let Some(vz) = &signals.vertical_velocity {
        if vz.is_usable() {
            if let Some((time, _)) = vz.descending_zero_crossing() {
                if let Some(altitude) = &signals.altitude {
                    if let Some(alt) = altitude.at(time) {
                        if alt >= options.minimum_apogee_altitude {
                            events.push(DetectedEvent::new(
                                "apogee",
                                "Apogee",
                                time,
                                vz.name.clone(),
                                alt,
                                EventEvidence::SignChange,
                            ));
                            apogee_from_velocity = true;
                        }
                    }
                } else {
                    events.push(
                        DetectedEvent::new(
                            "apogee",
                            "Apogee",
                            time,
                            vz.name.clone(),
                            0.0,
                            EventEvidence::SignChange,
                        )
                        .with_note(
                            "No altitude channel is available to confirm the apogee height.",
                        ),
                    );
                    apogee_from_velocity = true;
                }
            }
        }
    }
    if !apogee_from_velocity {
        if let Some(altitude) = &signals.altitude {
            if altitude.is_usable() {
                if let Some((peak, time)) = altitude.peak() {
                    if peak >= options.minimum_apogee_altitude {
                        events.push(
                            DetectedEvent::new(
                                "apogee",
                                "Apogee",
                                time,
                                altitude.name.clone(),
                                peak,
                                EventEvidence::Peak,
                            )
                            .with_note(
                                "Inferred from the altitude peak because no vertical velocity channel is available. A pressure sensor's thermal lag shifts this estimate.",
                            ),
                        );
                    }
                }
            }
        }
    }

    // Ground impact.
    if let Some(altitude) = &signals.altitude {
        if altitude.is_usable() {
            // Only look after apogee, so a pre-launch reading at zero is not a
            // ground impact.
            let search_from = events
                .iter()
                .find(|e| e.code == "apogee")
                .map(|e| e.time)
                .unwrap_or(altitude.times[0]);
            for i in 1..altitude.times.len() {
                if altitude.times[i - 1] < search_from {
                    continue;
                }
                let a = altitude.values[i - 1];
                let b = altitude.values[i];
                if a.is_finite() && b.is_finite() && a > 0.0 && b <= 0.0 && b < a {
                    let f = a / (a - b);
                    let time =
                        altitude.times[i - 1] + f * (altitude.times[i] - altitude.times[i - 1]);
                    events.push(DetectedEvent::new(
                        "ground_impact",
                        "Ground impact",
                        time,
                        altitude.name.clone(),
                        0.0,
                        EventEvidence::SignChange,
                    ));
                    break;
                }
            }
        }
    }

    events.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    events
}

/// Infer apogee directly from an acceleration history by integration.
///
/// This is the weakest form of apogee detection and the result is tagged as
/// integrated evidence. It exists because a rocket with no barometer and no GNSS
/// still has an accelerometer, and an approximate apogee is more useful than
/// none, provided the uncertainty is stated.
pub fn infer_apogee_from_acceleration(
    times: &[Real],
    vertical_acceleration: &[Real],
    specific_force: Option<&[Real]>,
    initial_velocity: Real,
) -> Option<DetectedEvent> {
    if times.len() != vertical_acceleration.len() || times.len() < 3 {
        return None;
    }
    // Remove the gravity component when a specific force channel is available,
    // because the accelerometer measures specific force, not acceleration.
    let net: Vec<Real> = match specific_force {
        Some(sf) if sf.len() == vertical_acceleration.len() => {
            sf.iter().map(|f| *f - hex_core::STANDARD_GRAVITY).collect()
        }
        _ => vertical_acceleration.to_vec(),
    };
    let signal = Signal::new("net vertical acceleration", times.to_vec(), net, "m/s^2");
    let velocities = signal.integrate(initial_velocity);
    let velocity = Signal::new(
        "integrated vertical velocity",
        times.to_vec(),
        velocities,
        "m/s",
    );
    let (time, _) = velocity.descending_zero_crossing()?;
    Some(
        DetectedEvent::new(
            "apogee",
            "Apogee",
            time,
            velocity.name.clone(),
            0.0,
            EventEvidence::Integrated,
        )
        .with_note(
            "Position and velocity from integrating an accelerometer drift substantially, so this apogee time is approximate. Treat it as indicative.",
        ),
    )
}

/// The events worth offering as alignment anchors, best evidence first.
pub fn alignment_anchors(events: &[DetectedEvent]) -> Vec<&DetectedEvent> {
    let mut anchors: Vec<&DetectedEvent> = events
        .iter()
        .filter(|e| e.is_reliable_for_alignment())
        .collect();
    anchors.sort_by(|a, b| {
        b.confidence()
            .partial_cmp(&a.confidence())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.time
                    .partial_cmp(&b.time)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    anchors
}

/// A timeline summary for the event panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventTimeline {
    pub events: Vec<DetectedEvent>,
    /// Duration of the recorded series, seconds.
    pub duration: Real,
}

impl EventTimeline {
    pub fn new(events: Vec<DetectedEvent>, duration: Real) -> Self {
        Self { events, duration }
    }

    /// The event with the given code, first occurrence.
    pub fn get(&self, code: &str) -> Option<&DetectedEvent> {
        self.events.iter().find(|e| e.code == code)
    }

    /// Time difference between two event codes.
    pub fn interval(&self, from: &str, to: &str) -> Option<Real> {
        let a = self.get(from)?;
        let b = self.get(to)?;
        Some(b.time - a.time)
    }

    /// Event position as a fraction of the total duration, for the timeline ruler.
    pub fn fraction(&self, index: usize) -> Option<Real> {
        let e = self.events.get(index)?;
        if self.duration <= 0.0 {
            return Some(0.0);
        }
        Some((e.time / self.duration).clamp(0.0, 1.0))
    }

    /// A plain-text rendering for the report.
    pub fn to_text(&self) -> String {
        let mut out = String::from("Event timeline\n==============\n");
        if self.events.is_empty() {
            out.push_str("No events were detected.\n");
            return out;
        }
        for e in &self.events {
            out.push_str(&e.one_line());
            out.push('\n');
            if let Some(note) = &e.note {
                out.push_str(&format!("  note: {}\n", note));
            }
        }
        out
    }

    /// Time from lift-off to apogee, a common figure of merit.
    pub fn ascent_duration(&self) -> Option<Real> {
        self.interval("lift_off", "apogee")
    }

    /// Time from apogee to ground impact, when both exist.
    pub fn descent_duration(&self) -> Option<Real> {
        self.interval("apogee", "ground_impact")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_times(n: usize, dt: Real) -> Vec<Real> {
        (0..n).map(|i| i as Real * dt).collect()
    }

    /// A parabolic altitude history with its apogee at `apogee_time`.
    fn parabolic_altitude(apogee_time: Real, peak: Real, dt: Real, n: usize) -> Signal {
        let times = ramp_times(n, dt);
        let values = times
            .iter()
            .map(|t| peak - 4.0 * (t - apogee_time) * (t - apogee_time))
            .collect();
        Signal::new("altitude", times, values, "m")
    }

    #[test]
    fn signal_peak_finds_the_maximum() {
        let s = Signal::new("x", vec![0.0, 1.0, 2.0, 3.0], vec![1.0, 5.0, 3.0, 2.0], "m");
        let (value, time) = s.peak().unwrap();
        assert!((value - 5.0).abs() < 1e-12);
        assert!((time - 1.0).abs() < 1e-12);
    }

    #[test]
    fn signal_peak_needs_enough_samples() {
        let s = Signal::new("x", vec![0.0], vec![1.0], "m");
        assert!(!s.is_usable());
        assert!(s.peak().is_none());
    }

    #[test]
    fn crossing_above_is_interpolated() {
        let s = Signal::new("x", vec![0.0, 1.0, 2.0], vec![0.0, 10.0, 20.0], "m");
        let (time, value) = s.first_crossing_above(5.0).unwrap();
        assert!((time - 0.5).abs() < 1e-12);
        assert!((value - 5.0).abs() < 1e-12);
        assert!(s.first_crossing_above(100.0).is_none());
    }

    #[test]
    fn crossing_below_is_interpolated() {
        let s = Signal::new("x", vec![0.0, 1.0, 2.0], vec![20.0, 10.0, 0.0], "N");
        let (time, _) = s.first_crossing_below(5.0).unwrap();
        assert!((time - 1.5).abs() < 1e-12);
    }

    #[test]
    fn descending_zero_crossing_finds_the_apogee_instant() {
        let s = Signal::new(
            "vz",
            vec![0.0, 1.0, 2.0, 3.0],
            vec![5.0, 2.0, -1.0, -4.0],
            "m/s",
        );
        let (time, _) = s.descending_zero_crossing().unwrap();
        // The crossing is two thirds of the way from t = 1 to t = 2.
        assert!((time - (1.0 + 2.0 / 3.0)).abs() < 1e-12);
    }

    #[test]
    fn descending_zero_crossing_ignores_a_rising_crossing() {
        let s = Signal::new("vz", vec![0.0, 1.0, 2.0], vec![-5.0, 0.0, 5.0], "m/s");
        assert!(s.descending_zero_crossing().is_none());
    }

    #[test]
    fn signal_at_interpolates_and_refuses_extrapolation() {
        let s = Signal::new("x", vec![0.0, 1.0, 2.0], vec![0.0, 10.0, 20.0], "m");
        assert_eq!(s.at(0.25), Some(2.5));
        assert_eq!(s.at(5.0), None);
        assert_eq!(s.at(-1.0), None);
    }

    #[test]
    fn integration_of_a_constant_acceleration_is_exact() {
        let time = ramp_times(11, 0.1);
        let s = Signal::new("a", time.clone(), vec![2.0; 11], "m/s^2");
        let v = s.integrate(0.0);
        assert_eq!(v.len(), 11);
        for (i, value) in v.iter().enumerate() {
            let expected = 2.0 * time[i];
            assert!(
                (value - expected).abs() < 1e-9,
                "at {} got {}",
                time[i],
                value
            );
        }
        // A non-zero initial condition is carried.
        let v = s.integrate(5.0);
        assert!((v[0] - 5.0).abs() < 1e-12);
    }

    #[test]
    fn magnitude_signal_of_components() {
        let times = vec![0.0, 1.0];
        let components = vec![Vec3::new(3.0, 4.0, 0.0), Vec3::new(0.0, 0.0, 12.0)];
        let s = Signal::magnitude("accel", times, &components, "m/s^2");
        assert!((s.values[0] - 5.0).abs() < 1e-12);
        assert!((s.values[1] - 12.0).abs() < 1e-12);
    }

    #[test]
    fn ignition_and_burnout_are_detected_from_thrust() {
        let times = vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5];
        let values = vec![0.0, 0.0, 100.0, 100.0, 0.0, 0.0];
        let signals = FlightSignals {
            thrust: Some(Signal::new("thrust", times, values, "N")),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        let ignition = events
            .iter()
            .find(|e| e.code == "ignition")
            .expect("ignition");
        // The step lands between the 0.1 s and 0.2 s samples, so the interpolated
        // crossing sits inside that interval rather than exactly on a sample.
        assert!(
            ignition.time > 0.1 && ignition.time <= 0.2,
            "ignition at {}",
            ignition.time
        );
        assert_eq!(ignition.evidence, EventEvidence::Direct);
        // Burnout is the first return below the threshold after the peak.
        let burnout = events
            .iter()
            .find(|e| e.code == "burnout")
            .expect("burnout");
        assert!(burnout.time > 0.3 && burnout.time <= 0.4);
    }

    #[test]
    fn burnout_is_not_reported_without_ignition() {
        let times = vec![0.0, 0.1, 0.2];
        let values = vec![0.0, 0.0, 0.0];
        let signals = FlightSignals {
            thrust: Some(Signal::new("thrust", times, values, "N")),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        assert!(events.iter().all(|e| e.code != "burnout"));
    }

    #[test]
    fn liftoff_is_detected_from_vertical_velocity() {
        let times = vec![0.0, 0.1, 0.2, 0.3];
        let values = vec![0.0, 0.2, 1.0, 3.0];
        let signals = FlightSignals {
            vertical_velocity: Some(Signal::new("vz", times, values, "m/s")),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        let liftoff = events
            .iter()
            .find(|e| e.code == "lift_off")
            .expect("lift-off");
        assert_eq!(liftoff.evidence, EventEvidence::SignChange);
        assert!(liftoff.time > 0.1 && liftoff.time < 0.3);
    }

    #[test]
    fn liftoff_from_acceleration_is_flagged_as_inferred() {
        let times = vec![0.0, 0.1, 0.2, 0.3];
        let values = vec![9.8, 15.0, 40.0, 40.0];
        let signals = FlightSignals {
            specific_force: Some(Signal::new("accel", times, values, "m/s^2")),
            ..Default::default()
        };
        let options = EventDetectionOptions {
            infer_liftoff_from_acceleration: true,
            liftoff_specific_force_margin: 2.0,
            ..Default::default()
        };
        let events = detect_events(&signals, &options);
        let liftoff = events
            .iter()
            .find(|e| e.code == "lift_off")
            .expect("lift-off");
        assert!(liftoff.note.as_ref().unwrap().contains("Inferred"));
    }

    #[test]
    fn liftoff_is_not_inferred_when_the_option_is_off() {
        let times = vec![0.0, 0.1, 0.2];
        let values = vec![9.8, 40.0, 40.0];
        let signals = FlightSignals {
            specific_force: Some(Signal::new("accel", times, values, "m/s^2")),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        assert!(events.iter().all(|e| e.code != "lift_off"));
    }

    #[test]
    fn max_dynamic_pressure_is_detected_above_the_floor() {
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let values = vec![0.0, 500.0, 9000.0, 400.0];
        let signals = FlightSignals {
            dynamic_pressure: Some(Signal::new("q", times, values, "Pa")),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        let max_q = events.iter().find(|e| e.code == "max_q").expect("max q");
        assert!((max_q.time - 2.0).abs() < 1e-12);
        assert!((max_q.value - 9000.0).abs() < 1e-12);
        assert_eq!(max_q.evidence, EventEvidence::Peak);
    }

    #[test]
    fn max_dynamic_pressure_below_the_floor_is_ignored() {
        let times = vec![0.0, 1.0, 2.0];
        let values = vec![0.0, 10.0, 5.0];
        let signals = FlightSignals {
            dynamic_pressure: Some(Signal::new("q", times, values, "Pa")),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        assert!(events.iter().all(|e| e.code != "max_q"));
    }

    #[test]
    fn apogee_prefers_the_velocity_crossing() {
        let times = vec![0.0, 1.0, 2.0, 3.0, 4.0];
        let altitude = Signal::new(
            "altitude",
            times.clone(),
            vec![0.0, 50.0, 90.0, 80.0, 40.0],
            "m",
        );
        let vz = Signal::new(
            "vz",
            times.clone(),
            vec![60.0, 40.0, -5.0, -30.0, -50.0],
            "m/s",
        );
        let signals = FlightSignals {
            altitude: Some(altitude),
            vertical_velocity: Some(vz),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        let apogee = events.iter().find(|e| e.code == "apogee").expect("apogee");
        assert_eq!(apogee.evidence, EventEvidence::SignChange);
        assert!(apogee.note.is_none());
        // The crossing is two thirds from t = 1 to t = 2.
        assert!((apogee.time - (1.0 + 40.0 / 45.0)).abs() < 1e-9);
    }

    #[test]
    fn apogee_falls_back_to_the_altitude_peak_with_a_note() {
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let altitude = Signal::new("altitude", times, vec![0.0, 50.0, 90.0, 40.0], "m");
        let signals = FlightSignals {
            altitude: Some(altitude),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        let apogee = events.iter().find(|e| e.code == "apogee").expect("apogee");
        assert_eq!(apogee.evidence, EventEvidence::Peak);
        assert!(apogee.note.as_ref().unwrap().contains("thermal lag"));
    }

    #[test]
    fn apogee_below_the_minimum_altitude_is_ignored() {
        let times = vec![0.0, 0.5, 1.0];
        let altitude = Signal::new("altitude", times.clone(), vec![0.0, 0.5, 0.0], "m");
        let vz = Signal::new("vz", times, vec![5.0, -1.0, -5.0], "m/s");
        let signals = FlightSignals {
            altitude: Some(altitude),
            vertical_velocity: Some(vz),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        assert!(events.iter().all(|e| e.code != "apogee"));
    }

    #[test]
    fn ground_impact_is_detected_after_apogee() {
        let times = vec![0.0, 1.0, 2.0, 3.0, 4.0];
        let altitude = Signal::new("altitude", times, vec![0.0, 50.0, 90.0, 30.0, -1.0], "m");
        let signals = FlightSignals {
            altitude: Some(altitude),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        let impact = events
            .iter()
            .find(|e| e.code == "ground_impact")
            .expect("ground impact");
        assert!(impact.time > 3.0 && impact.time <= 4.0);
    }

    #[test]
    fn a_zero_altitude_before_apogee_is_not_a_ground_impact() {
        let times = vec![0.0, 0.5, 1.0, 2.0];
        let altitude = Signal::new("altitude", times, vec![0.0, 0.0, 20.0, 10.0], "m");
        let signals = FlightSignals {
            altitude: Some(altitude),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        assert!(events.iter().all(|e| e.code != "ground_impact"));
    }

    #[test]
    fn events_are_returned_in_time_order() {
        let times = vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5];
        let signals = FlightSignals {
            thrust: Some(Signal::new(
                "thrust",
                times.clone(),
                vec![0.0, 100.0, 100.0, 0.0, 0.0, 0.0],
                "N",
            )),
            vertical_velocity: Some(Signal::new(
                "vz",
                times.clone(),
                vec![0.0, 5.0, 8.0, -2.0, -10.0, -20.0],
                "m/s",
            )),
            altitude: Some(Signal::new(
                "altitude",
                times,
                vec![0.0, 10.0, 40.0, 55.0, 40.0, 0.0],
                "m",
            )),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        assert!(events.len() >= 4);
        for w in events.windows(2) {
            assert!(w[0].time <= w[1].time, "{:?} before {:?}", w[0], w[1]);
        }
    }

    #[test]
    fn no_signals_produces_no_events() {
        let signals = FlightSignals::default();
        assert!(!signals.is_usable());
        assert!(detect_events(&signals, &EventDetectionOptions::default()).is_empty());
        assert!(signals.available().is_empty());
    }

    #[test]
    fn available_lists_the_signals_that_are_present() {
        let signals = FlightSignals {
            altitude: Some(parabolic_altitude(2.0, 100.0, 0.1, 50)),
            ..Default::default()
        };
        assert_eq!(signals.available(), vec!["altitude"]);
    }

    #[test]
    fn integrated_apogee_is_tagged_as_weak_evidence() {
        let times = ramp_times(60, 0.1);
        // 30 m/s^2 of specific force for 1 s then a coast: the net acceleration
        // is 20 m/s^2 while burning and -9.8 afterwards.
        // While burning, a net 20.2 m/s^2 upward means the accelerometer reads
        // 30.0 m/s^2 of specific force. In free fall the accelerometer reads zero,
        // which leaves a net -g and decelerates the vehicle.
        let specific: Vec<Real> = times
            .iter()
            .map(|t| if *t < 1.0 { 30.0 } else { 0.0 })
            .collect();
        let event = infer_apogee_from_acceleration(&times, &specific, Some(&specific), 0.0)
            .expect("apogee");
        assert_eq!(event.evidence, EventEvidence::Integrated);
        assert!(event.confidence() < 0.7);
        assert!(!event.is_reliable_for_alignment());
        assert!(event.note.as_ref().unwrap().contains("drift"));
        // Burn for 1 s at a net 20.2 m/s^2 gives 20.2 m/s, and a coast at 9.8
        // reaches apogee about 2 s later.
        assert!(
            event.time > 2.5 && event.time < 3.5,
            "apogee at {}",
            event.time
        );
    }

    #[test]
    fn integrated_apogee_needs_an_upward_phase() {
        let times = ramp_times(20, 0.1);
        // A specific force equal to gravity at every sample means the vehicle is
        // in free fall and never climbs, so there is no apogee to report.
        let specific = vec![9.80665; 20];
        assert!(infer_apogee_from_acceleration(&times, &specific, Some(&specific), 0.0).is_none());
    }

    #[test]
    fn integrated_apogee_from_a_net_acceleration_channel() {
        // When the caller has already removed gravity, the net acceleration can be
        // integrated directly. This is the case a research user with a modelled
        // gravity vector would be in.
        let times = ramp_times(60, 0.1);
        let net: Vec<Real> = times
            .iter()
            .map(|t| if *t < 1.0 { 20.2 } else { -9.80665 })
            .collect();
        let event = infer_apogee_from_acceleration(&times, &net, None, 0.0).expect("apogee");
        assert_eq!(event.evidence, EventEvidence::Integrated);
        assert!(
            event.time > 2.5 && event.time < 3.5,
            "apogee at {}",
            event.time
        );
    }

    #[test]
    fn alignment_anchors_rank_by_confidence() {
        let events = vec![
            DetectedEvent::new(
                "apogee",
                "Apogee",
                5.0,
                "vz",
                0.0,
                EventEvidence::Integrated,
            ),
            DetectedEvent::new(
                "burnout",
                "Burnout",
                2.0,
                "thrust",
                0.0,
                EventEvidence::Direct,
            ),
            DetectedEvent::new("max_q", "Max Q", 3.0, "q", 0.0, EventEvidence::Peak),
        ];
        let anchors = alignment_anchors(&events);
        assert_eq!(anchors.len(), 2);
        assert_eq!(anchors[0].code, "burnout");
        assert_eq!(anchors[1].code, "max_q");
    }

    #[test]
    fn timeline_intervals_and_fractions() {
        let events = vec![
            DetectedEvent::new(
                "lift_off",
                "Lift-off",
                2.0,
                "vz",
                0.0,
                EventEvidence::SignChange,
            ),
            DetectedEvent::new(
                "apogee",
                "Apogee",
                12.0,
                "vz",
                0.0,
                EventEvidence::SignChange,
            ),
            DetectedEvent::new(
                "ground_impact",
                "Ground impact",
                40.0,
                "altitude",
                0.0,
                EventEvidence::SignChange,
            ),
        ];
        let timeline = EventTimeline::new(events, 40.0);
        assert!((timeline.ascent_duration().unwrap() - 10.0).abs() < 1e-12);
        assert!((timeline.descent_duration().unwrap() - 28.0).abs() < 1e-12);
        assert!((timeline.fraction(0).unwrap() - 0.05).abs() < 1e-12);
        assert!((timeline.fraction(2).unwrap() - 1.0).abs() < 1e-12);
        assert!(timeline.fraction(9).is_none());
        assert!(timeline.get("apogee").is_some());
        assert!(timeline.interval("lift_off", "ground_impact").is_some());
        assert!(timeline.interval("lift_off", "nope").is_none());
    }

    #[test]
    fn timeline_text_includes_notes() {
        let events = vec![DetectedEvent::new(
            "apogee",
            "Apogee",
            5.0,
            "altitude",
            100.0,
            EventEvidence::Peak,
        )
        .with_note("barometer lag")];
        let timeline = EventTimeline::new(events, 10.0);
        let text = timeline.to_text();
        assert!(text.contains("Event timeline"));
        assert!(text.contains("barometer lag"));
        assert!(EventTimeline::new(vec![], 1.0)
            .to_text()
            .contains("No events"));
    }

    #[test]
    fn zero_duration_timeline_does_not_divide_by_zero() {
        let events = vec![DetectedEvent::new(
            "apogee",
            "Apogee",
            5.0,
            "altitude",
            100.0,
            EventEvidence::Peak,
        )];
        let timeline = EventTimeline::new(events, 0.0);
        assert_eq!(timeline.fraction(0), Some(0.0));
    }

    #[test]
    fn user_markers_are_manual_evidence() {
        let m = DetectedEvent::marker(3.0, "Camera start");
        assert!(m.user_added);
        assert_eq!(m.evidence, EventEvidence::Manual);
        assert_eq!(m.confidence(), 1.0);
        assert!(m.is_reliable_for_alignment());
    }

    #[test]
    fn evidence_confidence_ordering_is_documented() {
        assert_eq!(EventEvidence::Direct.confidence(), 1.0);
        assert!(EventEvidence::Peak.confidence() > EventEvidence::SignChange.confidence());
        assert!(EventEvidence::SignChange.confidence() > EventEvidence::Integrated.confidence());
        for e in [
            EventEvidence::Direct,
            EventEvidence::SignChange,
            EventEvidence::Peak,
            EventEvidence::Integrated,
            EventEvidence::Manual,
        ] {
            assert!(!e.label().is_empty());
        }
    }

    #[test]
    fn signal_with_a_non_finite_sample_does_not_break_detection() {
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let values = vec![0.0, Real::NAN, 9000.0, 10.0];
        let signals = FlightSignals {
            dynamic_pressure: Some(Signal::new("q", times, values, "Pa")),
            ..Default::default()
        };
        let events = detect_events(&signals, &EventDetectionOptions::default());
        let max_q = events.iter().find(|e| e.code == "max_q").unwrap();
        assert!((max_q.time - 2.0).abs() < 1e-12);
    }
}
