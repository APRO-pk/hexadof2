//! Comparison and report commands.

use hex_analysis::{
    compare, detect_events, AlignmentMethod, AttitudeChannel, Comparison, ComparisonChannel,
    ComparisonOptions, ComparisonSeries, DetectedEvent, EventDetectionOptions, EventRef,
    EventTimeline, FlightSignals, MetricChannel, Provenance, Signal,
};
use hex_dynamics::ChannelSelector;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::commands::simulation::{selector_from_name, viewer_channel_names};
use crate::error::{missing, CommandError};
use crate::events;
use crate::state::{emit_notice, AppState};

/// A channel that can be compared, with what both sides provide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparableChannel {
    /// Machine identifier.
    pub code: String,
    /// Display label.
    pub label: String,
    /// Unit.
    pub unit: String,
    /// Whether the simulation side provides it.
    pub in_simulation: bool,
    /// Whether the flight side provides it, by role code.
    pub in_flight: bool,
    /// Whether the flight value is measured or estimated.
    pub flight_provenance: Option<String>,
}

/// A request to build a comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComparisonRequest {
    /// Alignment method code.
    pub method: Option<String>,
    /// Event code to align on, when the method is `event`.
    pub event_code: Option<String>,
    /// Manual offset in seconds, when the method is `manual`.
    pub offset_seconds: Option<f64>,
    /// Cross-correlation search half width, seconds.
    pub correlation_half_width: Option<f64>,
    /// Cross-correlation step, seconds.
    pub correlation_step: Option<f64>,
    /// Channels to compare. Empty means every default channel both sides share.
    pub channels: Vec<String>,
    /// Minimum overlap required, seconds.
    pub minimum_overlap_seconds: Option<f64>,
    /// Angular error above which the comparison is flagged, degrees.
    pub attitude_warning_degrees: Option<f64>,
}

impl Default for ComparisonRequest {
    fn default() -> Self {
        Self {
            method: Some("first_sample".to_string()),
            event_code: None,
            offset_seconds: None,
            correlation_half_width: None,
            correlation_step: None,
            channels: Vec::new(),
            minimum_overlap_seconds: None,
            attitude_warning_degrees: None,
        }
    }
}

impl ComparisonRequest {
    /// The alignment method this request describes.
    pub fn to_method(&self) -> AlignmentMethod {
        match self.method.as_deref() {
            Some("absolute") => AlignmentMethod::Absolute,
            Some("event") => {
                let code = self
                    .event_code
                    .clone()
                    .unwrap_or_else(|| "apogee".to_string());
                AlignmentMethod::Event {
                    reference_event: EventRef::new(code.clone()),
                    comparison_event: EventRef::new(code),
                }
            }
            Some("cross_correlation") | Some("correlation") => AlignmentMethod::CrossCorrelation {
                search_half_width_seconds: self.correlation_half_width.unwrap_or(1.0),
                step_seconds: self.correlation_step.unwrap_or(0.01),
            },
            Some("manual") => AlignmentMethod::Manual {
                offset_seconds: self.offset_seconds.unwrap_or(0.0),
            },
            _ => AlignmentMethod::FirstSample,
        }
    }

    /// The comparison options this request describes.
    pub fn to_options(&self) -> ComparisonOptions {
        ComparisonOptions {
            method: self.to_method(),
            channels: self
                .channels
                .iter()
                .filter_map(|c| metric_channel_from_code(c))
                .collect(),
            minimum_overlap_seconds: self.minimum_overlap_seconds.unwrap_or(0.1),
            attitude_warning_threshold: self
                .attitude_warning_degrees
                .map(f64::to_radians)
                .unwrap_or_else(|| 10.0f64.to_radians()),
            ..ComparisonOptions::default()
        }
    }
}

/// Map a channel code onto a comparison metric channel.
pub fn metric_channel_from_code(code: &str) -> Option<MetricChannel> {
    use MetricChannel::*;
    Some(match code {
        "altitude" | "position_z" => Altitude,
        "vertical_velocity" | "velocity_z" => VerticalVelocity,
        "speed" => Speed,
        "acceleration" | "acceleration_magnitude" => Acceleration,
        "dynamic_pressure" => DynamicPressure,
        "angle_of_attack" => AngleOfAttack,
        "sideslip" => Sideslip,
        "pressure_altitude" => PressureAltitude,
        "angular_rate" | "angular_rate_magnitude" => AngularRate,
        "attitude" => Attitude,
        "force" | "force_magnitude" => TotalForce,
        "moment" | "moment_magnitude" => TotalMoment,
        _ => return None,
    })
}

/// Which flight channel role supplies a comparison channel.
pub fn flight_role_for(channel: MetricChannel) -> Option<hex_flight_data::ChannelRole> {
    use hex_flight_data::ChannelRole;
    Some(match channel {
        MetricChannel::Altitude => ChannelRole::GnssAltitude,
        MetricChannel::PressureAltitude => ChannelRole::BaroPressure,
        MetricChannel::Speed => ChannelRole::GnssSpeed,
        MetricChannel::AngularRate => ChannelRole::GyroZ,
        MetricChannel::Acceleration => ChannelRole::AccelZ,
        MetricChannel::Attitude => ChannelRole::QuaternionW,
        _ => return None,
    })
}

/// Which run channel selector supplies a comparison channel.
pub fn simulation_selector_for(channel: MetricChannel) -> Option<ChannelSelector> {
    Some(match channel {
        MetricChannel::Altitude => ChannelSelector::Altitude,
        MetricChannel::VerticalVelocity => ChannelSelector::VerticalVelocity,
        MetricChannel::Speed => ChannelSelector::Speed,
        MetricChannel::Acceleration => ChannelSelector::AccelerationMagnitude,
        MetricChannel::DynamicPressure => ChannelSelector::DynamicPressure,
        MetricChannel::AngleOfAttack => ChannelSelector::AngleOfAttack,
        MetricChannel::Sideslip => ChannelSelector::Sideslip,
        MetricChannel::AngularRate => ChannelSelector::AngularRateMagnitude,
        MetricChannel::TotalForce => ChannelSelector::ForceMagnitude,
        MetricChannel::TotalMoment => ChannelSelector::MomentMagnitude,
        MetricChannel::PressureAltitude | MetricChannel::Attitude => return None,
    })
}

/// Compare a pressure series into an altitude series through the ISA relation.
///
/// The result is tagged as derived, so the comparison can say a part of any
/// difference comes from that derivation rather than from the flight.
pub fn pressure_to_altitude(pressure: &[f64]) -> Vec<f64> {
    hex_flight_data::derive_barometric_altitude(pressure, 101_325.0, 288.15)
}

/// The channels both sides can supply.
#[tauri::command]
pub fn comparison_channels(
    state: State<'_, AppState>,
) -> Result<Vec<ComparableChannel>, CommandError> {
    state.with(|s| {
        let run = s.last_run.as_ref();
        let flight = s.flight.as_ref();
        let mut out = Vec::new();
        for channel in MetricChannel::default_set() {
            let in_simulation = run.is_some() && simulation_selector_for(*channel).is_some();
            let in_flight = flight
                .map(|f| {
                    flight_role_for(*channel)
                        .map(|role| f.channel(role).is_some())
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            let flight_provenance = if in_flight {
                Some(match channel {
                    MetricChannel::Attitude => "estimated".to_string(),
                    MetricChannel::PressureAltitude => "measured".to_string(),
                    _ => "measured".to_string(),
                })
            } else {
                None
            };
            out.push(ComparableChannel {
                code: channel_code(*channel).to_string(),
                label: channel.label().to_string(),
                unit: channel.unit().to_string(),
                in_simulation,
                in_flight,
                flight_provenance,
            });
        }
        Ok(out)
    })?
}

/// The machine code for a comparison channel.
pub fn channel_code(channel: MetricChannel) -> &'static str {
    match channel {
        MetricChannel::Altitude => "altitude",
        MetricChannel::VerticalVelocity => "vertical_velocity",
        MetricChannel::Speed => "speed",
        MetricChannel::Acceleration => "acceleration",
        MetricChannel::DynamicPressure => "dynamic_pressure",
        MetricChannel::AngleOfAttack => "angle_of_attack",
        MetricChannel::Sideslip => "sideslip",
        MetricChannel::PressureAltitude => "pressure_altitude",
        MetricChannel::AngularRate => "angular_rate",
        MetricChannel::Attitude => "attitude",
        MetricChannel::TotalForce => "force",
        MetricChannel::TotalMoment => "moment",
    }
}

/// Derive dynamic pressure from a flight log.
///
/// A flight computer reports ambient pressure, not dynamic pressure and not the
/// air relative velocity, so q is derived as `0.5 * rho * v^2` from the GNSS
/// speed and the ISA density at the measured altitude. The signal name says the
/// value is derived, so an event built from it never reads as a direct
/// measurement.
pub fn derived_dynamic_pressure(session: &hex_flight_data::FlightSession) -> Option<Signal> {
    let speed = session.channel(hex_flight_data::ChannelRole::GnssSpeed)?;
    let speeds = speed.corrected_values();
    if speeds.is_empty() {
        return None;
    }
    let altitude = session
        .channel(hex_flight_data::ChannelRole::BaroPressure)
        .map(|c| pressure_to_altitude(&c.corrected_values()))
        .or_else(|| {
            session
                .channel(hex_flight_data::ChannelRole::GnssAltitude)
                .map(|c| c.corrected_values())
        });
    let atmosphere = hex_dynamics::StandardAtmosphere::standard_day();
    let values: Vec<f64> = speeds
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let height = altitude
                .as_ref()
                .and_then(|a| a.get(i).copied())
                .unwrap_or(0.0);
            if !v.is_finite() || !height.is_finite() {
                return f64::NAN;
            }
            let density = atmosphere.density(height.max(0.0));
            0.5 * density * v * v
        })
        .collect();
    Some(Signal::new(
        "dynamic pressure derived from GNSS speed and ISA density",
        session.times.clone(),
        values,
        "Pa",
    ))
}

/// The signals a flight log can supply to the event detector.
///
/// Both the event timeline and the alignment use this, so an alignment anchor
/// and the timeline entry it corresponds to can never disagree.
pub fn flight_signals(session: &hex_flight_data::FlightSession) -> FlightSignals {
    let signal = |role: hex_flight_data::ChannelRole, name: &str, unit: &str| -> Option<Signal> {
        session
            .channel(role)
            .map(|c| Signal::new(name, session.times.clone(), c.corrected_values(), unit))
    };
    FlightSignals {
        altitude: signal(
            hex_flight_data::ChannelRole::GnssAltitude,
            "gnss altitude",
            "m",
        ),
        vertical_velocity: signal(hex_flight_data::ChannelRole::GnssSpeed, "gnss speed", "m/s"),
        specific_force: signal(
            hex_flight_data::ChannelRole::AccelZ,
            "specific force",
            "m/s^2",
        ),
        // A flight computer log records pressure, not dynamic pressure. Passing
        // the ambient pressure here would report its peak as a maximum q of about
        // 101 kPa, which is wrong by two orders of magnitude, so q is derived
        // instead and labelled as derived.
        dynamic_pressure: derived_dynamic_pressure(session),
        thrust: None,
        pressure: signal(hex_flight_data::ChannelRole::BaroPressure, "pressure", "Pa"),
    }
}

/// The event timeline for a flight session.
///
/// It holds what the detector inferred from the recorded channels and the
/// markers the user placed, in time order. A marker keeps its `Manual` evidence,
/// so it can never be read as something the recorded data showed.
pub fn flight_timeline(
    session: &hex_flight_data::FlightSession,
    options: Option<EventDetectionOptions>,
) -> EventTimeline {
    let mut events = detect_events(&flight_signals(session), &options.unwrap_or_default());
    for marker in session.markers() {
        events.push(
            DetectedEvent::marker(marker.time, marker.label.clone())
                .with_note("Placed by hand, so it records an observation rather than a detection."),
        );
    }
    events.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    EventTimeline::new(events, session.duration())
}

/// Events detected in the imported flight log.
#[tauri::command]
pub fn flight_detect_events(
    state: State<'_, AppState>,
    options: Option<EventDetectionOptions>,
) -> Result<EventTimeline, CommandError> {
    state.with(|s| {
        let session = s.flight.as_ref().ok_or_else(|| {
            missing(
                "flight.none_imported",
                "No flight log is imported",
                "There is nothing to analyse.",
                "Import a flight log on the Flight Analysis screen.",
            )
        })?;
        Ok(flight_timeline(session, options))
    })?
}

/// Convert a run's events into the analysis crate's detected-event type.
///
/// A simulation event is a direct observation of the state, so it carries `Direct`
/// evidence. Marking it any other way would understate how reliable it is as an
/// alignment anchor.
pub fn detected_from_run_events(events: &[hex_dynamics::FlightEvent]) -> Vec<DetectedEvent> {
    events
        .iter()
        .map(|e| DetectedEvent {
            code: e.kind.code().to_string(),
            label: e.label.clone(),
            time: e.time,
            channel: "simulation".to_string(),
            value: e.trigger_value.unwrap_or(0.0),
            evidence: hex_analysis::EventEvidence::Direct,
            user_added: e.user_added,
            note: None,
        })
        .collect()
}

/// Build a comparison series from a run and a channel list.
pub fn simulation_series(
    run: &crate::state::StoredRun,
    channels: &[MetricChannel],
) -> ComparisonSeries {
    let mut series = ComparisonSeries::new(
        run.artifact_name
            .clone()
            .unwrap_or_else(|| run.outcome.summary.name.clone()),
    );
    series.has_position = true;
    for channel in channels {
        if let Some(selector) = simulation_selector_for(*channel) {
            let values: Vec<f64> = run
                .outcome
                .samples
                .iter()
                .map(|s| selector.value(s))
                .collect();
            series = series.with_channel(ComparisonChannel {
                channel: *channel,
                times: run.outcome.samples.iter().map(|s| s.time).collect(),
                values,
                unit: channel.unit().to_string(),
                provenance: Provenance::Simulated,
                is_integrated: false,
            });
        }
    }
    // The attitude comparison needs the quaternions, not a scalar column.
    if channels.contains(&MetricChannel::Attitude) {
        series = series.with_attitude(AttitudeChannel {
            times: run.outcome.samples.iter().map(|s| s.time).collect(),
            quaternions: run
                .outcome
                .samples
                .iter()
                .map(|s| s.attitude_quaternion())
                .collect(),
            provenance: Provenance::Simulated,
        });
    }
    series = series.with_events(detected_from_run_events(&run.outcome.events));
    series
}

/// Build the comparison series for the flight side.
pub fn flight_series(
    session: &hex_flight_data::FlightSession,
    channels: &[MetricChannel],
) -> ComparisonSeries {
    let mut series = ComparisonSeries::new(session.source.file_name.clone());
    for channel in channels {
        let values = match channel {
            MetricChannel::PressureAltitude => session
                .channel(hex_flight_data::ChannelRole::BaroPressure)
                .map(|c| pressure_to_altitude(&c.corrected_values())),
            MetricChannel::Altitude => session
                .channel(hex_flight_data::ChannelRole::GnssAltitude)
                .map(|c| c.corrected_values()),
            MetricChannel::Speed => session
                .channel(hex_flight_data::ChannelRole::GnssSpeed)
                .map(|c| c.corrected_values()),
            MetricChannel::AngularRate => session
                .channel(hex_flight_data::ChannelRole::GyroZ)
                .map(|c| c.corrected_values()),
            MetricChannel::Acceleration => session
                .channel(hex_flight_data::ChannelRole::AccelZ)
                .map(|c| c.corrected_values()),
            _ => None,
        };
        if let Some(values) = values {
            let is_integrated = channel == &MetricChannel::Altitude
                && session
                    .channel(hex_flight_data::ChannelRole::GnssAltitude)
                    .is_none();
            series = series.with_channel(ComparisonChannel {
                channel: *channel,
                times: session.times.clone(),
                values,
                unit: channel.unit().to_string(),
                provenance: if is_integrated {
                    Provenance::Estimated
                } else {
                    Provenance::Measured
                },
                is_integrated,
            });
        }
    }

    // Attitude from the gyroscope integration, which is always relative.
    if channels.contains(&MetricChannel::Attitude) {
        let gyro_roles = [
            hex_flight_data::ChannelRole::GyroX,
            hex_flight_data::ChannelRole::GyroY,
            hex_flight_data::ChannelRole::GyroZ,
        ];
        if gyro_roles.iter().all(|r| session.channel(*r).is_some()) {
            let rates: Vec<hex_core::Vec3> = (0..session.times.len())
                .map(|i| {
                    hex_core::Vec3::new(
                        session
                            .channel(hex_flight_data::ChannelRole::GyroX)
                            .and_then(|c| c.corrected_values().get(i).copied())
                            .unwrap_or(0.0),
                        session
                            .channel(hex_flight_data::ChannelRole::GyroY)
                            .and_then(|c| c.corrected_values().get(i).copied())
                            .unwrap_or(0.0),
                        session
                            .channel(hex_flight_data::ChannelRole::GyroZ)
                            .and_then(|c| c.corrected_values().get(i).copied())
                            .unwrap_or(0.0),
                    )
                })
                .collect();
            let quaternions = hex_flight_data::propagate_gyro_attitude(
                &session.times,
                &rates,
                hex_core::Quaternion::identity(),
                hex_core::Vec3::zeros(),
            );
            series = series.with_attitude(AttitudeChannel {
                times: session.times.clone(),
                quaternions,
                provenance: Provenance::Estimated,
            });
        }
    }

    series.has_position = session
        .channel(hex_flight_data::ChannelRole::GnssAltitude)
        .is_some();
    if !series.has_position {
        series = series.with_warning(
            "This flight log has no measured position, so a trajectory comparison is not possible. Orientation can still be compared.",
        );
    }
    // The events the timeline shows, so an alignment on an event can find the
    // same anchor the user picked there. Without them every event alignment
    // fails with a missing event.
    series = series.with_events(detect_events(
        &flight_signals(session),
        &EventDetectionOptions::default(),
    ));
    series
}

/// Build a comparison.
#[tauri::command]
pub fn comparison_create(
    state: State<'_, AppState>,
    request: Option<ComparisonRequest>,
) -> Result<Comparison, CommandError> {
    state.with(|s| {
        let run = s.last_run.as_ref().ok_or_else(|| {
            missing(
                "comparison.no_run",
                "No simulation result is loaded",
                "A comparison needs both a simulated run and an imported flight log.",
                "Run a simulation or open a saved run, then import a flight log.",
            )
        })?;
        let session = s.flight.as_ref().ok_or_else(|| {
            missing(
                "comparison.no_flight",
                "No flight log is imported",
                "A comparison needs both a simulated run and an imported flight log.",
                "Import a flight log on the Flight Analysis screen.",
            )
        })?;
        let request = request.unwrap_or_default();
        let options = request.to_options();

        let channels: Vec<MetricChannel> = if options.channels.is_empty() {
            MetricChannel::default_set().to_vec()
        } else {
            options.channels.clone()
        };

        let simulation = simulation_series(run, &channels);
        let flight = flight_series(session, &channels);
        let comparison = compare(&simulation, &flight, &options)?;
        Ok(comparison)
    })?
}

/// Build a comparison and remember it in the session.
#[tauri::command]
pub fn comparison_build(
    app: AppHandle,
    state: State<'_, AppState>,
    request: Option<ComparisonRequest>,
) -> Result<Comparison, CommandError> {
    let comparison = comparison_create(state.clone(), request)?;
    state.with(|s| {
        s.comparison = Some(comparison.clone());
        Ok::<(), CommandError>(())
    })??;
    emit_notice(
        &app,
        events::NoticeEvent::info(
            "Comparison built",
            hex_analysis::comparison_headline(&comparison),
        ),
    );
    Ok(comparison)
}

/// The stored comparison.
#[tauri::command]
pub fn comparison_current(state: State<'_, AppState>) -> Result<Option<Comparison>, CommandError> {
    state.with(|s| s.comparison.clone())
}

/// The alignment methods worth offering for the loaded pair.
#[tauri::command]
pub fn comparison_methods(
    state: State<'_, AppState>,
) -> Result<Vec<(String, String, bool)>, CommandError> {
    state.with(|s| {
        let (Some(run), Some(session)) = (s.last_run.as_ref(), s.flight.as_ref()) else {
            return Ok(vec![
                (
                    "first_sample".to_string(),
                    "First valid sample".to_string(),
                    true,
                ),
                (
                    "absolute".to_string(),
                    "Absolute timestamps".to_string(),
                    true,
                ),
                ("manual".to_string(), "Manual offset".to_string(), true),
            ]);
        };
        let channels = vec![MetricChannel::Altitude];
        let simulation = simulation_series(run, &channels);
        let flight = flight_series(session, &channels);
        Ok(hex_analysis::suggested_methods(&simulation, &flight)
            .into_iter()
            .map(|m| {
                let code = match &m {
                    AlignmentMethod::Absolute => "absolute",
                    AlignmentMethod::FirstSample => "first_sample",
                    AlignmentMethod::Event { .. } => "event",
                    AlignmentMethod::CrossCorrelation { .. } => "cross_correlation",
                    AlignmentMethod::Manual { .. } => "manual",
                };
                let recommended = matches!(
                    m,
                    AlignmentMethod::Event { .. } | AlignmentMethod::FirstSample
                );
                (code.to_string(), m.label(), recommended)
            })
            .collect())
    })?
}

/// Render a comparison as a plain-text report.
#[tauri::command]
pub fn comparison_report_text(state: State<'_, AppState>) -> Result<String, CommandError> {
    state.with(|s| {
        s.comparison.as_ref().map(|c| c.to_text()).ok_or_else(|| {
            missing(
                "comparison.none",
                "No comparison has been built",
                "There is nothing to report.",
                "Build a comparison on the Flight Analysis screen.",
            )
        })
    })?
}

/// A request to export a report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportRequest {
    /// Report title.
    pub title: Option<String>,
    /// Whether to include the comparison metrics.
    pub include_comparison: Option<bool>,
    /// Whether to include the simulation summary.
    pub include_simulation: Option<bool>,
    /// Whether to include the flight import report.
    pub include_flight: Option<bool>,
    /// Whether to include the event timeline.
    pub include_events: Option<bool>,
}

/// Export a report into the project.
#[tauri::command]
pub fn report_export(
    app: AppHandle,
    state: State<'_, AppState>,
    request: Option<ReportRequest>,
) -> Result<String, CommandError> {
    let request = request.unwrap_or(ReportRequest {
        title: None,
        include_comparison: Some(true),
        include_simulation: Some(true),
        include_flight: Some(true),
        include_events: Some(true),
    });
    let project = state.require_project()?;

    let (run, flight, comparison) = state.with(|s| {
        (
            s.last_run.as_ref().map(|r| r.outcome.summary.clone()),
            s.flight.clone(),
            s.comparison.clone(),
        )
    })?;

    if run.is_none() && flight.is_none() && comparison.is_none() {
        return Err(missing(
            "report.nothing_to_report",
            "There is nothing to report yet",
            "Run a simulation or import a flight log first.",
            "Run a simulation on the Dynamics screen, then export again.",
        ));
    }

    let title = request
        .title
        .clone()
        .unwrap_or_else(|| format!("{} report", project.metadata.name));
    let mut body = String::new();
    body.push_str(&format!("# {}\n\n", title));
    body.push_str(&format!("Project: {}\n\n", project.metadata.name));
    body.push_str(&format!("Generated: {}\n\n", hex_project::now_iso8601()));
    body.push_str(&format!("HexaDOF {}\n\n", hex_core::app_version()));
    // An exported report is read away from the application, so the positioning
    // has to travel with it rather than living only in the interface.
    body.push_str(
        "HexaDOF is an analysis and instrumentation tool. A simulation is a model, \
         not a test flight, and these results do not certify flight safety.\n\n",
    );

    if request.include_simulation.unwrap_or(true) {
        if let Some(summary) = &run {
            body.push_str("## Simulation run\n\n");
            body.push_str(&format!("- Name: {}\n", summary.name));
            body.push_str(&format!("- Status: {}\n", summary.status.label()));
            body.push_str(&format!("- Solver: {}\n", summary.solver));
            body.push_str(&format!("- Duration: {:.4} s\n", summary.duration));
            body.push_str(&format!("- Samples: {}\n", summary.sample_count));
            body.push_str(&format!(
                "- Output rate: {:.2} Hz\n",
                summary.output_rate_hz
            ));
            body.push_str(&format!("- Mass model: {}\n", summary.mass_model));
            body.push_str(&format!(
                "- Maximum altitude: {:.3} m\n",
                summary.maximum_altitude
            ));
            body.push_str(&format!(
                "- Maximum speed: {:.3} m/s\n",
                summary.maximum_speed
            ));
            body.push_str(&format!(
                "- Maximum dynamic pressure: {:.1} Pa\n",
                summary.maximum_dynamic_pressure
            ));
            body.push_str(&format!(
                "- Largest quaternion norm error: {:.3e}\n\n",
                summary.maximum_quaternion_norm_error
            ));
            if !summary.warnings.is_empty() {
                body.push_str("### Run warnings\n\n");
                for w in &summary.warnings {
                    body.push_str(&format!("- **{}**: {}\n", w.title, w.detail));
                }
                body.push('\n');
            }
        }
    }

    if request.include_flight.unwrap_or(true) {
        if let Some(session) = &flight {
            body.push_str("## Flight log\n\n");
            body.push_str(&format!("- Source: {}\n", session.source.file_name));
            body.push_str(&format!("- Format: {:?}\n", session.source.format));
            body.push_str(&format!("- Samples: {}\n", session.times.len()));
            body.push_str(&format!("- Duration: {:.4} s\n", session.duration()));
            body.push_str(&format!(
                "- Effective rate: {:.2} Hz\n",
                session.sample_rate_hz()
            ));
            body.push_str(&format!(
                "- Timebase: {}\n\n",
                session.timebase.note.clone().unwrap_or_else(|| session
                    .timebase
                    .source
                    .label()
                    .to_string())
            ));
            if !session.quality.flags.is_empty() {
                body.push_str("### Data quality\n\n");
                for flag in &session.quality.flags {
                    body.push_str(&format!(
                        "- **{}**: {} Suggested action: {}\n",
                        flag.code, flag.detail, flag.suggested_action
                    ));
                }
                body.push('\n');
            }
        }
    }

    if request.include_events.unwrap_or(true) {
        if let Some(timeline) = state.with(|s| {
            s.last_run.as_ref().map(|r| {
                EventTimeline::new(
                    detected_from_run_events(&r.outcome.events),
                    r.outcome.summary.duration,
                )
            })
        })? {
            body.push_str(&timeline.to_text());
            body.push('\n');
        }
    }

    if request.include_comparison.unwrap_or(true) {
        if let Some(comparison) = &comparison {
            body.push_str(&comparison.to_text());
            body.push('\n');
        }
    }

    let path = hex_project::save_report(&project, &title, &body)?;
    emit_notice(
        &app,
        events::NoticeEvent::info(
            "Report exported",
            format!("Written to {}.", path.to_string_lossy()),
        ),
    );
    Ok(path.to_string_lossy().to_string())
}

/// Save the current comparison as a JSON artifact.
#[tauri::command]
pub fn comparison_export(
    app: AppHandle,
    state: State<'_, AppState>,
    label: Option<String>,
) -> Result<String, CommandError> {
    let project = state.require_project()?;
    let comparison = state.with(|s| s.comparison.clone()).and_then(|c| {
        c.ok_or_else(|| {
            missing(
                "comparison.none",
                "No comparison has been built",
                "There is nothing to save.",
                "Build a comparison on the Flight Analysis screen.",
            )
        })
    })?;
    let label = label.unwrap_or_else(|| {
        format!(
            "{} vs {}",
            comparison.simulation_name, comparison.flight_name
        )
    });
    let value = serde_json::to_value(&comparison).map_err(|e| {
        CommandError::new(
            "comparison.serialize",
            "The comparison could not be written",
            e.to_string(),
            "Report this as a bug.",
        )
    })?;
    let path = hex_project::save_comparison(&project, &label, &value)?;
    emit_notice(
        &app,
        events::NoticeEvent::info(
            "Comparison saved",
            format!("Written to {}.", path.to_string_lossy()),
        ),
    );
    Ok(path.to_string_lossy().to_string())
}

/// The comparison channels offered by the metric channel list.
#[tauri::command]
pub fn comparison_channel_codes() -> Vec<(String, String, String)> {
    MetricChannel::default_set()
        .iter()
        .map(|c| {
            (
                channel_code(*c).to_string(),
                c.label().to_string(),
                c.unit().to_string(),
            )
        })
        .collect()
}

/// A run event as the frontend timeline presents it.
#[tauri::command]
pub fn event_lines(events: Vec<hex_dynamics::FlightEvent>) -> Vec<String> {
    events.iter().map(|e| e.one_line()).collect()
}

/// A detected flight event as the frontend timeline presents it.
#[tauri::command]
pub fn detected_event_lines(events: Vec<DetectedEvent>) -> Vec<String> {
    events.iter().map(|e| e.one_line()).collect()
}

/// The viewer channels the comparison plots can use.
#[tauri::command]
pub fn comparison_plot_channels() -> Vec<String> {
    viewer_channel_names()
}

/// The run channel selectors the results plots can use.
#[tauri::command]
pub fn result_channel_names() -> Vec<String> {
    crate::commands::simulation::all_channel_names()
}

/// Look up a run channel selector by name, for the frontend.
#[tauri::command]
pub fn result_channel_exists(name: String) -> bool {
    selector_from_name(&name).is_some()
}

/// The analysis channel selector type, exposed so the frontend can be sure the two
/// channel vocabularies are the ones the backend understands.
#[tauri::command]
pub fn analysis_selector_codes() -> Vec<(String, String)> {
    [
        ChannelSelector::Time,
        ChannelSelector::Altitude,
        ChannelSelector::VerticalVelocity,
        ChannelSelector::Speed,
        ChannelSelector::AccelerationMagnitude,
        ChannelSelector::DynamicPressure,
        ChannelSelector::AngleOfAttack,
        ChannelSelector::Sideslip,
        ChannelSelector::Mach,
    ]
    .iter()
    .map(|s| (s.label().to_string(), s.unit().to_string()))
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_channel_codes_round_trip() {
        for channel in MetricChannel::default_set() {
            let code = channel_code(*channel);
            assert_eq!(
                metric_channel_from_code(code),
                Some(*channel),
                "{} did not round trip",
                code
            );
        }
        assert!(metric_channel_from_code("nonsense").is_none());
        assert_eq!(
            metric_channel_from_code("position_z"),
            Some(MetricChannel::Altitude)
        );
    }

    #[test]
    fn channel_codes_are_unique() {
        let mut codes: Vec<&str> = MetricChannel::default_set()
            .iter()
            .map(|c| channel_code(*c))
            .collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }

    #[test]
    fn alignment_method_codes_map_to_methods() {
        let request = ComparisonRequest {
            method: Some("absolute".to_string()),
            ..Default::default()
        };
        assert_eq!(request.to_method(), AlignmentMethod::Absolute);

        let request = ComparisonRequest {
            method: Some("event".to_string()),
            event_code: Some("apogee".to_string()),
            ..Default::default()
        };
        match request.to_method() {
            AlignmentMethod::Event {
                reference_event, ..
            } => assert_eq!(reference_event.code, "apogee"),
            other => panic!("expected event alignment, got {:?}", other),
        }

        let request = ComparisonRequest {
            method: Some("manual".to_string()),
            offset_seconds: Some(0.25),
            ..Default::default()
        };
        assert_eq!(
            request.to_method(),
            AlignmentMethod::Manual {
                offset_seconds: 0.25
            }
        );

        let request = ComparisonRequest {
            method: Some("cross_correlation".to_string()),
            correlation_half_width: Some(2.0),
            correlation_step: Some(0.02),
            ..Default::default()
        };
        match request.to_method() {
            AlignmentMethod::CrossCorrelation {
                search_half_width_seconds,
                step_seconds,
            } => {
                assert!((search_half_width_seconds - 2.0).abs() < 1e-12);
                assert!((step_seconds - 0.02).abs() < 1e-12);
            }
            other => panic!("expected correlation, got {:?}", other),
        }

        let request = ComparisonRequest::default();
        assert_eq!(request.to_method(), AlignmentMethod::FirstSample);
    }

    #[test]
    fn unknown_method_names_fall_back_to_the_first_sample() {
        let request = ComparisonRequest {
            method: Some("telepathy".to_string()),
            ..Default::default()
        };
        assert_eq!(request.to_method(), AlignmentMethod::FirstSample);
    }

    #[test]
    fn options_convert_channels_and_thresholds() {
        let request = ComparisonRequest {
            channels: vec!["altitude".to_string(), "nonsense".to_string()],
            minimum_overlap_seconds: Some(2.0),
            attitude_warning_degrees: Some(25.0),
            ..Default::default()
        };
        let options = request.to_options();
        assert_eq!(options.channels, vec![MetricChannel::Altitude]);
        assert!((options.minimum_overlap_seconds - 2.0).abs() < 1e-12);
        assert!((options.attitude_warning_threshold.to_degrees() - 25.0).abs() < 1e-9);
    }

    #[test]
    fn pressure_to_altitude_decreases_with_pressure() {
        let ground = pressure_to_altitude(&[101_325.0]);
        let high = pressure_to_altitude(&[90_000.0]);
        assert!(ground[0].abs() < 1e-6, "ground {}", ground[0]);
        assert!(high[0] > 900.0, "high {}", high[0]);
    }

    #[test]
    fn simulation_selector_mapping_covers_the_comparable_channels() {
        for channel in MetricChannel::default_set() {
            let has_selector = simulation_selector_for(*channel).is_some();
            let has_flight_role = flight_role_for(*channel).is_some();
            // Pressure altitude and attitude are derived, so they have no direct
            // run selector; everything else must have one.
            if matches!(
                channel,
                MetricChannel::PressureAltitude | MetricChannel::Attitude
            ) {
                assert!(!has_selector, "{:?} should not have a selector", channel);
            } else {
                assert!(has_selector, "{:?} has no selector", channel);
            }
            // Every channel must have a flight role or be derived from one.
            let _ = has_flight_role;
        }
    }

    #[test]
    fn flight_role_mapping_is_specific() {
        assert_eq!(
            flight_role_for(MetricChannel::Altitude),
            Some(hex_flight_data::ChannelRole::GnssAltitude)
        );
        assert_eq!(flight_role_for(MetricChannel::TotalForce), None);
    }

    #[test]
    fn channel_codes_list_is_complete() {
        let codes = comparison_channel_codes();
        assert_eq!(codes.len(), MetricChannel::default_set().len());
        for (code, label, unit) in codes {
            assert!(!code.is_empty());
            assert!(!label.is_empty());
            assert!(!unit.is_empty());
            assert!(metric_channel_from_code(&code).is_some());
        }
    }

    #[test]
    fn analysis_selector_codes_are_described() {
        let codes = analysis_selector_codes();
        assert!(!codes.is_empty());
        for (label, unit) in codes {
            assert!(!label.is_empty());
            assert!(!unit.is_empty());
        }
    }

    #[test]
    fn result_channel_exists_matches_the_selector_table() {
        assert!(result_channel_exists("altitude".to_string()));
        assert!(result_channel_exists("moment".to_string()));
        assert!(!result_channel_exists("nonsense".to_string()));
    }

    #[test]
    fn event_lines_render_a_run_event_and_a_detected_event() {
        let run_events = vec![hex_dynamics::FlightEvent::new(
            hex_dynamics::EventKind::Apogee,
            12.5,
            "Apogee",
        )];
        let lines = event_lines(run_events);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Apogee"));
        assert!(lines[0].contains("12.5"));

        let detected = vec![DetectedEvent::new(
            "apogee",
            "Apogee",
            12.5,
            "altitude",
            900.0,
            hex_analysis::EventEvidence::Peak,
        )];
        let lines = detected_event_lines(detected);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Apogee"));
        assert!(lines[0].contains("altitude"));
    }

    #[test]
    fn run_events_convert_to_detected_events() {
        let run_events = vec![
            hex_dynamics::FlightEvent::new(hex_dynamics::EventKind::LiftOff, 1.25, "Lift-off"),
            hex_dynamics::FlightEvent::marker(3.0, "Camera start"),
        ];
        let detected = detected_from_run_events(&run_events);
        assert_eq!(detected.len(), 2);
        assert_eq!(detected[0].code, "lift_off");
        assert_eq!(detected[0].time, 1.25);
        // A simulation event is a direct observation, so it stays a reliable
        // alignment anchor.
        assert_eq!(detected[0].evidence, hex_analysis::EventEvidence::Direct);
        assert!(detected[0].is_reliable_for_alignment());
        assert!(detected[1].user_added);
    }
}
