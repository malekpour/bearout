// SPDX-License-Identifier: Apache-2.0

//! One side's contract projection: discovery, parsing, structural
//! validation, and the identifier graph over one tree. The candidate and
//! the comparison baseline go through the same steps; what differs is the
//! authority. Each side's own bootstrap decides which paths it classified
//! as resources and schema-less documents, while the candidate's limits
//! bound both sides and the candidate's registered schemas and shapes
//! interpret both. The baseline bootstrap is passive historical data: it
//! grants nothing, and no baseline rule module, generator, or template is
//! ever loaded or executed.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::bootstrap::{Bootstrap, Limits, MANIFEST_NAME};
use crate::changes::{Classification, Surface, SurfaceEntry};
use crate::document::{self, Document};
use crate::envelope::{self, Resource};
use crate::graph::{self, Graph};
use crate::paths::ProjectPath;
use crate::policy::Policy;
use crate::report::Code as C;
use crate::report::{Diagnostic, Side};
use crate::shape::{self, Shape};
use crate::tree::ReadTree;
use crate::{markdown, references};

/// Everything read from one tree before policy interprets it.
pub struct Gathered {
    pub files: Vec<ProjectPath>,
    pub parsed: Vec<Parsed>,
    pub documents: Vec<Document>,
    pub document_paths: Vec<ProjectPath>,
    /// Every file whose bytes were read, with the digest of those bytes.
    pub surface: Surface,
}

/// One side after structural validation and graph construction.
pub struct Projection {
    /// Every resource that parsed, in path order.
    pub resources: Vec<Resource>,
    /// Whether each resource is structurally valid.
    pub validity: Vec<bool>,
    /// Every document that was read, in path order.
    pub documents: Vec<Document>,
    pub graph: Graph,
    /// Every file whose bytes were read, with the digest of those bytes.
    pub surface: Surface,
}

impl Projection {
    /// Positions of the structurally valid resources.
    #[must_use]
    pub fn valid_indexes(&self) -> Vec<usize> {
        self.validity
            .iter()
            .enumerate()
            .filter(|(_, valid)| **valid)
            .map(|(index, _)| index)
            .collect()
    }
}

/// Every resource root must be a directory of the tree.
pub fn check_resource_roots(tree: &dyn ReadTree, bootstrap: &Bootstrap) -> Result<(), String> {
    for root_path in &bootstrap.resource_roots {
        if !tree.is_dir(root_path) {
            return Err(format!(
                "resource root `{root_path}` is not a directory inside the project"
            ));
        }
    }
    Ok(())
}

/// Discover and parse one side. `bootstrap` selects the paths; `limits`
/// are always the candidate's. A discovery failure is fatal.
pub fn gather(
    tree: &dyn ReadTree,
    bootstrap: &Bootstrap,
    limits: &Limits,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Gathered, String> {
    let files = discover(tree, bootstrap, limits)?;
    let document_paths = document::discover(tree, bootstrap, limits, &files)?;
    let mut surface = Surface::new();
    let parsed = parse_all(tree, limits, &files, &mut surface, diagnostics);
    let documents = read_documents(tree, limits, &document_paths, diagnostics);
    for document in &documents {
        surface.insert(
            document.path.clone(),
            SurfaceEntry {
                classification: Classification::Document,
                digest: document.digest.clone(),
                bytes: document.bytes,
            },
        );
    }
    Ok(Gathered {
        files,
        parsed,
        documents,
        document_paths,
        surface,
    })
}

/// Validate structure with the candidate's policy and shapes, then build
/// the identifier graph, on either side: duplicate identifiers and typed
/// relations are checked for the baseline too, since policy pairs records
/// through them. Markdown links, images, and anchors are checked for the
/// candidate only; the baseline's were checked when its revision was.
pub fn settle(
    tree: &dyn ReadTree,
    gathered: Gathered,
    policy: &Policy,
    shapes: &BTreeMap<String, Shape>,
    side: Side,
    diagnostics: &mut Vec<Diagnostic>,
) -> Projection {
    let Gathered {
        files,
        mut parsed,
        documents,
        document_paths,
        surface,
    } = gathered;
    validate_structure(&mut parsed, policy, shapes, side, diagnostics);
    let resources: Vec<Resource> = parsed
        .iter_mut()
        .map(|entry| std::mem::replace(&mut entry.resource, placeholder()))
        .collect();
    let validity: Vec<bool> = parsed.iter().map(|entry| entry.valid).collect();
    let graph = graph::build(&resources, &validity, shapes, diagnostics);
    if side == Side::Candidate {
        references::check(
            tree,
            &resources,
            &validity,
            &files,
            &documents,
            &document_paths,
            diagnostics,
        );
    }
    Projection {
        resources,
        validity,
        documents,
        graph,
        surface,
    }
}

/// Project the comparison baseline. Its bootstrap, when present, is parsed
/// as passive historical data that only selects paths; a revision without
/// a bootstrap is an empty historical project. The candidate's limits,
/// policy, and shapes apply. Every diagnostic is tagged with the baseline
/// side; a discovery failure is fatal and names the baseline.
pub fn baseline(
    tree: &dyn ReadTree,
    revision: &str,
    limits: &Limits,
    policy: &Policy,
    shapes: &BTreeMap<String, Shape>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Projection, String> {
    let fatal = |message: String| format!("baseline `{revision}`: {message}");
    let manifest_path = ProjectPath::parse(MANIFEST_NAME).expect("constant path");
    let mut own = Vec::new();
    let gathered = if tree.exists(&manifest_path) {
        let text = tree
            .read_text(&manifest_path)
            .map_err(|error| fatal(format!("cannot read {MANIFEST_NAME}: {error}")))?;
        let bootstrap = crate::bootstrap::parse(&text)
            .map_err(|error| fatal(format!("{MANIFEST_NAME} is not usable: {error}")))?;
        check_resource_roots(tree, &bootstrap).map_err(fatal)?;
        let mut gathered = gather(tree, &bootstrap, limits, &mut own).map_err(fatal)?;
        gathered.surface.insert(
            manifest_path,
            SurfaceEntry::new(Classification::Manifest, text.as_bytes()),
        );
        gathered
    } else {
        Gathered {
            files: Vec::new(),
            parsed: Vec::new(),
            documents: Vec::new(),
            document_paths: Vec::new(),
            surface: Surface::new(),
        }
    };
    let projection = settle(tree, gathered, policy, shapes, Side::Baseline, &mut own);
    diagnostics.extend(
        own.into_iter()
            .map(|diagnostic| diagnostic.on_side(Side::Baseline)),
    );
    Ok(projection)
}

pub struct Parsed {
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

fn discover(
    tree: &dyn ReadTree,
    bootstrap: &Bootstrap,
    limits: &Limits,
) -> Result<Vec<ProjectPath>, String> {
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
    if files.len() > limits.resources {
        return Err(format!(
            "{} resources exceed `limits.resources` = {}",
            files.len(),
            limits.resources
        ));
    }
    Ok(files)
}

fn parse_all(
    tree: &dyn ReadTree,
    limits: &Limits,
    files: &[ProjectPath],
    surface: &mut Surface,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Parsed> {
    let mut parsed = Vec::new();
    for path in files {
        match tree.file_len(path) {
            Ok(len) if len > limits.resource_bytes => {
                diagnostics.push(Diagnostic::new(
                    C::Unreadable,
                    path.as_str(),
                    format!(
                        "resource is {len} bytes, above `limits.resource_bytes` = {}",
                        limits.resource_bytes
                    ),
                ));
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                diagnostics.push(Diagnostic::new(
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
                diagnostics.push(Diagnostic::new(
                    C::Unreadable,
                    path.as_str(),
                    format!("cannot read resource: {error}"),
                ));
                continue;
            }
        };
        surface.insert(
            path.clone(),
            SurfaceEntry::new(Classification::Resource, &bytes),
        );
        let mut envelope_diagnostics = Vec::new();
        match envelope::parse(path, &bytes, &mut envelope_diagnostics) {
            Ok(resource) => {
                let valid = envelope_diagnostics.is_empty();
                diagnostics.extend(envelope_diagnostics);
                parsed.push(Parsed { resource, valid });
            }
            Err(diagnostic) => {
                diagnostics.push(diagnostic);
                diagnostics.extend(envelope_diagnostics);
            }
        }
    }
    parsed
}

fn read_documents(
    tree: &dyn ReadTree,
    limits: &Limits,
    paths: &[ProjectPath],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<document::Document> {
    let mut documents = Vec::new();
    for path in paths {
        match document::read(tree, limits, path) {
            Ok(document) => documents.push(document),
            Err(diagnostic) => diagnostics.push(diagnostic),
        }
    }
    documents
}

/// Validate every parsed resource against its registered schema's shape:
/// fields, required sections, and fragments. A resource whose schema is
/// unregistered or whose shape failed to load is not valid.
fn validate_structure(
    parsed: &mut [Parsed],
    policy: &Policy,
    shapes: &BTreeMap<String, Shape>,
    side: Side,
    diagnostics: &mut Vec<Diagnostic>,
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
            let message = match side {
                Side::Candidate => format!(
                    "schema `{}` is not registered by the policy",
                    resource.schema
                ),
                Side::Baseline => format!(
                    "schema `{}` is not registered by the current policy; comparison interprets history with the candidate's schemas, so the policy must keep every schema its baseline uses",
                    resource.schema
                ),
            };
            diagnostics.push(Diagnostic::new(C::SchemaIdentity, path, message));
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
        let before = diagnostics.len();
        for violation in shape.check(&resource.fields) {
            let key = if violation.location.is_empty() {
                violation.unexpected.as_deref()
            } else {
                violation.location.split('.').next()
            };
            let line = key.and_then(|key| resource.field_lines.get(key)).copied();
            diagnostics
                .push(Diagnostic::new(C::ShapeViolation, path, describe(&violation)).at_line(line));
        }
        for title in &shape.sections {
            if !resource
                .doc
                .sections
                .iter()
                .any(|section| &section.title == title)
            {
                diagnostics.push(Diagnostic::new(
                    C::MissingSection,
                    path,
                    format!("body must contain a `{title}` section"),
                ));
            }
        }
        for fragment in &resource.fragments {
            match shape.check_fragment(&fragment.kind, &fragment.fields) {
                None => diagnostics.push(
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
                        diagnostics.push(
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
        if diagnostics.len() > before {
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
