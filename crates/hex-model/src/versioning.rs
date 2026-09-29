//! Schema version parsing and the migration step.
//!
//! Versioning keeps one promise: a document written for a different major schema
//! is refused rather than half-read. Minor differences are compatible by
//! definition, because the schema only adds fields with defaults.

use std::fmt;
use std::str::FromStr;

use crate::error::ModelError;
use crate::schema::ModelDocument;

/// Schema version this build reads and writes.
pub const SCHEMA_VERSION: &str = "1.0";

/// A parsed schema or model version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModelVersion {
    /// Incompatible-change counter.
    pub major: u32,
    /// Backward-compatible addition counter.
    pub minor: u32,
    /// Fix counter.
    pub patch: u32,
}

impl ModelVersion {
    /// Build a version from its three components.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse `"1.0"` or `"1.2.3"`. A missing component counts as zero.
    ///
    /// Text that is not a dotted number is reported as
    /// [`ModelError::UnsupportedSchemaVersion`], because a version this build
    /// cannot parse is by definition one it cannot claim to support.
    pub fn parse(text: &str) -> Result<Self, ModelError> {
        let trimmed = text.trim();
        let mut parts = trimmed.split('.');
        let major = parse_component(parts.next(), trimmed)?;
        let minor = match parts.next() {
            Some(part) => parse_component(Some(part), trimmed)?,
            None => 0,
        };
        let patch = match parts.next() {
            Some(part) => parse_component(Some(part), trimmed)?,
            None => 0,
        };
        if parts.next().is_some() {
            return Err(unsupported(trimmed));
        }
        Ok(Self {
            major,
            minor,
            patch,
        })
    }

    /// Whether the two versions can be read by the same code.
    ///
    /// Only the major component matters: a minor or patch difference adds
    /// optional fields, which an older reader ignores.
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.major == other.major
    }
}

impl fmt::Display for ModelVersion {
    /// Writes the shortest exact form, so `1.0.0` prints as `1.0`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.patch == 0 {
            write!(f, "{}.{}", self.major, self.minor)
        } else {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        }
    }
}

impl FromStr for ModelVersion {
    type Err = ModelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// The version this build supports, parsed.
fn supported_version() -> ModelVersion {
    ModelVersion::parse(SCHEMA_VERSION).unwrap_or(ModelVersion::new(1, 0, 0))
}

fn unsupported(found: &str) -> ModelError {
    ModelError::UnsupportedSchemaVersion {
        found: found.to_string(),
        supported: SCHEMA_VERSION.to_string(),
    }
}

fn parse_component(part: Option<&str>, whole: &str) -> Result<u32, ModelError> {
    match part {
        Some(text) if !text.is_empty() => text.parse::<u32>().map_err(|_| unsupported(whole)),
        _ => Err(unsupported(whole)),
    }
}

/// Check that a document's schema version is one this build can read.
pub fn check_schema_version(version: &str) -> Result<ModelVersion, ModelError> {
    let parsed = ModelVersion::parse(version)?;
    if !parsed.is_compatible_with(&supported_version()) {
        return Err(ModelError::UnsupportedSchemaVersion {
            found: version.trim().to_string(),
            supported: SCHEMA_VERSION.to_string(),
        });
    }
    Ok(parsed)
}

/// Bring a document up to the current schema in place.
///
/// Returns one description per migration that was applied, in the order they ran.
/// The MVP has a single migration: a document with no schema version at all is
/// treated as version 1.0, which is what the first published schema was.
pub fn migrate_document(doc: &mut ModelDocument) -> Vec<String> {
    let mut applied = Vec::new();
    if doc.schema_version.trim().is_empty() {
        doc.schema_version = SCHEMA_VERSION.to_string();
        applied.push(format!(
            "schema_version was missing and was read as {SCHEMA_VERSION}"
        ));
    }
    applied
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_two_and_three_component_versions() {
        assert_eq!(
            ModelVersion::parse("1.0").unwrap(),
            ModelVersion::new(1, 0, 0)
        );
        assert_eq!(
            ModelVersion::parse("1.2.3").unwrap(),
            ModelVersion::new(1, 2, 3)
        );
        assert_eq!(
            ModelVersion::parse(" 2.5 ").unwrap(),
            ModelVersion::new(2, 5, 0)
        );
        assert_eq!(
            ModelVersion::parse("3").unwrap(),
            ModelVersion::new(3, 0, 0)
        );
    }

    #[test]
    fn rejects_text_that_is_not_a_version() {
        for bad in ["", " ", "one.two", "1.x", "1.2.3.4", "1.2.3-beta", "-1.0"] {
            assert!(
                ModelVersion::parse(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn bad_version_reports_the_unsupported_schema_error() {
        let err = ModelVersion::parse("banana").unwrap_err();
        assert!(matches!(err, ModelError::UnsupportedSchemaVersion { .. }));
        assert!(err.user_message().contains("schema version"));
    }

    #[test]
    fn from_str_matches_parse() {
        let parsed: ModelVersion = "1.2.3".parse().unwrap();
        assert_eq!(parsed, ModelVersion::parse("1.2.3").unwrap());
    }

    #[test]
    fn display_uses_the_shortest_exact_form() {
        assert_eq!(ModelVersion::new(1, 0, 0).to_string(), "1.0");
        assert_eq!(ModelVersion::new(1, 2, 0).to_string(), "1.2");
        assert_eq!(ModelVersion::new(1, 2, 3).to_string(), "1.2.3");
        assert_eq!(SCHEMA_VERSION, ModelVersion::new(1, 0, 0).to_string());
    }

    #[test]
    fn display_round_trips_to_an_equal_version() {
        for text in ["1.0", "1.2.3", "2.0"] {
            let version = ModelVersion::parse(text).unwrap();
            assert_eq!(ModelVersion::parse(&version.to_string()).unwrap(), version);
        }
    }

    #[test]
    fn ordering_follows_major_then_minor_then_patch() {
        assert!(ModelVersion::new(1, 0, 0) < ModelVersion::new(1, 1, 0));
        assert!(ModelVersion::new(1, 1, 0) < ModelVersion::new(1, 1, 1));
        assert!(ModelVersion::new(1, 9, 9) < ModelVersion::new(2, 0, 0));
        assert_eq!(
            ModelVersion::new(1, 2, 3).max(ModelVersion::new(1, 2, 2)),
            ModelVersion::new(1, 2, 3)
        );
    }

    #[test]
    fn compatibility_only_depends_on_major() {
        let base = ModelVersion::new(1, 0, 0);
        assert!(base.is_compatible_with(&ModelVersion::new(1, 7, 3)));
        assert!(!base.is_compatible_with(&ModelVersion::new(2, 0, 0)));
    }

    #[test]
    fn schema_check_accepts_the_current_major() {
        assert_eq!(
            check_schema_version(SCHEMA_VERSION).unwrap(),
            ModelVersion::new(1, 0, 0)
        );
        assert_eq!(
            check_schema_version("1.4").unwrap(),
            ModelVersion::new(1, 4, 0)
        );
    }

    #[test]
    fn schema_check_rejects_another_major() {
        let err = check_schema_version("2.0").unwrap_err();
        match err {
            ModelError::UnsupportedSchemaVersion { found, supported } => {
                assert_eq!(found, "2.0");
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected UnsupportedSchemaVersion, got {other:?}"),
        }
    }

    #[test]
    fn schema_check_rejects_an_unparseable_version() {
        assert!(check_schema_version("next").is_err());
    }

    #[test]
    fn migration_fills_in_a_missing_schema_version() {
        let mut doc = ModelDocument {
            schema_version: String::new(),
            ..ModelDocument::default()
        };
        let applied = migrate_document(&mut doc);
        assert_eq!(doc.schema_version, SCHEMA_VERSION);
        assert_eq!(applied.len(), 1);
        assert!(applied[0].contains(SCHEMA_VERSION));
    }

    #[test]
    fn migration_leaves_a_current_version_alone() {
        let mut doc = ModelDocument::minimal("m", "n", 1.0, [1.0, 1.0, 1.0]);
        let applied = migrate_document(&mut doc);
        assert!(applied.is_empty());
        assert_eq!(doc.schema_version, SCHEMA_VERSION);
    }

    #[test]
    fn migration_is_idempotent() {
        let mut doc = ModelDocument {
            schema_version: "   ".to_string(),
            ..ModelDocument::default()
        };
        assert_eq!(migrate_document(&mut doc).len(), 1);
        assert!(migrate_document(&mut doc).is_empty());
    }
}
