//! Application state shared by the Tauri commands.
//!
//! Long-running work never runs on the command thread. A simulation runs in a
//! spawned task and reports progress through events. Telemetry ingestion runs in a
//! dedicated thread whose only job is to read bytes and hand whole packets to the
//! pipeline, so a slow interface cannot change what gets recorded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use hex_analysis::Comparison;
use hex_dynamics::{CancellationToken, RunOutcome, RunProgress};
use hex_flight_data::FlightSession;
use hex_model::ImportedModel;
use hex_project::{Project, Settings};
use hex_telemetry::{
    ByteSource, DeviceProfile, IngestionPipeline, ProcessedPacket, RecorderStatus,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::error::CommandError;
use crate::events;

/// A simulation that has been run and is available to inspect.
pub struct StoredRun {
    /// The outcome, which is the authoritative state history.
    pub outcome: RunOutcome,
    /// Directory name, when the run was saved into the project.
    pub artifact_name: Option<String>,
}

/// A simulation currently running.
pub struct ActiveSimulation {
    pub cancel: CancellationToken,
    /// Progress reported by the most recent event, for a polled fallback.
    pub progress: Arc<Mutex<Option<RunProgress>>>,
}

/// The telemetry ingestion worker.
///
/// The reader thread owns the byte source and the pipeline is behind a mutex, so
/// commands can inspect health and drain display frames while reading continues.
pub struct TelemetryWorker {
    /// Requested stop flag.
    stop: Arc<AtomicBool>,
    /// The ingestion pipeline, shared with the reader thread.
    pipeline: Arc<Mutex<IngestionPipeline>>,
    /// The reader thread handle.
    handle: Option<std::thread::JoinHandle<()>>,
    /// Port name the worker is reading from.
    pub port: String,
    /// Baud rate in use.
    pub baud_rate: u32,
    /// Whether the byte source reported end of stream, which only happens for a
    /// scripted or file source.
    pub finished: Arc<AtomicBool>,
    /// The most recent read error, if any.
    pub last_error: Arc<Mutex<Option<String>>>,
}

impl TelemetryWorker {
    /// Start reading from a byte source on a background thread.
    pub fn spawn(
        mut source: Box<dyn ByteSource + Send>,
        profile: DeviceProfile,
        calibration: hex_telemetry::CalibrationSet,
        buffer_samples: usize,
        display_rate_hz: f64,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let last_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let pipeline = Arc::new(Mutex::new(
            IngestionPipeline::new(profile.clone(), calibration, buffer_samples)
                .with_display_policy(hex_telemetry::DisplayPolicy {
                    update_rate_hz: display_rate_hz,
                    ..hex_telemetry::DisplayPolicy::default()
                }),
        ));

        let stop_for_thread = stop.clone();
        let finished_for_thread = finished.clone();
        let error_for_thread = last_error.clone();
        let pipeline_for_thread = pipeline.clone();
        let port = source.describe();
        let baud_rate = profile.serial.baud_rate;

        let handle = std::thread::Builder::new()
            .name("hexadof-serial-reader".to_string())
            .spawn(move || {
                let mut buffer = vec![0u8; 4096];
                while !stop_for_thread.load(Ordering::SeqCst) {
                    match source.read_available(&mut buffer) {
                        Ok(0) => {
                            // A file or scripted source that has handed out
                            // everything it has must end the loop rather than wait
                            // for data that cannot arrive.
                            if source.is_exhausted() {
                                break;
                            }
                            // A serial port has nothing available this call. Its
                            // blocking read has already used its timeout, so a
                            // short sleep keeps the loop from spinning.
                            std::thread::sleep(std::time::Duration::from_millis(2));
                        }
                        Ok(n) => {
                            if let Ok(mut pipeline) = pipeline_for_thread.lock() {
                                pipeline.ingest(&buffer[..n]);
                            }
                        }
                        Err(e) => {
                            if let Ok(mut slot) = error_for_thread.lock() {
                                *slot = Some(e.to_string());
                            }
                            break;
                        }
                    }
                }
                source.close();
                finished_for_thread.store(true, Ordering::SeqCst);
            })
            .ok();

        Self {
            stop,
            pipeline,
            handle,
            port,
            baud_rate,
            finished,
            last_error,
        }
    }

    /// Run the pipeline on the calling thread until the source is exhausted, for
    /// replaying a captured stream.
    pub fn pump_to_end(&mut self, mut source: Box<dyn ByteSource + Send>) {
        let mut buffer = vec![0u8; 4096];
        loop {
            match source.read_available(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut pipeline) = self.pipeline.lock() {
                        pipeline.ingest(&buffer[..n]);
                    }
                }
                Err(e) => {
                    if let Ok(mut slot) = self.last_error.lock() {
                        *slot = Some(e.to_string());
                    }
                    break;
                }
            }
        }
        self.finished.store(true, Ordering::SeqCst);
    }

    /// Whether the source has stopped.
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    /// The most recent read error.
    pub fn take_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|mut e| e.take())
    }

    /// Run a closure against the pipeline.
    pub fn with_pipeline<T>(&self, f: impl FnOnce(&mut IngestionPipeline) -> T) -> Option<T> {
        self.pipeline.lock().ok().map(|mut p| f(&mut p))
    }

    /// Drain at most `limit` display frames.
    pub fn drain_display(&self, limit: usize) -> Vec<ProcessedPacket> {
        self.with_pipeline(|p| p.drain_display(limit))
            .unwrap_or_default()
    }

    /// The recorder status.
    pub fn recorder_status(&self) -> Option<RecorderStatus> {
        self.with_pipeline(|p| p.recorder().status())
    }

    /// Stop the reader and wait briefly for the thread to finish.
    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            // A blocking port read uses a bounded timeout, so this join cannot
            // hang indefinitely.
            let _ = handle.join();
        }
    }
}

impl Drop for TelemetryWorker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The mutable application state.
#[derive(Default)]
pub struct AppStateInner {
    /// The open project.
    pub project: Option<Project>,
    /// Imported dynamics models, newest last.
    pub models: Vec<ImportedModel>,
    /// The imported flight session, when one has been imported.
    pub flight: Option<FlightSession>,
    /// The directory name of the imported flight session.
    pub flight_name: Option<String>,
    /// The most recent run.
    pub last_run: Option<StoredRun>,
    /// The most recent comparison.
    pub comparison: Option<Comparison>,
    /// The active simulation, when one is running.
    pub active_simulation: Option<ActiveSimulation>,
    /// The telemetry worker, when a port is open.
    pub telemetry: Option<TelemetryWorker>,
    /// Application settings.
    pub settings: Settings,
    /// Warnings raised while loading settings.
    pub settings_warnings: Vec<String>,
    /// Whether a saved telemetry session was written since the last stop.
    pub last_telemetry_artifact: Option<String>,
}

/// Shared application state handle.
pub struct AppState(pub Mutex<AppStateInner>);

impl AppState {
    /// Build the state, loading settings from disk.
    pub fn new() -> Self {
        let (settings, settings_warnings) = Settings::load();
        Self(Mutex::new(AppStateInner {
            settings,
            settings_warnings,
            ..Default::default()
        }))
    }

    /// Run a closure with the state locked.
    pub fn with<T>(&self, f: impl FnOnce(&mut AppStateInner) -> T) -> Result<T, CommandError> {
        let mut guard = self
            .0
            .lock()
            .map_err(|_| crate::error::internal("the application state lock was poisoned"))?;
        Ok(f(&mut guard))
    }

    /// Borrow the open project or produce a standard refusal.
    pub fn require_project(&self) -> Result<Project, CommandError> {
        self.with(|s| s.project.clone()).and_then(|p| {
            p.ok_or_else(|| {
                crate::error::missing(
                    "project.none_open",
                    "No project is open",
                    "This action needs an open project.",
                    "Create or open a project on the Projects screen.",
                )
            })
        })
    }

    /// Borrow the most recent model or produce a standard refusal.
    pub fn require_model(&self) -> Result<ImportedModel, CommandError> {
        self.with(|s| s.models.last().cloned()).and_then(|m| {
            m.ok_or_else(|| {
                crate::error::missing(
                    "model.none_imported",
                    "No dynamics model is imported",
                    "A simulation needs an imported dynamics model.",
                    "Import a model on the Dynamics screen, or use the built-in example.",
                )
            })
        })
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Emit an event, ignoring a serialisation failure rather than failing the command.
///
/// A dropped notification must never break the operation that produced it.
pub fn emit<T: Serialize + Clone>(app: &AppHandle, channel: &str, payload: T) {
    let _ = app.emit(channel, payload);
}

/// Emit a simulation progress event.
pub fn emit_progress(app: &AppHandle, progress: &RunProgress) {
    emit(
        app,
        events::channel::SIMULATION_PROGRESS,
        events::SimulationProgressEvent {
            time: progress.time,
            fraction: progress.fraction,
            samples: progress.samples,
            accepted_steps: progress.accepted_steps,
            rejected_steps: progress.rejected_steps,
            events: progress.events,
        },
    );
}

/// Emit a notice.
pub fn emit_notice(app: &AppHandle, notice: events::NoticeEvent) {
    emit(app, events::channel::NOTICE, notice);
}

/// A saved telemetry session summary returned to the frontend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedSessionSummary {
    /// Directory or file name.
    pub name: String,
    /// Label shown in the list.
    pub label: String,
    /// Samples recorded.
    pub samples: usize,
    /// Duration in seconds.
    pub duration_seconds: f64,
    /// Whether the recording reported integrity problems.
    pub has_issues: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_telemetry::ScriptedSource;

    fn profile() -> DeviceProfile {
        DeviceProfile::example_csv_profile()
    }

    fn csv_lines(count: usize) -> Vec<u8> {
        let mut text = String::new();
        for i in 0..count {
            let t = i as f64 * 0.01;
            text.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                t, 0.0, 0.0, -9.80665, 0.0, 0.0, 0.1, 101325.0
            ));
        }
        text.into_bytes()
    }

    #[test]
    fn state_starts_with_defaults_and_no_project() {
        let state = AppState::new();
        assert!(state.with(|s| s.project.is_none()).unwrap());
        assert!(state.with(|s| s.models.is_empty()).unwrap());
        assert!(state.require_project().is_err());
        assert!(state.require_model().is_err());
        let err = state.require_project().unwrap_err();
        assert_eq!(err.severity, crate::error::ErrorSeverity::Warning);
    }

    #[test]
    fn pump_to_end_ingests_every_packet() {
        let mut worker = TelemetryWorker::spawn(
            Box::new(ScriptedSource::new(Vec::new())),
            profile(),
            hex_telemetry::CalibrationSet::default(),
            4096,
            30.0,
        );
        assert_eq!(worker.port, "scripted source");
        assert_eq!(worker.baud_rate, 115_200);
        worker.pump_to_end(Box::new(
            ScriptedSource::new(csv_lines(200)).with_chunk(4096),
        ));
        assert!(worker.is_finished());
        assert!(worker.take_error().is_none());
        let status = worker.recorder_status().unwrap();
        // Recording is off until asked, so nothing is recorded yet.
        assert_eq!(status.samples, 0);
        worker.shutdown();
    }

    #[test]
    fn the_reader_thread_ingests_from_a_scripted_source() {
        let worker = TelemetryWorker::spawn(
            Box::new(ScriptedSource::new(csv_lines(500)).with_chunk(256)),
            profile(),
            hex_telemetry::CalibrationSet::default(),
            4096,
            30.0,
        );
        // The reader thread runs asynchronously, so wait for it to finish rather
        // than sleeping for a fixed time.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !worker.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(worker.is_finished(), "the reader thread did not finish");
        let counters = worker.with_pipeline(|p| p.counters()).unwrap();
        assert_eq!(counters.packets_decoded, 500);
        let mut worker = worker;
        worker.shutdown();
    }

    #[test]
    fn display_frames_can_be_drained_and_are_throttled() {
        let mut worker = TelemetryWorker::spawn(
            Box::new(ScriptedSource::new(Vec::new())),
            profile(),
            hex_telemetry::CalibrationSet::default(),
            4096,
            30.0,
        );
        worker.pump_to_end(Box::new(
            ScriptedSource::new(csv_lines(100)).with_chunk(4096),
        ));
        let frames = worker.drain_display(1000);
        assert!(!frames.is_empty());
        assert!(frames.len() < 100, "the display must be throttled");
        let downsampled = worker
            .with_pipeline(|p| p.display().is_downsampled())
            .unwrap();
        assert!(downsampled);
        worker.shutdown();
    }

    #[test]
    fn a_read_error_is_recorded_and_stops_the_thread() {
        struct Failing;
        impl ByteSource for Failing {
            fn read_available(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "device disconnected",
                ))
            }
            fn describe(&self) -> String {
                "failing source".to_string()
            }
            fn close(&mut self) {}
        }

        let worker = TelemetryWorker::spawn(
            Box::new(Failing),
            profile(),
            hex_telemetry::CalibrationSet::default(),
            128,
            30.0,
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !worker.is_finished() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(worker.is_finished());
        let error = worker.take_error().unwrap();
        assert!(error.contains("device disconnected"), "{}", error);
        let mut worker = worker;
        worker.shutdown();
    }

    #[test]
    fn recording_is_off_until_started() {
        let mut worker = TelemetryWorker::spawn(
            Box::new(ScriptedSource::new(Vec::new())),
            profile(),
            hex_telemetry::CalibrationSet::default(),
            4096,
            30.0,
        );
        worker.with_pipeline(|p| p.start_recording());
        worker.pump_to_end(Box::new(
            ScriptedSource::new(csv_lines(50)).with_chunk(4096),
        ));
        let status = worker.recorder_status().unwrap();
        assert!(status.recording);
        assert_eq!(status.samples, 50);
        worker.shutdown();
    }
}
