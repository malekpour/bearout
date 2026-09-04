// SPDX-License-Identifier: Apache-2.0

//! The contract fixture runner, `bearout test`. **Experimental.**
//!
//! A repository declares fixture files in `[fixtures] files`. Each file
//! holds named cases; each case derives a virtual candidate from the
//! selected source tree by applying an ordered list of mutations through a
//! read-only overlay, optionally supplies the unmodified source tree as
//! the comparison baseline, and declares what the candidate's check must
//! produce: nothing, a structured set of diagnostics, or a fatal outcome.
//! The whole suite, payloads included, is read from the selected tree
//! before any case runs, so no mutation can change which cases execute or
//! what they write. Nothing is ever written: not the working directory,
//! the index, Git objects, or the fixture files.
//!
//! Three kinds of failure are kept apart. A contract diagnostic is test
//! data: it fails a case only when the case did not expect it. A case
//! whose result does not match its expectation is an assertion failure
//! and fails the suite with exit code 1. A suite that cannot be run at
//! all, because a fixture is malformed, a mutation is invalid, a payload
//! is missing, a name repeats, a limit is exceeded, or the source cannot
//! be opened, is fatal with exit code 2 and reports no case at all, so a
//! broken suite is never mistaken for a passing one.

pub mod matching;
pub mod overlay;
pub mod report;

use std::path::Path;
use std::sync::Arc;

use toml_edit::{DocumentMut, Item, TableLike, Value};

use self::matching::Expectation;
use self::overlay::{Mutation, Overlay};
pub use self::report::{CaseResult, Matching, Outcome, TestReport};
use crate::bootstrap::{self, Bootstrap, Limits, MANIFEST_NAME};
use crate::paths::ProjectPath;
use crate::policy::views::BaselineIdentity;
use crate::report::{Code, Report, Severity, Side};
use crate::tree::ReadTree;
use crate::{BaselineInput, Command, Inputs, Opened, Options, hygiene};

/// The longest case name accepted.
const MAX_NAME_CHARS: usize = 200;

/// One case, ready to run.
#[derive(Debug)]
struct Case {
    name: String,
    file: ProjectPath,
    mutations: Vec<Mutation>,
    baseline: bool,
    expect: Outcome,
    matching: Matching,
    expectations: Vec<Expectation>,
    fatal: Option<String>,
}

/// A parsed case before its payloads are loaded.
struct Declared {
    name: String,
    file: ProjectPath,
    mutations: Vec<DeclaredMutation>,
    baseline: bool,
    expect: Outcome,
    matching: Matching,
    expectations: Vec<Expectation>,
    fatal: Option<String>,
}

enum DeclaredMutation {
    Write { path: ProjectPath, content: Content },
    Delete { path: ProjectPath },
    Move { from: ProjectPath, to: ProjectPath },
}

enum Content {
    Inline(String),
    Payload(ProjectPath),
}

/// The whole suite, captured from the selected tree before any case
/// runs.
struct Suite {
    cases: Vec<Case>,
}

/// Bytes read for the suite, charged against `limits.fixture_bytes`.
struct FixtureBudget {
    limit: u64,
    remaining: u64,
}

impl FixtureBudget {
    /// Read one file of the suite within what remains of the budget. The
    /// file must be a regular file of the tree not reached through a link.
    fn read(
        &mut self,
        tree: &dyn ReadTree,
        path: &ProjectPath,
        what: &str,
    ) -> Result<Vec<u8>, String> {
        match tree.symlink_component(path) {
            Ok(None) => {}
            Ok(Some(link)) => {
                return Err(format!(
                    "{what} `{path}` is reached through the symbolic link `{link}`; fixtures are never read through links"
                ));
            }
            Err(error) => return Err(format!("cannot inspect {what} `{path}`: {error}")),
        }
        if !tree.is_file(path) {
            return Err(format!(
                "{what} `{path}` is not a file inside the selected tree"
            ));
        }
        let (bytes, over) = tree
            .read_bounded(path, self.remaining)
            .map_err(|error| format!("cannot read {what} `{path}`: {error}"))?;
        let pulled = bytes.len() as u64;
        if over || pulled > self.remaining {
            return Err(format!(
                "fixture inputs exceed `limits.fixture_bytes` = {} while reading {what} `{path}`",
                self.limit
            ));
        }
        self.remaining -= pulled;
        Ok(bytes)
    }
}

impl Suite {
    /// Read and parse every declared fixture file, then every payload.
    fn load(tree: &dyn ReadTree, bootstrap: &Bootstrap) -> Result<Self, String> {
        let limits = &bootstrap.limits;
        let mut budget = FixtureBudget {
            limit: limits.fixture_bytes,
            remaining: limits.fixture_bytes,
        };
        let mut declared = Vec::new();
        for file in &bootstrap.fixture_files {
            let bytes = budget.read(tree, file, "fixture file")?;
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("fixture file `{file}` is not valid UTF-8"))?;
            declared.extend(parse_file(file, &text)?);
        }
        check_limits(&declared, limits)?;
        let mut cases = Vec::with_capacity(declared.len());
        for case in declared {
            let mut mutations = Vec::with_capacity(case.mutations.len());
            for (index, mutation) in case.mutations.into_iter().enumerate() {
                mutations.push(match mutation {
                    DeclaredMutation::Write { path, content } => {
                        let bytes: Arc<[u8]> = match content {
                            Content::Inline(text) => Arc::from(text.into_bytes()),
                            Content::Payload(payload) => {
                                let what = format!(
                                    "payload of case `{}` mutation {}",
                                    case.name,
                                    index + 1
                                );
                                Arc::from(budget.read(tree, &payload, &what)?)
                            }
                        };
                        Mutation::Write { path, bytes }
                    }
                    DeclaredMutation::Delete { path } => Mutation::Delete { path },
                    DeclaredMutation::Move { from, to } => Mutation::Move { from, to },
                });
            }
            cases.push(Case {
                name: case.name,
                file: case.file,
                mutations,
                baseline: case.baseline,
                expect: case.expect,
                matching: case.matching,
                expectations: case.expectations,
                fatal: case.fatal,
            });
        }
        Ok(Self { cases })
    }
}

/// Unique names and the count limits, over the whole suite.
fn check_limits(declared: &[Declared], limits: &Limits) -> Result<(), String> {
    for (index, case) in declared.iter().enumerate() {
        if let Some(earlier) = declared[..index]
            .iter()
            .find(|earlier| earlier.name == case.name)
        {
            return Err(format!(
                "case `{}` in `{}` repeats the name of a case in `{}`; case names must be unique across the suite",
                case.name, case.file, earlier.file
            ));
        }
    }
    if declared.len() > limits.fixture_cases {
        return Err(format!(
            "{} fixture cases exceed `limits.fixture_cases` = {}",
            declared.len(),
            limits.fixture_cases
        ));
    }
    let mutations: usize = declared.iter().map(|case| case.mutations.len()).sum();
    if mutations > limits.fixture_mutations {
        return Err(format!(
            "{mutations} fixture mutations exceed `limits.fixture_mutations` = {}",
            limits.fixture_mutations
        ));
    }
    Ok(())
}

/// Parse one fixture file: `[[cases]]` in file order.
fn parse_file(file: &ProjectPath, text: &str) -> Result<Vec<Declared>, String> {
    let context = |message: String| format!("fixture file `{file}`: {message}");
    let doc: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| {
        context(format!("not valid TOML: {}", error.message()))
    })?;
    let root = doc.as_table();
    reject_unknown(root, &["cases"]).map_err(context)?;
    let cases = table_list(
        root.get("cases")
            .ok_or_else(|| context("`[[cases]]` is required".to_owned()))?,
        "cases",
    )
    .map_err(context)?;
    if cases.is_empty() {
        return Err(context(
            "`[[cases]]` must declare at least one case".to_owned(),
        ));
    }
    let mut declared = Vec::with_capacity(cases.len());
    for (index, case) in cases.iter().enumerate() {
        declared.push(
            parse_case(file, *case)
                .map_err(|message| context(format!("case {}: {message}", index + 1)))?,
        );
    }
    Ok(declared)
}

const CASE_KEYS: [&str; 7] = [
    "name",
    "expect",
    "match",
    "baseline",
    "fatal",
    "mutations",
    "diagnostics",
];

fn parse_case(file: &ProjectPath, table: &dyn TableLike) -> Result<Declared, String> {
    reject_unknown(table, &CASE_KEYS)?;
    let name = string(table, "name")?.ok_or_else(|| "`name` is required".to_owned())?;
    if name.is_empty() || name.trim() != name {
        return Err("`name` must be non-empty without leading or trailing whitespace".to_owned());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("`name` exceeds {MAX_NAME_CHARS} characters"));
    }
    if name.chars().any(char::is_control) {
        return Err("`name` must not contain control characters".to_owned());
    }
    let expect = match string(table, "expect")?.ok_or_else(|| "`expect` is required".to_owned())? {
        "clean" => Outcome::Clean,
        "diagnostics" => Outcome::Diagnostics,
        "fatal" => Outcome::Fatal,
        other => {
            return Err(format!(
                "`expect` must be `clean`, `diagnostics`, or `fatal`, not `{other}`"
            ));
        }
    };
    let matching = match string(table, "match")? {
        Some(_) if expect != Outcome::Diagnostics => {
            return Err("`match` applies only when `expect = \"diagnostics\"`".to_owned());
        }
        None | Some("exact") => Matching::Exact,
        Some("contains") => Matching::Contains,
        Some(other) => {
            return Err(format!(
                "`match` must be `exact` or `contains`, not `{other}`"
            ));
        }
    };
    let baseline = match table.get("baseline") {
        None => false,
        Some(item) => item
            .as_bool()
            .ok_or_else(|| "`baseline` must be a boolean".to_owned())?,
    };
    let fatal = match string(table, "fatal")? {
        None => None,
        Some(_) if expect != Outcome::Fatal => {
            return Err("`fatal` applies only when `expect = \"fatal\"`".to_owned());
        }
        Some("") => {
            return Err("`fatal` must be a non-empty text the fatal message contains".to_owned());
        }
        Some(text) => Some(text.to_owned()),
    };
    let mutations = match table.get("mutations") {
        None => Vec::new(),
        Some(item) => table_list(item, "mutations")?
            .iter()
            .enumerate()
            .map(|(index, mutation)| {
                parse_mutation(*mutation)
                    .map_err(|message| format!("mutation {}: {message}", index + 1))
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    let expectations = match table.get("diagnostics") {
        None if expect == Outcome::Diagnostics => {
            return Err(
                "`expect = \"diagnostics\"` needs at least one `[[cases.diagnostics]]` entry"
                    .to_owned(),
            );
        }
        None => Vec::new(),
        Some(_) if expect != Outcome::Diagnostics => {
            return Err("`diagnostics` applies only when `expect = \"diagnostics\"`".to_owned());
        }
        Some(item) => {
            let listed = table_list(item, "diagnostics")?;
            if listed.is_empty() {
                return Err(
                    "`expect = \"diagnostics\"` needs at least one `[[cases.diagnostics]]` entry"
                        .to_owned(),
                );
            }
            listed
                .iter()
                .enumerate()
                .map(|(index, expectation)| {
                    parse_expectation(*expectation)
                        .map_err(|message| format!("diagnostic {}: {message}", index + 1))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok(Declared {
        name: name.to_owned(),
        file: file.clone(),
        mutations,
        baseline,
        expect,
        matching,
        expectations,
        fatal,
    })
}

fn parse_mutation(table: &dyn TableLike) -> Result<DeclaredMutation, String> {
    reject_unknown(
        table,
        &["write", "delete", "move", "to", "content", "payload"],
    )?;
    let operations: Vec<&str> = ["write", "delete", "move"]
        .into_iter()
        .filter(|key| table.get(key).is_some())
        .collect();
    let operation = match operations.as_slice() {
        [one] => *one,
        [] => {
            return Err("exactly one of `write`, `delete`, or `move` is required".to_owned());
        }
        _ => return Err("`write`, `delete`, and `move` are mutually exclusive".to_owned()),
    };
    let forbid = |keys: &[&str]| -> Result<(), String> {
        for key in keys {
            if table.get(key).is_some() {
                return Err(format!("`{key}` does not apply to `{operation}`"));
            }
        }
        Ok(())
    };
    match operation {
        "write" => {
            forbid(&["to"])?;
            let path = path_field(table, "write")?;
            let content = match (string(table, "content")?, table.get("payload")) {
                (Some(text), None) => Content::Inline(text.to_owned()),
                (None, Some(_)) => Content::Payload(path_field(table, "payload")?),
                (None, None) => {
                    return Err("`write` needs exactly one of `content` or `payload`".to_owned());
                }
                (Some(_), Some(_)) => {
                    return Err("`content` and `payload` are mutually exclusive".to_owned());
                }
            };
            Ok(DeclaredMutation::Write { path, content })
        }
        "delete" => {
            forbid(&["to", "content", "payload"])?;
            Ok(DeclaredMutation::Delete {
                path: path_field(table, "delete")?,
            })
        }
        _ => {
            forbid(&["content", "payload"])?;
            let from = path_field(table, "move")?;
            if table.get("to").is_none() {
                return Err("`move` needs `to`".to_owned());
            }
            let to = path_field(table, "to")?;
            Ok(DeclaredMutation::Move { from, to })
        }
    }
}

fn parse_expectation(table: &dyn TableLike) -> Result<Expectation, String> {
    reject_unknown(
        table,
        &[
            "code", "severity", "path", "line", "side", "rule", "message",
        ],
    )?;
    let code_text = string(table, "code")?.ok_or_else(|| "`code` is required".to_owned())?;
    let code = Code::ALL
        .into_iter()
        .find(|code| code.as_str() == code_text)
        .ok_or_else(|| format!("`code` `{code_text}` is not a Bearout diagnostic code"))?;
    let severity = match string(table, "severity")? {
        None => None,
        Some("error") => Some(Severity::Error),
        Some("warning") => Some(Severity::Warning),
        Some(other) => {
            return Err(format!(
                "`severity` must be `error` or `warning`, not `{other}`"
            ));
        }
    };
    if let Some(severity) = severity
        && severity != code.severity()
    {
        return Err(format!(
            "`severity` contradicts `code`: {code} is always {}",
            match code.severity() {
                Severity::Error => "an error",
                Severity::Warning => "a warning",
            }
        ));
    }
    let path = match table.get("path") {
        None => None,
        Some(_) => Some(path_field(table, "path")?.as_str().to_owned()),
    };
    let line = match table.get("line") {
        None => None,
        Some(item) => Some(
            item.as_integer()
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| "`line` must be a positive integer".to_owned())?,
        ),
    };
    let side = match string(table, "side")? {
        None => None,
        Some("candidate") => Some(Side::Candidate),
        Some("baseline") => Some(Side::Baseline),
        Some(other) => {
            return Err(format!(
                "`side` must be `candidate` or `baseline`, not `{other}`"
            ));
        }
    };
    let rule = match string(table, "rule")? {
        None => None,
        Some("") => return Err("`rule` must be non-empty".to_owned()),
        Some(rule) => Some(rule.to_owned()),
    };
    let message = string(table, "message")?.map(str::to_owned);
    Ok(Expectation {
        code,
        severity,
        path,
        line,
        side,
        rule,
        message,
    })
}

fn reject_unknown(table: &dyn TableLike, allowed: &[&str]) -> Result<(), String> {
    for (key, _) in table.iter() {
        if !allowed.contains(&key) {
            return Err(format!("unknown key `{key}`; expected one of {allowed:?}"));
        }
    }
    Ok(())
}

/// An array of tables written either as `[[key]]` sections or as an
/// inline array of inline tables.
fn table_list<'a>(item: &'a Item, label: &str) -> Result<Vec<&'a dyn TableLike>, String> {
    match item {
        Item::ArrayOfTables(tables) => {
            Ok(tables.iter().map(|table| table as &dyn TableLike).collect())
        }
        Item::Value(Value::Array(array)) => array
            .iter()
            .map(|value| {
                value
                    .as_inline_table()
                    .map(|table| table as &dyn TableLike)
                    .ok_or_else(|| format!("`{label}` must be an array of tables"))
            })
            .collect(),
        _ => Err(format!("`{label}` must be an array of tables")),
    }
}

fn string<'a>(table: &'a dyn TableLike, key: &str) -> Result<Option<&'a str>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_str()
            .map(Some)
            .ok_or_else(|| format!("`{key}` must be a string")),
    }
}

fn path_field(table: &dyn TableLike, key: &str) -> Result<ProjectPath, String> {
    let text = string(table, key)?.ok_or_else(|| format!("`{key}` is required"))?;
    let path = ProjectPath::parse(text).map_err(|error| format!("`{key}`: {error}"))?;
    if path.as_str().is_empty() {
        return Err(format!("`{key}` must not be the project root"));
    }
    Ok(path)
}

/// Run the suite the project at `root` declares, reading everything from
/// [`Options::source`]. Never panics on project content; a suite that
/// cannot run is reported in [`TestReport::fatal`].
#[must_use]
pub fn run(root: &Path, options: &Options) -> TestReport {
    match run_inner(root, options) {
        Ok(report) => report,
        Err(message) => TestReport::fatal(message),
    }
}

fn run_inner(root: &Path, options: &Options) -> Result<TestReport, String> {
    if options.baseline.is_some() {
        return Err(
            "`bearout test` takes no comparison baseline; each fixture case decides whether the unmodified source is compared"
                .to_owned(),
        );
    }
    let opened = Opened::open(root, &options.source)?;
    let tree = opened.tree();
    let manifest_path = ProjectPath::parse(MANIFEST_NAME).expect("constant path");
    let manifest_text = tree
        .read_text(&manifest_path)
        .map_err(|error| format!("cannot read {MANIFEST_NAME} in {}: {error}", root.display()))?;
    let bootstrap = bootstrap::parse(&manifest_text)?;
    if !bootstrap.declares_fixtures() {
        return Err(format!(
            "{MANIFEST_NAME} declares no `[fixtures]`; there is nothing to test"
        ));
    }
    if !bootstrap.formatters.is_empty() && !options.allow_formatters {
        return Err(format!(
            "{MANIFEST_NAME} declares formatters ({}), which run as trusted host programs; fixture cases check with them only under --allow-formatters (library: `Options::allow_formatters`)",
            bootstrap
                .formatters
                .iter()
                .map(|formatter| format!("`{}`", formatter.name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // The suite is captured in full before any mutation is applied, and
    // every case's overlay is validated before any case is evaluated, so
    // an invalid later case stops the suite before an earlier case runs
    // the policy or an authorized formatter.
    let suite = Suite::load(tree, &bootstrap)?;
    let base = opened.shared()?;
    let overlays = suite
        .cases
        .iter()
        .map(|case| {
            // Every case starts from the same unchanged base.
            Overlay::build(Arc::clone(&base), &case.mutations)
                .map_err(|message| format!("case `{}`: {message}", case.name))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let source = opened.info();
    let mut report = TestReport {
        source: source.clone(),
        ..TestReport::default()
    };
    for (case, overlay) in suite.cases.iter().zip(&overlays) {
        let introduced = overlay.introduced();
        let universe = match &opened {
            Opened::Working(_) => hygiene::Universe::WorkingDirectory {
                root,
                introduced: &introduced,
            },
            Opened::Git(..) => hygiene::Universe::Frozen,
        };
        let baseline = case.baseline.then(|| BaselineInput {
            tree: base.as_ref(),
            label: "unmodified source".to_owned(),
            identity: source
                .as_ref()
                .map(BaselineIdentity::from)
                .unwrap_or_default(),
            info: None,
        });
        let inputs = Inputs {
            tree: overlay,
            universe,
            source: source.clone(),
            baseline,
            writer: None,
        };
        let outcome = match crate::evaluate(root, Command::Check, options, &inputs) {
            Ok(report) => report,
            Err(message) => Report::fatal(message),
        };
        report.cases.push(judge(case, &outcome));
    }
    report.finish();
    Ok(report)
}

/// Compare one case's expectation with what its candidate produced.
fn judge(case: &Case, outcome: &Report) -> CaseResult {
    let actual = if outcome.fatal.is_some() {
        Outcome::Fatal
    } else if outcome.diagnostics.is_empty() {
        Outcome::Clean
    } else {
        Outcome::Diagnostics
    };
    let mut result = CaseResult {
        name: case.name.clone(),
        file: case.file.as_str().to_owned(),
        passed: false,
        expected: case.expect,
        actual,
        missing: Vec::new(),
        unexpected: Vec::new(),
        expected_fatal: case.fatal.clone(),
        fatal: outcome.fatal.clone(),
    };
    if case.expect != actual {
        result.missing.clone_from(&case.expectations);
        result.unexpected.clone_from(&outcome.diagnostics);
        return result;
    }
    match actual {
        Outcome::Clean => result.passed = true,
        Outcome::Fatal => {
            result.passed = match (&case.fatal, &outcome.fatal) {
                (Some(expected), Some(message)) => message.contains(expected.as_str()),
                _ => true,
            };
        }
        Outcome::Diagnostics => {
            let assignment = matching::assign(&case.expectations, &outcome.diagnostics);
            result.missing = assignment
                .missing
                .iter()
                .map(|index| case.expectations[*index].clone())
                .collect();
            if case.matching == Matching::Exact {
                result.unexpected = assignment
                    .unexpected
                    .iter()
                    .map(|index| outcome.diagnostics[*index].clone())
                    .collect();
            }
            result.passed = result.missing.is_empty() && result.unexpected.is_empty();
        }
    }
    result
}
