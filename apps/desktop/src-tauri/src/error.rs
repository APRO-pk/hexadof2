//! Errors returned to the frontend.
//!
//! Every command returns `Result<T, CommandError>`, and the error carries what the
//! UI needs to render an actionable message: a title, an explanation, a severity, a
//! suggested fix, and the technical detail an advanced user can expand.

use serde::{Deserialize, Serialize};

/// How serious a command failure is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity {
    /// The operation was refused because the input is incomplete.
    Warning,
    /// The operation failed.
    Error,
}

impl ErrorSeverity {
    pub fn label(self) -> &'static str {
        match self {
            ErrorSeverity::Warning => "Warning",
            ErrorSeverity::Error => "Error",
        }
    }
}

/// A failure with everything the interface needs to explain it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandError {
    /// Stable machine code, for example `project.not_found`.
    pub code: String,
    /// Short human-readable title.
    pub title: String,
    /// Plain-language explanation.
    pub detail: String,
    pub severity: ErrorSeverity,
    /// What the user can do about it.
    pub suggestion: String,
    /// Expanded technical detail, shown only when the user asks for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical: Option<String>,
}

impl CommandError {
    pub fn new(
        code: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            title: title.into(),
            detail: detail.into(),
            severity: ErrorSeverity::Error,
            suggestion: suggestion.into(),
            technical: None,
        }
    }

    /// A refusal rather than a failure, for example a missing selection.
    pub fn warning(
        code: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            severity: ErrorSeverity::Warning,
            ..Self::new(code, title, detail, suggestion)
        }
    }

    pub fn with_technical(mut self, technical: impl Into<String>) -> Self {
        self.technical = Some(technical.into());
        self
    }

    /// A one-line rendering for a log entry.
    pub fn one_line(&self) -> String {
        format!("[{}] {}: {}", self.code, self.title, self.detail)
    }
}

impl From<hex_project::ProjectError> for CommandError {
    fn from(value: hex_project::ProjectError) -> Self {
        CommandError {
            code: value.code().to_string(),
            title: "Project operation failed".to_string(),
            detail: value.user_message(),
            severity: ErrorSeverity::Error,
            suggestion: value.suggested_action().to_string(),
            technical: Some(format!("{:?}", value)),
        }
    }
}

impl From<hex_model::ModelError> for CommandError {
    fn from(value: hex_model::ModelError) -> Self {
        CommandError {
            code: "model.error".to_string(),
            title: "The dynamics model could not be read".to_string(),
            detail: value.user_message(),
            severity: ErrorSeverity::Error,
            suggestion: MODEL_SUGGESTION.to_string(),
            technical: Some(format!("{:?}", value)),
        }
    }
}

/// What to tell a user whose model could not be interpreted.
///
/// A fixed sentence rather than a per-variant one, because the model validator
/// already reports every specific problem in its validation report; this
/// conversion is only reached when the file could not be read at all.
const MODEL_SUGGESTION: &str =
    "Check the unit declarations and the mass properties in the model file, then import it again.";

impl From<hex_flight_data::FlightDataError> for CommandError {
    fn from(value: hex_flight_data::FlightDataError) -> Self {
        CommandError {
            code: "flight.error".to_string(),
            title: "The flight log could not be imported".to_string(),
            detail: value.user_message(),
            severity: ErrorSeverity::Error,
            suggestion: value.suggested_action().to_string(),
            technical: Some(format!("{:?}", value)),
        }
    }
}

impl From<hex_telemetry::TransportError> for CommandError {
    fn from(value: hex_telemetry::TransportError) -> Self {
        CommandError {
            code: "telemetry.transport".to_string(),
            title: "The serial port could not be opened".to_string(),
            detail: value.to_string(),
            severity: ErrorSeverity::Error,
            suggestion: value.suggested_action().to_string(),
            technical: None,
        }
    }
}

impl From<hex_analysis::AlignmentError> for CommandError {
    fn from(value: hex_analysis::AlignmentError) -> Self {
        CommandError {
            code: "comparison.alignment".to_string(),
            title: "The two series could not be aligned".to_string(),
            detail: value.user_message(),
            severity: ErrorSeverity::Error,
            suggestion: value.suggested_action().to_string(),
            technical: None,
        }
    }
}

/// A command that needs something the caller did not supply.
pub fn missing(
    code: impl Into<String>,
    title: impl Into<String>,
    detail: impl Into<String>,
    suggestion: impl Into<String>,
) -> CommandError {
    CommandError::warning(code, title, detail, suggestion)
}

/// A refused operation because the input failed validation.
pub fn invalid(detail: impl Into<String>, suggestion: impl Into<String>) -> CommandError {
    CommandError::new(
        "input.invalid",
        "The supplied values are not usable",
        detail,
        suggestion,
    )
}

/// A poisoned lock, which means another thread panicked while holding it.
pub fn internal(detail: impl Into<String>) -> CommandError {
    CommandError::new(
        "internal.lock",
        "An internal error occurred",
        detail,
        "Report this as a bug with the diagnostic bundle.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_keep_the_code_and_the_action() {
        let e: CommandError = hex_project::ProjectError::RunNotFound {
            name: "run-001".to_string(),
        }
        .into();
        assert_eq!(e.code, "project.run_not_found");
        assert!(e.detail.contains("run-001"));
        assert!(!e.suggestion.is_empty());
        assert!(e.technical.is_some());
        assert_eq!(e.severity, ErrorSeverity::Error);
    }

    #[test]
    fn model_errors_convert() {
        let e: CommandError = hex_model::ModelError::UnsupportedSchemaVersion {
            found: "9.9".to_string(),
            supported: "1.0".to_string(),
        }
        .into();
        assert_eq!(e.code, "model.error");
        assert!(!e.suggestion.is_empty());
    }

    #[test]
    fn telemetry_errors_convert() {
        let e: CommandError = hex_telemetry::TransportError::PortNotFound {
            port: "COM9".to_string(),
        }
        .into();
        assert_eq!(e.code, "telemetry.transport");
        assert!(!e.suggestion.is_empty());
    }

    #[test]
    fn alignment_errors_convert() {
        let e: CommandError = hex_analysis::AlignmentError::NoOverlap.into();
        assert_eq!(e.code, "comparison.alignment");
        assert!(!e.suggestion.is_empty());
    }

    #[test]
    fn missing_input_is_a_warning_not_a_failure() {
        let e = missing(
            "project.none_open",
            "No project is open",
            "Open or create a project first.",
            "Use the Projects screen to create one.",
        );
        assert_eq!(e.severity, ErrorSeverity::Warning);
        assert_eq!(e.severity.label(), "Warning");
        assert!(e.one_line().contains("No project is open"));
    }

    #[test]
    fn invalid_and_internal_builders_are_distinct() {
        let a = invalid("bad", "fix it");
        let b = internal("poisoned");
        assert_eq!(a.code, "input.invalid");
        assert_eq!(b.code, "internal.lock");
        assert_ne!(a.detail, b.detail);
    }
}
