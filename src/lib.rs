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
//!
//! Every phase before delivery reads the project through one read-only
//! tree, selected by [`Source`] before anything is read: the live working
//! directory, the Git index as captured at the start of the run, or one
//! resolved Git revision. Delivery needs the working directory's write
//! capability, which the Git-backed sources do not have.

pub mod bootstrap;
mod document;
mod envelope;
mod fs;
mod generate;
mod git;
mod graph;
mod identity;
mod markdown;
mod paths;
mod policy;
mod references;
pub mod report;
mod shape;
mod tree;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

pub use bootstrap::{Bootstrap, Limits, MANIFEST_NAME, STATE_NAME};
pub use generate::Mode;
pub use paths::ProjectPath;
pub use report::{Code, Diagnostic, Report, Severity, SourceInfo};

use envelope::Resource;
use fs::WorkingDir;
use git::GitTree;
use policy::values::Finding;
use policy::{CallOutcome, Policy};
use report::Code as C;
use shape::Shape;
use tree::ReadTree;

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

/// Where a run reads the project from.
///
/// **Experimental.** The Git-backed sources are new and their surface may
/// change. They require the `git` executable and are read-only: checking
/// and generation checking work against them, writing generation does not.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Source {
    /// The live working directory at the project root, read through a
    /// filesystem capability. It may change concurrently with the run;
    /// Bearout makes no snapshot of it. The only source generation can
    /// write to.
    #[default]
    WorkingDirectory,
    /// The Git index of the repository owning the project root, as it
    /// stands when the run starts: the tree a commit would record. Staged
    /// additions and modifications are present; unstaged modifications,
    /// untracked files, staged deletions, and intent-to-add entries are
    /// absent; a staged rename appears only at its destination. An unmerged
    /// index is a fatal outcome.
    Index,
    /// One Git revision of the repository owning the project root: any
    /// commit-ish or tree-ish Git resolves. The name is resolved exactly
    /// once and the resolved tree is used for the whole run, even if a
    /// branch or tag moves meanwhile. An unknown name is a fatal outcome.
    Revision(String),
}

/// Run options.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// Set to `true` from another thread to cancel Starlark evaluation.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Where to read the project from. The default is the working
    /// directory.
    pub source: Source,
}

/// Run Bearout on the project rooted at `root`, reading it from
/// [`Options::source`].
///
/// Never panics on project content. A failure that prevents the run from
/// completing, such as a missing or invalid bootstrap, a source that cannot
/// be opened, a revision that does not resolve, or writing generation
/// requested against a Git-backed source, is reported in [`Report::fatal`];
/// everything else is a diagnostic.
#[must_use]
pub fn run(root: &Path, command: Command, options: &Options) -> Report {
    match run_inner(root, command, options) {
        Ok(report) => report,
        Err(message) => Report::fatal(message),
    }
}

/// Check a project in its working directory.
#[must_use]
pub fn check(root: &Path) -> Report {
    run(root, Command::Check, &Options::default())
}

/// Check a project in its working directory, then generate.
#[must_use]
pub fn generate(root: &Path, mode: Mode) -> Report {
    run(root, Command::Generate(mode), &Options::default())
}

/// The tree a run reads, opened before anything else is read.
enum Opened {
    Working(WorkingDir),
    Git(GitTree, SourceInfo),
}

impl Opened {
    fn open(root: &Path, source: &Source) -> Result<Self, String> {
        match source {
            Source::WorkingDirectory => WorkingDir::open(root)
                .map(Self::Working)
                .map_err(|error| format!("cannot open project {}: {error}", root.display())),
            Source::Index => GitTree::index(root)
                .map(|tree| {
                    let info = SourceInfo {
                        kind: "index".to_owned(),
                        revision: None,
                        tree: None,
                        digest: tree.digest().to_owned(),
                    };
                    Self::Git(tree, info)
                })
                .map_err(|error| format!("cannot read the Git index: {error}")),
            Source::Revision(name) => GitTree::revision(root, name)
                .map(|(tree, id)| {
                    let info = SourceInfo {
                        kind: "revision".to_owned(),
                        revision: Some(name.clone()),
                        tree: Some(id.to_string()),
                        digest: tree.digest().to_owned(),
                    };
                    Self::Git(tree, info)
                })
                .map_err(|error| format!("cannot read Git revision: {error}")),
        }
    }

    fn tree(&self) -> &dyn ReadTree {
        match self {
            Self::Working(working) => working,
            Self::Git(tree, _) => tree,
        }
    }

    fn info(&self) -> Option<SourceInfo> {
        match self {
            Self::Working(_) => None,
            Self::Git(_, info) => Some(info.clone()),
        }
    }
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

    if command == Command::Generate(Mode::Write) && options.source != Source::WorkingDirectory {
        return Err(
            "generation writes to the working directory; the index and revision sources are read-only and support checking only"
                .to_owned(),
        );
    }

    // Phase: bootstrap. The source is opened before anything is read, so
    // every input of the run, the bootstrap included, comes from one tree.
    let opened = Opened::open(root, &options.source)?;
    let tree = opened.tree();
    let manifest_path = ProjectPath::parse(MANIFEST_NAME).expect("constant path");
    let manifest_text = tree
        .read_text(&manifest_path)
        .map_err(|error| format!("cannot read {MANIFEST_NAME} in {}: {error}", root.display()))?;
    let bootstrap = bootstrap::parse(&manifest_text)?;
    for root_path in &bootstrap.resource_roots {
        if !tree.is_dir(root_path) {
            return Err(format!(
                "resource root `{root_path}` is not a directory inside the project"
            ));
        }
    }
    if !tree.is_dir(&bootstrap.rules_root) {
        return Err(format!(
            "rules root `{}` is not a directory inside the project",
            bootstrap.rules_root
        ));
    }
    if let Some(templates) = &bootstrap.templates_root
        && !tree.is_dir(templates)
    {
        return Err(format!(
            "templates root `{templates}` is not a directory inside the project"
        ));
    }
    if !tree.is_file(&bootstrap.entry) {
        return Err(format!(
            "entry module `{}` is not a file inside the project",
            bootstrap.entry
        ));
    }

    // Phase: discovery. Resources first; a path they claim is never also a
    // schema-less document.
    let files = discover(tree, &bootstrap)?;
    report.resources = files.len();
    let document_paths = document::discover(tree, &bootstrap, &files)?;
    report.documents = document_paths.len();

    // Phase: parsing.
    let mut parsed = parse_all(tree, &bootstrap, &files, &mut report);
    let documents = read_documents(tree, &bootstrap, &document_paths, &mut report);
    // Repository policy receives the document views in the next phase; until
    // then the view is built only to keep the model exercised.
    let _ = documents.iter().map(document::Document::view).count();

    // Repository policy is loaded before structural validation because the
    // entry module registers the schemas and shapes that validation needs.
    let mut policy_diagnostics = Vec::new();
    let policy = policy::load(tree, &bootstrap, cancel, &mut policy_diagnostics);
    report.extend(policy_diagnostics);
    let Some(policy) = policy else {
        report.source = opened.info();
        report.finish();
        return Ok(report);
    };

    // Phase: structural validation.
    let shapes = load_shapes(tree, &policy, &mut report);
    validate_structure(&mut parsed, &policy, &shapes, &mut report);

    // Phase: graph construction. Every parsed resource defines identifiers;
    // only structurally valid ones have their relations checked. Markdown
    // references of valid resources and parsed documents are checked
    // together against the tree and the discovered Markdown set.
    let resources: Vec<Resource> = parsed
        .iter_mut()
        .map(|entry| std::mem::replace(&mut entry.resource, placeholder()))
        .collect();
    let validity: Vec<bool> = parsed.iter().map(|entry| entry.valid).collect();
    let mut graph_diagnostics = Vec::new();
    let graph = graph::build(&resources, &validity, &shapes, &mut graph_diagnostics);
    references::check(
        tree,
        &resources,
        &validity,
        &files,
        &documents,
        &document_paths,
        &mut graph_diagnostics,
    );
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
            let delivery = match (mode, &opened) {
                (Mode::Check, _) => generate::Delivery::Check,
                (Mode::Write, Opened::Working(working)) => {
                    generate::Delivery::Write(working.writer())
                }
                (Mode::Write, Opened::Git(..)) => unreachable!("rejected before the tree opened"),
            };
            let outcome = generate::run(
                tree,
                &bootstrap,
                planned,
                delivery,
                VERSION,
                &mut diagnostics,
            );
            report.extend(diagnostics);
            report.outputs = outcome.outputs;
            report.max_fuel = outcome.max_fuel;
            report.max_output_bytes = outcome.max_output_bytes;
        }
    }

    report.max_ticks = policy.max_ticks.get();
    report.max_heap_bytes = policy.max_heap_bytes.get();
    report.source = opened.info();
    report.finish();
    Ok(report)
}

fn discover(tree: &dyn ReadTree, bootstrap: &Bootstrap) -> Result<Vec<ProjectPath>, String> {
    let mut files = Vec::new();
    for root in &bootstrap.resource_roots {
        let found = tree
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
    tree: &dyn ReadTree,
    bootstrap: &Bootstrap,
    files: &[ProjectPath],
    report: &mut Report,
) -> Vec<Parsed> {
    let mut parsed = Vec::new();
    for path in files {
        match tree.file_len(path) {
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
        let bytes = match tree.read(path) {
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

/// Read and parse every registered shape. Like rule modules, shapes are
/// never reached through a symbolic link.
fn read_documents(
    tree: &dyn ReadTree,
    bootstrap: &Bootstrap,
    paths: &[ProjectPath],
    report: &mut Report,
) -> Vec<document::Document> {
    let mut documents = Vec::new();
    for path in paths {
        match document::read(tree, bootstrap, path) {
            Ok(document) => documents.push(document),
            Err(diagnostic) => report.push(diagnostic),
        }
    }
    documents
}

fn load_shapes(
    tree: &dyn ReadTree,
    policy: &Policy,
    report: &mut Report,
) -> BTreeMap<String, Shape> {
    let mut shapes = BTreeMap::new();
    for (id, schema) in &policy.schemas {
        let Some(path) = &schema.shape else {
            continue;
        };
        match tree.symlink_component(path) {
            Ok(None) => {}
            Ok(Some(link)) => {
                report.push(Diagnostic::new(
                    C::Unreadable,
                    path.as_str(),
                    format!(
                        "cannot read shape for `{id}`: `{link}` is a symbolic link; shapes must not be reached through links"
                    ),
                ));
                continue;
            }
            Err(error) => {
                report.push(Diagnostic::new(
                    C::Unreadable,
                    path.as_str(),
                    format!("cannot read shape for `{id}`: cannot inspect shape path: {error}"),
                ));
                continue;
            }
        }
        let text = match tree.read_text(path) {
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
