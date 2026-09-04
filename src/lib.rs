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
mod changes;
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
mod projection;
mod references;
pub mod report;
mod shape;
mod tree;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde_json::Value;

pub use bootstrap::{Bootstrap, Limits, MANIFEST_NAME, STATE_NAME};
pub use generate::Mode;
pub use paths::ProjectPath;
pub use report::{Code, Diagnostic, Report, Severity, Side, SourceInfo};

use envelope::Resource;
use fs::WorkingDir;
use git::GitTree;
use policy::values::Finding;
use policy::{CallOutcome, Policy};
use projection::Projection;
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
    /// An explicit Git revision to compare the project against, from the
    /// same repository. Nothing is inferred: no `HEAD`, parent, merge base,
    /// or default branch. The baseline is read-only historical evidence;
    /// only the candidate's policy runs. **Experimental.**
    pub baseline: Option<String>,
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

/// The comparison baseline: one resolved revision and its identity.
struct Baseline {
    tree: GitTree,
    info: SourceInfo,
}

impl Baseline {
    fn open(root: &Path, revision: &str) -> Result<Self, String> {
        let (tree, id) = GitTree::baseline(root, revision)
            .map_err(|error| format!("cannot read the baseline: {error}"))?;
        let info = SourceInfo {
            kind: "revision".to_owned(),
            revision: Some(revision.to_owned()),
            tree: Some(id.to_string()),
            digest: tree.digest().to_owned(),
        };
        Ok(Self { tree, info })
    }
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

fn run_inner(root: &Path, command: Command, options: &Options) -> Result<Report, String> {
    let mut report = Report::default();
    let cancel = options.cancel.clone().unwrap_or_default();

    if command == Command::Generate(Mode::Write) && options.source != Source::WorkingDirectory {
        return Err(
            "generation writes to the working directory; the index and revision sources are read-only and support checking only"
                .to_owned(),
        );
    }

    // Phase: bootstrap. The sources are opened before anything is read, so
    // every input of the run, the bootstrap included, comes from one tree,
    // and the baseline is pinned before the candidate is examined.
    let opened = Opened::open(root, &options.source)?;
    let baseline = match &options.baseline {
        Some(revision) => Some(Baseline::open(root, revision)?),
        None => None,
    };
    let tree = opened.tree();
    let manifest_path = ProjectPath::parse(MANIFEST_NAME).expect("constant path");
    let manifest_text = tree
        .read_text(&manifest_path)
        .map_err(|error| format!("cannot read {MANIFEST_NAME} in {}: {error}", root.display()))?;
    let bootstrap = bootstrap::parse(&manifest_text)?;
    let manifest_entry =
        changes::SurfaceEntry::new(changes::Classification::Manifest, manifest_text.as_bytes());
    projection::check_resource_roots(tree, &bootstrap)?;
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

    // Phases: discovery and parsing. Resources first; a path they claim is
    // never also a schema-less document.
    let mut gathered_diagnostics = Vec::new();
    let mut gathered = projection::gather(
        tree,
        &bootstrap,
        &bootstrap.limits,
        &mut gathered_diagnostics,
    )?;
    gathered.surface.insert(manifest_path, manifest_entry);
    report.extend(gathered_diagnostics);
    report.resources = gathered.files.len();
    report.documents = gathered.document_paths.len();

    // Repository policy is loaded before structural validation because the
    // entry module registers the schemas and shapes that validation needs.
    let mut policy_diagnostics = Vec::new();
    let policy = policy::load(tree, &bootstrap, cancel, &mut policy_diagnostics);
    report.extend(policy_diagnostics);
    let Some(policy) = policy else {
        report.source = opened.info();
        report.baseline = baseline.as_ref().map(|baseline| baseline.info.clone());
        report.finish();
        return Ok(report);
    };

    // Phases: structural validation and graph construction. Every parsed
    // resource defines identifiers; only structurally valid ones have their
    // relations checked. Markdown references of valid resources and parsed
    // documents are checked together against the tree and the discovered
    // Markdown set.
    let shapes = load_shapes(tree, &policy, &mut report);
    let mut settled_diagnostics = Vec::new();
    let candidate = projection::settle(
        tree,
        gathered,
        &policy,
        &shapes,
        Side::Candidate,
        &mut settled_diagnostics,
    );
    report.extend(settled_diagnostics);

    // The baseline is projected through the same steps with the candidate's
    // limits, policy, and shapes, and its own bootstrap selecting its paths.
    let historical = match &baseline {
        Some(baseline) => {
            let mut baseline_diagnostics = Vec::new();
            let projection = projection::baseline(
                &baseline.tree,
                baseline.info.revision.as_deref().unwrap_or_default(),
                &bootstrap.limits,
                &policy,
                &shapes,
                &mut baseline_diagnostics,
            )?;
            report.extend(baseline_diagnostics);
            Some(projection)
        }
        None => None,
    };

    let Projection {
        resources,
        documents,
        graph,
        ..
    } = &candidate;
    let valid_indexes = candidate.valid_indexes();
    let valid: Vec<&Resource> = valid_indexes
        .iter()
        .map(|index| &resources[*index])
        .collect();

    // Phase: repository policy. Change facts come from the surfaces both
    // sides recorded while reading, never from a second read.
    let comparison = baseline
        .as_ref()
        .zip(historical.as_ref())
        .map(|(baseline, projection)| {
            let changes = changes::between(&projection.surface, &candidate.surface);
            (
                policy::views::SideView {
                    info: Some(&baseline.info),
                    resources: &projection.resources,
                    indexes: projection.valid_indexes(),
                    graph: &projection.graph,
                    documents: &projection.documents,
                },
                changes,
            )
        });
    let views = policy::views::Views::build(
        policy::views::SideView {
            info: None,
            resources,
            indexes: valid_indexes.clone(),
            graph,
            documents,
        },
        comparison,
    )
    .map_err(|error| format!("cannot build script views: {error}"))?;
    let line_counts: BTreeMap<&str, u32> = valid
        .iter()
        .map(|resource| (resource.id.as_str(), resource.line_count))
        .collect();
    let paths_by_id: BTreeMap<&str, &str> = valid
        .iter()
        .map(|resource| (resource.id.as_str(), resource.path.as_str()))
        .collect();
    let targets = Targets {
        line_counts: &line_counts,
        paths_by_id: &paths_by_id,
        document_lines: &documents
            .iter()
            .map(|document| (document.path.as_str(), document.line_count))
            .collect(),
    };
    run_validators(&valid, &views, &policy, &targets, &mut report);
    if report.errors() == 0 {
        run_checks(&views, &policy, &targets, &mut report);
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
    report.baseline = baseline.map(|baseline| {
        // The baseline tree is historical evidence only; nothing is ever
        // written to it, and dropping it here ends its life with the run.
        drop(baseline.tree);
        baseline.info
    });
    report.finish();
    Ok(report)
}

/// Read and parse every registered shape. Like rule modules, shapes are
/// never reached through a symbolic link.
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

/// What a finding may be admitted against: the structurally valid
/// resources by id and the parsed schema-less documents by path.
struct Targets<'a> {
    line_counts: &'a BTreeMap<&'a str, u32>,
    paths_by_id: &'a BTreeMap<&'a str, &'a str>,
    document_lines: &'a BTreeMap<&'a str, u32>,
}

/// Turn a finding into a diagnostic, checking its target against the ABI.
/// A validator may report only its own resource; a check must name a known
/// resource or a discovered document, and a line within it.
fn admit(
    finding: &Finding,
    label: &str,
    script: &str,
    own_resource: Option<(&str, &str)>,
    targets: &Targets<'_>,
) -> Diagnostic {
    // (diagnostic path, name used in the line message, line count)
    let target: Result<(&str, &str, u32), String> =
        match (&finding.resource, &finding.path, own_resource) {
            (_, Some(path), Some((own_id, _))) => Err(format!(
                "a validator may only report its own resource `{own_id}`, not document `{path}`"
            )),
            (_, Some(path), None) => targets
                .document_lines
                .get(path.as_str())
                .map(|count| (path.as_str(), path.as_str(), *count))
                .ok_or_else(|| format!("finding names unknown document `{path}`")),
            (None, None, Some((id, path))) => Ok((path, id, resource_lines(targets, id))),
            (None, None, None) => {
                Err("a check finding must name a `resource` or a `path`".to_owned())
            }
            (Some(id), None, Some((own_id, path))) if id == own_id => {
                Ok((path, own_id, resource_lines(targets, own_id)))
            }
            (Some(id), None, Some((own_id, _))) => Err(format!(
                "a validator may only report its own resource `{own_id}`, not `{id}`"
            )),
            (Some(id), None, None) => targets
                .paths_by_id
                .get(id.as_str())
                .map(|path| (*path, id.as_str(), resource_lines(targets, id)))
                .ok_or_else(|| format!("finding names unknown resource `{id}`")),
        };
    let (target, name, count) = match target {
        Ok(target) => target,
        Err(error) => return Diagnostic::new(C::ScriptResult, script, format!("{label} {error}")),
    };
    if let Some(line) = finding.line
        && line > count
    {
        return Diagnostic::new(
            C::ScriptResult,
            script,
            format!("{label} finding line {line} is beyond the {count} line(s) of `{name}`"),
        );
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

fn resource_lines(targets: &Targets<'_>, id: &str) -> u32 {
    targets.line_counts.get(id).copied().unwrap_or(0)
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
    targets: &Targets<'_>,
    report: &mut Report,
) {
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
                        targets,
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
    targets: &Targets<'_>,
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
                    report.push(admit(finding, &label, script, None, targets));
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
