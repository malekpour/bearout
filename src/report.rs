// SPDX-License-Identifier: Apache-2.0

//! Diagnostics and the report: the machine-facing result of every run.

use std::fmt;

use serde::Serialize;

/// Stable diagnostic identifier. The `Bnnn` text is the public surface; the
/// variant names are internal. See `docs/diagnostics.md` for the catalog and
/// its stability policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Code {
    /// A resource or shape file could not be read, or a resource exceeds the size limit.
    Unreadable,
    /// The resource envelope is malformed: front matter, TOML, or reserved keys.
    Envelope,
    /// A schema identifier is malformed or names a schema no policy registered.
    SchemaIdentity,
    /// A shape file is not a usable JSON Schema or its `x-bearout` vocabulary is invalid.
    ShapeInvalid,
    /// Front matter or a fragment violates its declared shape.
    ShapeViolation,
    /// A section the shape requires is missing from the body.
    MissingSection,
    /// A fenced fragment is malformed or of an undeclared kind.
    FragmentMalformed,
    /// The same identifier is defined more than once.
    DuplicateId,
    /// A reference names an identifier that nothing defines.
    UnresolvedReference,
    /// A typed relation resolves to a node of the wrong kind.
    ReferenceKind,
    /// A Markdown link points at a missing file or anchor.
    UnresolvedLink,
    /// A Starlark module could not be loaded, parsed, resolved, or typechecked.
    ScriptLoad,
    /// A Starlark call failed, was cancelled, or exceeded a resource limit.
    ScriptFailure,
    /// A Starlark call returned a value the ABI does not accept.
    ScriptResult,
    /// An error reported by repository policy.
    PolicyError,
    /// A warning reported by repository policy.
    PolicyWarning,
    /// A script printed text. Reported as a warning.
    ScriptOutput,
    /// A Starlark lint finding. Reported as a warning.
    ScriptLint,
    /// A generation plan entry is invalid or could not be rendered.
    PlanInvalid,
    /// A generated output is missing, stale, orphaned, or changed ownership.
    OutputState,
    /// Delivering a generated output to the project tree failed.
    Delivery,
    /// A schema-less document could not be read, is not valid UTF-8, or
    /// exceeds the document size limit.
    DocumentUnreadable,
    /// An `.editorconfig` of the selected tree is unusable, or a property
    /// it sets for a selected file has a value Bearout cannot enforce.
    HygieneConfig,
    /// A selected file could not be read or exceeds the file size limit.
    FileUnreadable,
    /// A selected text file is not valid UTF-8 or contradicts its charset.
    Encoding,
    /// A line terminator contradicts `end_of_line`.
    LineEnding,
    /// The end of the file contradicts `insert_final_newline`.
    FinalNewline,
    /// A line ends with whitespace that `trim_trailing_whitespace` forbids.
    TrailingWhitespace,
    /// A selected file differs from its formatter's output.
    FormatDifference,
    /// A formatter run failed for a selected file.
    FormatterFailed,
    /// A formatting write was refused or failed.
    FormatWrite,
    /// An error reported by a repository history check.
    HistoryError,
    /// A warning reported by a repository history check.
    HistoryWarning,
}

impl Code {
    /// The stable textual identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unreadable => "B001",
            Self::Envelope => "B002",
            Self::SchemaIdentity => "B003",
            Self::ShapeInvalid => "B004",
            Self::ShapeViolation => "B005",
            Self::MissingSection => "B006",
            Self::FragmentMalformed => "B007",
            Self::DuplicateId => "B008",
            Self::UnresolvedReference => "B009",
            Self::ReferenceKind => "B010",
            Self::UnresolvedLink => "B011",
            Self::ScriptLoad => "B012",
            Self::ScriptFailure => "B013",
            Self::ScriptResult => "B014",
            Self::PolicyError => "B015",
            Self::PolicyWarning => "B016",
            Self::ScriptOutput => "B017",
            Self::ScriptLint => "B018",
            Self::PlanInvalid => "B019",
            Self::OutputState => "B020",
            Self::Delivery => "B021",
            Self::DocumentUnreadable => "B022",
            Self::HygieneConfig => "B023",
            Self::FileUnreadable => "B024",
            Self::Encoding => "B025",
            Self::LineEnding => "B026",
            Self::FinalNewline => "B027",
            Self::TrailingWhitespace => "B028",
            Self::FormatDifference => "B029",
            Self::FormatterFailed => "B030",
            Self::FormatWrite => "B031",
            Self::HistoryError => "B032",
            Self::HistoryWarning => "B033",
        }
    }

    /// The severity every diagnostic with this code carries.
    #[must_use]
    pub const fn severity(self) -> Severity {
        match self {
            Self::PolicyWarning | Self::ScriptOutput | Self::ScriptLint | Self::HistoryWarning => {
                Severity::Warning
            }
            _ => Severity::Error,
        }
    }

    /// Every code, in catalog order.
    pub const ALL: [Self; 33] = [
        Self::Unreadable,
        Self::Envelope,
        Self::SchemaIdentity,
        Self::ShapeInvalid,
        Self::ShapeViolation,
        Self::MissingSection,
        Self::FragmentMalformed,
        Self::DuplicateId,
        Self::UnresolvedReference,
        Self::ReferenceKind,
        Self::UnresolvedLink,
        Self::ScriptLoad,
        Self::ScriptFailure,
        Self::ScriptResult,
        Self::PolicyError,
        Self::PolicyWarning,
        Self::ScriptOutput,
        Self::ScriptLint,
        Self::PlanInvalid,
        Self::OutputState,
        Self::Delivery,
        Self::DocumentUnreadable,
        Self::HygieneConfig,
        Self::FileUnreadable,
        Self::Encoding,
        Self::LineEnding,
        Self::FinalNewline,
        Self::TrailingWhitespace,
        Self::FormatDifference,
        Self::FormatterFailed,
        Self::FormatWrite,
        Self::HistoryError,
        Self::HistoryWarning,
    ];
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Whether a diagnostic fails the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Fails the run.
    Error,
    /// Reported but does not fail the run.
    Warning,
}

/// Which tree a diagnostic or finding is about. The candidate is the
/// checked project; the baseline is the comparison revision.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The project being checked.
    #[default]
    Candidate,
    /// The historical revision it is compared against.
    Baseline,
}

impl Side {
    /// `true` for the candidate, which JSON leaves implicit.
    #[must_use]
    pub fn is_candidate(&self) -> bool {
        *self == Self::Candidate
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Candidate => "candidate",
            Self::Baseline => "baseline",
        })
    }
}

/// One deterministic finding. Ordering is by side, path, code, line, rule,
/// and message, which is also the order of the report: every candidate
/// finding precedes every baseline finding.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct Diagnostic {
    /// Which tree the finding is about. Serialized only for the baseline.
    #[serde(skip_serializing_if = "Side::is_candidate")]
    pub side: Side,
    /// Project-relative path with forward slashes on every platform.
    pub path: String,
    /// Stable machine-readable identifier.
    pub code: Code,
    /// One-based line in `path`, when known.
    pub line: Option<u32>,
    /// Repository-owned rule identifier attached by policy, when given.
    pub rule: Option<String>,
    /// Whether the finding fails the run.
    pub severity: Severity,
    /// Human-readable explanation.
    pub message: String,
}

impl Diagnostic {
    /// Build a diagnostic with the severity its code implies.
    #[must_use]
    pub fn new(code: Code, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            side: Side::Candidate,
            path: path.into(),
            code,
            line: None,
            rule: None,
            severity: code.severity(),
            message: message.into(),
        }
    }

    /// Attach a one-based line number.
    #[must_use]
    pub fn at_line(mut self, line: Option<u32>) -> Self {
        self.line = line;
        self
    }

    /// Attach a repository rule identifier.
    #[must_use]
    pub fn with_rule(mut self, rule: Option<String>) -> Self {
        self.rule = rule;
        self
    }

    /// Attribute the finding to a side.
    #[must_use]
    pub fn on_side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.side == Side::Baseline {
            f.write_str("baseline:")?;
        }
        f.write_str(&self.path)?;
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

/// Which Git-backed source a report describes. **Experimental**: present
/// only for the index and revision sources, so that a report can be tied
/// to the exact content it examined; absent for the working directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceInfo {
    /// `index` or `revision`.
    pub kind: String,
    /// The revision name as given, for the revision source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// The resolved tree object identity, for the revision source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    /// A deterministic BLAKE3 digest of the captured entries beneath the
    /// project (kind, object identity, and path of every file, link, and
    /// gitlink), the same for identical content from either source. Not a
    /// Git object identity.
    pub digest: String,
}

/// Result of checking or generating one project. Serialized as the JSON
/// report for every outcome, including fatal failures.
#[derive(Debug, Default, Serialize)]
pub struct Report {
    /// `true` when the run completed without errors.
    pub ok: bool,
    /// Number of discovered resources.
    pub resources: usize,
    /// Number of discovered schema-less documents; zero when the bootstrap
    /// selects none.
    pub documents: usize,
    /// Number of files selected for hygiene and formatting; zero when the
    /// bootstrap selects none. Experimental.
    pub files: usize,
    /// The Git-backed source examined, when one was selected. Experimental.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceInfo>,
    /// The comparison baseline, when one was requested: always a revision.
    /// Experimental.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<SourceInfo>,
    /// Findings in stable order.
    pub diagnostics: Vec<Diagnostic>,
    /// Files rewritten by `format`, as project-relative paths, in path
    /// order; empty for every other command and when nothing changed.
    /// Experimental.
    pub formatted: Vec<String>,
    /// Generated outputs, as project-relative paths, only when generation
    /// succeeded: in write mode the outputs delivered or already current; in
    /// check mode the outputs verified as current. Empty when rendering,
    /// state validation, checking, or delivery reported an error, and empty
    /// for `check` runs.
    pub outputs: Vec<String>,
    /// A failure that prevented the run from completing, when one occurred.
    pub fatal: Option<String>,
    /// Highest Starlark tick count observed in one call. Not serialized;
    /// used to derive default limits.
    #[serde(skip)]
    pub max_ticks: u64,
    /// Highest Starlark heap allocation observed in one call. Not
    /// serialized; used to derive default limits.
    #[serde(skip)]
    pub max_heap_bytes: u64,
    /// Highest `MiniJinja` fuel consumed by one rendered output. Not
    /// serialized; used to derive default limits.
    #[serde(skip)]
    pub max_fuel: u64,
    /// Largest rendered output in bytes. Not serialized.
    #[serde(skip)]
    pub max_output_bytes: u64,
}

impl Report {
    /// Returns `true` when no error-severity finding was produced and the
    /// run did not fail fatally.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.fatal.is_none() && self.errors() == 0
    }

    /// Number of error-severity findings.
    #[must_use]
    pub fn errors(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count()
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn extend(&mut self, diagnostics: impl IntoIterator<Item = Diagnostic>) {
        self.diagnostics.extend(diagnostics);
    }

    /// Sort, dedupe, and settle `ok`.
    pub(crate) fn finish(&mut self) {
        self.diagnostics.sort();
        self.diagnostics.dedup();
        self.outputs.sort();
        self.outputs.dedup();
        self.formatted.sort();
        self.formatted.dedup();
        self.ok = self.is_clean();
    }

    /// A report for a run that failed before producing findings.
    #[must_use]
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            fatal: Some(message.into()),
            ..Self::default()
        }
    }
}
