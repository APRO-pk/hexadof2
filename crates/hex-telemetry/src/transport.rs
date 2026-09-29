//! The live ingestion pipeline, the lossless recorder, and the throttled
//! display stream.
//!
//! # Ordering
//!
//! Bytes are read, framed, checked, timestamped, written to the recorder, and
//! only then published. The recorder is never behind the UI: a slow interface can
//! drop display frames but must never cost a recorded sample.
//!
//! ```text
//! byte source -> frame decoder -> integrity check -> packet parser
//!             -> channel mapping -> calibration -> estimator
//!             -> recorder -> throttled display stream
//! ```
//!
//! # Backpressure
//!
//! Every stage has an explicit policy rather than an implicit assumption:
//!
//! * If the reader cannot keep up, bytes are lost at the driver, so the reader
//!   thread does the minimum work per byte and hands whole packets onward.
//! * If the display consumer is slow, the oldest display frames are dropped and
//!   counted. The full-rate recording is unaffected.
//! * If the recorder cannot write, ingestion stops with a visible error rather
//!   than silently dropping recorded samples.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use hex_core::{Real, TimeSource, Vec3};
use serde::{Deserialize, Serialize};

use crate::calibration::CalibrationSet;
use crate::estimation::{AttitudeEstimator, EstimatorInput, EstimatorState};
use crate::mapping::{ChannelDestination, DeviceProfile, SensorFamily};
use crate::protocol::{DecodedPacket, PacketDecoder};
use crate::serial::{ByteSource, ConnectionHealth, ConnectionState};

/// A recorded telemetry sample, in SI units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetrySample {
    /// Device time in seconds, when the device stamped the packet.
    pub device_time: Option<Real>,
    /// Host time in seconds since the session started.
    pub host_time: Real,
    /// Packet sequence number, when the format carries one.
    pub sequence: Option<u32>,
    /// Accelerometer in the body frame, m/s^2.
    pub acceleration: Option<Vec3>,
    /// Gyroscope in the body frame, rad/s.
    pub angular_rate: Option<Vec3>,
    /// Magnetometer in the body frame, tesla.
    pub magnetic_field: Option<Vec3>,
    /// Barometric pressure in pascals.
    pub pressure: Option<Real>,
    /// Temperature in kelvin.
    pub temperature: Option<Real>,
    /// GNSS latitude in radians.
    pub latitude: Option<Real>,
    /// GNSS longitude in radians.
    pub longitude: Option<Real>,
    /// GNSS altitude in metres.
    pub gnss_altitude: Option<Real>,
    /// GNSS speed in metres per second.
    pub gnss_speed: Option<Real>,
    /// Motor command as a ratio.
    pub motor: Option<Real>,
    /// Supply voltage in volts.
    pub voltage: Option<Real>,
    /// Current in amperes.
    pub current: Option<Real>,
}

impl TelemetrySample {
    /// An empty sample stamped with a host time.
    pub fn at(host_time: Real) -> Self {
        Self {
            device_time: None,
            host_time,
            sequence: None,
            acceleration: None,
            angular_rate: None,
            magnetic_field: None,
            pressure: None,
            temperature: None,
            latitude: None,
            longitude: None,
            gnss_altitude: None,
            gnss_speed: None,
            motor: None,
            voltage: None,
            current: None,
        }
    }

    /// Whether any physical channel was populated.
    pub fn has_data(&self) -> bool {
        self.acceleration.is_some()
            || self.angular_rate.is_some()
            || self.magnetic_field.is_some()
            || self.pressure.is_some()
            || self.temperature.is_some()
            || self.latitude.is_some()
            || self.gnss_altitude.is_some()
            || self.motor.is_some()
            || self.voltage.is_some()
            || self.current.is_some()
    }
}

/// A packet after mapping and calibration.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessedPacket {
    pub sample: TelemetrySample,
    /// Every mapped field with its converted value, for the packet inspector.
    pub mapped: Vec<(ChannelDestination, Real)>,
    /// Estimator report for this sample, when an estimator is running.
    pub estimator: Option<EstimatorState>,
    /// The raw decoded packet, kept so the inspector can show raw bytes.
    pub raw: DecodedPacket,
}

/// Convert a decoded packet into a telemetry sample using a profile.
///
/// Fields that are not mapped stay `None`, which is what lets the display show
/// which channels a device actually provides instead of inventing zeros.
pub fn map_packet(
    packet: &DecodedPacket,
    profile: &DeviceProfile,
    calibration: &CalibrationSet,
    host_time: Real,
) -> ProcessedPacket {
    let mut sample = TelemetrySample {
        device_time: packet.device_time,
        host_time,
        sequence: packet.sequence,
        ..TelemetrySample::at(host_time)
    };
    let mut mapped = Vec::new();
    let mut vector_accel = Vec::new();
    let mut vector_gyro = Vec::new();
    let mut vector_mag = Vec::new();

    for m in profile.mappings.iter().filter(|m| m.enabled) {
        let raw = match m.field_index {
            Some(i) => packet.values.get(i).copied(),
            None => packet.value(&m.packet_field),
        };
        let Some(raw) = raw else { continue };
        let Some(scale) = m.value_scale() else {
            continue;
        };
        let converted = scale.apply(raw);
        if !converted.is_finite() {
            continue;
        }
        mapped.push((m.destination, converted));

        // Collect three-axis readings so the family calibration and the
        // sensor-to-body rotation can be applied to the vector as a whole.
        match m.destination.sensor_family() {
            Some(SensorFamily::Accelerometer) => {
                vector_accel.push((m.destination.axis_index().unwrap_or(0), converted))
            }
            Some(SensorFamily::Gyroscope) => {
                vector_gyro.push((m.destination.axis_index().unwrap_or(0), converted))
            }
            Some(SensorFamily::Magnetometer) => {
                vector_mag.push((m.destination.axis_index().unwrap_or(0), converted))
            }
            None => match m.destination {
                ChannelDestination::Barometer => sample.pressure = Some(converted),
                ChannelDestination::Temperature => sample.temperature = Some(converted),
                ChannelDestination::GnssLatitude => sample.latitude = Some(converted),
                ChannelDestination::GnssLongitude => sample.longitude = Some(converted),
                ChannelDestination::GnssAltitude => sample.gnss_altitude = Some(converted),
                ChannelDestination::GnssSpeed => sample.gnss_speed = Some(converted),
                ChannelDestination::Motor => sample.motor = Some(converted),
                ChannelDestination::ControlSurface => sample.motor = Some(converted),
                ChannelDestination::Voltage => sample.voltage = Some(converted),
                ChannelDestination::Current => sample.current = Some(converted),
                _ => {}
            },
        }
    }

    sample.acceleration =
        assemble_vector(&vector_accel).map(|v| calibration.correct(SensorFamily::Accelerometer, v));
    sample.angular_rate =
        assemble_vector(&vector_gyro).map(|v| calibration.correct(SensorFamily::Gyroscope, v));
    sample.magnetic_field =
        assemble_vector(&vector_mag).map(|v| calibration.correct(SensorFamily::Magnetometer, v));

    ProcessedPacket {
        sample,
        mapped,
        estimator: None,
        raw: packet.clone(),
    }
}

/// Assemble three axis readings into a vector, or `None` when any axis is absent.
///
/// A partial vector is deliberately not produced: a missing axis silently read as
/// zero would corrupt every downstream attitude estimate.
fn assemble_vector(axes: &[(usize, Real)]) -> Option<Vec3> {
    if axes.len() != 3 {
        return None;
    }
    let mut v = Vec3::zeros();
    let mut seen = [false; 3];
    for (i, value) in axes {
        if *i > 2 || seen[*i] {
            return None;
        }
        seen[*i] = true;
        v[*i] = *value;
    }
    if seen.iter().all(|s| *s) {
        Some(v)
    } else {
        None
    }
}

/// How the UI stream handles a slow consumer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisplayPolicy {
    /// Target display update rate, hertz. The stream emits at most this often.
    pub update_rate_hz: Real,
    /// Maximum packets held for the display before the oldest are dropped.
    pub maximum_queue: usize,
    /// Whether the display is downsampled by taking the newest packet rather
    /// than every packet.
    pub keep_newest: bool,
}

impl Default for DisplayPolicy {
    fn default() -> Self {
        Self {
            update_rate_hz: 30.0,
            maximum_queue: 64,
            keep_newest: true,
        }
    }
}

impl DisplayPolicy {
    /// The interval between display frames, seconds.
    pub fn interval(&self) -> Real {
        if self.update_rate_hz > 0.0 {
            1.0 / self.update_rate_hz
        } else {
            0.0
        }
    }

    /// A policy that passes every packet straight through, for tests.
    pub fn unbounded() -> Self {
        Self {
            update_rate_hz: 0.0,
            maximum_queue: usize::MAX,
            keep_newest: false,
        }
    }
}

/// The throttled display stream.
///
/// Counts downsampled and dropped frames so the UI can say that it is showing a
/// reduced rate, which distinguishes display downsampling from lost data.
#[derive(Debug, Clone)]
pub struct DisplayStream {
    policy: DisplayPolicy,
    queue: VecDeque<ProcessedPacket>,
    last_emit_time: Option<Real>,
    /// Frames dropped because the consumer was behind.
    pub dropped_frames: u64,
    /// Frames deliberately skipped between emissions.
    pub downsampled_frames: u64,
}

impl DisplayStream {
    pub fn new(policy: DisplayPolicy) -> Self {
        Self {
            queue: VecDeque::with_capacity(policy.maximum_queue.min(1024)),
            policy,
            last_emit_time: None,
            dropped_frames: 0,
            downsampled_frames: 0,
        }
    }

    pub fn policy(&self) -> &DisplayPolicy {
        &self.policy
    }

    /// Offer a packet to the display stream.
    pub fn offer(&mut self, packet: ProcessedPacket) {
        let now = packet.sample.host_time;
        let due = match self.last_emit_time {
            None => true,
            Some(t) => now - t >= self.policy.interval(),
        };
        if !due {
            self.downsampled_frames += 1;
            return;
        }
        self.last_emit_time = Some(now);
        if self.queue.len() >= self.policy.maximum_queue {
            if self.policy.keep_newest {
                self.queue.pop_front();
                self.dropped_frames += 1;
            } else {
                self.dropped_frames += 1;
                return;
            }
        }
        self.queue.push_back(packet);
    }

    /// Take the next frame for display.
    ///
    /// Named rather than `next`, so it cannot be mistaken for the standard
    /// iterator method on a type that is not an iterator.
    pub fn take_frame(&mut self) -> Option<ProcessedPacket> {
        self.queue.pop_front()
    }

    /// Frames waiting.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Whether the display is showing fewer frames than the device produced.
    pub fn is_downsampled(&self) -> bool {
        self.downsampled_frames > 0 || self.dropped_frames > 0
    }

    /// A note the UI shows when it is not displaying every sample.
    pub fn note(&self) -> Option<String> {
        if !self.is_downsampled() {
            return None;
        }
        Some(format!(
            "Display downsampled to {:.0} Hz: {} frames combined, {} dropped. The recording holds every sample.",
            self.policy.update_rate_hz, self.downsampled_frames, self.dropped_frames
        ))
    }
}

/// The time a recorded sample is stored against.
///
/// A device stamp survives a burst read; host arrival time does not, because a
/// serial buffer drain delivers hundreds of samples within one host tick. The
/// device stamp is preferred for exactly that reason.
pub fn recorded_time(sample: &TelemetrySample) -> Real {
    sample.device_time.unwrap_or(sample.host_time)
}

/// The lossless recorder.
///
/// The recorder accepts every processed packet in arrival order and never drops
/// one. Capacity is bounded only by the configured sample limit, which the caller
/// sets from the available memory or from a chunked-storage policy.
#[derive(Debug, Clone)]
pub struct Recorder {
    samples: Vec<TelemetrySample>,
    mapped: Vec<Vec<(ChannelDestination, Real)>>,
    /// Maximum samples before the recorder reports being full.
    pub capacity: usize,
    recording: bool,
    /// Samples accepted since recording started.
    pub recorded: u64,
    /// Samples rejected because the recorder was full.
    pub rejected: u64,
}

impl Recorder {
    pub fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::new(),
            mapped: Vec::new(),
            capacity: capacity.max(1),
            recording: false,
            recorded: 0,
            rejected: 0,
        }
    }

    pub fn start(&mut self) {
        self.recording = true;
    }

    pub fn stop(&mut self) {
        self.recording = false;
    }

    pub fn is_recording(&self) -> bool {
        self.recording
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.samples.len() >= self.capacity
    }

    /// Accept a packet. Returns whether it was recorded.
    pub fn push(&mut self, packet: &ProcessedPacket) -> bool {
        if !self.recording {
            return false;
        }
        if self.is_full() {
            self.rejected += 1;
            return false;
        }
        self.samples.push(packet.sample.clone());
        self.mapped.push(packet.mapped.clone());
        self.recorded += 1;
        true
    }

    pub fn samples(&self) -> &[TelemetrySample] {
        &self.samples
    }

    /// The mapped field values, aligned with `samples`.
    pub fn mapped(&self) -> &[Vec<(ChannelDestination, Real)>] {
        &self.mapped
    }

    /// Extract one channel as a column, for plotting.
    ///
    /// Missing values become NaN rather than being interpolated, so a plot shows
    /// a genuine dropout as a gap.
    pub fn column(&self, selector: TelemetryChannel) -> Vec<Real> {
        self.samples.iter().map(|s| selector.value(s)).collect()
    }

    /// The sample times on the axis a recording is stored against.
    ///
    /// Host time is measured when bytes reach the application, so a burst read
    /// from a serial buffer gives hundreds of samples almost the same instant.
    /// The device's own stamp is therefore the only time axis that survives a
    /// recording, and it is used whenever the device supplied one. Host arrival
    /// time is the fallback for a device that sends no timestamp.
    pub fn times(&self) -> Vec<Real> {
        self.samples.iter().map(recorded_time).collect()
    }

    /// Drop every recorded sample, for a fresh session.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.mapped.clear();
        self.recorded = 0;
        self.rejected = 0;
    }

    /// A summary line for the recording indicator.
    pub fn status(&self) -> RecorderStatus {
        RecorderStatus {
            recording: self.recording,
            samples: self.samples.len(),
            capacity: self.capacity,
            rejected: self.rejected,
            duration_seconds: match (self.samples.first(), self.samples.last()) {
                (Some(a), Some(b)) => recorded_time(b) - recorded_time(a),
                _ => 0.0,
            },
        }
    }
}

/// A snapshot of recorder state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RecorderStatus {
    pub recording: bool,
    pub samples: usize,
    pub capacity: usize,
    pub rejected: u64,
    pub duration_seconds: Real,
}

impl RecorderStatus {
    /// Fraction of the configured capacity used.
    pub fn fill_fraction(&self) -> Real {
        if self.capacity == 0 {
            0.0
        } else {
            self.samples as Real / self.capacity as Real
        }
    }

    /// Whether the recorder is nearly full, so the user can be warned.
    pub fn is_nearly_full(&self) -> bool {
        self.fill_fraction() > 0.9
    }

    pub fn label(&self) -> String {
        format!(
            "{} samples, {:.1} s{}",
            self.samples,
            self.duration_seconds,
            if self.rejected > 0 {
                format!(", {} rejected", self.rejected)
            } else {
                String::new()
            }
        )
    }
}

/// A selectable channel of a recorded telemetry sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryChannel {
    HostTime,
    DeviceTime,
    AccelX,
    AccelY,
    AccelZ,
    AccelMagnitude,
    GyroX,
    GyroY,
    GyroZ,
    GyroMagnitude,
    MagX,
    MagY,
    MagZ,
    Pressure,
    Temperature,
    Latitude,
    Longitude,
    GnssAltitude,
    GnssSpeed,
    Motor,
    Voltage,
    Current,
}

impl TelemetryChannel {
    /// Every channel, in display order.
    pub fn all() -> &'static [TelemetryChannel] {
        use TelemetryChannel::*;
        &[
            HostTime,
            DeviceTime,
            AccelX,
            AccelY,
            AccelZ,
            AccelMagnitude,
            GyroX,
            GyroY,
            GyroZ,
            GyroMagnitude,
            MagX,
            MagY,
            MagZ,
            Pressure,
            Temperature,
            Latitude,
            Longitude,
            GnssAltitude,
            GnssSpeed,
            Motor,
            Voltage,
            Current,
        ]
    }

    pub fn label(self) -> &'static str {
        use TelemetryChannel::*;
        match self {
            HostTime => "Host time",
            DeviceTime => "Device time",
            AccelX => "Accelerometer X",
            AccelY => "Accelerometer Y",
            AccelZ => "Accelerometer Z",
            AccelMagnitude => "Acceleration",
            GyroX => "Gyroscope X",
            GyroY => "Gyroscope Y",
            GyroZ => "Gyroscope Z",
            GyroMagnitude => "Angular rate",
            MagX => "Magnetometer X",
            MagY => "Magnetometer Y",
            MagZ => "Magnetometer Z",
            Pressure => "Pressure",
            Temperature => "Temperature",
            Latitude => "Latitude",
            Longitude => "Longitude",
            GnssAltitude => "GNSS altitude",
            GnssSpeed => "GNSS speed",
            Motor => "Motor command",
            Voltage => "Voltage",
            Current => "Current",
        }
    }

    pub fn unit(self) -> &'static str {
        use TelemetryChannel::*;
        match self {
            HostTime | DeviceTime => "s",
            AccelX | AccelY | AccelZ | AccelMagnitude => "m/s^2",
            GyroX | GyroY | GyroZ | GyroMagnitude => "rad/s",
            MagX | MagY | MagZ => "T",
            Pressure => "Pa",
            Temperature => "K",
            Latitude | Longitude => "deg",
            GnssAltitude => "m",
            GnssSpeed => "m/s",
            Motor => "-",
            Voltage => "V",
            Current => "A",
        }
    }

    /// Read the channel from a sample. A missing channel reads NaN.
    pub fn value(self, s: &TelemetrySample) -> Real {
        use TelemetryChannel::*;
        let vec_component = |v: Option<Vec3>, i: usize| match v {
            Some(v) => v[i],
            None => Real::NAN,
        };
        match self {
            HostTime => s.host_time,
            DeviceTime => s.device_time.unwrap_or(Real::NAN),
            AccelX => vec_component(s.acceleration, 0),
            AccelY => vec_component(s.acceleration, 1),
            AccelZ => vec_component(s.acceleration, 2),
            AccelMagnitude => s.acceleration.map(|v| v.norm()).unwrap_or(Real::NAN),
            GyroX => vec_component(s.angular_rate, 0),
            GyroY => vec_component(s.angular_rate, 1),
            GyroZ => vec_component(s.angular_rate, 2),
            GyroMagnitude => s.angular_rate.map(|v| v.norm()).unwrap_or(Real::NAN),
            MagX => vec_component(s.magnetic_field, 0),
            MagY => vec_component(s.magnetic_field, 1),
            MagZ => vec_component(s.magnetic_field, 2),
            Pressure => s.pressure.unwrap_or(Real::NAN),
            Temperature => s.temperature.unwrap_or(Real::NAN),
            Latitude => s.latitude.unwrap_or(Real::NAN),
            Longitude => s.longitude.unwrap_or(Real::NAN),
            GnssAltitude => s.gnss_altitude.unwrap_or(Real::NAN),
            GnssSpeed => s.gnss_speed.unwrap_or(Real::NAN),
            Motor => s.motor.unwrap_or(Real::NAN),
            Voltage => s.voltage.unwrap_or(Real::NAN),
            Current => s.current.unwrap_or(Real::NAN),
        }
    }

    /// Whether the channel is a rate that benefits from a bias correction.
    pub fn is_gyro(self) -> bool {
        use TelemetryChannel::*;
        matches!(self, GyroX | GyroY | GyroZ | GyroMagnitude)
    }
}

/// Counters for the whole pipeline, shown in the health panel.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct PipelineCounters {
    pub bytes_read: u64,
    pub packets_decoded: u64,
    pub packets_rejected: u64,
    pub samples_recorded: u64,
    pub samples_dropped_for_display: u64,
    pub estimator_updates: u64,
    pub reconnect_attempts: u64,
}

/// The live ingestion pipeline.
///
/// Owns the decoder, the profile, the calibration, the estimator, the recorder,
/// and the display throttle. It is driven by `ingest`, which takes whatever
/// bytes are available, so the same code path serves a real port, a scripted
/// source, and a replayed log.
pub struct IngestionPipeline {
    profile: DeviceProfile,
    calibration: CalibrationSet,
    decoder: PacketDecoder,
    estimator: AttitudeEstimator,
    recorder: Recorder,
    display: DisplayStream,
    host_clock: HostClock,
    counters: PipelineCounters,
    health: ConnectionHealth,
    /// Most recent processed packet, for the packet inspector.
    pub last_packet: Option<ProcessedPacket>,
    /// Most recent estimator report.
    pub last_estimator: Option<EstimatorState>,
    /// Most recent rejections, capped so the log cannot grow without bound.
    pub recent_rejections: Vec<String>,
    /// Maximum retention for `recent_rejections`.
    pub rejection_log_limit: usize,
}

impl IngestionPipeline {
    /// Build a pipeline for a profile and calibration.
    pub fn new(
        profile: DeviceProfile,
        calibration: CalibrationSet,
        recorder_capacity: usize,
    ) -> Self {
        let decoder = PacketDecoder::new(profile.format.clone());
        let estimator =
            AttitudeEstimator::new(profile.estimator).with_gyro_bias(calibration.gyroscope.offset);
        Self {
            profile,
            calibration,
            decoder,
            estimator,
            recorder: Recorder::new(recorder_capacity),
            display: DisplayStream::new(DisplayPolicy::default()),
            host_clock: HostClock::new(),
            counters: PipelineCounters::default(),
            health: ConnectionHealth::default(),
            last_packet: None,
            last_estimator: None,
            recent_rejections: Vec::new(),
            rejection_log_limit: 64,
        }
    }

    /// Replace the display policy, for a high-rate device.
    pub fn with_display_policy(mut self, policy: DisplayPolicy) -> Self {
        self.display = DisplayStream::new(policy);
        self
    }

    /// Replace the estimator.
    pub fn with_estimator(mut self, estimator: AttitudeEstimator) -> Self {
        self.estimator = estimator;
        self
    }

    pub fn profile(&self) -> &DeviceProfile {
        &self.profile
    }

    pub fn calibration(&self) -> &CalibrationSet {
        &self.calibration
    }

    pub fn calibration_mut(&mut self) -> &mut CalibrationSet {
        &mut self.calibration
    }

    pub fn recorder(&self) -> &Recorder {
        &self.recorder
    }

    pub fn recorder_mut(&mut self) -> &mut Recorder {
        &mut self.recorder
    }

    pub fn display(&self) -> &DisplayStream {
        &self.display
    }

    pub fn display_mut(&mut self) -> &mut DisplayStream {
        &mut self.display
    }

    pub fn counters(&self) -> PipelineCounters {
        self.counters
    }

    pub fn health(&self) -> ConnectionHealth {
        self.health
    }

    pub fn decoder(&self) -> &PacketDecoder {
        &self.decoder
    }

    pub fn estimator(&self) -> &AttitudeEstimator {
        &self.estimator
    }

    /// Start recording.
    pub fn start_recording(&mut self) {
        self.recorder.start();
    }

    /// Stop recording.
    pub fn stop_recording(&mut self) {
        self.recorder.stop();
    }

    /// Reset the decoder and estimator, for a reconnect.
    pub fn reset_for_reconnect(&mut self) {
        self.decoder.reset();
        self.estimator = AttitudeEstimator::new(self.profile.estimator)
            .with_gyro_bias(self.calibration.gyroscope.offset);
        self.host_clock.reset();
        self.counters.reconnect_attempts += 1;
    }

    /// Feed bytes into the pipeline.
    ///
    /// Returns the number of packets that produced a sample.
    pub fn ingest(&mut self, bytes: &[u8]) -> usize {
        self.counters.bytes_read += bytes.len() as u64;
        self.health.bytes_received += bytes.len() as u64;

        let outcome = self.decoder.push(bytes);
        self.counters.packets_decoded += outcome.packets.len() as u64;
        self.health.packets_received += outcome.packets.len() as u64;
        self.counters.packets_rejected += outcome.rejections.len() as u64;
        self.health.packets_rejected += outcome.rejections.len() as u64;
        self.health.discarded_bytes += outcome.bytes_discarded as u64;

        for rejection in &outcome.rejections {
            if matches!(
                rejection,
                crate::protocol::FrameRejection::ChecksumMismatch { .. }
            ) {
                self.health.crc_failures += 1;
            }
            if self.recent_rejections.len() >= self.rejection_log_limit {
                self.recent_rejections.remove(0);
            }
            self.recent_rejections
                .push(format!("{}: {}", rejection.code(), rejection.detail()));
        }

        self.health.packets_dropped = self.decoder.sequence_gaps;

        let mut produced = 0usize;
        for packet in &outcome.packets {
            let host_time = self.host_clock.observe(packet.device_time);
            self.health.last_packet_time = Some(host_time);

            let mut processed = map_packet(packet, &self.profile, &self.calibration, host_time);

            // The estimator runs on every packet, before the display throttle, so
            // a slower display never coarsens the attitude history.
            if let (Some(accel), Some(gyro)) =
                (processed.sample.acceleration, processed.sample.angular_rate)
            {
                let dt = self.host_clock.last_dt().unwrap_or(0.0);
                let input = EstimatorInput {
                    time: host_time,
                    dt,
                    gyro_body: gyro,
                    accel_body: accel,
                    mag_body: processed.sample.magnetic_field,
                    device_quaternion: None,
                    accelerometer_saturated: false,
                    gyroscope_saturated: false,
                };
                let state = self.estimator.update(&input);
                self.counters.estimator_updates += 1;
                self.last_estimator = Some(state.clone());
                processed.estimator = Some(state);
            }

            if self.recorder.push(&processed) {
                self.counters.samples_recorded += 1;
            }
            self.display.offer(processed.clone());
            self.last_packet = Some(processed);
            produced += 1;
        }
        produced
    }

    /// The connection state implied by the current health and time.
    pub fn connection_state(&self, stall_seconds: Real) -> ConnectionState {
        let now = self.host_clock.elapsed();
        self.health.state(now, stall_seconds)
    }

    /// Drain the display stream, up to `limit` frames.
    pub fn drain_display(&mut self, limit: usize) -> Vec<ProcessedPacket> {
        let mut out = Vec::new();
        for _ in 0..limit {
            match self.display.take_frame() {
                Some(p) => out.push(p),
                None => break,
            }
        }
        self.counters.samples_dropped_for_display = self.display.dropped_frames;
        out
    }

    /// A one-line pipeline status for the health panel.
    pub fn status_line(&self, stall_seconds: Real) -> String {
        let state = self.connection_state(stall_seconds);
        format!(
            "{}: {} packets, {} rejected, {} recorded, display {}",
            state.label(),
            self.decoder.packets_accepted,
            self.counters.packets_rejected,
            self.counters.samples_recorded,
            if self.display.is_downsampled() {
                "downsampled"
            } else {
                "full rate"
            }
        )
    }

    /// Read from a byte source and ingest whatever is available.
    ///
    /// Returns the number of packets processed. `Ok(0)` with no error means the
    /// source had nothing to give this call, which is normal.
    pub fn pump(
        &mut self,
        source: &mut dyn ByteSource,
        buffer_bytes: usize,
    ) -> std::io::Result<usize> {
        let mut buffer = vec![0u8; buffer_bytes.max(64)];
        let n = source.read_available(&mut buffer)?;
        if n == 0 {
            return Ok(0);
        }
        Ok(self.ingest(&buffer[..n]))
    }
}

/// Host clock assignment and device-to-host offset tracking.
///
/// Device time and host time are different clocks, and a log must record which
/// one a value came from. The clock keeps both and smooths the offset.
///
/// # Step size
///
/// The sample interval used by the estimator is taken from the device clock when
/// the device supplies one, and from the host clock only as a fallback. Host time
/// is measured when a packet reaches the application, so a burst read from a
/// serial buffer can deliver a hundred samples with a host interval near zero.
/// Using that interval would freeze every integrator, so the device's own clock is
/// the authoritative sample interval whenever it is available and plausible.
#[derive(Debug, Clone)]
pub struct HostClock {
    started: std::time::Instant,
    device_clock: hex_core::DeviceClock,
    last_host: Option<Real>,
    last_device: Option<Real>,
    last_dt: Option<Real>,
    source: TimeSource,
    /// Samples where the device clock supplied no time and host time was used.
    pub host_stamped: u64,
    /// Samples where the device supplied the time.
    pub device_stamped: u64,
    /// Samples whose interval came from the device clock.
    pub intervals_from_device: u64,
    /// Samples whose interval came from the host clock.
    pub intervals_from_host: u64,
    /// Largest interval accepted, seconds. Anything longer is treated as a gap and
    /// replaced by the host interval, so a device clock jump cannot blow up an
    /// integrator.
    pub maximum_plausible_dt: Real,
}

impl Default for HostClock {
    fn default() -> Self {
        Self::new()
    }
}

impl HostClock {
    pub fn new() -> Self {
        Self {
            started: std::time::Instant::now(),
            device_clock: hex_core::DeviceClock::default(),
            last_host: None,
            last_device: None,
            last_dt: None,
            source: TimeSource::Device,
            host_stamped: 0,
            device_stamped: 0,
            intervals_from_device: 0,
            intervals_from_host: 0,
            maximum_plausible_dt: 10.0,
        }
    }

    /// Seconds since the clock was created.
    pub fn elapsed(&self) -> Real {
        self.started.elapsed().as_secs_f64()
    }

    /// Reset the clock, for a reconnect.
    pub fn reset(&mut self) {
        self.started = std::time::Instant::now();
        self.device_clock = hex_core::DeviceClock::default();
        self.last_host = None;
        self.last_device = None;
        self.last_dt = None;
    }

    /// Take an observation of the device time and return the host time.
    pub fn observe(&mut self, device_time: Option<Real>) -> Real {
        let host = self.elapsed();
        let host_dt = self.last_host.map(|previous| host - previous);

        let device_dt = device_time.and_then(|device| {
            let dt = self.last_device.map(|previous| device - previous);
            self.last_device = Some(device);
            dt
        });

        if let Some(device) = device_time {
            self.device_clock.update(device, host);
            self.device_stamped += 1;
            self.source = TimeSource::Device;
        } else {
            self.host_stamped += 1;
            self.source = TimeSource::Host;
        }

        // Prefer the device interval when it is usable, because it reflects the
        // device's own sampling and is immune to how bytes were buffered.
        self.last_dt = match device_dt {
            Some(dt) if dt > 0.0 && dt.is_finite() && dt <= self.maximum_plausible_dt => {
                self.intervals_from_device += 1;
                Some(dt)
            }
            _ => {
                if host_dt.is_some() {
                    self.intervals_from_host += 1;
                }
                host_dt.filter(|dt| *dt > 0.0 && dt.is_finite())
            }
        };

        self.last_host = Some(host);
        host
    }

    /// Time since the previous sample, if there was one.
    pub fn last_dt(&self) -> Option<Real> {
        self.last_dt
    }

    /// Which clock the recent samples used.
    pub fn source(&self) -> TimeSource {
        self.source
    }

    /// Whether the device clock offset has stabilised.
    pub fn device_clock_locked(&self) -> bool {
        self.device_clock.is_locked()
    }

    /// A note describing the timebase, for the session metadata.
    pub fn note(&self) -> String {
        match self.source {
            TimeSource::Device => format!(
                "Device stamped {} samples, host stamped {}. Intervals came from the device {} times and the host {} times.",
                self.device_stamped, self.host_stamped, self.intervals_from_device, self.intervals_from_host
            ),
            TimeSource::Host => format!(
                "The device supplied no timestamp, so host time was used for all {} samples",
                self.host_stamped
            ),
            _ => "Time source not yet established".to_string(),
        }
    }
}

/// A guard that stops a session when dropped, so a closed window cannot leave the
/// recorder running.
#[derive(Debug)]
pub struct SessionGuard {
    running: Arc<AtomicBool>,
    samples: Arc<AtomicU64>,
}

impl SessionGuard {
    pub fn new(running: Arc<AtomicBool>, samples: Arc<AtomicU64>) -> Self {
        Self { running, samples }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn sample_count(&self) -> u64 {
        self.samples.load(Ordering::SeqCst)
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

/// A shared, thread-safe inbox for bytes read on a background thread.
///
/// The reader thread only appends, and the consumer drains, so neither blocks
/// the other on a lock for more than the length of a memory copy.
#[derive(Debug, Default)]
pub struct ByteInbox {
    inner: Mutex<VecDeque<u8>>,
    /// Maximum bytes held. Beyond this the oldest bytes are discarded, which is
    /// what a real serial buffer does.
    pub limit: usize,
    /// Bytes discarded because the consumer fell behind.
    pub overflowed: u64,
    /// Total bytes accepted.
    pub accepted: u64,
}

impl ByteInbox {
    pub fn new(limit: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::new()),
            limit: limit.max(1024),
            overflowed: 0,
            accepted: 0,
        }
    }

    /// Accept bytes from the reader thread.
    pub fn push(&mut self, bytes: &[u8]) {
        let mut q = self.inner.lock().expect("byte inbox poisoned");
        q.extend(bytes.iter().copied());
        self.accepted += bytes.len() as u64;
        while q.len() > self.limit {
            q.pop_front();
            self.overflowed += 1;
        }
    }

    /// Drain at most `max` bytes.
    pub fn drain(&mut self, max: usize) -> Vec<u8> {
        let mut q = self.inner.lock().expect("byte inbox poisoned");
        let take = max.min(q.len());
        q.drain(..take).collect()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|q| q.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&mut self) {
        if let Ok(mut q) = self.inner.lock() {
            q.clear();
        }
    }
}

/// Convert a recorded session into parallel columns for the log file.
///
/// Returns the channel names and one column per channel, with NaN where a sample
/// did not carry the channel.
pub fn recorder_columns(
    recorder: &Recorder,
    channels: &[TelemetryChannel],
) -> (Vec<String>, Vec<Vec<Real>>) {
    let names = channels.iter().map(|c| c.label().to_string()).collect();
    let columns = channels
        .iter()
        .map(|c| c.values(&recorder.samples))
        .collect();
    (names, columns)
}

impl TelemetryChannel {
    /// Every value of this channel across a recording.
    pub fn values(&self, samples: &[TelemetrySample]) -> Vec<Real> {
        samples.iter().map(|s| self.value(s)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PacketFormat;
    use crate::serial::ScriptedSource;
    use hex_core::STANDARD_GRAVITY;

    fn pipeline(capacity: usize) -> IngestionPipeline {
        IngestionPipeline::new(
            DeviceProfile::example_csv_profile(),
            CalibrationSet::default(),
            capacity,
        )
    }

    // The helper mirrors the eight columns of the example CSV packet format, so
    // its argument count is the format rather than an accident.
    #[allow(clippy::too_many_arguments)]
    fn csv_line(
        t: Real,
        ax: Real,
        ay: Real,
        az: Real,
        gx: Real,
        gy: Real,
        gz: Real,
        baro: Real,
    ) -> String {
        format!("{},{},{},{},{},{},{},{}\n", t, ax, ay, az, gx, gy, gz, baro)
    }

    #[test]
    fn ingest_maps_a_csv_packet_into_a_sample() {
        let mut p = pipeline(100);
        p.start_recording();
        let n = p
            .ingest(csv_line(0.0, 0.0, 0.0, -STANDARD_GRAVITY, 0.0, 0.0, 0.0, 101325.0).as_bytes());
        assert_eq!(n, 1);
        let sample = &p.last_packet.as_ref().unwrap().sample;
        assert!((sample.acceleration.unwrap().z + STANDARD_GRAVITY).abs() < 1e-9);
        assert!((sample.angular_rate.unwrap().norm()).abs() < 1e-12);
        assert!((sample.pressure.unwrap() - 101_325.0).abs() < 1e-9);
        assert_eq!(p.recorder().len(), 1);
    }

    #[test]
    fn recorder_only_accumulates_while_recording() {
        let mut p = pipeline(100);
        p.ingest(csv_line(0.0, 0.0, 0.0, -9.81, 0.0, 0.0, 0.0, 101325.0).as_bytes());
        assert_eq!(p.recorder().len(), 0);
        p.start_recording();
        p.ingest(csv_line(0.01, 0.0, 0.0, -9.81, 0.0, 0.0, 0.0, 101325.0).as_bytes());
        assert_eq!(p.recorder().len(), 1);
        p.stop_recording();
        p.ingest(csv_line(0.02, 0.0, 0.0, -9.81, 0.0, 0.0, 0.0, 101325.0).as_bytes());
        assert_eq!(p.recorder().len(), 1);
    }

    #[test]
    fn a_burst_read_records_the_device_time_not_the_arrival_time() {
        // A serial buffer drain hands hundreds of packets to the pipeline at
        // once, so they all arrive within one host tick. A recording stored
        // against arrival time would report a four second flight as lasting a
        // fraction of a millisecond.
        let mut p = pipeline(1_000);
        let mut text = String::new();
        for i in 0..400 {
            text.push_str(&csv_line(
                i as Real * 0.01,
                0.0,
                0.0,
                -9.81,
                0.0,
                0.0,
                0.0,
                101_325.0,
            ));
        }
        p.start_recording();
        let recorded = p.ingest(text.as_bytes());
        p.stop_recording();
        assert_eq!(recorded, 400);
        assert_eq!(p.recorder().len(), 400);

        let times = p.recorder().times();
        assert!((times[0] - 0.0).abs() < 1e-9);
        assert!(
            (times[399] - 3.99).abs() < 1e-9,
            "the recorded axis is the device clock: {}",
            times[399]
        );
        assert!(times.windows(2).all(|w| w[1] > w[0]));
        let status = p.recorder().status();
        assert!(
            (status.duration_seconds - 3.99).abs() < 1e-9,
            "duration was {}",
            status.duration_seconds
        );
        // Host arrival time is still available as its own channel, so the
        // difference between the two clocks stays visible.
        let host = p.recorder().column(TelemetryChannel::HostTime);
        assert!(host[399] - host[0] < 0.5);
        let device = p.recorder().column(TelemetryChannel::DeviceTime);
        assert!((device[399] - 3.99).abs() < 1e-9);
    }

    #[test]
    fn recorder_reports_when_it_is_full() {
        let mut r = Recorder::new(2);
        r.start();
        for i in 0..3 {
            let s = TelemetrySample::at(i as Real);
            let packet = ProcessedPacket {
                sample: s,
                mapped: Vec::new(),
                estimator: None,
                raw: DecodedPacket {
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
            r.push(&packet);
        }
        assert_eq!(r.len(), 2);
        assert_eq!(r.rejected, 1);
        let status = r.status();
        assert!(status.is_nearly_full());
        assert!(status.label().contains("rejected"));
    }

    #[test]
    fn display_stream_downsamples_to_the_configured_rate() {
        let mut stream = DisplayStream::new(DisplayPolicy {
            update_rate_hz: 10.0,
            maximum_queue: 8,
            keep_newest: true,
        });
        for i in 0..100 {
            let t = i as Real * 0.001;
            stream.offer(ProcessedPacket {
                sample: TelemetrySample::at(t),
                mapped: Vec::new(),
                estimator: None,
                raw: empty_packet(t),
            });
        }
        // 100 ms of data at a 10 Hz display rate is about one or two frames.
        assert!(stream.pending() <= 2, "pending {}", stream.pending());
        assert!(stream.is_downsampled());
        assert!(stream.note().unwrap().contains("downsampled"));
    }

    #[test]
    fn display_stream_drops_the_oldest_when_the_queue_is_full() {
        let mut stream = DisplayStream::new(DisplayPolicy {
            update_rate_hz: 0.0,
            maximum_queue: 2,
            keep_newest: true,
        });
        for i in 0..5 {
            stream.offer(ProcessedPacket {
                sample: TelemetrySample::at(i as Real),
                mapped: Vec::new(),
                estimator: None,
                raw: empty_packet(i as Real),
            });
        }
        assert_eq!(stream.pending(), 2);
        assert_eq!(stream.dropped_frames, 3);
        // The retained frames are the newest.
        assert!((stream.take_frame().unwrap().sample.host_time - 3.0).abs() < 1e-12);
    }

    #[test]
    fn display_stream_note_is_absent_at_full_rate() {
        let mut stream = DisplayStream::new(DisplayPolicy::unbounded());
        stream.offer(ProcessedPacket {
            sample: TelemetrySample::at(0.0),
            mapped: Vec::new(),
            estimator: None,
            raw: empty_packet(0.0),
        });
        assert!(!stream.is_downsampled());
        assert!(stream.note().is_none());
    }

    #[test]
    fn estimator_runs_on_every_packet_not_every_display_frame() {
        let mut p = pipeline(1000).with_display_policy(DisplayPolicy {
            update_rate_hz: 1.0,
            maximum_queue: 4,
            keep_newest: true,
        });
        let mut text = String::new();
        for i in 0..100 {
            text.push_str(&csv_line(
                i as Real * 0.01,
                0.0,
                0.0,
                -STANDARD_GRAVITY,
                0.0,
                0.0,
                0.5,
                101_325.0,
            ));
        }
        let packets = p.ingest(text.as_bytes());
        assert_eq!(packets, 100);
        assert_eq!(p.counters().estimator_updates, 100);
        assert!(p.display().is_downsampled());
    }

    #[test]
    fn pipeline_counts_rejections_and_records_them_with_codes() {
        let mut p = pipeline(100);
        p.ingest(b"0.0,1.0\n");
        assert_eq!(p.counters().packets_rejected, 1);
        assert!(!p.recent_rejections.is_empty());
        assert!(p.recent_rejections[0].contains("packet.field_count"));
    }

    #[test]
    fn rejection_log_is_bounded() {
        let mut p = pipeline(100);
        p.rejection_log_limit = 4;
        for _ in 0..20 {
            p.ingest(b"0.0,1.0\n");
        }
        assert_eq!(p.recent_rejections.len(), 4);
    }

    #[test]
    fn pump_reads_from_a_scripted_source() {
        let mut p = pipeline(100);
        p.start_recording();
        let data = csv_line(0.0, 0.0, 0.0, -9.81, 0.0, 0.0, 0.0, 101325.0)
            + &csv_line(0.01, 0.0, 0.0, -9.81, 0.0, 0.0, 0.0, 101325.0);
        let mut source = ScriptedSource::new(data.into_bytes()).with_chunk(4096);
        let processed = p.pump(&mut source, 1024).unwrap();
        assert_eq!(processed, 2);
        assert_eq!(p.recorder().len(), 2);
        // A second pump on an exhausted source is not an error.
        assert_eq!(p.pump(&mut source, 1024).unwrap(), 0);
    }

    #[test]
    fn reset_for_reconnect_clears_partial_state() {
        let mut p = pipeline(100);
        p.ingest(b"0.0,1.0,2.0,");
        assert!(p.decoder().buffered_bytes() > 0);
        p.reset_for_reconnect();
        assert_eq!(p.decoder().buffered_bytes(), 0);
        assert_eq!(p.counters().reconnect_attempts, 1);
    }

    #[test]
    fn recorder_columns_align_with_the_channel_list() {
        let mut r = Recorder::new(10);
        r.start();
        for i in 0..3 {
            let mut s = TelemetrySample::at(i as Real);
            s.acceleration = Some(Vec3::new(i as Real, 0.0, 0.0));
            r.push(&ProcessedPacket {
                sample: s,
                mapped: Vec::new(),
                estimator: None,
                raw: empty_packet(i as Real),
            });
        }
        let (names, columns) =
            recorder_columns(&r, &[TelemetryChannel::AccelX, TelemetryChannel::Pressure]);
        assert_eq!(names.len(), 2);
        assert_eq!(columns[0], vec![0.0, 1.0, 2.0]);
        // The pressure channel was never set, so it must be NaN, not zero.
        assert!(columns[1].iter().all(|v| v.is_nan()));
    }

    #[test]
    fn missing_channels_read_as_nan_rather_than_zero() {
        let sample = TelemetrySample::at(1.0);
        for channel in TelemetryChannel::all() {
            let v = channel.value(&sample);
            if *channel == TelemetryChannel::HostTime {
                assert!((v - 1.0).abs() < 1e-12);
            } else {
                assert!(v.is_nan(), "{:?} returned {}", channel, v);
            }
        }
    }

    #[test]
    fn telemetry_channel_metadata_is_complete() {
        for c in TelemetryChannel::all() {
            assert!(!c.label().is_empty());
            assert!(!c.unit().is_empty());
        }
        assert!(TelemetryChannel::GyroY.is_gyro());
        assert!(!TelemetryChannel::AccelX.is_gyro());
    }

    #[test]
    fn host_clock_records_device_and_host_stamps() {
        let mut clock = HostClock::new();
        let t1 = clock.observe(Some(1000.0));
        let t2 = clock.observe(Some(1000.01));
        // Host time is monotonic, not strictly increasing per call: two calls can
        // land inside one clock tick. The device interval is what the estimator
        // relies on, so that is the value that must be exact.
        assert!(t2 >= t1);
        assert_eq!(clock.device_stamped, 2);
        assert_eq!(clock.source(), TimeSource::Device);
        assert!(clock.note().contains("Device stamped 2"));
        let dt = clock.last_dt().expect("a device interval was available");
        assert!((dt - 0.01).abs() < 1e-9, "dt was {}", dt);
        assert_eq!(clock.intervals_from_device, 1);

        let t3 = clock.observe(None);
        assert!(t3 >= t2);
        assert_eq!(clock.host_stamped, 1);
        assert_eq!(clock.source(), TimeSource::Host);
        assert!(clock.note().contains("host time was used"));
    }

    #[test]
    fn a_burst_read_uses_the_device_interval_not_the_host_interval() {
        // A serial read can deliver a whole buffer at once, so every packet sees a
        // host interval near zero. If that interval were used, every integrator
        // would freeze.
        let mut clock = HostClock::new();
        for i in 0..50 {
            clock.observe(Some(0.005 * i as Real));
        }
        assert_eq!(clock.intervals_from_device, 49);
        let dt = clock.last_dt().expect("a device interval was available");
        assert!((dt - 0.005).abs() < 1e-9, "dt was {}", dt);
    }

    #[test]
    fn an_implausible_device_jump_is_not_accepted_as_a_step() {
        let mut clock = HostClock::new();
        clock.observe(Some(0.0));
        // A device clock that jumps by an hour must not become a one hour step.
        clock.observe(Some(3600.0));
        if let Some(dt) = clock.last_dt() {
            assert!(dt < clock.maximum_plausible_dt, "dt was {}", dt);
        }
        assert_eq!(clock.intervals_from_device, 0);
    }

    #[test]
    fn host_clock_locks_after_enough_observations() {
        let mut clock = HostClock::new();
        assert!(!clock.device_clock_locked());
        for i in 0..6 {
            clock.observe(Some(100.0 + i as Real));
        }
        assert!(clock.device_clock_locked());
    }

    #[test]
    fn byte_inbox_bounds_its_buffer_and_counts_overflow() {
        let mut inbox = ByteInbox::new(1024);
        inbox.push(&vec![7u8; 2000]);
        assert_eq!(inbox.len(), 1024);
        assert_eq!(inbox.overflowed, 976);
        assert_eq!(inbox.accepted, 2000);
        let drained = inbox.drain(100);
        assert_eq!(drained.len(), 100);
        assert_eq!(inbox.len(), 924);
        assert!(drained.iter().all(|b| *b == 7));
        inbox.clear();
        assert!(inbox.is_empty());
        // A tiny limit is raised to a workable minimum.
        let small = ByteInbox::new(1);
        assert!(small.limit >= 1024);
    }

    #[test]
    fn session_guard_stops_on_drop() {
        let running = Arc::new(AtomicBool::new(true));
        let samples = Arc::new(AtomicU64::new(5));
        {
            let guard = SessionGuard::new(running.clone(), samples.clone());
            assert!(guard.is_running());
            assert_eq!(guard.sample_count(), 5);
        }
        assert!(!running.load(Ordering::SeqCst));
    }

    #[test]
    fn pipeline_status_line_reports_the_counts() {
        let mut p = pipeline(100);
        p.start_recording();
        p.ingest(csv_line(0.0, 0.0, 0.0, -9.81, 0.0, 0.0, 0.0, 101325.0).as_bytes());
        let line = p.status_line(1.0);
        assert!(line.contains("1 recorded"), "{}", line);
    }

    #[test]
    fn connection_state_becomes_stalled_after_the_window() {
        let mut p = pipeline(100);
        p.ingest(csv_line(0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 101325.0).as_bytes());
        assert_eq!(p.connection_state(60.0), ConnectionState::Receiving);
        assert_eq!(p.connection_state(0.0), ConnectionState::Stalled);
    }

    #[test]
    fn partial_vector_is_not_assembled_from_two_axes() {
        // A profile that maps only two accelerometer axes must not produce a
        // vector with a silent zero on the third.
        let mut profile = DeviceProfile::example_csv_profile();
        profile
            .mappings
            .retain(|m| m.destination != ChannelDestination::AccelZ);
        let mut p = IngestionPipeline::new(profile, CalibrationSet::default(), 10);
        p.ingest(csv_line(0.0, 1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 101325.0).as_bytes());
        let sample = &p.last_packet.as_ref().unwrap().sample;
        assert!(sample.acceleration.is_none());
    }

    #[test]
    fn calibration_is_applied_before_the_estimator() {
        let mut calibration = CalibrationSet::default();
        calibration.gyroscope = crate::calibration::ThreeAxisCalibration {
            offset: Vec3::new(0.0, 0.0, 0.5),
            gain: Vec3::new(1.0, 1.0, 1.0),
            observations: 1,
            complete: true,
            residual: None,
            note: None,
        };
        let mut p = IngestionPipeline::new(DeviceProfile::example_csv_profile(), calibration, 100);
        p.ingest(csv_line(0.0, 0.0, 0.0, -STANDARD_GRAVITY, 0.0, 0.0, 0.5, 101325.0).as_bytes());
        let gyro = p.last_packet.as_ref().unwrap().sample.angular_rate.unwrap();
        // The 0.5 rad/s bias must be removed.
        assert!(gyro.norm() < 1e-12, "gyro {:?}", gyro);
    }

    #[test]
    fn binary_format_pipeline_end_to_end() {
        use crate::protocol::encode_frame;
        let profile = DeviceProfile::example_binary_profile(4);
        let spec = match &profile.format {
            PacketFormat::BinaryFramed(s) => s.clone(),
            _ => panic!("expected binary"),
        };
        let mut bytes = Vec::new();
        for i in 0..5u32 {
            let frame = encode_frame(
                &spec,
                i,
                (i as u64) * 1000,
                &[0.0, 0.0, -STANDARD_GRAVITY as f32 as Real, 0.1],
            )
            .unwrap();
            bytes.extend_from_slice(&frame);
        }
        let mut p = IngestionPipeline::new(profile, CalibrationSet::default(), 100);
        p.start_recording();
        let n = p.ingest(&bytes);
        assert_eq!(n, 5);
        assert_eq!(p.recorder().len(), 5);
        assert_eq!(p.counters().packets_rejected, 0);
        // Device time came from the 32-bit microsecond counter.
        let sample = &p.recorder().samples()[4];
        assert!((sample.device_time.unwrap() - 0.004).abs() < 1e-9);
    }

    #[test]
    fn binary_spec_default_helper_is_used() {
        let spec = crate::protocol::BinaryFrameSpec::hexadof_default(4);
        assert_eq!(spec.fields.len(), 4);
        assert_eq!(spec.fields[0].name, "ax");
    }

    fn empty_packet(t: Real) -> DecodedPacket {
        DecodedPacket {
            device_time: Some(t),
            host_time: None,
            message_type: None,
            sequence: None,
            values: Vec::new(),
            field_names: Vec::new(),
            frame_bytes: 0,
            raw_payload: Vec::new(),
        }
    }
}
