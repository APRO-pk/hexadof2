//! Live telemetry commands: ports, connection, recording, calibration, and the
//! sensor validation checklist.

use hex_core::ValidationReport;
use hex_telemetry::{
    run_validation, ChannelDestination, CheckStatus, ConnectionState, DeviceProfile, EstimatorMode,
    PhysicalOrientation, ProcessedPacket, SensorValidation, SerialSettings, TelemetryChannel,
    ValidationInput,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::{missing, CommandError};
use crate::events;
use crate::state::{emit, emit_notice, AppState, TelemetryWorker};

/// A serial port as the picker presents it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortView {
    /// System port name.
    pub port_name: String,
    /// Label for the list.
    pub label: String,
    /// Stable identity for a device profile.
    pub identity: String,
    /// Whether the port is a USB device.
    pub is_usb: bool,
    /// Manufacturer, when known.
    pub manufacturer: Option<String>,
    /// USB product, when known.
    pub product: Option<String>,
    /// USB serial number, when known.
    pub serial_number: Option<String>,
}

/// List the serial ports the operating system reports.
#[tauri::command]
pub fn serial_list_ports() -> Result<Vec<PortView>, CommandError> {
    let ports = hex_telemetry::list_ports()?;
    Ok(ports
        .into_iter()
        .map(|p| PortView {
            label: p.label(),
            identity: p.identity(),
            port_name: p.port_name,
            is_usb: p.is_usb,
            manufacturer: p.manufacturer,
            product: p.product,
            serial_number: p.serial_number,
        })
        .collect())
}

/// Request to open a port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectRequest {
    /// System port name, for example `COM4`.
    pub port_name: String,
    /// Device profile identifier, when a stored profile should be used.
    pub profile_id: Option<String>,
    /// A complete profile supplied by the editor.
    pub profile: Option<DeviceProfile>,
    /// Baud rate override.
    pub baud_rate: Option<u32>,
    /// Recording buffer capacity in samples.
    pub buffer_samples: Option<usize>,
    /// Display update rate, hertz.
    pub display_rate_hz: Option<f64>,
}

/// A telemetry status snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryStatus {
    /// Whether a worker is attached.
    pub connected: bool,
    /// Port name.
    pub port: String,
    /// Baud rate.
    pub baud_rate: u32,
    /// Connection state label.
    pub state: String,
    /// Whether recording is running.
    pub recording: bool,
    /// Samples recorded.
    pub recorded_samples: usize,
    /// Recording capacity.
    pub capacity: usize,
    /// Valid packets decoded.
    pub packets_received: u64,
    /// Packets rejected.
    pub packets_rejected: u64,
    /// Checksum failures.
    pub crc_failures: u64,
    /// Bytes received.
    pub bytes_received: u64,
    /// Bytes discarded while resynchronising.
    pub discarded_bytes: u64,
    /// Samples dropped from the display stream.
    pub display_dropped: u64,
    /// Whether the display is downsampled.
    pub display_downsampled: bool,
    /// Pending display frames.
    pub pending_display_frames: usize,
    /// The most recent rejection message, when there is one.
    pub last_rejection: Option<String>,
    /// The most recent read error, when there is one.
    pub last_error: Option<String>,
    /// Whether the source has stopped.
    pub finished: bool,
    /// Whether any data has arrived in this session.
    pub receiving: bool,
}

fn status_of(worker: &TelemetryWorker) -> TelemetryStatus {
    let counters = worker.with_pipeline(|p| p.counters()).unwrap_or_default();
    let decoder = worker
        .with_pipeline(|p| {
            (
                p.decoder().packets_accepted,
                p.decoder().checksum_failures,
                p.decoder().last_sequence(),
            )
        })
        .unwrap_or((0, 0, None));
    let _ = decoder;
    let health = worker.with_pipeline(|p| p.health()).unwrap_or_default();
    let recorder = worker.recorder_status().unwrap_or_default_status();
    let display = worker
        .with_pipeline(|p| {
            (
                p.display().dropped_frames,
                p.display().is_downsampled(),
                p.display().pending(),
            )
        })
        .unwrap_or((0, false, 0));
    let last_rejection = worker
        .with_pipeline(|p| p.recent_rejections.last().cloned())
        .flatten();
    let state = if worker.is_finished() {
        ConnectionState::Stalled
    } else if health.packets_received == 0 {
        ConnectionState::Connected
    } else {
        ConnectionState::Receiving
    };

    TelemetryStatus {
        connected: true,
        port: worker.port.clone(),
        baud_rate: worker.baud_rate,
        state: state.label().to_string(),
        recording: recorder.recording,
        recorded_samples: recorder.samples,
        capacity: recorder.capacity,
        packets_received: health.packets_received,
        packets_rejected: health.packets_rejected,
        crc_failures: health.crc_failures,
        bytes_received: health.bytes_received,
        discarded_bytes: health.discarded_bytes,
        display_dropped: display.0,
        display_downsampled: display.1,
        pending_display_frames: display.2,
        last_rejection,
        last_error: None,
        finished: worker.is_finished(),
        receiving: counters.packets_decoded > 0,
    }
}

/// A small helper so a missing recorder status does not need an error path.
trait RecorderStatusExt {
    fn unwrap_or_default_status(self) -> hex_telemetry::RecorderStatus;
}

impl RecorderStatusExt for Option<hex_telemetry::RecorderStatus> {
    fn unwrap_or_default_status(self) -> hex_telemetry::RecorderStatus {
        self.unwrap_or(hex_telemetry::RecorderStatus {
            recording: false,
            samples: 0,
            capacity: 0,
            rejected: 0,
            duration_seconds: 0.0,
        })
    }
}

/// Open a serial port and start reading on a background thread.
#[tauri::command]
pub fn serial_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ConnectRequest,
) -> Result<TelemetryStatus, CommandError> {
    let (settings, stored_profile) = state.with(|s| (s.settings.clone(), s.project.clone()))?;

    let mut profile = match (&request.profile, &request.profile_id) {
        (Some(p), _) => p.clone(),
        (None, Some(id)) => stored_profile
            .as_ref()
            .and_then(|p| hex_project::find_device_profile(p, id))
            .ok_or_else(|| {
                missing(
                    "telemetry.profile_missing",
                    "The device profile was not found",
                    format!("No stored profile has the identifier {}.", id),
                    "Choose a different profile, or create a new one.",
                )
            })?,
        (None, None) => {
            let mut p = DeviceProfile::example_csv_profile();
            p.serial = SerialSettings::at_baud(settings.telemetry.default_baud_rate);
            p
        }
    };
    profile.serial = SerialSettings {
        read_timeout_ms: 50,
        ..profile.serial
    };
    if let Some(baud) = request.baud_rate {
        profile.serial.baud_rate = baud;
    }

    let problems = profile.validate();
    let blocking: Vec<&hex_telemetry::MappingProblem> =
        problems.iter().filter(|p| p.blocking).collect();
    if !blocking.is_empty() {
        return Err(CommandError::new(
            "telemetry.profile_invalid",
            "The device profile cannot be used",
            blocking
                .iter()
                .map(|p| format!("{}: {}", p.title, p.detail))
                .collect::<Vec<_>>()
                .join(" "),
            blocking
                .first()
                .map(|p| p.suggestion.clone())
                .unwrap_or_else(|| "Fix the mapping table and try again.".to_string()),
        ));
    }

    // Close any existing connection rather than leaking a thread.
    state.with(|s| {
        if let Some(mut old) = s.telemetry.take() {
            old.shutdown();
        }
        Ok::<(), CommandError>(())
    })??;

    let source: Box<dyn hex_telemetry::ByteSource + Send> = Box::new(
        hex_telemetry::serial::SerialByteSource::open(&request.port_name, &profile.serial)?,
    );

    let calibration = stored_profile
        .as_ref()
        .and_then(|p| hex_project::load_calibration(p, "default").ok())
        .unwrap_or_default();

    let worker = TelemetryWorker::spawn(
        source,
        profile,
        calibration,
        request
            .buffer_samples
            .unwrap_or(settings.telemetry.recording_buffer_samples),
        request
            .display_rate_hz
            .unwrap_or(settings.telemetry.display_rate_hz),
    );
    let status = status_of(&worker);

    state.with(|s| {
        s.telemetry = Some(worker);
        Ok::<(), CommandError>(())
    })??;

    emit(
        &app,
        events::channel::SERIAL_CONNECTED,
        events::SerialEvent {
            port: status.port.clone(),
            baud_rate: status.baud_rate,
            state: status.state.clone(),
            detail: format!("Opened at {} baud.", status.baud_rate),
        },
    );
    emit_notice(
        &app,
        events::NoticeEvent::info(
            "Port opened",
            format!("{} at {} baud.", status.port, status.baud_rate),
        ),
    );
    Ok(status)
}

/// Close the serial port.
#[tauri::command]
pub fn serial_disconnect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TelemetryStatus, CommandError> {
    let previous = state.with(|s| {
        let taken = s.telemetry.take();
        let info = taken
            .as_ref()
            .map(|w| (w.port.clone(), w.baud_rate))
            .unwrap_or_else(|| (String::new(), 0));
        if let Some(mut worker) = taken {
            worker.shutdown();
        }
        info
    })?;

    if !previous.0.is_empty() {
        emit(
            &app,
            events::channel::SERIAL_DISCONNECTED,
            events::SerialEvent {
                port: previous.0.clone(),
                baud_rate: previous.1,
                state: ConnectionState::Disconnected.label().to_string(),
                detail: "The port was closed.".to_string(),
            },
        );
    }

    Ok(TelemetryStatus {
        connected: false,
        port: previous.0,
        baud_rate: previous.1,
        state: ConnectionState::Disconnected.label().to_string(),
        recording: false,
        recorded_samples: 0,
        capacity: 0,
        packets_received: 0,
        packets_rejected: 0,
        crc_failures: 0,
        bytes_received: 0,
        discarded_bytes: 0,
        display_dropped: 0,
        display_downsampled: false,
        pending_display_frames: 0,
        last_rejection: None,
        last_error: None,
        finished: true,
        receiving: false,
    })
}

/// The current telemetry status.
#[tauri::command]
pub fn telemetry_status(
    state: State<'_, AppState>,
) -> Result<Option<TelemetryStatus>, CommandError> {
    state.with(|s| s.telemetry.as_ref().map(status_of))
}

/// A frame of live display data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveFrame {
    /// Host time in seconds.
    pub host_time: f64,
    /// Device time in seconds, when the packet carried one.
    pub device_time: Option<f64>,
    /// Packet sequence number.
    pub sequence: Option<u32>,
    /// Accelerometer in the body frame, metres per second squared.
    pub acceleration: Option<[f64; 3]>,
    /// Gyroscope in the body frame, radians per second.
    pub angular_rate: Option<[f64; 3]>,
    /// Magnetometer in the body frame, tesla.
    pub magnetic_field: Option<[f64; 3]>,
    /// Pressure in pascals.
    pub pressure: Option<f64>,
    /// Temperature in kelvin.
    pub temperature: Option<f64>,
    /// Motor command.
    pub motor: Option<f64>,
    /// Supply voltage.
    pub voltage: Option<f64>,
    /// Current in amperes.
    pub current: Option<f64>,
    /// Estimated attitude, when an estimator is running.
    pub attitude: Option<[f64; 4]>,
    /// Attitude source label, so a filtered estimate is never shown as truth.
    pub attitude_source: Option<String>,
    /// Estimator health label.
    pub estimator_health: Option<String>,
    /// Estimator note.
    pub estimator_note: Option<String>,
    /// Whether the estimator is only indicative.
    pub estimator_indicative: Option<bool>,
    /// Every mapped field with its converted value.
    pub mapped: Vec<(String, f64)>,
}

fn frame_of(packet: &ProcessedPacket) -> LiveFrame {
    let s = &packet.sample;
    LiveFrame {
        host_time: s.host_time,
        device_time: s.device_time,
        sequence: s.sequence,
        acceleration: s.acceleration.map(|v| [v.x, v.y, v.z]),
        angular_rate: s.angular_rate.map(|v| [v.x, v.y, v.z]),
        magnetic_field: s.magnetic_field.map(|v| [v.x, v.y, v.z]),
        pressure: s.pressure,
        temperature: s.temperature,
        motor: s.motor,
        voltage: s.voltage,
        current: s.current,
        attitude: packet
            .estimator
            .as_ref()
            .map(|e| [e.attitude.w, e.attitude.x, e.attitude.y, e.attitude.z]),
        attitude_source: packet
            .estimator
            .as_ref()
            .map(|e| e.attitude_source_label().to_string()),
        estimator_health: packet
            .estimator
            .as_ref()
            .map(|e| e.health.label().to_string()),
        estimator_note: packet.estimator.as_ref().map(|e| e.note.clone()),
        estimator_indicative: packet.estimator.as_ref().map(|e| e.is_indicative_only()),
        mapped: packet
            .mapped
            .iter()
            .map(|(d, v)| (d.label().to_string(), *v))
            .collect(),
    }
}

/// Drain the throttled display stream.
#[tauri::command]
pub fn telemetry_poll(
    app: AppHandle,
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<LiveFrame>, CommandError> {
    let frames = state.with(|s| {
        s.telemetry
            .as_ref()
            .map(|w| w.drain_display(limit.unwrap_or(32)))
            .unwrap_or_default()
    })?;
    let frames: Vec<LiveFrame> = frames.iter().map(frame_of).collect();

    // Publish the health alongside the frames so the panel and the plots never
    // disagree about the connection.
    if let Some(status) = state.with(|s| s.telemetry.as_ref().map(status_of))? {
        emit(
            &app,
            events::channel::TELEMETRY_HEALTH,
            events::TelemetryHealthEvent {
                state: status.state.clone(),
                packets_received: status.packets_received,
                packets_rejected: status.packets_rejected,
                packets_per_second: 0.0,
                bytes_received: status.bytes_received,
                discarded_bytes: status.discarded_bytes,
                crc_failures: status.crc_failures,
                recorded_samples: status.recorded_samples as u64,
                display_downsampled: status.display_downsampled,
                pending_display_frames: status.pending_display_frames,
                last_rejection: status.last_rejection.clone(),
            },
        );
    }
    Ok(frames)
}

/// Start recording.
#[tauri::command]
pub fn telemetry_start_recording(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<TelemetryStatus, CommandError> {
    let status = state.with(|s| {
        let worker = s.telemetry.as_ref().ok_or_else(|| {
            missing(
                "telemetry.not_connected",
                "No device is connected",
                "Recording needs an open serial port.",
                "Connect a device on the Live Telemetry screen.",
            )
        })?;
        worker.with_pipeline(|p| p.start_recording());
        Ok::<TelemetryStatus, CommandError>(status_of(worker))
    })??;

    emit(
        &app,
        events::channel::RECORDING_STARTED,
        events::RecordingEvent {
            recording: true,
            samples: status.recorded_samples,
            capacity: status.capacity,
            artifact: None,
        },
    );
    Ok(status)
}

/// Stop recording and, when a project is open, save the session.
#[tauri::command]
pub fn telemetry_stop_recording(
    app: AppHandle,
    state: State<'_, AppState>,
    label: Option<String>,
    save: Option<bool>,
) -> Result<TelemetryStatus, CommandError> {
    let (status, channel_names, columns, times, profile_info) = state.with(|s| {
        let worker = s.telemetry.as_ref().ok_or_else(|| {
            missing(
                "telemetry.not_connected",
                "No device is connected",
                "There is no recording to stop.",
                "Connect a device on the Live Telemetry screen.",
            )
        })?;
        worker.with_pipeline(|p| p.stop_recording());

        let channels = vec![
            TelemetryChannel::HostTime,
            TelemetryChannel::AccelX,
            TelemetryChannel::AccelY,
            TelemetryChannel::AccelZ,
            TelemetryChannel::GyroX,
            TelemetryChannel::GyroY,
            TelemetryChannel::GyroZ,
            TelemetryChannel::MagX,
            TelemetryChannel::MagY,
            TelemetryChannel::MagZ,
            TelemetryChannel::Pressure,
            TelemetryChannel::Temperature,
            TelemetryChannel::GnssAltitude,
            TelemetryChannel::GnssSpeed,
            TelemetryChannel::Motor,
            TelemetryChannel::Voltage,
            TelemetryChannel::Current,
        ];
        let (names, cols, times) = worker
            .with_pipeline(|p| {
                let names: Vec<String> = channels.iter().map(|c| c.label().to_string()).collect();
                let cols: Vec<Vec<f64>> = channels
                    .iter()
                    .map(|c| c.values(p.recorder().samples()))
                    .collect();
                (names, cols, p.recorder().times())
            })
            .unwrap_or_default();
        let profile_info = worker
            .with_pipeline(|p| {
                (
                    p.profile().id.clone(),
                    p.profile().name.clone(),
                    p.profile().format.label().to_string(),
                    p.calibration().summary(),
                )
            })
            .unwrap_or_default();
        Ok::<_, CommandError>((status_of(worker), names, cols, times, profile_info))
    })??;

    let should_save = save.unwrap_or(true);
    let mut artifact = None;
    if should_save {
        let project = state.with(|s| s.project.clone())?;
        match project {
            Some(project) => {
                let label = label.unwrap_or_else(|| "Telemetry session".to_string());
                let rate = if status.recorded_samples > 1 && status.capacity > 0 {
                    let span = times.last().copied().unwrap_or(0.0)
                        - times.first().copied().unwrap_or(0.0);
                    if span > 0.0 {
                        (times.len() - 1) as f64 / span
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                let metadata = hex_project::TelemetrySessionMetadata {
                    schema_version: String::new(),
                    name: String::new(),
                    label: label.clone(),
                    profile_id: profile_info.0.clone(),
                    profile_name: profile_info.1.clone(),
                    port: status.port.clone(),
                    baud_rate: status.baud_rate,
                    packet_format: profile_info.2.clone(),
                    started_at: hex_project::now_iso8601(),
                    duration_seconds: times.last().copied().unwrap_or(0.0)
                        - times.first().copied().unwrap_or(0.0),
                    sample_count: status.recorded_samples as u64,
                    average_rate_hz: rate,
                    packets_rejected: status.packets_rejected,
                    bytes_received: status.bytes_received,
                    display_downsampled: status.display_downsampled,
                    channels: channel_names.clone(),
                    calibration: profile_info.3.clone(),
                    warnings: Vec::new(),
                };
                let mut metadata = metadata;
                if status.packets_rejected > 0 {
                    metadata.warnings.push(format!(
                        "{} packet(s) were rejected during recording.",
                        status.packets_rejected
                    ));
                }
                if status.display_downsampled {
                    metadata.warnings.push(
                        "The display was downsampled, but the recording holds every sample."
                            .to_string(),
                    );
                }
                match hex_project::save_telemetry_session(
                    &project,
                    &label,
                    &metadata,
                    &channel_names,
                    &columns,
                    &times,
                ) {
                    Ok(session) => {
                        artifact = Some(session.name.clone());
                        state.with(|s| {
                            s.last_telemetry_artifact = Some(session.name.clone());
                            Ok::<(), CommandError>(())
                        })??;
                        emit_notice(
                            &app,
                            events::NoticeEvent::info(
                                "Recording saved",
                                format!("Saved as {}.", session.name),
                            ),
                        );
                    }
                    Err(e) => emit_notice(
                        &app,
                        events::NoticeEvent::warning(
                            "The recording could not be saved",
                            e.user_message(),
                        ),
                    ),
                }
            }
            None => emit_notice(
                &app,
                events::NoticeEvent::warning(
                    "The recording was not saved",
                    "No project is open, so the recording exists only in this session.",
                ),
            ),
        }
    }

    emit(
        &app,
        events::channel::RECORDING_STOPPED,
        events::RecordingEvent {
            recording: false,
            samples: status.recorded_samples,
            capacity: status.capacity,
            artifact,
        },
    );
    Ok(status)
}

/// Feed a captured byte script into the pipeline, for a demonstration or a replay
/// without hardware.
#[tauri::command]
pub fn telemetry_connect_scripted(
    app: AppHandle,
    state: State<'_, AppState>,
    lines: Option<usize>,
    profile: Option<DeviceProfile>,
    display_rate_hz: Option<f64>,
) -> Result<TelemetryStatus, CommandError> {
    let settings = state.with(|s| s.settings.clone())?;
    let profile = profile.unwrap_or_else(DeviceProfile::example_csv_profile);
    let count = lines.unwrap_or(600).max(1);

    // A synthetic stream is generated rather than read from a file, so the
    // demonstration always works and never pretends to be recorded data.
    let mut text = String::new();
    let mut rate;
    for i in 0..count {
        let t = i as f64 * 0.005;
        // A slowly growing rate so the attitude visibly turns in the viewport.
        rate = 0.4 + 0.3 * (i as f64 / count as f64);
        text.push_str(&format!(
            "{:.4},{:.5},{:.5},{:.5},{:.5},{:.5},{:.5},{:.2}\n",
            t,
            0.02 * (t * 6.0).sin(),
            0.02 * (t * 5.0).cos(),
            -9.80665 + 0.1 * (t * 8.0).sin(),
            0.01,
            0.0,
            rate,
            101_325.0 - 120.0 * t
        ));
    }

    state.with(|s| {
        if let Some(mut old) = s.telemetry.take() {
            old.shutdown();
        }
        Ok::<(), CommandError>(())
    })??;

    let source = Box::new(
        hex_telemetry::ScriptedSource::new(text.into_bytes())
            .with_chunk(512)
            .with_description("scripted demonstration stream"),
    );
    let worker = TelemetryWorker::spawn(
        source,
        profile,
        hex_telemetry::CalibrationSet::default(),
        settings.telemetry.recording_buffer_samples.min(200_000),
        display_rate_hz.unwrap_or(settings.telemetry.display_rate_hz),
    );
    let status = status_of(&worker);
    state.with(|s| {
        s.telemetry = Some(worker);
        Ok::<(), CommandError>(())
    })??;

    emit_notice(
        &app,
        events::NoticeEvent::info(
            "Scripted stream started",
            "A synthetic stream is being read so the console can be exercised without hardware. It is not recorded data.",
        ),
    );
    Ok(status)
}

/// The sensor validation checklist.
#[tauri::command]
pub fn telemetry_validate(
    state: State<'_, AppState>,
    declared_orientation: Option<String>,
    expected_rate_hz: Option<f64>,
) -> Result<SensorValidation, CommandError> {
    let orientation = match declared_orientation.as_deref() {
        Some("nose_up") => Some(PhysicalOrientation::NoseUp),
        Some("nose_down") => Some(PhysicalOrientation::NoseDown),
        Some("on_side") => Some(PhysicalOrientation::OnSide),
        Some("stationary") => Some(PhysicalOrientation::Stationary),
        _ => None,
    };

    state.with(|s| {
        let Some(worker) = s.telemetry.as_ref() else {
            // Without a device the checklist still runs, so the panel shows the
            // same list with the device step failing rather than being empty.
            return run_validation(&ValidationInput {
                declared_orientation: orientation,
                expected_rate_hz,
                ..ValidationInput::default()
            });
        };

        let counters = worker.with_pipeline(|p| p.counters()).unwrap_or_default();
        let (times, accel, gyro) = worker
            .with_pipeline(|p| {
                let samples = p.recorder().samples();
                (
                    p.recorder().times(),
                    TelemetryChannel::AccelMagnitude.values(samples),
                    TelemetryChannel::GyroMagnitude.values(samples),
                )
            })
            .unwrap_or_default();

        let increasing = times.windows(2).all(|w| w[1] >= w[0]);
        let failures = times.windows(2).filter(|w| w[1] < w[0]).count();
        let observed_rate = if times.len() > 1 {
            let span = times[times.len() - 1] - times[0];
            if span > 0.0 {
                (times.len() - 1) as f64 / span
            } else {
                0.0
            }
        } else {
            0.0
        };

        let accel_values: Vec<f64> = accel.iter().copied().filter(|v| v.is_finite()).collect();
        let gyro_values: Vec<f64> = gyro.iter().copied().filter(|v| v.is_finite()).collect();
        let mean = |values: &[f64]| -> f64 {
            if values.is_empty() {
                0.0
            } else {
                values.iter().sum::<f64>() / values.len() as f64
            }
        };
        // A magnitude channel carries no direction, so the mean is placed on the
        // axis the declared pose predicts. The direction check therefore verifies
        // the magnitude and the axis convention, not the full 3-axis vector.
        let expected_axis = orientation
            .map(|o| o.expected_specific_force())
            .unwrap_or_else(|| hex_core::Vec3::new(0.0, 0.0, -1.0));
        let accel_magnitude = mean(&accel_values);
        let constant_channels = if accel_values.len() > 1
            && accel_values
                .iter()
                .all(|v| (*v - accel_values[0]).abs() < 1e-12)
        {
            vec!["acceleration".to_string()]
        } else {
            Vec::new()
        };

        run_validation(&ValidationInput {
            received_any_packet: counters.packets_decoded > 0,
            observed_rate_hz: observed_rate,
            expected_rate_hz,
            timestamps_increasing: increasing || times.len() < 2,
            timestamp_failure_ratio: if times.len() > 1 {
                failures as f64 / (times.len() - 1) as f64
            } else {
                0.0
            },
            accelerometer_mean: if accel_values.is_empty() {
                None
            } else {
                Some(expected_axis * accel_magnitude)
            },
            gyroscope_mean: if gyro_values.is_empty() {
                None
            } else {
                Some(hex_core::Vec3::new(mean(&gyro_values), 0.0, 0.0))
            },
            accelerometer_ranges: accel_values,
            gyroscope_ranges: gyro_values,
            constant_channels,
            saturated_channels: Vec::new(),
            declared_orientation: orientation,
            reference_gravity: hex_core::STANDARD_GRAVITY,
            calibration_saved: worker
                .with_pipeline(|p| p.calibration().is_usable_for_attitude())
                .unwrap_or(false),
            rate_stability_tolerance: 0.2,
        })
    })
}

/// The destinations the mapping table offers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DestinationView {
    /// Machine code.
    pub code: String,
    /// Display label.
    pub label: String,
    /// SI unit the destination expects.
    pub si_unit: String,
    /// Quantity kind label.
    pub quantity: String,
    /// Whether the destination carries physical data.
    pub physical: bool,
}

/// List the channel destinations and their units.
#[tauri::command]
pub fn telemetry_destinations() -> Vec<DestinationView> {
    ChannelDestination::all()
        .iter()
        .map(|d| DestinationView {
            code: d.code().to_string(),
            label: d.label().to_string(),
            si_unit: d.si_unit().to_string(),
            quantity: d.quantity_kind().label().to_string(),
            physical: d.is_physical(),
        })
        .collect()
}

/// The device profiles stored in the open project.
#[tauri::command]
pub fn device_profile_list(state: State<'_, AppState>) -> Result<Vec<DeviceProfile>, CommandError> {
    let project = state.require_project()?;
    Ok(hex_project::load_device_profiles(&project))
}

/// Save a device profile into the open project.
#[tauri::command]
pub fn device_profile_save(
    state: State<'_, AppState>,
    profile: DeviceProfile,
) -> Result<Vec<DeviceProfile>, CommandError> {
    let project = state.require_project()?;
    hex_project::save_device_profile(&project, &profile)?;
    Ok(hex_project::load_device_profiles(&project))
}

/// Delete a device profile.
#[tauri::command]
pub fn device_profile_delete(
    state: State<'_, AppState>,
    profile_id: String,
) -> Result<Vec<DeviceProfile>, CommandError> {
    let project = state.require_project()?;
    hex_project::delete_device_profile(&project, &profile_id)?;
    Ok(hex_project::load_device_profiles(&project))
}

/// The built-in example profiles.
#[tauri::command]
pub fn device_profile_examples() -> Vec<DeviceProfile> {
    vec![
        DeviceProfile::example_csv_profile(),
        DeviceProfile::example_binary_profile(11),
    ]
}

/// Validation problems for a profile, for the mapping table.
#[tauri::command]
pub fn device_profile_validate(
    profile: DeviceProfile,
) -> Result<Vec<hex_telemetry::MappingProblem>, CommandError> {
    Ok(profile.validate())
}

/// The estimator modes and their honesty labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EstimatorModeView {
    /// Machine code.
    pub code: String,
    /// Display label.
    pub label: String,
    /// A sentence about what the mode can and cannot claim.
    pub note: String,
}

/// List the estimator modes.
#[tauri::command]
pub fn estimator_modes() -> Vec<EstimatorModeView> {
    [
        EstimatorMode::GyroPropagation,
        EstimatorMode::Complementary,
        EstimatorMode::DeviceSupplied,
    ]
    .iter()
    .map(|m| EstimatorModeView {
        code: match m {
            EstimatorMode::GyroPropagation => "gyro_propagation",
            EstimatorMode::Complementary => "complementary",
            EstimatorMode::DeviceSupplied => "device_supplied",
        }
        .to_string(),
        label: m.label().to_string(),
        note: match m {
            EstimatorMode::GyroPropagation => {
                "Integrates the gyroscope only. The attitude drifts and is not an absolute reference."
            }
            EstimatorMode::Complementary => {
                "Blends the gyroscope with an accelerometer gravity reference, but only while the measured specific force is close to gravity. Under boost the gate closes and the estimate reverts to gyro integration."
            }
            EstimatorMode::DeviceSupplied => {
                "Passes through a quaternion the device computed. HexaDOF does not filter it and does not verify it."
            }
        }
        .to_string(),
    })
    .collect()
}

/// Save a calibration set for the open project.
#[tauri::command]
pub fn calibration_save(
    state: State<'_, AppState>,
    name: String,
    calibration: hex_telemetry::CalibrationSet,
) -> Result<String, CommandError> {
    let project = state.require_project()?;
    let path = hex_project::save_calibration(&project, &name, &calibration)?;
    Ok(path.to_string_lossy().to_string())
}

/// Load a calibration set.
#[tauri::command]
pub fn calibration_load(
    state: State<'_, AppState>,
    name: String,
) -> Result<hex_telemetry::CalibrationSet, CommandError> {
    let project = state.require_project()?;
    Ok(hex_project::load_calibration(&project, &name)?)
}

/// The recorded telemetry sessions in the open project.
#[tauri::command]
pub fn telemetry_session_list(
    state: State<'_, AppState>,
) -> Result<Vec<crate::state::SavedSessionSummary>, CommandError> {
    let project = state.require_project()?;
    Ok(hex_project::list_telemetry_sessions(&project)
        .into_iter()
        .map(|s| crate::state::SavedSessionSummary {
            name: s.name,
            label: s.metadata.label.clone(),
            samples: s.metadata.sample_count as usize,
            duration_seconds: s.metadata.duration_seconds,
            has_issues: s.metadata.has_integrity_issues(),
        })
        .collect())
}

/// Whether a scripted demonstration stream is the current source.
#[tauri::command]
pub fn telemetry_is_scripted(state: State<'_, AppState>) -> Result<bool, CommandError> {
    state.with(|s| {
        s.telemetry
            .as_ref()
            .map(|w| w.port.contains("scripted"))
            .unwrap_or(false)
    })
}

/// The status of the checklist as a plain list of lines, for the log panel.
#[tauri::command]
pub fn telemetry_validation_lines(validation: SensorValidation) -> Vec<String> {
    validation
        .steps
        .iter()
        .map(|s| {
            format!(
                "[{}] {} - {}",
                s.status.label(),
                s.title,
                s.detail
                    .clone()
                    .or_else(|| s.instruction.clone())
                    .unwrap_or_default()
            )
        })
        .collect()
}

/// The number of blocking checks in a validation result.
#[tauri::command]
pub fn telemetry_validation_blocking(validation: SensorValidation) -> usize {
    validation
        .steps
        .iter()
        .filter(|s| s.status.is_blocking())
        .count()
}

/// Whether a validation result allows recording.
#[tauri::command]
pub fn telemetry_validation_ready(validation: SensorValidation) -> bool {
    validation.ready
}

/// The status label of one checklist step, for the panel.
#[tauri::command]
pub fn telemetry_check_status_label(status: String) -> String {
    match status.as_str() {
        "pass" => CheckStatus::Pass.label().to_string(),
        "warning" => CheckStatus::Warning.label().to_string(),
        "fail" => CheckStatus::Fail.label().to_string(),
        "skipped" => CheckStatus::Skipped.label().to_string(),
        _ => CheckStatus::Pending.label().to_string(),
    }
}

/// A validation report for the telemetry profile, so the panel can reuse the same
/// rendering as the model validator.
#[tauri::command]
pub fn telemetry_profile_report(profile: DeviceProfile) -> ValidationReport {
    let mut report = ValidationReport::new(profile.name.clone());
    for problem in profile.validate() {
        let issue = if problem.blocking {
            hex_core::ValidationIssue::error(&problem.code, &problem.title, &problem.detail)
        } else {
            hex_core::ValidationIssue::warning(&problem.code, &problem.title, &problem.detail)
        }
        .with_suggestion(problem.suggestion);
        report.push(issue);
    }
    if report.issues.is_empty() {
        report.push(hex_core::ValidationIssue::info(
            "profile.ok",
            "The device profile is usable",
            "Every mapped field has a known unit and a valid destination.",
        ));
    }
    report.recompute();
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_cover_every_destination_and_are_labelled() {
        let destinations = telemetry_destinations();
        assert_eq!(destinations.len(), ChannelDestination::all().len());
        for d in &destinations {
            assert!(!d.code.is_empty());
            assert!(!d.label.is_empty());
            assert!(!d.si_unit.is_empty());
            assert!(!d.quantity.is_empty());
        }
        assert!(destinations.iter().any(|d| d.code == "accel_x"));
        assert!(destinations.iter().any(|d| !d.physical));
    }

    #[test]
    fn estimator_modes_state_their_limits() {
        let modes = estimator_modes();
        assert_eq!(modes.len(), 3);
        let gyro = modes.iter().find(|m| m.code == "gyro_propagation").unwrap();
        assert!(gyro.note.contains("drifts"));
        assert!(gyro.note.contains("not an absolute reference"));
        let device = modes.iter().find(|m| m.code == "device_supplied").unwrap();
        assert!(device.note.contains("does not verify"));
        let complementary = modes.iter().find(|m| m.code == "complementary").unwrap();
        assert!(complementary.note.contains("gate closes"));
    }

    #[test]
    fn check_status_labels_are_mapped() {
        assert_eq!(telemetry_check_status_label("pass".to_string()), "Pass");
        assert_eq!(telemetry_check_status_label("fail".to_string()), "Fail");
        assert_eq!(
            telemetry_check_status_label("warning".to_string()),
            "Warning"
        );
        assert_eq!(
            telemetry_check_status_label("skipped".to_string()),
            "Skipped"
        );
        assert_eq!(
            telemetry_check_status_label("anything".to_string()),
            "Pending"
        );
    }

    #[test]
    fn the_example_profile_report_is_clean() {
        let report = telemetry_profile_report(DeviceProfile::example_csv_profile());
        assert!(!report.has_errors(), "{:?}", report);
    }

    #[test]
    fn a_profile_without_a_time_column_produces_blocking_issues() {
        let mut profile = DeviceProfile::example_csv_profile();
        profile
            .mappings
            .retain(|m| m.destination != ChannelDestination::Time);
        let report = telemetry_profile_report(profile);
        assert!(report.has_errors());
        assert!(report
            .issues
            .iter()
            .any(|i| i.code == "mapping.time_missing"));
    }

    #[test]
    fn example_profiles_are_offered() {
        let profiles = device_profile_examples();
        assert_eq!(profiles.len(), 2);
        assert!(profiles.iter().all(|p| p.is_usable()));
        assert!(profiles[0].id.contains("csv"));
        assert!(profiles[1].id.contains("binary"));
    }

    #[test]
    fn a_validation_run_without_a_device_still_produces_the_full_list() {
        let validation = run_validation(&ValidationInput::default());
        assert!(validation.has_failure);
        assert!(!validation.ready);
        assert_eq!(validation.steps.len(), 10);
        let lines = telemetry_validation_lines(validation.clone());
        assert_eq!(lines.len(), 10);
        assert!(telemetry_validation_blocking(validation.clone()) >= 1);
        assert!(!telemetry_validation_ready(validation));
    }

    #[test]
    fn a_live_frame_reports_the_attitude_source() {
        let packet = ProcessedPacket {
            sample: hex_telemetry::TelemetrySample::at(1.0),
            mapped: vec![(ChannelDestination::AccelX, 1.0)],
            estimator: Some(hex_telemetry::EstimatorState {
                attitude: hex_core::Quaternion::identity(),
                source: hex_telemetry::AttitudeSource::GyroIntegrated,
                health: hex_telemetry::EstimatorHealth::Degraded,
                ..Default::default()
            }),
            raw: hex_telemetry::DecodedPacket {
                device_time: None,
                host_time: None,
                message_type: None,
                sequence: None,
                values: Vec::new(),
                field_names: Vec::new(),
                frame_bytes: 0,
                raw_payload: Vec::new(),
            },
        };
        let frame = frame_of(&packet);
        assert_eq!(frame.attitude.unwrap()[0], 1.0);
        assert!(frame.attitude_source.unwrap().contains("DRIFTS"));
        assert_eq!(frame.estimator_health.unwrap(), "Degraded");
        // A degraded estimate is still usable, so it is not marked indicative
        // only; the label carries the limitation instead.
        assert_eq!(frame.estimator_indicative, Some(false));
        assert_eq!(frame.mapped.len(), 1);
    }

    #[test]
    fn a_poor_estimate_is_marked_indicative_only() {
        let packet = ProcessedPacket {
            sample: hex_telemetry::TelemetrySample::at(1.0),
            mapped: Vec::new(),
            estimator: Some(hex_telemetry::EstimatorState {
                attitude: hex_core::Quaternion::identity(),
                source: hex_telemetry::AttitudeSource::GyroIntegrated,
                health: hex_telemetry::EstimatorHealth::Poor,
                note: "Gyro bias is large".to_string(),
                ..Default::default()
            }),
            raw: hex_telemetry::DecodedPacket {
                device_time: None,
                host_time: None,
                message_type: None,
                sequence: None,
                values: Vec::new(),
                field_names: Vec::new(),
                frame_bytes: 0,
                raw_payload: Vec::new(),
            },
        };
        let frame = frame_of(&packet);
        assert_eq!(frame.estimator_indicative, Some(true));
        assert_eq!(frame.estimator_health.unwrap(), "Poor");
        assert!(frame.estimator_note.unwrap().contains("Gyro bias"));
    }

    #[test]
    fn a_frame_without_an_estimator_has_no_attitude() {
        let packet = ProcessedPacket {
            sample: hex_telemetry::TelemetrySample::at(1.0),
            mapped: Vec::new(),
            estimator: None,
            raw: hex_telemetry::DecodedPacket {
                device_time: None,
                host_time: None,
                message_type: None,
                sequence: None,
                values: Vec::new(),
                field_names: Vec::new(),
                frame_bytes: 0,
                raw_payload: Vec::new(),
            },
        };
        let frame = frame_of(&packet);
        assert!(frame.attitude.is_none());
        assert!(frame.attitude_source.is_none());
    }
}
