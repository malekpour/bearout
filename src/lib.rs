// SPDX-License-Identifier: Apache-2.0

//! Bearout: a deterministic repository contract engine.
//!
//! A *contract* in Bearout is a machine-checkable agreement about the
//! resources in a repository: their envelope, shape, relations, and the
//! artifacts generated from them. The Rust kernel owns discovery, parsing,
//! graph construction, diagnostics, resource limits, path confinement, and
//! filesystem writes. Repository policy, written in Starlark and JSON
//! Schema, owns every domain rule.
//!
//! A run proceeds through fixed phases: bootstrap, discovery, parsing,
//! structural validation, graph construction, repository policy, generation
//! planning, rendering, and delivery. A resource that fails an earlier phase
//! never reaches a later one, and generation runs only on an error-free
//! project.

pub mod bootstrap;
mod envelope;
mod fs;
mod generate;
mod graph;
mod identity;
mod markdown;
mod paths;
mod policy;
pub mod report;
mod shape;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

pub use bootstrap::{Bootstrap, Limits, MANIFEST_NAME, STATE_NAME};
pub use generate::Mode;
pub use paths::ProjectPath;
pub use report::{Code, Diagnostic, Report, Severity};

use envelope::Resource;
use fs::ProjectDir;
use policy::values::Finding;
use policy::{CallOutcome, Policy};
use report::Code as C;
use shape::Shape;

/// The Bearout version stamped into provenance.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The Starlark ABI version scripts are written against. Bumped on any
/// breaking change to the entry-module functions, host constructors, or
/// view layout.
pub const ABI_VERSION: u32 = 0;

/// What a run should do after checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Check only.
    Check,
    /// Check, then generate in the given mode.
    Generate(Mode),
}

/// Run options.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// Set to `true` from another thread to cancel Starlark evaluation.
    pub cancel: Option<Arc<AtomicBool>>,
}

/// Run Bearout on the project rooted at `root`.
///
/// Never panics on project content. A failure that prevents the run from
/// completing, such as a missing or invalid bootstrap, is reported in
/// [`Report::fatal`]; everything else is a diagnostic.
#[must_use]
pub fn run(root: &Path, command: Command, options: &Options) -> Report {
    match run_inner(root, command, options) {
        Ok(report) => report,
        Err(message) => Report::fatal(message),
    }
}

/// Check a project.
#[must_use]
pub fn check(root: &Path) -> Report {
    run(root, Command::Check, &Options::default())
}

/// Check a project, then generate.
#[must_use]
pub fn generate(root: &Path, mode: Mode) -> Report {
    run(root, Command::Generate(mode), &Options::default())
}

struct Parsed {
    resource: Resource,
    valid: bool,
}

/// An empty resource used to move parsed resources out of their entries.
fn placeholder() -> Resource {
    Resource {
        path: ProjectPath::root(),
        schema: String::new(),
        id: String::new(),
        refs: Vec::new(),
        fields: Value::Null,
        field_lines: BTreeMap::new(),
        body: String::new(),
        line_count: 0,
        doc: markdown::Document::default(),
        fragments: Vec::new(),
    }
}

fn run_inner(root: &Path, command: Command, options: &Options) -> Result<Report, String> {
    let mut report = Report::default();
    let cancel = options.cancel.clone().unwrap_or_default();

    // Phase: bootstrap.
    let fs = ProjectDir::open(root)
        .map_err(|error| format!("cannot open project {}: {error}", root.display()))?;
    let manifest_path = ProjectPath::parse(MANIFEST_NAME).expect("constant path");
    let manifest_text = fs
        .read_text(&manifest_path)
        .map_err(|error| format!("cannot read {MANIFEST_NAME} in {}: {error}", root.display()))?;
    let bootstrap = bootstrap::parse(&manifest_text)?;
    for root_path in &bootstrap.resource_roots {
        if !fs.is_dir(root_path) {
            return Err(format!(
                "resource root `{root_path}` is not a directory inside the project"
            ));
        }
    }
    if !fs.is_dir(&bootstrap.rules_root) {
        return Err(format!(
            "rules root `{}` is not a directory inside the project",
            bootstrap.rules_root
        ));
    }
    if let Some(templates) = &bootstrap.templates_root
        && !fs.is_dir(templates)
    {
        return Err(format!(
            "templates root `{templates}` is not a directory inside the project"
        ));
    }
    if !fs.is_file(&bootstrap.entry) {
        return Err(format!(
            "entry module `{}` is not a file inside the project",
            bootstrap.entry
        ));
    }

    // Phase: discovery.
    let files = discover(&fs, &bootstrap)?;
    report.resources = files.len();

    // Phase: parsing.
    let mut parsed = parse_all(&fs, &bootstrap, &files, &mut report);

    // Repository policy is loaded before structural validation because the
    // entry module registers the schemas and shapes that validation needs.
    let mut policy_diagnostics = Vec::new();
    let policy = policy::load(&fs, &bootstrap, cancel, &mut policy_diagnostics);
    report.extend(policy_diagnostics);
    let Some(policy) = policy else {
        report.finish();
        return Ok(report);
    };

    // Phase: structural validation.
    let shapes = load_shapes(&fs, &policy, &mut report);
    validate_structure(&mut parsed, &policy, &shapes, &mut report);

    // Phase: graph construction. Every parsed resource defines identifiers;
    // only structurally valid ones have their relations and links checked.
    let resources: Vec<Resource> = parsed
        .iter_mut()
        .map(|entry| std::mem::replace(&mut entry.resource, placeholder()))
        .collect();
    let validity: Vec<bool> = parsed.iter().map(|entry| entry.valid).collect();
    let mut graph_diagnostics = Vec::new();
    let graph = graph::build(&fs, &resources, &validity, &shapes, &mut graph_diagnostics);
    report.extend(graph_diagnostics);
    let valid_indexes: Vec<usize> = validity
        .iter()
        .enumerate()
        .filter(|(_, valid)| **valid)
        .map(|(index, _)| index)
        .collect();
    let valid: Vec<&Resource> = valid_indexes
        .iter()
        .map(|index| &resources[*index])
        .collect();

    // Phase: repository policy.
    let views = policy::views::Views::build(&resources, &valid_indexes, &graph)
        .map_err(|error| format!("cannot build script views: {error}"))?;
    let line_counts: BTreeMap<&str, u32> = valid
        .iter()
        .map(|resource| (resource.id.as_str(), resource.line_count))
        .collect();
    let paths_by_id: BTreeMap<&str, &str> = valid
        .iter()
        .map(|resource| (resource.id.as_str(), resource.path.as_str()))
        .collect();
    run_validators(&valid, &views, &policy, &line_counts, &mut report);
    if report.errors() == 0 {
        run_checks(&views, &policy, &line_counts, &paths_by_id, &mut report);
    }

    // Phases: generation planning, rendering, delivery.
    if let Command::Generate(mode) = command
        && report.errors() == 0
    {
        let planned = plan(&views, &policy, &mut report);
        if report.errors() == 0 {
            let mut diagnostics = Vec::new();
            let outcome = generate::run(&fs, &bootstrap, planned, mode, VERSION, &mut diagnostics);
            report.extend(diagnostics);
            report.outputs = outcome.outputs;
            report.max_fuel = outcome.max_fuel;
            report.max_output_bytes = outcome.max_output_bytes;
        }
    }

    report.max_ticks = policy.max_ticks.get();
    report.max_heap_bytes = policy.max_heap_bytes.get();
    report.finish();
    Ok(report)
}

fn discover(fs: &ProjectDir, bootstrap: &Bootstrap) -> Result<Vec<ProjectPath>, String> {
    let mut files = Vec::new();
    for root in &bootstrap.resource_roots {
        let found = fs
            .walk(root)
            .map_err(|error| format!("cannot walk resource root `{root}`: {error}"))?;
        files.extend(
            found
                .into_iter()
                .filter(|path| matches!(path.extension(), Some("md" | "toml"))),
        );
    }
    files.sort();
    files.dedup();
    if files.len() > bootstrap.limits.resources {
        return Err(format!(
            "{} resources exceed `limits.resources` = {}",
            files.len(),
            bootstrap.limits.resources
        ));
    }
    Ok(files)
}

fn parse_all(
    fs: &ProjectDir,
    bootstrap: &Bootstrap,
    files: &[ProjectPath],
    report: &mut Report,
) -> Vec<Parsed> {
    let mut parsed = Vec::new();
    for path in files {
        match fs.file_len(path) {
            Ok(len) if len > bootstrap.limits.resource_bytes => {
                report.push(Diagnostic::new(
                    C::Unreadable,
                    path.as_str(),
                    format!(
                        "resource is {len} bytes, above `limits.resource_bytes` = {}",
                        bootstrap.limits.resource_bytes
                    ),
                ));
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                report.push(Diagnostic::new(
                    C::Unreadable,
                    path.as_str(),
                    format!("cannot read resource: {error}"),
                ));
                continue;
            }
        }
        let bytes = match fs.read(path) {
            Ok(bytes) => bytes,
            Err(error) => {
                report.push(Diagnostic::new(
                    C::Unreadable,
                    path.as_str(),
                    format!("cannot read resource: {error}"),
                ));
                continue;
            }
        };
        let mut diagnostics = Vec::new();
        match envelope::parse(path, &bytes, &mut diagnostics) {
            Ok(resource) => {
                let valid = diagnostics.is_empty();
                report.extend(diagnostics);
                parsed.push(Parsed { resource, valid });
            }
            Err(diagnostic) => {
                report.push(diagnostic);
                report.extend(diagnostics);
            }
        }
    }
    parsed
}

fn load_shapes(fs: &ProjectDir, policy: &Policy, report: &mut Report) -> BTreeMap<String, Shape> {
    let mut shapes = BTreeMap::new();
    for (id, schema) in &policy.schemas {
        let Some(path) = &schema.shape else {
            continue;
        };
        let text = match fs.read_text(path) {
            Ok(text) => text,
            Err(error) => {
                report.push(Diagnostic::new(
                    C::Unreadable,
                    path.as_str(),
                    format!("cannot read shape for `{id}`: {error}"),
                ));
                continue;
            }
        };
        match shape::parse(&text) {
            Ok(shape) => {
                shapes.insert(id.clone(), shape);
            }
            Err(error) => report.push(Diagnostic::new(
                C::ShapeInvalid,
                path.as_str(),
                format!("shape for `{id}`: {error}"),
            )),
        }
    }
    shapes
}

/// Validate every parsed resource against its registered schema's shape:
/// fields, required sections, and fragments. A resource whose schema is
/// unregistered or whose shape failed to load is not valid.
fn validate_structure(
    parsed: &mut [Parsed],
    policy: &Policy,
    shapes: &BTreeMap<String, Shape>,
    report: &mut Report,
) {
    let unloadable: BTreeSet<&str> = policy
        .schemas
        .iter()
        .filter(|(id, schema)| schema.shape.is_some() && !shapes.contains_key(*id))
        .map(|(id, _)| id.as_str())
        .collect();
    for entry in parsed.iter_mut() {
        let resource = &entry.resource;
        let path = resource.path.as_str();
        if !policy.schemas.contains_key(&resource.schema) {
            report.push(Diagnostic::new(
                C::SchemaIdentity,
                path,
                format!(
                    "schema `{}` is not registered by the policy",
                    resource.schema
                ),
            ));
            entry.valid = false;
            continue;
        }
        if unloadable.contains(resource.schema.as_str()) {
            entry.valid = false;
            continue;
        }
        let Some(shape) = shapes.get(&resource.schema) else {
            continue;
        };
        let before = report.diagnostics.len();
        for violation in shape.check(&resource.fields) {
            let key = if violation.location.is_empty() {
                violation.unexpected.as_deref()
            } else {
                violation.location.split('.').next()
            };
            let line = key.and_then(|key| resource.field_lines.get(key)).copied();
            report
                .push(Diagnostic::new(C::ShapeViolation, path, describe(&violation)).at_line(line));
        }
        for title in &shape.sections {
            if !resource
                .doc
                .sections
                .iter()
                .any(|section| &section.title == title)
            {
                report.push(Diagnostic::new(
                    C::MissingSection,
                    path,
                    format!("body must contain a `{title}` section"),
                ));
            }
        }
        for fragment in &resource.fragments {
            match shape.check_fragment(&fragment.kind, &fragment.fields) {
                None => report.push(
                    Diagnostic::new(
                        C::FragmentMalformed,
                        path,
                        format!(
                            "fragment kind `{}` is not declared by schema `{}`",
                            fragment.kind, resource.schema
                        ),
                    )
                    .at_line(Some(fragment.line)),
                ),
                Some(violations) => {
                    for violation in violations {
                        report.push(
                            Diagnostic::new(
                                C::ShapeViolation,
                                path,
                                format!("fragment `{}`: {}", fragment.id, describe(&violation)),
                            )
                            .at_line(Some(fragment.line)),
                        );
                    }
                }
            }
        }
        if report.diagnostics.len() > before {
            entry.valid = false;
        }
    }
}

fn describe(violation: &shape::Violation) -> String {
    if violation.location.is_empty() {
        violation.message.clone()
    } else {
        format!("`{}`: {}", violation.location, violation.message)
    }
}

/// Turn a finding into a diagnostic, checking its target against the ABI.
fn admit(
    finding: &Finding,
    label: &str,
    script: &str,
    own_resource: Option<(&str, &str)>,
    line_counts: &BTreeMap<&str, u32>,
    paths_by_id: &BTreeMap<&str, &str>,
) -> Diagnostic {
    let target = match (&finding.resource, own_resource) {
        (None, Some((_, path))) => Ok(path),
        (None, None) => Err("a check finding must name a `resource`".to_owned()),
        (Some(id), Some((own_id, path))) if id == own_id => Ok(path),
        (Some(id), Some((own_id, _))) => Err(format!(
            "a validator may only report its own resource `{own_id}`, not `{id}`"
        )),
        (Some(id), None) => paths_by_id
            .get(id.as_str())
            .copied()
            .ok_or_else(|| format!("finding names unknown resource `{id}`")),
    };
    let target = match target {
        Ok(target) => target,
        Err(error) => return Diagnostic::new(C::ScriptResult, script, format!("{label} {error}")),
    };
    if let Some(line) = finding.line {
        let id = finding
            .resource
            .as_deref()
            .or(own_resource.map(|(id, _)| id))
            .unwrap_or_default();
        let count = line_counts.get(id).copied().unwrap_or(0);
        if line > count {
            return Diagnostic::new(
                C::ScriptResult,
                script,
                format!("{label} finding line {line} is beyond the {count} line(s) of `{id}`"),
            );
        }
    }
    let code = if finding.is_error {
        C::PolicyError
    } else {
        C::PolicyWarning
    };
    Diagnostic::new(code, target, format!("{label}: {}", finding.message))
        .at_line(finding.line)
        .with_rule(finding.rule.clone())
}

fn printed(outcome: &CallOutcome<impl Sized>, script: &str, label: &str) -> Vec<Diagnostic> {
    outcome
        .printed
        .iter()
        .map(|line| Diagnostic::new(C::ScriptOutput, script, format!("{label} printed: {line}")))
        .collect()
}

fn run_validators(
    valid: &[&Resource],
    views: &policy::views::Views,
    policy: &Policy,
    line_counts: &BTreeMap<&str, u32>,
    report: &mut Report,
) {
    let empty = BTreeMap::new();
    let script = policy.entry.as_str();
    for (resource, view) in valid.iter().zip(&views.resources) {
        let Some(outcome) = policy.validate(&resource.schema, view) else {
            continue;
        };
        let label = format!("schema `{}` validate", resource.schema);
        report.extend(printed(&outcome, script, &label));
        match outcome.result {
            Ok(findings) => {
                for finding in &findings {
                    report.push(admit(
                        finding,
                        &label,
                        script,
                        Some((&resource.id, resource.path.as_str())),
                        line_counts,
                        &empty,
                    ));
                }
            }
            Err(error) => report.push(policy::failure_diagnostic(script, &label, &error)),
        }
    }
}

fn run_checks(
    views: &policy::views::Views,
    policy: &Policy,
    line_counts: &BTreeMap<&str, u32>,
    paths_by_id: &BTreeMap<&str, &str>,
    report: &mut Report,
) {
    let script = policy.entry.as_str();
    for (name, callback) in &policy.checks {
        let label = format!("check `{name}`");
        let outcome = policy.check(callback, &views.project);
        report.extend(printed(&outcome, script, &label));
        match outcome.result {
            Ok(findings) => {
                for finding in &findings {
                    report.push(admit(
                        finding,
                        &label,
                        script,
                        None,
                        line_counts,
                        paths_by_id,
                    ));
                }
            }
            Err(error) => report.push(policy::failure_diagnostic(script, &label, &error)),
        }
    }
}

fn plan(
    views: &policy::views::Views,
    policy: &Policy,
    report: &mut Report,
) -> Vec<generate::Planned> {
    let script = policy.entry.as_str();
    let mut planned = Vec::new();
    for (name, callback) in &policy.generators {
        let label = format!("generator `{name}`");
        let outcome = policy.plan(callback, &views.project);
        report.extend(printed(&outcome, script, &label));
        match outcome.result {
            Ok(outputs) => {
                for output in outputs {
                    let context: Value =
                        serde_json::from_str(&output.context).unwrap_or(Value::Null);
                    planned.push(generate::Planned {
                        generator: name.clone(),
                        script: script.to_owned(),
                        template: ProjectPath::parse(&output.template)
                            .expect("validated at construction"),
                        output: ProjectPath::parse(&output.path)
                            .expect("validated at construction"),
                        context,
                    });
                }
            }
            Err(error) => report.push(policy::failure_diagnostic(script, &label, &error)),
        }
    }
    planned
}
