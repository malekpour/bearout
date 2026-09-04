// SPDX-License-Identifier: Apache-2.0

//! Repository history and commit policy, `bearout history`.
//! **Experimental.**
//!
//! The kernel establishes exact Git facts (a commit range, or the commit
//! a `commit-msg` hook is about to make), loads the repository policy
//! from the tree those facts belong to, and runs the policy's registered
//! history checks over one immutable view. What a commit message must
//! look like, which types or scopes are allowed, whether merges are
//! exempt, and what a sign-off means are the repository's rules, written
//! in Starlark; the kernel holds no Conventional Commits parser and no
//! DCO semantics.
//!
//! Authority: a range reads `bearout.toml`, the entry module, and every
//! loaded module from the resolved head's tree; a pending commit reads
//! them from the captured index. Neither the working tree nor an
//! unstaged edit can change the policy a run applies, and the facts come
//! from the same repository as the policy. Only history checks run: no
//! resource discovery, document or hygiene checks, ordinary checks,
//! generators, formatters, or fixtures.

pub mod capture;
pub mod report;
pub mod view;

use std::path::{Path, PathBuf};

use self::capture::{History, Mode};
pub use self::report::{HistoryDiagnostic, HistoryReport, Resolved, Target};
use crate::bootstrap::{self, MANIFEST_NAME};
use crate::paths::ProjectPath;
use crate::policy::values::Finding;
use crate::policy::{self, Policy};
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;
use crate::{Options, Source};

/// What `bearout history` inspects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryMode {
    /// The commits reachable from `head` (default `HEAD`) but not from
    /// `base`; every commit reachable from `head` without a base.
    Range {
        base: Option<String>,
        head: Option<String>,
    },
    /// The pending commit whose message is in `file`, as a `commit-msg`
    /// hook sees it.
    Message { file: PathBuf },
}

/// Run the history checks the project at `root` registers. Never panics
/// on repository content; a run that cannot establish its facts or load
/// its policy is reported in [`HistoryReport::fatal`].
#[must_use]
pub fn run(root: &Path, mode: &HistoryMode, options: &Options) -> HistoryReport {
    match run_inner(root, mode, options) {
        Ok(report) => report,
        Err(message) => HistoryReport {
            mode: match mode {
                HistoryMode::Range { .. } => Mode::Range,
                HistoryMode::Message { .. } => Mode::Message,
            }
            .as_str()
            .to_owned(),
            ..HistoryReport::fatal(message)
        },
    }
}

fn run_inner(root: &Path, mode: &HistoryMode, options: &Options) -> Result<HistoryReport, String> {
    if options.source != Source::WorkingDirectory || options.baseline.is_some() {
        return Err(
            "`bearout history` reads its policy from the resolved head or the captured index; it takes no source selection or comparison baseline"
                .to_owned(),
        );
    }
    let mut opened = match mode {
        HistoryMode::Range { base, head } => {
            capture::range(root, base.as_deref(), head.as_deref().unwrap_or("HEAD"))?
        }
        HistoryMode::Message { file } => capture::pending(root, file)?,
    };

    // The bootstrap and its limits come from the policy tree; the facts
    // are then read under those limits.
    let tree = &opened.policy_tree;
    let manifest_path = ProjectPath::parse(MANIFEST_NAME).expect("constant path");
    let manifest_text = tree.read_text(&manifest_path).map_err(|error| {
        format!(
            "cannot read {MANIFEST_NAME} from the {}: {error}",
            policy_source_name(&opened.source)
        )
    })?;
    let bootstrap = bootstrap::parse(&manifest_text)?;
    if !tree.is_dir(&bootstrap.rules_root) {
        return Err(format!(
            "rules root `{}` is not a directory inside the project",
            bootstrap.rules_root
        ));
    }
    if !tree.is_file(&bootstrap.entry) {
        return Err(format!(
            "entry module `{}` is not a file inside the project",
            bootstrap.entry
        ));
    }
    let history = opened.capture(&bootstrap.limits)?;
    let source = opened.source.clone();
    let tree = &opened.policy_tree;

    let mut report = HistoryReport {
        ok: false,
        mode: history.mode.as_str().to_owned(),
        source: Some(source),
        base: history.base.as_ref().map(resolved),
        head: history.head.as_ref().map(resolved),
        commits: history.commits.len(),
        diagnostics: Vec::new(),
        fatal: None,
    };
    let cancel = options.cancel.clone().unwrap_or_default();
    let mut load_diagnostics = Vec::new();
    let policy = policy::load(tree, &bootstrap, cancel, &mut load_diagnostics);
    report
        .diagnostics
        .extend(load_diagnostics.into_iter().map(from_contract));
    let Some(policy) = policy else {
        finish(&mut report, &history);
        report.fatal =
            Some("the repository policy did not load; the diagnostics name the problem".to_owned());
        return Ok(report);
    };
    if policy.history_checks.is_empty() {
        return Err(
            "the policy registers no history check; register one with `history_check(name, function)` in the entry module"
                .to_owned(),
        );
    }
    report.diagnostics.extend(run_checks(&policy, &history)?);
    finish(&mut report, &history);
    Ok(report)
}

/// Run every registered history check over the view of `history` and
/// admit its findings. Shared with history fixture cases.
pub(crate) fn run_checks(
    policy: &Policy,
    history: &History,
) -> Result<Vec<HistoryDiagnostic>, String> {
    let view = view::frozen(history)?;
    let script = policy.entry.as_str();
    let mut diagnostics = Vec::new();
    for (name, callback) in &policy.history_checks {
        let label = format!("history check `{name}`");
        let outcome = policy.history(callback, &view);
        diagnostics.extend(outcome.printed.iter().map(|line| {
            HistoryDiagnostic::new(
                path_target(script),
                Code::ScriptOutput,
                format!("{label} printed: {line}"),
            )
        }));
        match outcome.result {
            Ok(findings) => {
                for finding in &findings {
                    diagnostics.push(admit(finding, name, &label, script, history));
                }
            }
            Err(error) => diagnostics.push(from_contract(policy::failure_diagnostic(
                script, &label, &error,
            ))),
        }
    }
    Ok(diagnostics)
}

fn path_target(path: &str) -> Target {
    Target::Path {
        path: path.to_owned(),
    }
}

fn resolved(reference: &capture::Reference) -> Resolved {
    Resolved {
        revision: reference.revision.clone(),
        id: reference.id.to_string(),
    }
}

fn policy_source_name(source: &crate::report::SourceInfo) -> String {
    match source.kind.as_str() {
        "index" => "captured index".to_owned(),
        _ => format!(
            "head tree of `{}`",
            source.revision.as_deref().unwrap_or_default()
        ),
    }
}

/// A policy loading, execution, or result diagnostic, targeting the
/// script it came from.
pub(crate) fn from_contract(diagnostic: Diagnostic) -> HistoryDiagnostic {
    HistoryDiagnostic {
        target: path_target(&diagnostic.path),
        code: diagnostic.code,
        line: diagnostic.line,
        rule: diagnostic.rule,
        severity: diagnostic.severity,
        message: diagnostic.message,
    }
}

/// Turn a history finding into a diagnostic, checking its target against
/// the view: a commit key present in the view (`pending` only for a
/// pending commit) with a line within its message, or no commit at all
/// for a range-wide finding, which then carries no line. A finding that
/// names a resource, a path, or the baseline side is refused.
fn admit(
    finding: &Finding,
    check: &str,
    label: &str,
    script: &str,
    history: &History,
) -> HistoryDiagnostic {
    let reject = |error: String| {
        HistoryDiagnostic::new(
            path_target(script),
            Code::ScriptResult,
            format!("{label} {error}"),
        )
    };
    if finding.resource.is_some() || finding.path.is_some() || finding.baseline {
        return reject(
            "a history finding names a `commit` or nothing; it never names a resource, a document, or a comparison side"
                .to_owned(),
        );
    }
    let target = if let Some(key) = &finding.commit {
        let Some(commit) = history.commits.iter().find(|commit| commit.key == *key) else {
            return reject(if key == "pending" && history.mode == Mode::Range {
                "finding names `pending`, which exists only for a pending-message check".to_owned()
            } else {
                format!("finding names unknown commit `{key}`")
            });
        };
        if let Some(line) = finding.line {
            let count = commit.line_count();
            if line > count {
                return reject(format!(
                    "finding line {line} is beyond the {count} line(s) of the message of commit `{key}`"
                ));
            }
        }
        Target::Commit {
            commit: key.clone(),
        }
    } else {
        if finding.line.is_some() {
            return reject("a finding line needs a `commit` target".to_owned());
        }
        Target::Range {}
    };
    let code = if finding.is_error {
        Code::HistoryError
    } else {
        Code::HistoryWarning
    };
    let rule = finding.rule.clone().unwrap_or_else(|| check.to_owned());
    HistoryDiagnostic::new(target, code, format!("{label}: {}", finding.message))
        .at_line(finding.line)
        .with_rule(Some(rule))
}

/// Sort into the documented order, deduplicate, and settle `ok`.
fn finish(report: &mut HistoryReport, history: &History) {
    sort_diagnostics(&mut report.diagnostics, history);
    report.ok = report.fatal.is_none() && report.diagnostics.is_empty();
}

/// The documented order, then deduplication: script diagnostics by path,
/// then range-wide findings, then commit findings in commit order; within
/// a target by line, code, rule, and message.
pub(crate) fn sort_diagnostics(diagnostics: &mut Vec<HistoryDiagnostic>, history: &History) {
    let position = |key: &str| {
        history
            .commits
            .iter()
            .position(|commit| commit.key == key)
            .unwrap_or(usize::MAX)
    };
    diagnostics.sort_by(|a, b| {
        let key = |d: &HistoryDiagnostic| {
            let (group, path, index) = match &d.target {
                Target::Path { path } => (0u8, path.clone(), 0usize),
                Target::Range {} => (1, String::new(), 0),
                Target::Commit { commit } => (2, String::new(), position(commit)),
            };
            (
                group,
                path,
                index,
                d.line,
                d.code,
                d.rule.clone(),
                d.message.clone(),
            )
        };
        key(a).cmp(&key(b))
    });
    diagnostics.dedup();
}
