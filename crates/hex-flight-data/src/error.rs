//! The single error type every flight-data entry point returns.
//!
//! Import failures reach the user through the UI, so each variant carries both a
//! plain-language message ([`FlightDataError::user_message`]) and a concrete
//! next step ([`FlightDataError::suggested_action`]). Technical detail stays in
//! the `Display` text so logs remain diagnosable without leaking jargon into the
//! primary surface.

use hex_core::ValidationReport;
use thiserror::Error;

use crate::binary::BinaryDecodeError;

/// Everything that can go wrong while reading or normalising flight data.
#[derive(Debug, Error)]
pub enum FlightDataError {
    /// The file or stream could not be read from disk.
    #[error("I/O failure: {source}")]
    Io {
        /// Operating-system error reported by the read or write call.
        #[from]
        source: std::io::Error,
    },
    /// The CSV text could not be tokenised.
    #[error("CSV failure: {detail}")]
    Csv {
        /// Description produced by the tokenizer.
        detail: String,
    },
    /// The HexaDOF binary log could not be decoded.
    #[error("binary log failure: {0}")]
    Binary(#[from] BinaryDecodeError),
    /// No column in the source could be identified as the time axis.
    #[error("the source contains no usable timestamp column")]
    NoTimestampColumn,
    /// The file extension does not select a known codec.
    #[error("unsupported file format for extension '{extension}'")]
    UnsupportedFormat {
        /// Extension as found on disk, without the leading dot.
        extension: String,
    },
    /// The source contained no data at all.
    #[error("the source file is empty")]
    EmptyFile,
    /// The source exceeded the caller's row limit.
    #[error("the source contains more than {limit} rows, which is the configured limit")]
    TooManyRows {
        /// Configured maximum number of data rows.
        limit: usize,
    },
    /// A required channel role could not be filled from the supplied mapping.
    #[error("the channel mapping does not cover: {}", missing.join(", "))]
    MappingIncomplete {
        /// Roles or source columns that the pipeline still needs.
        missing: Vec<String>,
    },
    /// A validation pass produced blocking issues.
    #[error("validation failed: {}", .0.summary())]
    Validation(Box<ValidationReport>),
    /// A session metadata document could not be serialised or parsed.
    #[error("session metadata failure: {detail}")]
    Serialization {
        /// Description produced by the JSON layer.
        detail: String,
    },
}

impl From<csv::Error> for FlightDataError {
    fn from(value: csv::Error) -> Self {
        Self::Csv {
            detail: value.to_string(),
        }
    }
}

impl From<serde_json::Error> for FlightDataError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization {
            detail: value.to_string(),
        }
    }
}

impl FlightDataError {
    /// Short message suitable for a banner in the import dialog.
    pub fn user_message(&self) -> String {
        match self {
            Self::Io { source } => format!("The file could not be read: {source}"),
            Self::Csv { detail } => format!("The CSV file could not be read: {detail}"),
            Self::Binary(err) => err.user_message(),
            Self::NoTimestampColumn => {
                "No column in this file looks like a timestamp, so HexaDOF cannot place the samples in time."
                    .to_string()
            }
            Self::UnsupportedFormat { extension } => {
                if extension.is_empty() {
                    "This file has no extension, so HexaDOF cannot tell which format it uses."
                        .to_string()
                } else {
                    format!("HexaDOF does not know how to read '.{extension}' files.")
                }
            }
            Self::EmptyFile => "The selected file contains no data.".to_string(),
            Self::TooManyRows { limit } => format!(
                "This file has more than {limit} rows, which is more than the import limit allows."
            ),
            Self::MappingIncomplete { missing } => {
                format!("The channel mapping is incomplete: {}.", missing.join(", "))
            }
            Self::Validation(report) => format!(
                "The imported data did not pass validation: {}.",
                report.summary()
            ),
            Self::Serialization { detail } => {
                format!("The session metadata could not be processed: {detail}")
            }
        }
    }

    /// Concrete next step the user (or the caller) can take.
    pub fn suggested_action(&self) -> String {
        match self {
            Self::Io { .. } => {
                "Check that the file still exists and that you have permission to read it.".to_string()
            }
            Self::Csv { .. } => {
                "Open the file in a text editor and confirm the delimiter and the header row, then set the CSV options by hand."
                    .to_string()
            }
            Self::Binary(err) => err.suggested_action(),
            Self::NoTimestampColumn => {
                "Choose the time column explicitly in the import dialog, or record a timestamp with the log."
                    .to_string()
            }
            Self::UnsupportedFormat { .. } => {
                "Convert the log to CSV or to a .hlog file, or select the correct file.".to_string()
            }
            Self::EmptyFile => "Select a file that contains at least one sample.".to_string(),
            Self::TooManyRows { .. } => {
                "Raise the row limit in the import options or split the log into smaller files."
                    .to_string()
            }
            Self::MappingIncomplete { .. } => {
                "Assign the missing channels in the mapping table before importing.".to_string()
            }
            Self::Validation(_) => {
                "Review the validation issues and fix the source log before importing.".to_string()
            }
            Self::Serialization { .. } => {
                "Regenerate the session metadata file; it may have been edited or truncated."
                    .to_string()
            }
        }
    }

    /// Machine-readable code used to look up help text in the UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "flight_data.io",
            Self::Csv { .. } => "flight_data.csv",
            Self::Binary(BinaryDecodeError::BadMagic) => "flight_data.binary.bad_magic",
            Self::Binary(BinaryDecodeError::UnsupportedVersion { .. }) => {
                "flight_data.binary.unsupported_version"
            }
            Self::Binary(BinaryDecodeError::ChannelTableCrcMismatch { .. }) => {
                "flight_data.binary.channel_table_crc"
            }
            Self::Binary(BinaryDecodeError::CrcMismatch { .. }) => "flight_data.binary.crc",
            Self::Binary(BinaryDecodeError::PayloadLengthMismatch { .. }) => {
                "flight_data.binary.payload_length"
            }
            Self::Binary(BinaryDecodeError::Io { .. }) => "flight_data.binary.io",
            Self::Binary(BinaryDecodeError::Truncated) => "flight_data.binary.truncated",
            Self::NoTimestampColumn => "flight_data.no_timestamp",
            Self::UnsupportedFormat { .. } => "flight_data.unsupported_format",
            Self::EmptyFile => "flight_data.empty_file",
            Self::TooManyRows { .. } => "flight_data.too_many_rows",
            Self::MappingIncomplete { .. } => "flight_data.mapping_incomplete",
            Self::Validation(_) => "flight_data.validation",
            Self::Serialization { .. } => "flight_data.serialization",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::ValidationIssue;

    #[test]
    fn user_message_and_action_are_present_for_every_variant() {
        let cases = vec![
            FlightDataError::NoTimestampColumn,
            FlightDataError::UnsupportedFormat {
                extension: "xyz".to_string(),
            },
            FlightDataError::UnsupportedFormat {
                extension: String::new(),
            },
            FlightDataError::EmptyFile,
            FlightDataError::TooManyRows { limit: 10 },
            FlightDataError::MappingIncomplete {
                missing: vec!["accel.x".to_string()],
            },
            FlightDataError::Csv {
                detail: "bad".to_string(),
            },
            FlightDataError::Serialization {
                detail: "bad".to_string(),
            },
            FlightDataError::Binary(BinaryDecodeError::BadMagic),
            FlightDataError::Validation(Box::new(ValidationReport::failed(
                "log",
                ValidationIssue::error("x", "X", "boom"),
            ))),
        ];
        for case in cases {
            assert!(!case.user_message().is_empty(), "{case:?}");
            assert!(!case.suggested_action().is_empty(), "{case:?}");
            assert!(case.code().starts_with("flight_data."), "{case:?}");
        }
    }

    #[test]
    fn unsupported_format_mentions_the_extension() {
        let e = FlightDataError::UnsupportedFormat {
            extension: "tsv2".to_string(),
        };
        assert!(e.user_message().contains("tsv2"));
    }

    #[test]
    fn csv_errors_convert_from_the_csv_crate() {
        let err = csv::Error::from(std::io::Error::other("boom"));
        let converted: FlightDataError = err.into();
        assert!(matches!(converted, FlightDataError::Csv { .. }));
    }

    #[test]
    fn io_errors_convert_through_the_from_impl() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let converted: FlightDataError = io.into();
        assert!(matches!(converted, FlightDataError::Io { .. }));
        assert!(converted.user_message().contains("could not be read"));
    }
}
