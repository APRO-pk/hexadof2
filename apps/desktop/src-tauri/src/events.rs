//! Typed event names and payloads the backend emits to the frontend.
//!
//! Every event carries a small payload. Raw telemetry packets never travel this
//! path at full rate: the ingestion thread buffers them and the frontend polls a
//! throttled display stream, so a slow interface cannot change the recording.

use serde::{Deserialize, Serialize};

/// Event channel names, kept in one place so a typo cannot silently disconnect a
/// listener.
pub mod channel {
    /// A simulation made progress.
    pub const SIMULATION_PROGRESS: &str = "simulation://progress";
    /// A simulation finished successfully.
    pub const SIMULATION_COMPLETED: &str = "simulation://completed";
    /// A simulation finished with warnings.
    pub const SIMULATION_WARNING: &str = "simulation://warning";
    /// A simulation failed.
    pub const SIMULATION_FAILED: &str = "simulation://failed";
    /// A serial port was opened.
    pub const SERIAL_CONNECTED: &str = "serial://connected";
    /// A serial port was closed.
    pub const SERIAL_DISCONNECTED: &str = "serial://disconnected";
    /// The telemetry connection health changed.
    pub const TELEMETRY_HEALTH: &str = "telemetry://health";
    /// Recording started.
    pub const RECORDING_STARTED: &str = "telemetry://recording-started";
    /// Recording stopped.
    pub const RECORDING_STOPPED: &str = "telemetry://recording-stopped";
    /// An import pipeline advanced a stage.
    pub const IMPORT_PROGRESS: &str = "import://progress";
    /// A validation pass finished.
    pub const VALIDATION_RESULT: &str = "validation://result";
    /// A long task reported a failure it recovered from.
    pub const NOTICE: &str = "app://notice";
}

/// Progress of a running simulation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SimulationProgressEvent {
    /// Simulation time reached, seconds.
    pub time: f64,
    /// Fraction complete, between 0 and 1.
    pub fraction: f64,
    /// Samples recorded so far.
    pub samples: usize,
    /// Steps accepted so far.
    pub accepted_steps: u64,
    /// Steps rejected so far.
    pub rejected_steps: u64,
    /// Events detected so far.
    pub events: usize,
}

/// A completed simulation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationCompletedEvent {
    /// Run directory name, when the run was saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_name: Option<String>,
    /// Status label.
    pub status: String,
    /// Samples in the history.
    pub samples: usize,
    /// Number of warnings.
    pub warnings: usize,
    /// Number of events.
    pub events: usize,
}

/// A simulation warning or failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimulationNoticeEvent {
    /// Machine code.
    pub code: String,
    /// Short title.
    pub title: String,
    /// Explanation.
    pub detail: String,
    /// Whether the finding blocks using the result.
    pub blocking: bool,
}

/// A serial connection change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SerialEvent {
    /// Port name.
    pub port: String,
    /// Baud rate.
    pub baud_rate: u32,
    /// Connection state label.
    pub state: String,
    /// Detail shown beside the state.
    pub detail: String,
}

/// The telemetry health snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryHealthEvent {
    /// Connection state label.
    pub state: String,
    /// Valid packets decoded.
    pub packets_received: u64,
    /// Packets rejected.
    pub packets_rejected: u64,
    /// Packets per second over the last window.
    pub packets_per_second: f64,
    /// Bytes received.
    pub bytes_received: u64,
    /// Bytes discarded while resynchronising.
    pub discarded_bytes: u64,
    /// Checksum failures.
    pub crc_failures: u64,
    /// Samples the recorder has accepted.
    pub recorded_samples: u64,
    /// Whether the display is showing fewer frames than the device produced.
    pub display_downsampled: bool,
    /// Pending display frames.
    pub pending_display_frames: usize,
    /// Most recent rejection message, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_rejection: Option<String>,
}

/// A recording state change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordingEvent {
    /// Whether recording is running.
    pub recording: bool,
    /// Samples recorded.
    pub samples: usize,
    /// Recording capacity.
    pub capacity: usize,
    /// Artifact name, when one was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
}

/// One import pipeline stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportProgressEvent {
    /// Stage name.
    pub stage: String,
    /// Stage status.
    pub status: String,
    /// Message for the stage.
    pub message: String,
    /// How long the stage took, milliseconds.
    pub duration_ms: u64,
}

/// A validation pass result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationResultEvent {
    /// What was validated.
    pub subject: String,
    /// Overall status label.
    pub status: String,
    /// Number of blocking errors.
    pub errors: usize,
    /// Number of warnings.
    pub warnings: usize,
}

/// A general notice for the notification area.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoticeEvent {
    /// Severity: `info`, `warning`, or `error`.
    pub level: String,
    /// Short title.
    pub title: String,
    /// Detail.
    pub detail: String,
}

impl NoticeEvent {
    pub fn info(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            level: "info".to_string(),
            title: title.into(),
            detail: detail.into(),
        }
    }

    pub fn warning(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            level: "warning".to_string(),
            title: title.into(),
            detail: detail.into(),
        }
    }

    pub fn error(title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            level: "error".to_string(),
            title: title.into(),
            detail: detail.into(),
        }
    }
}

/// Every channel name, for the frontend to assert against and for the diagnostics
/// bundle.
pub fn all_channels() -> Vec<&'static str> {
    vec![
        channel::SIMULATION_PROGRESS,
        channel::SIMULATION_COMPLETED,
        channel::SIMULATION_WARNING,
        channel::SIMULATION_FAILED,
        channel::SERIAL_CONNECTED,
        channel::SERIAL_DISCONNECTED,
        channel::TELEMETRY_HEALTH,
        channel::RECORDING_STARTED,
        channel::RECORDING_STOPPED,
        channel::IMPORT_PROGRESS,
        channel::VALIDATION_RESULT,
        channel::NOTICE,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_names_are_unique_and_namespaced() {
        let mut names = all_channels();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        for name in all_channels() {
            assert!(name.contains("://"), "{} is not namespaced", name);
        }
    }

    #[test]
    fn notice_levels_are_distinct() {
        assert_eq!(NoticeEvent::info("a", "b").level, "info");
        assert_eq!(NoticeEvent::warning("a", "b").level, "warning");
        assert_eq!(NoticeEvent::error("a", "b").level, "error");
    }

    #[test]
    fn progress_events_serialise_with_snake_case_keys() {
        let event = SimulationProgressEvent {
            time: 1.5,
            fraction: 0.5,
            samples: 150,
            accepted_steps: 3000,
            rejected_steps: 2,
            events: 3,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"accepted_steps\""));
        assert!(json.contains("\"rejected_steps\""));
        let back: SimulationProgressEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn health_event_round_trips_and_can_omit_the_rejection() {
        let event = TelemetryHealthEvent {
            state: "Receiving".to_string(),
            packets_received: 1000,
            packets_rejected: 2,
            packets_per_second: 199.5,
            bytes_received: 60_000,
            discarded_bytes: 7,
            crc_failures: 1,
            recorded_samples: 1000,
            display_downsampled: true,
            pending_display_frames: 3,
            last_rejection: None,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("last_rejection"));
        let back: TelemetryHealthEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }

    #[test]
    fn completed_event_omits_a_missing_run_name() {
        let event = SimulationCompletedEvent {
            run_name: None,
            status: "Completed".to_string(),
            samples: 10,
            warnings: 0,
            events: 2,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.contains("run_name"));
    }

    #[test]
    fn import_progress_round_trips() {
        let event = ImportProgressEvent {
            stage: "Parsed".to_string(),
            status: "Ok".to_string(),
            message: "read 100 rows".to_string(),
            duration_ms: 12,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ImportProgressEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back, event);
    }
}
