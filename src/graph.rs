// SPDX-License-Identifier: Apache-2.0

//! The identifier graph over structurally valid resources: which ids exist
//! and how resources relate. Markdown links are a document concern and are
//! checked by `references`, not here.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::envelope::Resource;
use crate::identity;
use crate::report::{Code, Diagnostic};
use crate::shape::Shape;

/// The untyped relation every resource carries.
pub const REFS_FIELD: &str = "refs";

/// What an identifier names.
#[derive(Debug, Clone)]
pub struct Node {
    /// Index of the defining resource.
    pub resource: usize,
    /// Kind: the resource schema, or `schema#fragment-kind` for a fragment.
    pub kind: String,
}

/// The graph.
pub struct Graph {
    /// Every identifier and what it names.
    pub nodes: BTreeMap<String, Node>,
    /// Per resource: relation field to the identifiers it names, as written.
    pub relations: Vec<BTreeMap<String, Vec<String>>>,
    /// Per resource: `(from id, field)` pairs that name it or its fragments.
    pub referenced_by: Vec<Vec<(String, String)>>,
}

/// Build the graph and report duplicate ids and unresolved or mistyped
/// relations.
///
/// Every parsed resource contributes its identifiers, so a reference to a
/// resource that failed structural validation still resolves and is not
/// reported a second time. Relations are only checked for resources marked
/// `valid`.
pub fn build(
    resources: &[Resource],
    valid: &[bool],
    shapes: &BTreeMap<String, Shape>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Graph {
    let nodes = index(resources, diagnostics);
    let relations: Vec<_> = resources
        .iter()
        .map(|resource| relations_of(resource, shapes.get(&resource.schema)))
        .collect();
    let mut referenced_by = vec![Vec::new(); resources.len()];

    for (index, resource) in resources.iter().enumerate() {
        if !valid[index] {
            continue;
        }
        let shape = shapes.get(&resource.schema);
        for (field, targets) in &relations[index] {
            let allowed = shape.and_then(|shape| shape.relations.get(field));
            let line = resource.field_lines.get(field).copied();
            for target in targets {
                match nodes.get(target) {
                    None => diagnostics.push(
                        Diagnostic::new(
                            Code::UnresolvedReference,
                            resource.path.as_str(),
                            format!("`{field}` names `{target}`, which nothing defines"),
                        )
                        .at_line(line),
                    ),
                    Some(node) => {
                        if let Some(kinds) = allowed
                            && !kinds.is_empty()
                            && !kinds.contains(&node.kind)
                        {
                            diagnostics.push(
                                Diagnostic::new(
                                    Code::ReferenceKind,
                                    resource.path.as_str(),
                                    format!("`{field}` names `{target}`, which is a `{}`, not one of {kinds:?}", node.kind),
                                )
                                .at_line(line),
                            );
                        }
                        referenced_by[node.resource].push((resource.id.clone(), field.clone()));
                    }
                }
            }
        }
    }
    for list in &mut referenced_by {
        list.sort();
        list.dedup();
    }

    Graph {
        nodes,
        relations,
        referenced_by,
    }
}

/// `(resource index, fragment line, kind)` for one definer of an identifier.
type Definer = (usize, Option<u32>, String);

fn index(resources: &[Resource], diagnostics: &mut Vec<Diagnostic>) -> BTreeMap<String, Node> {
    let mut definitions: BTreeMap<&str, Vec<Definer>> = BTreeMap::new();
    for (index, resource) in resources.iter().enumerate() {
        definitions
            .entry(&resource.id)
            .or_default()
            .push((index, None, resource.schema.clone()));
        for fragment in &resource.fragments {
            definitions.entry(&fragment.id).or_default().push((
                index,
                Some(fragment.line),
                identity::fragment_kind(&resource.schema, &fragment.kind),
            ));
        }
    }

    let mut nodes = BTreeMap::new();
    for (id, definers) in definitions {
        if definers.len() > 1 {
            for (index, line, _) in &definers {
                diagnostics.push(
                    Diagnostic::new(
                        Code::DuplicateId,
                        resources[*index].path.as_str(),
                        format!("identifier `{id}` is defined more than once"),
                    )
                    .at_line(*line),
                );
            }
        }
        let (resource, _, kind) = &definers[0];
        nodes.insert(
            id.to_owned(),
            Node {
                resource: *resource,
                kind: kind.clone(),
            },
        );
    }
    nodes
}

fn relations_of(resource: &Resource, shape: Option<&Shape>) -> BTreeMap<String, Vec<String>> {
    let mut relations = BTreeMap::new();
    relations.insert(REFS_FIELD.to_owned(), resource.refs.clone());
    let Some(shape) = shape else {
        return relations;
    };
    for field in shape.relations.keys() {
        let targets = match resource.fields.get(field) {
            Some(Value::String(target)) => vec![target.clone()],
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => continue,
        };
        relations.insert(field.clone(), targets);
    }
    relations
}
