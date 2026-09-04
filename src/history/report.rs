// SPDX-License-Identifier: Apache-2.0

//! The history report: the machine-facing result of `bearout history`,
//! distinct from the contract report. A history finding targets a commit
//! of the view, the whole range, or, for the policy's own loading and
//! execution problems, a script path; a commit identity is never encoded
//! as a path.

use std::fmt;

use serde::Serialize;

use crate::report::{Code, Severity, SourceInfo};

/// What a history diagnostic is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Target {
    /// A script of the policy: loading, execution, and result problems.
    Path { path: String },
    /// One commit of the view, by its key: the full identity or `pending`.
    Commit { commit: String },
    /// The whole range: a finding that names no commit.
    Range {},
}

/// One deterministic history finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HistoryDiagnostic {
    #[serde(flatten)]
    pub target: Target,
    pub code: Code,
    /// One-based line of the commit message or the script, when known.
    pub line: Option<u32>,
    /// The registered history check name, or the repository rule
    /// identifier a policy finding carries.
    pub rule: Option<String>,
    pub severity: Severity,
    pub message: String,
}

impl HistoryDiagnostic {
    #[must_use]
    pub fn new(target: Target, code: Code, message: impl Into<String>) -> Self {
        Self {
            target,
            code,
            line: None,
            rule: None,
            severity: code.severity(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn at_line(mut self, line: Option<u32>) -> Self {
        self.line = line;
        self
    }

    #[must_use]
    pub fn with_rule(mut self, rule: Option<String>) -> Self {
        self.rule = rule;
        self
    }
}

impl fmt::Display for HistoryDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.target {
            Target::Path { path } => f.write_str(path)?,
            Target::Commit { commit } => write!(f, "commit {commit}")?,
            Target::Range {} => f.write_str("range")?,
        }
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        write!(f, ":{}", self.code)?;
        if let Some(rule) = &self.rule {
            write!(f, "[{rule}]")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// A revision as supplied and as resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolved {
    pub revision: String,
    pub id: String,
}

/// The result of one `bearout history` run. Serialized as JSON for every
/// outcome, including a fatal one.
#[derive(Debug, Default, Serialize)]
pub struct HistoryReport {
    /// `true` when the facts were established and no history check
    /// reported anything, warnings included.
    pub ok: bool,
    /// `range` or `message`.
    pub mode: String,
    /// The tree the policy was read from: the resolved head as a
    /// revision, or the captured index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<Resolved>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head: Option<Resolved>,
    /// The commits inspected: the range's count, or one for a pending
    /// commit.
    pub commits: usize,
    /// Findings in stable order.
    pub diagnostics: Vec<HistoryDiagnostic>,
    /// A failure that prevented the facts from being established or the
    /// policy from running, when one occurred.
    pub fatal: Option<String>,
}

impl HistoryReport {
    /// A report for a run that failed before checking anything.
    #[must_use]
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            fatal: Some(message.into()),
            ..Self::default()
        }
    }
}
