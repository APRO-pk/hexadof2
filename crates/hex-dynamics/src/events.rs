//! Flight event detection.
//!
//! Events are the vocabulary the results timeline and the comparison views use.
//! Each event records the time it fired, the value that triggered it, and whether
//! it was detected automatically or added by the user. Detection is monotonic in
//! time and every rule fires at most once, so a run's event list is reproducible.

use hex_core::{Real, Vec3};
use serde::{Deserialize, Serialize};

/// The kind of event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Thrust first becomes non-zero.
    Ignition,
    /// The vehicle leaves the pad.
    LiftOff,
    /// The launch constraint releases.
    RailExit,
    /// Maximum dynamic pressure.
    MaximumDynamicPressure,
    /// Thrust returns to zero.
    Burnout,
    /// Vertical velocity changes from positive to negative.
    Apogee,
    /// A recovery device is expected or observed.
    RecoveryDeployment,
    /// A landing burn ignites during the descent.
    LandingBurn,
    /// The vehicle reaches the ground.
    GroundImpact,
    /// The solver detected a non-finite state.
    SolverFailure,
    /// The solver rejected an unusual number of steps.
    SolverWarning,
    /// A phase transition declared by the scenario rather than inferred.
    Custom,
}

impl EventKind {
    pub fn label(self) -> &'static str {
        match self {
            EventKind::Ignition => "Ignition",
            EventKind::LiftOff => "Lift-off",
            EventKind::RailExit => "Rail exit",
            EventKind::MaximumDynamicPressure => "Maximum dynamic pressure",
            EventKind::Burnout => "Motor burnout",
            EventKind::Apogee => "Apogee",
            EventKind::RecoveryDeployment => "Recovery deployment",
            EventKind::LandingBurn => "Landing burn",
            EventKind::GroundImpact => "Ground impact",
            EventKind::SolverFailure => "Solver failure",
            EventKind::SolverWarning => "Solver warning",
            EventKind::Custom => "Custom",
        }
    }

    /// Stable machine code used in saved comparisons and reports.
    pub fn code(self) -> &'static str {
        match self {
            EventKind::Ignition => "ignition",
            EventKind::LiftOff => "lift_off",
            EventKind::RailExit => "rail_exit",
            EventKind::MaximumDynamicPressure => "max_q",
            EventKind::Burnout => "burnout",
            EventKind::Apogee => "apogee",
            EventKind::RecoveryDeployment => "recovery_deployment",
            EventKind::LandingBurn => "landing_burn",
            EventKind::GroundImpact => "ground_impact",
            EventKind::SolverFailure => "solver_failure",
            EventKind::SolverWarning => "solver_warning",
            EventKind::Custom => "custom",
        }
    }

    /// Events that are generated automatically by the detector.
    pub fn is_automatic(self) -> bool {
        !matches!(self, EventKind::Custom | EventKind::RecoveryDeployment)
    }

    /// Whether this event is a warning rather than a flight milestone.
    pub fn is_diagnostic(self) -> bool {
        matches!(self, EventKind::SolverFailure | EventKind::SolverWarning)
    }
}

/// A detected or user-added event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlightEvent {
    /// Stable identifier, unique within a run.
    pub id: String,
    pub kind: EventKind,
    /// Time in seconds from the simulation start.
    pub time: Real,
    /// Short label shown on the timeline.
    pub label: String,
    /// Longer explanation for the event detail panel.
    pub description: String,
    /// The value that triggered the event, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_value: Option<Real>,
    /// The unit of `trigger_value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_unit: Option<String>,
    /// True when the user added the event rather than the detector.
    #[serde(default)]
    pub user_added: bool,
}

impl FlightEvent {
    pub fn new(kind: EventKind, time: Real, label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            id: format!("{}-{:.6}", kind.code(), time),
            kind,
            time,
            description: String::new(),
            label,
            trigger_value: None,
            trigger_unit: None,
            user_added: false,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn with_trigger(mut self, value: Real, unit: impl Into<String>) -> Self {
        self.trigger_value = Some(value);
        self.trigger_unit = Some(unit.into());
        self
    }

    /// A user-added marker, which is never overwritten by re-running detection.
    pub fn marker(time: Real, label: impl Into<String>) -> Self {
        let mut e = Self::new(EventKind::Custom, time, label);
        e.user_added = true;
        e
    }

    /// One-line rendering for a report table.
    pub fn one_line(&self) -> String {
        match (self.trigger_value, &self.trigger_unit) {
            (Some(v), Some(u)) => format!("{:>9.3} s  {}  ({:.4} {})", self.time, self.label, v, u),
            _ => format!("{:>9.3} s  {}", self.time, self.label),
        }
    }
}

/// Live variables an event rule can watch.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EventObservation {
    /// Simulation time, seconds.
    pub time: Real,
    /// Altitude above the ground plane, metres.
    pub altitude: Real,
    /// Vertical velocity along the world up axis, m/s.
    pub vertical_velocity: Real,
    /// Airspeed relative to the wind, m/s.
    pub airspeed: Real,
    /// Dynamic pressure, Pa.
    pub dynamic_pressure: Real,
    /// Current thrust magnitude, N.
    pub thrust: Real,
    /// Current mass, kg.
    pub mass: Real,
    /// Distance travelled along the launch rail, metres.
    pub rail_distance: Real,
    /// Whether the launch constraint is still active.
    pub on_rail: bool,
}

/// A single automatic detection rule with the memory it needs to avoid firing
/// repeatedly.
#[derive(Debug, Clone, PartialEq)]
struct Rule {
    kind: EventKind,
    fired: bool,
    /// Previous sample, used for crossing tests.
    previous: Option<EventObservation>,
    /// Best value seen so far, for peak detection.
    peak: Real,
    /// Highest altitude seen so far, used to reject pad-level apogee reports.
    peak_altitude: Real,
    /// Whether a peak is rising.
    rising: bool,
    /// Last sample that was still climbing above the apogee threshold, used to
    /// place the apogee time between two output samples.
    last_rising: Option<EventObservation>,
}

impl Rule {
    fn new(kind: EventKind) -> Self {
        Self {
            kind,
            fired: false,
            previous: None,
            peak: Real::NEG_INFINITY,
            peak_altitude: Real::NEG_INFINITY,
            rising: false,
            last_rising: None,
        }
    }
}

/// Which automatic rules are enabled for a run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EventRules {
    pub ignition: bool,
    pub lift_off: bool,
    pub rail_exit: bool,
    pub maximum_dynamic_pressure: bool,
    pub burnout: bool,
    pub apogee: bool,
    pub ground_impact: bool,
    pub solver_warning: bool,
    /// Vertical velocity band around zero, metres per second. The vehicle must
    /// have been climbing faster than this before a descent of the same size is
    /// accepted as the apogee, so a wobble on the pad is never reported as one.
    pub apogee_velocity_threshold: Real,
}

impl Default for EventRules {
    fn default() -> Self {
        Self {
            ignition: true,
            lift_off: true,
            rail_exit: true,
            maximum_dynamic_pressure: true,
            burnout: true,
            apogee: true,
            ground_impact: true,
            solver_warning: true,
            apogee_velocity_threshold: 0.5,
        }
    }
}

impl EventRules {
    /// A rule set with only the flight milestones enabled.
    pub fn milestones_only() -> Self {
        Self {
            solver_warning: false,
            ..Self::default()
        }
    }

    /// A rule set with everything enabled.
    pub fn all() -> Self {
        Self::default()
    }
}

/// Stateful automatic event detector.
///
/// Feed it one observation per output sample. It returns the events that fired at
/// that sample, in a deterministic order.
#[derive(Debug, Clone)]
pub struct EventDetector {
    rules: Vec<Rule>,
    enabled: EventRules,
    thrust_was_on: bool,
    rail_exited: bool,
    recovery_deployed: bool,
    /// Optional time at which recovery deployment is expected.
    pub recovery_time: Option<Real>,
    emitted: usize,
}

impl EventDetector {
    pub fn new(enabled: EventRules) -> Self {
        let mut rules = Vec::new();
        if enabled.ignition {
            rules.push(Rule::new(EventKind::Ignition));
        }
        if enabled.lift_off {
            rules.push(Rule::new(EventKind::LiftOff));
        }
        if enabled.rail_exit {
            rules.push(Rule::new(EventKind::RailExit));
        }
        if enabled.maximum_dynamic_pressure {
            rules.push(Rule::new(EventKind::MaximumDynamicPressure));
        }
        if enabled.burnout {
            rules.push(Rule::new(EventKind::Burnout));
        }
        if enabled.apogee {
            rules.push(Rule::new(EventKind::Apogee));
        }
        if enabled.ground_impact {
            rules.push(Rule::new(EventKind::GroundImpact));
        }
        Self {
            rules,
            enabled,
            thrust_was_on: false,
            rail_exited: false,
            recovery_deployed: false,
            recovery_time: None,
            emitted: 0,
        }
    }

    /// Events collected so far, in firing order.
    pub fn is_rule_fired(&self, kind: EventKind) -> bool {
        self.rules
            .iter()
            .find(|r| r.kind == kind)
            .map(|r| r.fired)
            .unwrap_or(false)
    }

    /// Number of events emitted.
    pub fn emitted_count(&self) -> usize {
        self.emitted
    }

    fn next_id(&mut self, kind: EventKind, time: Real) -> String {
        self.emitted += 1;
        format!("{}-{:.6}-{}", kind.code(), time, self.emitted)
    }

    /// Observe one sample and return the events that fired.
    pub fn observe(&mut self, obs: EventObservation) -> Vec<FlightEvent> {
        let mut out = Vec::new();

        // Rule indices are used so the event id counter can be borrowed mutably
        // while the rule list is being read.
        for rule_index in 0..self.rules.len() {
            if self.rules[rule_index].fired {
                continue;
            }
            let kind = self.rules[rule_index].kind;
            let previous = self.rules[rule_index].previous;
            // Rules that resolve a crossing between two samples can move the
            // reported time off the sample boundary.
            let mut fired_at = obs.time;
            let fired = match kind {
                EventKind::Ignition => obs.thrust > 0.0,
                EventKind::LiftOff => {
                    obs.altitude > 0.01 && obs.vertical_velocity > 0.01 && !obs.on_rail
                }
                EventKind::RailExit => !obs.on_rail && obs.rail_distance > 0.0,
                EventKind::MaximumDynamicPressure => {
                    // Latch the peak and fire when it turns over.
                    let peak = self.rules[rule_index].peak;
                    let rising = obs.dynamic_pressure > peak;
                    if rising {
                        self.rules[rule_index].peak = obs.dynamic_pressure;
                        self.rules[rule_index].rising = true;
                        false
                    } else if self.rules[rule_index].rising && peak > 0.0 {
                        self.rules[rule_index].rising = false;
                        true
                    } else {
                        false
                    }
                }
                EventKind::Burnout => self.thrust_was_on && obs.thrust <= 0.0,
                EventKind::Apogee => {
                    let threshold = self.enabled.apogee_velocity_threshold;
                    // The climbing state is latched rather than compared with the
                    // immediately previous sample. A fine output interval steps
                    // across the whole threshold band in one sample, so requiring
                    // one sample above `+threshold` and the very next below
                    // `-threshold` would hide the apogee of any normally sampled
                    // flight, which is the most important event in the run.
                    if obs.vertical_velocity > threshold {
                        self.rules[rule_index].rising = true;
                        self.rules[rule_index].last_rising = Some(obs);
                    }
                    let falling = obs.vertical_velocity < -threshold;
                    let was_rising = self.rules[rule_index].rising;
                    // A peak below one metre of altitude is not a flight apogee.
                    // It is the vehicle settling on the pad, or a numerical
                    // wobble at the start of the rail phase, and reporting it as
                    // apogee would be actively misleading.
                    let best_altitude = self.rules[rule_index].peak_altitude.max(obs.altitude);
                    self.rules[rule_index].peak_altitude = best_altitude;
                    if was_rising && falling && best_altitude > 1.0 {
                        self.rules[rule_index].rising = false;
                        // Interpolate the zero crossing, so the reported apogee
                        // time does not move with the output interval.
                        if let Some(last) = self.rules[rule_index].last_rising {
                            let span = last.vertical_velocity - obs.vertical_velocity;
                            if span > 0.0 {
                                let fraction = last.vertical_velocity / span;
                                fired_at = last.time + fraction * (obs.time - last.time);
                            }
                        }
                        true
                    } else {
                        false
                    }
                }
                EventKind::GroundImpact => {
                    let descending = previous.map(|p| p.vertical_velocity < 0.0).unwrap_or(false);
                    let touched = previous.map(|p| p.altitude > 0.0).unwrap_or(false);
                    descending && touched && obs.altitude <= 0.0
                }
                EventKind::RecoveryDeployment => match self.recovery_time {
                    Some(t) => obs.time >= t && obs.altitude > 0.0,
                    None => false,
                },
                EventKind::SolverFailure | EventKind::SolverWarning | EventKind::Custom => false,
                // A landing burn ignites on the state, not on a rule the detector
                // keeps. The runner reports it from the recorded history, where it
                // can also tell a landing burn from any other second burn.
                EventKind::LandingBurn => false,
            };

            if fired {
                self.rules[rule_index].fired = true;
                let peak = self.rules[rule_index].peak;
                let (label, value, unit, description): (&str, Real, &str, String) = match kind {
                    EventKind::Ignition => (
                        "Ignition",
                        obs.thrust,
                        "N",
                        "Thrust first became non-zero.".to_string(),
                    ),
                    EventKind::LiftOff => (
                        "Lift-off",
                        obs.vertical_velocity,
                        "m/s",
                        "The vehicle left the pad and is climbing.".to_string(),
                    ),
                    EventKind::RailExit => (
                        "Rail exit",
                        obs.rail_distance,
                        "m",
                        "The launch constraint released and the vehicle is free.".to_string(),
                    ),
                    EventKind::MaximumDynamicPressure => (
                        "Maximum dynamic pressure",
                        peak,
                        "Pa",
                        "Peak aerodynamic load. Structures and fins see their highest stress here."
                            .to_string(),
                    ),
                    EventKind::Burnout => (
                        "Motor burnout",
                        obs.mass,
                        "kg",
                        "Thrust returned to zero. Coast phase begins.".to_string(),
                    ),
                    EventKind::Apogee => (
                        "Apogee",
                        obs.altitude,
                        "m",
                        "Vertical velocity crossed zero. Highest point of the flight.".to_string(),
                    ),
                    EventKind::GroundImpact => (
                        "Ground impact",
                        obs.altitude,
                        "m",
                        "The vehicle reached the ground plane.".to_string(),
                    ),
                    _ => (kind.label(), 0.0, "", String::new()),
                };
                let id = self.next_id(kind, fired_at);
                out.push(FlightEvent {
                    id,
                    kind,
                    time: fired_at,
                    label: label.to_string(),
                    description,
                    trigger_value: Some(value),
                    trigger_unit: Some(unit.to_string()),
                    user_added: false,
                });
            }
            self.rules[rule_index].previous = Some(obs);
        }

        // Recovery deployment is produced once when its time is reached.
        if !self.recovery_deployed {
            if let Some(t) = self.recovery_time {
                if obs.time >= t && obs.altitude > 0.0 {
                    self.recovery_deployed = true;
                    let id = self.next_id(EventKind::RecoveryDeployment, obs.time);
                    out.push(FlightEvent {
                        id,
                        kind: EventKind::RecoveryDeployment,
                        time: obs.time,
                        label: "Recovery deployment".to_string(),
                        description: "The configured recovery device is expected to have deployed."
                            .to_string(),
                        trigger_value: Some(obs.altitude),
                        trigger_unit: Some("m".to_string()),
                        user_added: false,
                    });
                }
            }
        }

        self.thrust_was_on = obs.thrust > 0.0;
        if !obs.on_rail && obs.rail_distance > 0.0 {
            self.rail_exited = true;
        }
        let _ = self.rail_exited;

        out
    }

    /// Flush any peak-style rule that can only be resolved at the end of a run.
    ///
    /// This reports maximum dynamic pressure when the flight ended while the
    /// dynamic pressure was still rising, which happens in a short run that stops
    /// before coast.
    pub fn finish(&mut self, last: EventObservation) -> Vec<FlightEvent> {
        let mut out = Vec::new();
        for rule_index in 0..self.rules.len() {
            if self.rules[rule_index].fired
                || self.rules[rule_index].kind != EventKind::MaximumDynamicPressure
            {
                continue;
            }
            let peak = self.rules[rule_index].peak;
            if peak > 0.0 {
                self.rules[rule_index].fired = true;
                let id = self.next_id(EventKind::MaximumDynamicPressure, last.time);
                out.push(FlightEvent {
                    id,
                    kind: EventKind::MaximumDynamicPressure,
                    time: last.time,
                    label: "Maximum dynamic pressure".to_string(),
                    description: "Peak aerodynamic load, resolved at the end of the run because the dynamic pressure never turned over."
                        .to_string(),
                    trigger_value: Some(peak),
                    trigger_unit: Some("Pa".to_string()),
                    user_added: false,
                });
            }
        }
        out
    }
}

/// Sort events by time, keeping the firing order for equal times.
pub fn sort_events(events: &mut [FlightEvent]) {
    events.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// The time of the first event of a given kind, if present.
pub fn first_of_kind(events: &[FlightEvent], kind: EventKind) -> Option<&FlightEvent> {
    events.iter().find(|e| e.kind == kind)
}

/// Time difference between two event kinds, useful for duration metrics.
pub fn time_between(events: &[FlightEvent], from: EventKind, to: EventKind) -> Option<Real> {
    let a = first_of_kind(events, from)?;
    let b = first_of_kind(events, to)?;
    Some(b.time - a.time)
}

/// Detect the ground impact time directly from an altitude history.
///
/// Kept separate from [`EventDetector`] because replay and comparison need to
/// find events in already-recorded data without re-running the detector.
pub fn find_ground_impact(times: &[Real], altitudes: &[Real]) -> Option<Real> {
    if times.len() != altitudes.len() {
        return None;
    }
    for i in 1..times.len() {
        if altitudes[i - 1] > 0.0 && altitudes[i] <= 0.0 && times[i] > times[i - 1] {
            // Linear interpolation to the crossing instant.
            let t = altitudes[i - 1] / (altitudes[i - 1] - altitudes[i]);
            return Some(times[i - 1] + t * (times[i] - times[i - 1]));
        }
    }
    None
}

/// Detect the apogee time directly from an altitude history.
pub fn find_apogee(times: &[Real], altitudes: &[Real]) -> Option<Real> {
    if times.len() != altitudes.len() || times.is_empty() {
        return None;
    }
    let mut best = 0usize;
    for (i, a) in altitudes.iter().enumerate() {
        if *a > altitudes[best] {
            best = i;
        }
    }
    if best == 0 || best + 1 >= times.len() {
        return Some(times[best]);
    }
    // Parabolic refinement through the three samples around the peak.
    let t0 = times[best - 1];
    let t1 = times[best];
    let t2 = times[best + 1];
    let y0 = altitudes[best - 1];
    let y1 = altitudes[best];
    let y2 = altitudes[best + 1];
    let denom = (t0 - t1) * (t0 - t2) * (t1 - t2);
    if denom.abs() < 1e-18 {
        return Some(t1);
    }
    let a = (t2 * (y1 - y0) + t1 * (y0 - y2) + t0 * (y2 - y1)) / denom;
    let b = (t2 * t2 * (y0 - y1) + t1 * t1 * (y2 - y0) + t0 * t0 * (y1 - y2)) / denom;
    if a.abs() < 1e-18 {
        return Some(t1);
    }
    let t_peak = -b / (2.0 * a);
    if t_peak.is_finite() && t_peak >= t0 && t_peak <= t2 {
        Some(t_peak)
    } else {
        Some(t1)
    }
}

/// Detect the maximum-dynamic-pressure time from a history.
pub fn find_max_dynamic_pressure(times: &[Real], pressures: &[Real]) -> Option<Real> {
    if times.len() != pressures.len() || times.is_empty() {
        return None;
    }
    let mut best = 0usize;
    for (i, p) in pressures.iter().enumerate() {
        if *p > pressures[best] {
            best = i;
        }
    }
    Some(times[best])
}

/// Analyse a recorded vertical velocity and altitude history for apogee.
pub fn detect_apogee_from_state(
    times: &[Real],
    positions: &[Vec3],
    world_up: Vec3,
) -> Option<Real> {
    let altitudes: Vec<Real> = positions.iter().map(|p| p.dot(&world_up)).collect();
    find_apogee(times, &altitudes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(
        time: Real,
        altitude: Real,
        vertical_velocity: Real,
        q: Real,
        thrust: Real,
    ) -> EventObservation {
        EventObservation {
            time,
            altitude,
            vertical_velocity,
            airspeed: vertical_velocity.abs(),
            dynamic_pressure: q,
            thrust,
            mass: 10.0,
            rail_distance: altitude,
            on_rail: false,
        }
    }

    #[test]
    fn ignition_fires_once() {
        let mut d = EventDetector::new(EventRules::default());
        assert!(d.observe(obs(0.0, 0.0, 0.0, 0.0, 0.0)).is_empty());
        let events = d.observe(obs(0.1, 0.0, 0.0, 0.0, 100.0));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EventKind::Ignition);
        assert!(d.observe(obs(0.2, 0.0, 0.0, 0.0, 100.0)).is_empty());
        assert!(d.is_rule_fired(EventKind::Ignition));
    }

    #[test]
    fn burnout_needs_a_prior_ignition() {
        let mut d = EventDetector::new(EventRules::default());
        d.observe(obs(0.0, 0.0, 0.0, 0.0, 0.0));
        // Thrust never turned on, so there is no burnout to report.
        assert!(d.observe(obs(0.1, 0.0, 0.0, 0.0, 0.0)).is_empty());

        let mut d2 = EventDetector::new(EventRules::default());
        d2.observe(obs(0.0, 0.0, 0.0, 0.0, 50.0));
        let events = d2.observe(obs(2.0, 100.0, 50.0, 1000.0, 0.0));
        assert!(events.iter().any(|e| e.kind == EventKind::Burnout));
    }

    #[test]
    fn apogee_fires_on_the_velocity_crossing() {
        let mut d = EventDetector::new(EventRules::default());
        d.observe(obs(0.0, 0.0, 0.0, 0.0, 0.0));
        d.observe(obs(1.0, 50.0, 30.0, 500.0, 0.0));
        assert!(d.observe(obs(2.0, 80.0, 20.0, 600.0, 0.0)).is_empty());
        let events = d.observe(obs(3.0, 90.0, -5.0, 400.0, 0.0));
        assert!(events.iter().any(|e| e.kind == EventKind::Apogee));
        let apogee = events.iter().find(|e| e.kind == EventKind::Apogee).unwrap();
        assert!((apogee.trigger_value.unwrap() - 90.0).abs() < 1e-12);
    }

    #[test]
    fn apogee_fires_when_the_output_interval_steps_over_the_dead_band() {
        // At a 100 Hz output rate the vertical velocity changes by less than the
        // threshold band in one sample, so no single pair of consecutive samples
        // is ever `above +0.5` then `below -0.5`. The apogee is still the most
        // important event in the flight and must be reported.
        let mut d = EventDetector::new(EventRules::default());
        let mut events = Vec::new();
        for i in 0..60 {
            let t = i as Real * 0.01;
            // Decelerating under gravity from 2 m/s, with the altitude still
            // rising well above the one metre floor.
            let vz = 2.0 - 9.80665 * t;
            let altitude = 40.0 + 2.0 * t - 0.5 * 9.80665 * t * t;
            events.extend(d.observe(obs(t, altitude, vz, 200.0, 0.0)));
        }
        let apogee = events
            .iter()
            .find(|e| e.kind == EventKind::Apogee)
            .expect("the apogee of a normally sampled flight");
        // The crossing is at 2 / 9.80665 = 0.2039 s, between the 0.20 and 0.21
        // output samples, and the reported time is interpolated between them.
        assert!(
            (apogee.time - 0.203_94).abs() < 1e-3,
            "apogee time was {}",
            apogee.time
        );
        assert!(apogee.time > 0.20 && apogee.time < 0.21);
        assert!(apogee.trigger_value.unwrap() > 40.0);
    }

    #[test]
    fn an_interpolated_apogee_time_does_not_depend_on_the_output_interval() {
        let apogee_time = |interval: Real| {
            let mut d = EventDetector::new(EventRules::default());
            let steps = (0.6 / interval) as usize;
            for i in 0..steps {
                let t = i as Real * interval;
                let vz = 3.0 - 9.80665 * t;
                let altitude = 50.0 + 3.0 * t - 0.5 * 9.80665 * t * t;
                let fired = d.observe(obs(t, altitude, vz, 200.0, 0.0));
                if let Some(apogee) = fired.iter().find(|e| e.kind == EventKind::Apogee) {
                    return apogee.time;
                }
            }
            panic!("no apogee was detected at an interval of {}", interval);
        };
        let coarse = apogee_time(0.05);
        let fine = apogee_time(0.005);
        assert!(
            (coarse - fine).abs() < 5e-3,
            "coarse {} versus fine {}",
            coarse,
            fine
        );
    }

    #[test]
    fn apogee_does_not_fire_at_the_pad() {
        let mut d = EventDetector::new(EventRules::default());
        // The vehicle is still held by the rail and has not cleared one metre.
        let mut first = obs(0.0, 0.0, 0.0, 0.0, 0.0);
        first.on_rail = true;
        d.observe(first);
        let mut rising = obs(0.1, 0.5, 30.0, 10.0, 0.0);
        rising.on_rail = true;
        d.observe(rising);
        // A velocity reversal at half a metre is settling, not apogee.
        let mut falling = obs(0.2, 0.4, -20.0, 10.0, 0.0);
        falling.on_rail = true;
        let events = d.observe(falling);
        assert!(
            !events.iter().any(|e| e.kind == EventKind::Apogee),
            "events {:?}",
            events
        );
        assert!(!d.is_rule_fired(EventKind::Apogee));
    }

    #[test]
    fn max_q_fires_when_the_peak_turns_over() {
        let mut d = EventDetector::new(EventRules::default());
        d.observe(obs(0.0, 0.0, 0.0, 0.0, 0.0));
        d.observe(obs(1.0, 100.0, 100.0, 5000.0, 0.0));
        d.observe(obs(2.0, 300.0, 150.0, 12000.0, 0.0));
        assert!(d
            .observe(obs(3.0, 500.0, 140.0, 11000.0, 0.0))
            .iter()
            .any(|e| { e.kind == EventKind::MaximumDynamicPressure }));
    }

    #[test]
    fn max_q_is_reported_at_the_end_when_still_rising() {
        let mut d = EventDetector::new(EventRules::default());
        d.observe(obs(0.0, 0.0, 0.0, 0.0, 0.0));
        d.observe(obs(1.0, 100.0, 100.0, 5000.0, 0.0));
        d.observe(obs(2.0, 300.0, 150.0, 12000.0, 0.0));
        let events = d.finish(obs(2.5, 400.0, 150.0, 13000.0, 0.0));
        let max_q = events
            .iter()
            .find(|e| e.kind == EventKind::MaximumDynamicPressure)
            .expect("max q should be resolved at the end");
        assert!((max_q.trigger_value.unwrap() - 12000.0).abs() < 1e-9);
    }

    #[test]
    fn ground_impact_requires_descent() {
        let mut d = EventDetector::new(EventRules::default());
        d.observe(obs(0.0, 0.0, 0.0, 0.0, 0.0));
        d.observe(obs(1.0, 5.0, -10.0, 100.0, 0.0));
        let events = d.observe(obs(1.5, 0.0, -10.0, 50.0, 0.0));
        assert!(events.iter().any(|e| e.kind == EventKind::GroundImpact));
    }

    #[test]
    fn rail_exit_needs_a_rail_distance() {
        let mut d = EventDetector::new(EventRules::default());
        let mut o = obs(0.0, 0.0, 0.0, 0.0, 0.0);
        o.on_rail = true;
        o.rail_distance = 0.0;
        d.observe(o);

        let mut off = obs(0.5, 3.0, 20.0, 100.0, 0.0);
        off.on_rail = false;
        off.rail_distance = 3.0;
        let events = d.observe(off);
        assert!(events.iter().any(|e| e.kind == EventKind::RailExit));
    }

    #[test]
    fn recovery_deployment_fires_at_the_configured_time() {
        let mut d = EventDetector::new(EventRules::default());
        d.recovery_time = Some(10.0);
        d.observe(obs(0.0, 100.0, 0.0, 0.0, 0.0));
        assert!(d.observe(obs(5.0, 200.0, 0.0, 0.0, 0.0)).is_empty());
        let events = d.observe(obs(10.0, 150.0, -20.0, 0.0, 0.0));
        assert!(events
            .iter()
            .any(|e| e.kind == EventKind::RecoveryDeployment));
    }

    #[test]
    fn disabled_rules_never_fire() {
        let rules = EventRules {
            ignition: false,
            apogee: false,
            ..EventRules::default()
        };
        let mut d = EventDetector::new(rules);
        let events = d.observe(obs(0.0, 100.0, 0.0, 0.0, 500.0));
        assert!(!events.iter().any(|e| e.kind == EventKind::Ignition));
        assert!(!d.is_rule_fired(EventKind::Ignition));
    }

    #[test]
    fn event_ids_are_unique() {
        let mut d = EventDetector::new(EventRules::default());
        d.observe(obs(0.0, 0.0, 0.0, 0.0, 0.0));
        let first = d.observe(obs(0.1, 0.0, 0.0, 0.0, 10.0));
        let second = d.observe(obs(0.2, 100.0, 50.0, 100.0, 0.0));
        let mut ids: Vec<String> = first
            .iter()
            .chain(second.iter())
            .map(|e| e.id.clone())
            .collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn user_markers_are_flagged() {
        let m = FlightEvent::marker(3.5, "Camera start");
        assert!(m.user_added);
        assert_eq!(m.kind, EventKind::Custom);
        assert!(m.one_line().contains("Camera start"));
    }

    #[test]
    fn events_sort_by_time() {
        let mut events = vec![
            FlightEvent::new(EventKind::Apogee, 10.0, "Apogee"),
            FlightEvent::new(EventKind::Ignition, 0.0, "Ignition"),
            FlightEvent::new(EventKind::Burnout, 3.0, "Burnout"),
        ];
        sort_events(&mut events);
        assert_eq!(events[0].time, 0.0);
        assert_eq!(events[2].time, 10.0);
    }

    #[test]
    fn time_between_event_kinds() {
        let events = vec![
            FlightEvent::new(EventKind::LiftOff, 1.5, "Lift-off"),
            FlightEvent::new(EventKind::Apogee, 21.5, "Apogee"),
        ];
        let d = time_between(&events, EventKind::LiftOff, EventKind::Apogee).unwrap();
        assert!((d - 20.0).abs() < 1e-12);
        assert!(time_between(&events, EventKind::Ignition, EventKind::Apogee).is_none());
    }

    #[test]
    fn find_ground_impact_interpolates_the_crossing() {
        let times = vec![0.0, 1.0, 2.0];
        let altitudes = vec![10.0, 4.0, -2.0];
        let t = find_ground_impact(&times, &altitudes).unwrap();
        // Zero crossing is 4/(4+2) = 2/3 of the way from 1.0 to 2.0.
        assert!((t - (1.0 + 2.0 / 3.0)).abs() < 1e-12);
    }

    #[test]
    fn find_ground_impact_returns_none_when_never_landing() {
        let times = vec![0.0, 1.0, 2.0];
        let altitudes = vec![10.0, 20.0, 30.0];
        assert!(find_ground_impact(&times, &altitudes).is_none());
    }

    #[test]
    fn find_apogee_refines_between_samples() {
        // Parabola peaking exactly at t = 1.5.
        let times: Vec<Real> = (0..=4).map(|i| i as Real * 0.5).collect();
        let altitudes: Vec<Real> = times
            .iter()
            .map(|t| 100.0 - 4.0 * (t - 1.5) * (t - 1.5))
            .collect();
        let t = find_apogee(&times, &altitudes).unwrap();
        assert!((t - 1.5).abs() < 1e-9, "apogee at {}", t);
    }

    #[test]
    fn find_apogee_handles_a_monotonic_history() {
        let times = vec![0.0, 1.0, 2.0];
        let altitudes = vec![0.0, 5.0, 10.0];
        let t = find_apogee(&times, &altitudes).unwrap();
        assert!((0.0..=2.0).contains(&t));
    }

    #[test]
    fn find_max_dynamic_pressure_picks_the_peak_sample() {
        let times = vec![0.0, 1.0, 2.0, 3.0];
        let q = vec![0.0, 500.0, 900.0, 400.0];
        assert!((find_max_dynamic_pressure(&times, &q).unwrap() - 2.0).abs() < 1e-12);
    }

    #[test]
    fn apogee_from_state_uses_the_world_up_axis() {
        let times = vec![0.0, 1.0, 2.0];
        let positions = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 50.0),
            Vec3::new(0.0, 0.0, 20.0),
        ];
        let t = detect_apogee_from_state(&times, &positions, Vec3::z()).unwrap();
        assert!((t - 1.0).abs() < 0.5);
    }

    #[test]
    fn event_kind_metadata() {
        assert!(EventKind::Apogee.is_automatic());
        assert!(!EventKind::Custom.is_automatic());
        assert!(EventKind::SolverFailure.is_diagnostic());
        assert_eq!(EventKind::MaximumDynamicPressure.code(), "max_q");
    }
}
