//! Structured validation reporting shared by every HexaDOF crate.
//!
//! Errors must be actionable. Every issue carries a human-readable title, an
//! explanation, a severity, a machine-readable code, and (where possible) a
//! suggested fix and the technical detail an advanced user can expand.

use serde::{Deserialize, Serialize};

/// How serious an issue is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Purely informational; the data is usable as-is.
    Info,
    /// Worth checking but not a blocker. Typical of simplified models.
    Warning,
    /// Blocks the operation that produced the report.
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "Info",
            Severity::Warning => "Warning",
            Severity::Error => "Error",
        }
    }

    /// Whether a report containing only issues of this severity can proceed.
    pub fn is_blocking(self) -> bool {
        matches!(self, Severity::Error)
    }
}

/// A single actionable finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Stable machine code, for example `inertia.not_positive_definite`.
    pub code: String,
    /// Short human-readable title.
    pub title: String,
    /// Plain-language explanation of what is wrong and why it matters.
    pub detail: String,
    pub severity: Severity,
    /// Where the problem was found, for example a file and line or a field path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// What the user can do about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// Expanded technical detail, shown only when the user asks for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical: Option<String>,
}

impl ValidationIssue {
    pub fn new(
        code: impl Into<String>,
        severity: Severity,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            title: title.into(),
            detail: detail.into(),
            severity,
            context: None,
            suggestion: None,
            technical: None,
        }
    }

    pub fn error(
        code: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(code, Severity::Error, title, detail)
    }

    pub fn warning(
        code: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(code, Severity::Warning, title, detail)
    }

    pub fn info(
        code: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(code, Severity::Info, title, detail)
    }

    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.context = Some(context.into());
        self
    }

    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    pub fn with_technical(mut self, technical: impl Into<String>) -> Self {
        self.technical = Some(technical.into());
        self
    }

    /// One-line rendering used in logs and diagnostic bundles.
    pub fn one_line(&self) -> String {
        match &self.context {
            Some(c) => format!(
                "[{}] {} ({}): {}",
                self.severity.label(),
                self.title,
                c,
                self.detail
            ),
            None => format!(
                "[{}] {}: {}",
                self.severity.label(),
                self.title,
                self.detail
            ),
        }
    }
}

/// Overall outcome of a validation pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    /// No issues at all.
    Valid,
    /// Issues exist but none block the operation.
    ValidWithWarnings,
    /// At least one blocking issue exists.
    Invalid,
}

impl ValidationStatus {
    /// Whether the operation may proceed.
    pub fn can_proceed(self) -> bool {
        !matches!(self, ValidationStatus::Invalid)
    }

    pub fn label(self) -> &'static str {
        match self {
            ValidationStatus::Valid => "Valid",
            ValidationStatus::ValidWithWarnings => "Valid with warnings",
            ValidationStatus::Invalid => "Invalid",
        }
    }
}

/// A complete validation result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    /// What was validated, for example `models/rocket.dynamic.json`.
    pub subject: String,
    pub status: ValidationStatus,
    pub issues: Vec<ValidationIssue>,
}

impl ValidationReport {
    pub fn new(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            status: ValidationStatus::Valid,
            issues: Vec::new(),
        }
    }

    /// A report for something that could not be checked at all.
    pub fn failed(subject: impl Into<String>, issue: ValidationIssue) -> Self {
        let mut r = Self::new(subject);
        r.push(issue);
        r
    }

    pub fn push(&mut self, issue: ValidationIssue) {
        self.issues.push(issue);
        self.recompute();
    }

    pub fn extend(&mut self, issues: impl IntoIterator<Item = ValidationIssue>) {
        self.issues.extend(issues);
        self.recompute();
    }

    /// Record a plain problem with a code and message.
    pub fn add_error(&mut self, code: &str, title: &str, detail: impl Into<String>) {
        self.push(ValidationIssue::error(code, title, detail));
    }

    pub fn add_warning(&mut self, code: &str, title: &str, detail: impl Into<String>) {
        self.push(ValidationIssue::warning(code, title, detail));
    }

    pub fn add_info(&mut self, code: &str, title: &str, detail: impl Into<String>) {
        self.push(ValidationIssue::info(code, title, detail));
    }

    pub fn recompute(&mut self) {
        self.status = if self.issues.iter().any(|i| i.severity.is_blocking()) {
            ValidationStatus::Invalid
        } else if self.issues.is_empty() {
            ValidationStatus::Valid
        } else {
            ValidationStatus::ValidWithWarnings
        };
    }

    pub fn error_count(&self) -> usize {
        self.count(Severity::Error)
    }

    pub fn warning_count(&self) -> usize {
        self.count(Severity::Warning)
    }

    pub fn count(&self, severity: Severity) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == severity)
            .count()
    }

    pub fn has_errors(&self) -> bool {
        self.error_count() > 0
    }

    pub fn can_proceed(&self) -> bool {
        self.status.can_proceed()
    }

    pub fn errors(&self) -> impl Iterator<Item = &ValidationIssue> {
        self.issues.iter().filter(|i| i.severity == Severity::Error)
    }

    /// Merge another report into this one, prefixing the subject on conflicts.
    pub fn merge(&mut self, other: ValidationReport) {
        self.issues.extend(other.issues);
        self.recompute();
    }

    /// Short summary line for a status strip.
    pub fn summary(&self) -> String {
        if self.issues.is_empty() {
            return "No issues found".to_string();
        }
        format!(
            "{} error(s), {} warning(s), {} note(s)",
            self.error_count(),
            self.warning_count(),
            self.count(Severity::Info)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_is_valid() {
        let r = ValidationReport::new("test");
        assert_eq!(r.status, ValidationStatus::Valid);
        assert!(r.can_proceed());
        assert_eq!(r.summary(), "No issues found");
    }

    #[test]
    fn warnings_do_not_block_but_errors_do() {
        let mut r = ValidationReport::new("test");
        r.add_warning("w", "Warn", "detail");
        assert_eq!(r.status, ValidationStatus::ValidWithWarnings);
        assert!(r.can_proceed());

        r.add_error("e", "Err", "detail");
        assert_eq!(r.status, ValidationStatus::Invalid);
        assert!(!r.can_proceed());
        assert_eq!(r.error_count(), 1);
        assert_eq!(r.warning_count(), 1);
    }

    #[test]
    fn report_merges_and_recomputes() {
        let mut a = ValidationReport::new("a");
        let mut b = ValidationReport::new("b");
        b.add_error("x", "X", "boom");
        a.merge(b);
        assert!(a.has_errors());
        assert_eq!(a.issues.len(), 1);
    }

    #[test]
    fn issue_builder_keeps_all_fields() {
        let i = ValidationIssue::error("code", "Title", "Detail")
            .with_context("models/rocket.json:12")
            .with_suggestion("Fix the export")
            .with_technical("eigenvalue = -0.4");
        assert_eq!(i.context.as_deref(), Some("models/rocket.json:12"));
        assert!(i.one_line().contains("models/rocket.json:12"));
    }

    #[test]
    fn failed_report_records_the_blocking_issue() {
        let r = ValidationReport::failed(
            "models/x.json",
            ValidationIssue::error("io", "Unreadable", "File could not be parsed"),
        );
        assert_eq!(r.status, ValidationStatus::Invalid);
    }

    #[test]
    fn severity_ordering() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }
}
