// SPDX-License-Identifier: Apache-2.0

//! The test report: the machine-facing result of `bearout test`, distinct
//! from the contract report. A case that does not match its expectation
//! is an assertion failure, not a B-series finding; a suite that cannot
//! be run is fatal.

use std::fmt;

use serde::Serialize;

use super::matching::Expectation;
use crate::report::{Diagnostic, SourceInfo};

/// The class of what a case expects and what a candidate produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// The candidate reported nothing at all, warnings included.
    Clean,
    /// The candidate reported at least one diagnostic of any severity.
    Diagnostics,
    /// The candidate's evaluation failed fatally.
    Fatal,
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Clean => "clean",
            Self::Diagnostics => "diagnostics",
            Self::Fatal => "fatal",
        })
    }
}

/// How expected diagnostics are compared with actual ones.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Matching {
    /// Every expected diagnostic occurs and nothing else does.
    #[default]
    Exact,
    /// Every expected diagnostic occurs; other diagnostics are allowed.
    Contains,
}

/// The result of one case.
#[derive(Debug, Clone, Serialize)]
pub struct CaseResult {
    /// The case name, unique across the suite.
    pub name: String,
    /// The fixture file the case came from, project-relative.
    pub file: String,
    pub passed: bool,
    pub expected: Outcome,
    pub actual: Outcome,
    /// Expected diagnostics no actual diagnostic satisfied.
    pub missing: Vec<Expectation>,
    /// Actual diagnostics no expectation covered, when that fails the
    /// case; empty under `contains` matching once the outcomes agree.
    pub unexpected: Vec<Diagnostic>,
    /// The text an expected fatal outcome must contain, when asserted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_fatal: Option<String>,
    /// The candidate's fatal message, when its evaluation failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fatal: Option<String>,
}

/// The result of one `bearout test` run. Serialized as JSON for every
/// outcome, including a fatal one.
#[derive(Debug, Default, Serialize)]
pub struct TestReport {
    /// `true` when every case passed and the suite ran.
    pub ok: bool,
    /// The Git-backed source the suite was read from, when one was
    /// selected.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceInfo>,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    /// Every case in suite order: fixture files sorted, cases in file
    /// order.
    pub cases: Vec<CaseResult>,
    /// A failure that prevented the suite from running, when one
    /// occurred: no case result is reported then.
    pub fatal: Option<String>,
}

impl TestReport {
    /// A report for a suite that could not run.
    #[must_use]
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            fatal: Some(message.into()),
            ..Self::default()
        }
    }

    /// Settle the counts and `ok` from the case results.
    pub(crate) fn finish(&mut self) {
        self.total = self.cases.len();
        self.passed = self.cases.iter().filter(|case| case.passed).count();
        self.failed = self.total - self.passed;
        self.ok = self.fatal.is_none() && self.failed == 0;
    }
}
