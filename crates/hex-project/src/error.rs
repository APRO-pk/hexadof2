//! Errors from the project layer.
//!
//! Every variant states what failed and where, and every one carries a suggested
//! action, because a project problem is usually something the user can fix.

use std::path::PathBuf;

/// A project, artifact, or session failure.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProjectError {
    #[error("Unable to read or write {path}: {detail}")]
    Io { path: PathBuf, detail: String },
    #[error("The path {path} does not exist.")]
    NotFound { path: PathBuf },
    #[error("{path} is not empty. A new project needs an empty directory.")]
    DirectoryNotEmpty { path: PathBuf },
    #[error("No {name} was found in {path}.")]
    MissingProjectFile { name: &'static str, path: PathBuf },
    #[error("{path} could not be parsed: {detail}")]
    Parse { path: PathBuf, detail: String },
    #[error("A document could not be serialised: {detail}")]
    Serialize { detail: String },
    #[error("The project metadata is invalid: {detail}")]
    InvalidMetadata { detail: String },
    #[error("{reference} points outside the project directory, which is not allowed.")]
    ReferenceOutside { reference: String },
    #[error("{name} is not a usable file name.")]
    UnsafeFileName { name: String },
    #[error("{path} is a symbolic link, which HexaDOF will not follow.")]
    RefusedSymlink { path: PathBuf },
    #[error("The artifact {path} already exists, and results are immutable.")]
    ArtifactExists { path: PathBuf },
    #[error("No run named {name} exists in this project.")]
    RunNotFound { name: String },
    #[error("No flight session named {name} exists in this project.")]
    FlightNotFound { name: String },
    #[error("No telemetry session named {name} exists in this project.")]
    SessionNotFound { name: String },
    #[error(
        "The artifact at {path} declares schema version {found}, and this build reads {expected}."
    )]
    UnsupportedArtifactVersion {
        path: PathBuf,
        found: String,
        expected: String,
    },
    #[error("The settings file {path} could not be interpreted: {detail}")]
    Settings { path: PathBuf, detail: String },
}

impl ProjectError {
    /// Build an input/output error with the path attached.
    pub fn io(path: impl Into<PathBuf>, error: impl std::fmt::Display) -> Self {
        ProjectError::Io {
            path: path.into(),
            detail: error.to_string(),
        }
    }

    /// Whether the failure blocks the operation that produced it.
    ///
    /// Every variant here blocks its own operation, so this exists to give the UI
    /// one call site rather than a scattered judgement.
    pub fn is_blocking(&self) -> bool {
        true
    }

    /// A short machine code for the diagnostics bundle.
    pub fn code(&self) -> &'static str {
        match self {
            ProjectError::Io { .. } => "project.io",
            ProjectError::NotFound { .. } => "project.not_found",
            ProjectError::DirectoryNotEmpty { .. } => "project.directory_not_empty",
            ProjectError::MissingProjectFile { .. } => "project.missing_file",
            ProjectError::Parse { .. } => "project.parse",
            ProjectError::Serialize { .. } => "project.serialize",
            ProjectError::InvalidMetadata { .. } => "project.invalid_metadata",
            ProjectError::ReferenceOutside { .. } => "project.reference_outside",
            ProjectError::UnsafeFileName { .. } => "project.unsafe_name",
            ProjectError::RefusedSymlink { .. } => "project.symlink_refused",
            ProjectError::ArtifactExists { .. } => "project.artifact_exists",
            ProjectError::RunNotFound { .. } => "project.run_not_found",
            ProjectError::FlightNotFound { .. } => "project.flight_not_found",
            ProjectError::SessionNotFound { .. } => "project.session_not_found",
            ProjectError::UnsupportedArtifactVersion { .. } => "project.unsupported_version",
            ProjectError::Settings { .. } => "project.settings",
        }
    }

    /// The text the UI shows as the primary message.
    ///
    /// Kept separate from `Display` so a caller can choose a longer or shorter
    /// form without changing the error itself.
    pub fn user_message(&self) -> String {
        self.to_string()
    }

    /// What the user can do about it.
    pub fn suggested_action(&self) -> &'static str {
        match self {
            ProjectError::Io { .. } => {
                "Check that the folder exists and that HexaDOF has permission to write to it."
            }
            ProjectError::NotFound { .. } => "Choose an existing project folder.",
            ProjectError::DirectoryNotEmpty { .. } => {
                "Choose an empty folder, or open the existing project instead."
            }
            ProjectError::MissingProjectFile { .. } => {
                "Select the folder that contains project.hexadof.json."
            }
            ProjectError::Parse { .. } => {
                "The file may be damaged. Restore it from a backup, or open a different project."
            }
            ProjectError::Serialize { .. } => "Report this as a bug; the data could not be written.",
            ProjectError::InvalidMetadata { .. } => {
                "Edit project.hexadof.json to supply the missing fields, or create a new project."
            }
            ProjectError::ReferenceOutside { .. } => {
                "HexaDOF will not follow a path out of the project. Import the file into the project instead."
            }
            ProjectError::UnsafeFileName { .. } => "Use a plain file name without folders or colons.",
            ProjectError::RefusedSymlink { .. } => {
                "Replace the symbolic link with a real folder inside the project."
            }
            ProjectError::ArtifactExists { .. } => {
                "Saved runs and sessions are never overwritten. Save under a new name."
            }
            ProjectError::RunNotFound { .. } => "Refresh the run list and select an existing run.",
            ProjectError::FlightNotFound { .. } => {
                "Refresh the flight list and select an existing flight."
            }
            ProjectError::SessionNotFound { .. } => {
                "Refresh the session list and select an existing session."
            }
            ProjectError::UnsupportedArtifactVersion { .. } => {
                "This artifact was written by a different version of HexaDOF. Open it with that version, or re-run the work."
            }
            ProjectError::Settings { .. } => {
                "Delete the settings file to fall back to defaults, then reconfigure."
            }
        }
    }

    /// Reduced detail for a one-line log entry.
    pub fn one_line(&self) -> String {
        format!("[{}] {}", self.code(), self.user_message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<ProjectError> {
        vec![
            ProjectError::Io {
                path: PathBuf::from("a.json"),
                detail: "denied".to_string(),
            },
            ProjectError::NotFound {
                path: PathBuf::from("dir"),
            },
            ProjectError::DirectoryNotEmpty {
                path: PathBuf::from("dir"),
            },
            ProjectError::MissingProjectFile {
                name: "project.hexadof.json",
                path: PathBuf::from("dir"),
            },
            ProjectError::Parse {
                path: PathBuf::from("a.json"),
                detail: "bad".to_string(),
            },
            ProjectError::Serialize {
                detail: "bad".to_string(),
            },
            ProjectError::InvalidMetadata {
                detail: "empty name".to_string(),
            },
            ProjectError::ReferenceOutside {
                reference: "../x".to_string(),
            },
            ProjectError::UnsafeFileName {
                name: "../x".to_string(),
            },
            ProjectError::RefusedSymlink {
                path: PathBuf::from("link"),
            },
            ProjectError::ArtifactExists {
                path: PathBuf::from("run-001"),
            },
            ProjectError::RunNotFound {
                name: "run-001".to_string(),
            },
            ProjectError::FlightNotFound {
                name: "flight-001".to_string(),
            },
            ProjectError::SessionNotFound {
                name: "session-001".to_string(),
            },
            ProjectError::UnsupportedArtifactVersion {
                path: PathBuf::from("summary.json"),
                found: "2.0".to_string(),
                expected: "1.0".to_string(),
            },
            ProjectError::Settings {
                path: PathBuf::from("settings.json"),
                detail: "bad".to_string(),
            },
        ]
    }

    #[test]
    fn every_variant_is_actionable() {
        for e in every_variant() {
            assert!(!e.user_message().is_empty(), "{:?}", e);
            assert!(!e.suggested_action().is_empty(), "{:?}", e);
            assert!(!e.code().is_empty(), "{:?}", e);
            assert!(e.is_blocking());
            assert!(e.one_line().starts_with("[project."), "{}", e.one_line());
        }
    }

    #[test]
    fn codes_are_distinct() {
        let mut codes: Vec<&str> = every_variant().iter().map(|e| e.code()).collect();
        let count = codes.len();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), count);
    }

    #[test]
    fn io_helper_attaches_the_path() {
        let e = ProjectError::io("C:/x/y.json", "access denied");
        assert!(e.user_message().contains("y.json"));
        assert!(e.user_message().contains("access denied"));
    }

    #[test]
    fn messages_name_the_offending_artifact() {
        let e = ProjectError::UnsupportedArtifactVersion {
            path: PathBuf::from("simulations/run-003/summary.json"),
            found: "9.9".to_string(),
            expected: "1.0".to_string(),
        };
        let message = e.user_message();
        assert!(message.contains("run-003"));
        assert!(message.contains("9.9"));
        assert!(message.contains("1.0"));
    }

    #[test]
    fn immutability_error_explains_the_rule() {
        let e = ProjectError::ArtifactExists {
            path: PathBuf::from("run-001"),
        };
        assert!(e.suggested_action().contains("never overwritten"));
    }
}
