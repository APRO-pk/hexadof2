//! The error type every model-loading entry point returns.
//!
//! Errors in this crate describe a failure to read or interpret a document.
//! Problems with the physics of the model itself are reported as
//! [`ValidationIssue`](hex_core::ValidationIssue) values inside a
//! [`ValidationReport`], not as errors, because the user must see every finding
//! at once rather than only the first failure.

use hex_core::ValidationReport;

/// One-line summary of a validation report, used in the `ValidationFailed` text.
fn summarize(report: &ValidationReport) -> String {
    format!("{} ({})", report.summary(), report.subject)
}

/// Everything that can go wrong while reading a dynamics model.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// The document could not be read from disk.
    #[error("could not read the model file: {0}")]
    Io(#[from] std::io::Error),
    /// The text is not a valid model document.
    #[error("the model document is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The document declares a schema major version this build cannot read.
    #[error("unsupported schema version: found {found}, this build supports {supported}")]
    UnsupportedSchemaVersion {
        /// Version found in the document.
        found: String,
        /// Version this build reads and writes.
        supported: String,
    },
    /// A field that has no sensible default was absent or blank.
    #[error("the model document is missing the required field {field}")]
    MissingField {
        /// Name of the field that is missing.
        field: String,
    },
    /// A unit string could not be resolved to a conversion factor.
    ///
    /// The importer records unknown units as `units.unknown` warnings and keeps
    /// the raw value. A caller that refuses to import such a model can turn that
    /// finding into this error with [`ModelError::unit_unknown`].
    #[error("unknown unit {unit} for {quantity}")]
    UnitUnknown {
        /// Physical quantity the unit was declared for.
        quantity: String,
        /// Unit label exactly as written in the source.
        unit: String,
    },
    /// The model carried blocking validation issues.
    #[error("the model did not pass validation: {}", summarize(.report))]
    ValidationFailed {
        /// The report that blocked the import.
        report: Box<ValidationReport>,
    },
}

impl ModelError {
    /// Build the strict unknown-unit error.
    pub fn unit_unknown(quantity: impl Into<String>, unit: impl Into<String>) -> Self {
        Self::UnitUnknown {
            quantity: quantity.into(),
            unit: unit.into(),
        }
    }

    /// Wrap a blocking report.
    ///
    /// The report is boxed so a `Result` carrying a [`ModelError`] stays small.
    pub fn validation_failed(report: ValidationReport) -> Self {
        Self::ValidationFailed {
            report: Box::new(report),
        }
    }

    /// Whether this error stops the import completely.
    ///
    /// An unknown unit is not blocking: the value is carried through unconverted
    /// and the matching warning tells the user to check it.
    pub fn is_blocking(&self) -> bool {
        match self {
            Self::UnitUnknown { .. } => false,
            Self::ValidationFailed { report } => report.has_errors(),
            Self::Io(_)
            | Self::Json(_)
            | Self::UnsupportedSchemaVersion { .. }
            | Self::MissingField { .. } => true,
        }
    }

    /// Stable machine code for the UI to branch on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "io",
            Self::Json(_) => "json",
            Self::UnsupportedSchemaVersion { .. } => "schema.unsupported",
            Self::MissingField { .. } => "field.missing",
            Self::UnitUnknown { .. } => "units.unknown",
            Self::ValidationFailed { .. } => "validation.failed",
        }
    }

    /// Plain-language message the UI shows to the user.
    ///
    /// No file paths, no type names, no jargon: the technical form is available
    /// through the `Display` implementation and the validation panel.
    pub fn user_message(&self) -> String {
        match self {
            Self::Io(_) => {
                "The model file could not be read. Check that it still exists and that you can open it."
                    .to_string()
            }
            Self::Json(_) => {
                "The model file is not valid JSON. Check the file for a truncated or hand-edited line."
                    .to_string()
            }
            Self::UnsupportedSchemaVersion { found, supported } => format!(
                "This model was written for schema version {found}, but this version of the app reads {supported}."
            ),
            Self::MissingField { field } => {
                format!("The model is missing the required field '{field}'.")
            }
            Self::UnitUnknown { quantity, unit } => format!(
                "The unit '{unit}' used for {quantity} is not recognised. Choose the right unit before trusting the numbers."
            ),
            Self::ValidationFailed { report } => format!(
                "The model did not pass validation ({}). Open the validation list to review each finding.",
                report.summary()
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::ValidationIssue;

    fn json_error() -> ModelError {
        serde_json::from_str::<u32>("not a number")
            .unwrap_err()
            .into()
    }

    #[test]
    fn io_and_json_errors_are_blocking() {
        let io = ModelError::Io(std::io::Error::new(std::io::ErrorKind::NotFound, "gone"));
        assert!(io.is_blocking());
        assert_eq!(io.code(), "io");
        assert!(json_error().is_blocking());
        assert_eq!(json_error().code(), "json");
    }

    #[test]
    fn schema_and_missing_field_errors_are_blocking() {
        let schema = ModelError::UnsupportedSchemaVersion {
            found: "2.0".to_string(),
            supported: "1.0".to_string(),
        };
        assert!(schema.is_blocking());
        assert_eq!(schema.code(), "schema.unsupported");

        let missing = ModelError::MissingField {
            field: "model_id".to_string(),
        };
        assert!(missing.is_blocking());
        assert_eq!(missing.code(), "field.missing");
    }

    #[test]
    fn unknown_unit_is_not_blocking() {
        let err = ModelError::unit_unknown("mass", "stone");
        assert!(!err.is_blocking());
        assert_eq!(err.code(), "units.unknown");
        assert!(err.user_message().contains("stone"));
    }

    #[test]
    fn validation_failure_follows_the_report() {
        let mut report = ValidationReport::new("rocket");
        report.push(ValidationIssue::warning("w", "Warn", "detail"));
        let warnings_only = ModelError::validation_failed(report.clone());
        assert!(!warnings_only.is_blocking());

        report.push(ValidationIssue::error("e", "Err", "detail"));
        assert!(ModelError::validation_failed(report).is_blocking());
    }

    #[test]
    fn display_and_user_message_stay_separate() {
        let err = ModelError::MissingField {
            field: "mass".to_string(),
        };
        assert!(err.to_string().contains("mass"));
        assert!(err.user_message().contains("'mass'"));
        assert_eq!(err.code(), "field.missing");
    }

    #[test]
    fn validation_failure_message_summarises_the_report() {
        let mut report = ValidationReport::new("rocket");
        report.push(ValidationIssue::error("e", "Err", "detail"));
        let err = ModelError::validation_failed(report);
        assert!(err.to_string().contains("1 error(s)"));
        assert!(err.user_message().contains("1 error(s)"));
    }
}
