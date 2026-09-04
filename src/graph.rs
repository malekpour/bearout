// SPDX-License-Identifier: Apache-2.0

//! The identifier graph over structurally valid resources: which ids exist,
//! how resources relate, and whether Markdown links resolve.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::envelope::Resource;
use crate::identity;
use crate::paths::ProjectPath;
use crate::report::{Code, Diagnostic};
use crate::shape::Shape;
use crate::tree::ReadTree;

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

/// Build the graph and report duplicate ids, unresolved or mistyped
/// relations, and unresolved links.
///
/// Every parsed resource contributes its identifiers, so a reference to a
/// resource that failed structural validation still resolves and is not
/// reported a second time. Relations and links are only checked for
/// resources marked `valid`.
pub fn build(
    tree: &dyn ReadTree,
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

    check_links(tree, resources, valid, diagnostics);
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

/// Resolve every relative Markdown link against the project tree, and every
/// fragment identifier against the target's heading anchors. Targets with a
/// URL scheme are not checked. A query string is ignored for resolution.
fn check_links(
    tree: &dyn ReadTree,
    resources: &[Resource],
    valid: &[bool],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let by_path: BTreeMap<&str, usize> = resources
        .iter()
        .enumerate()
        .map(|(index, resource)| (resource.path.as_str(), index))
        .collect();

    for resource in resources
        .iter()
        .zip(valid)
        .filter(|(_, valid)| **valid)
        .map(|(resource, _)| resource)
    {
        let base = resource.path.parent();
        for link in &resource.doc.links {
            if link.target.is_empty() || has_scheme(&link.target) {
                continue;
            }
            let (location, anchor) = match link.target.split_once('#') {
                Some((location, anchor)) => (location, Some(anchor)),
                None => (link.target.as_str(), None),
            };
            let location = location.split_once('?').map_or(location, |(path, _)| path);
            let report = |message: String| {
                Diagnostic::new(Code::UnresolvedLink, resource.path.as_str(), message)
                    .at_line(Some(link.line))
            };

            let target = if location.is_empty() {
                Some(resource)
            } else {
                let decoded = match percent_decode(location) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        diagnostics.push(report(format!("link `{}`: {error}", link.target)));
                        continue;
                    }
                };
                let joined = match ProjectPath::resolve_relative(&base, &decoded) {
                    Ok(joined) => joined,
                    Err(error) => {
                        diagnostics.push(report(format!("link `{}`: {error}", link.target)));
                        continue;
                    }
                };
                match by_path.get(joined.as_str()) {
                    Some(index) => Some(&resources[*index]),
                    None if tree.is_file(&joined) => None,
                    None => {
                        diagnostics.push(report(format!(
                            "link `{}` points at a missing file",
                            link.target
                        )));
                        continue;
                    }
                }
            };

            if let (Some(target), Some(anchor)) = (target, anchor) {
                let anchor = match percent_decode(anchor) {
                    Ok(anchor) => anchor,
                    Err(error) => {
                        diagnostics.push(report(format!("link `{}`: {error}", link.target)));
                        continue;
                    }
                };
                if !target.doc.has_anchor(&anchor) {
                    diagnostics.push(report(format!(
                        "link `{}` names anchor `{anchor}`, which `{}` does not define",
                        link.target, target.path
                    )));
                }
            }
        }
    }
}

fn has_scheme(target: &str) -> bool {
    let Some((scheme, _)) = target.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
}

/// Decode `%XX` escapes on bytes. A `%` not followed by two hexadecimal
/// digits is kept literally. The result must be valid UTF-8; it is then
/// revalidated as a project path before use, so decoded traversal, control
/// characters, separators, or colons become link diagnostics, never panics.
fn percent_decode(text: &str) -> Result<String, String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (
                hex_value(bytes.get(index + 1)),
                hex_value(bytes.get(index + 2)),
            )
        {
            out.push(high << 4 | low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).map_err(|_| format!("`{text}` does not decode to valid UTF-8"))
}

fn hex_value(byte: Option<&u8>) -> Option<u8> {
    match byte? {
        b'0'..=b'9' => Some(byte? - b'0'),
        b'a'..=b'f' => Some(byte? - b'a' + 10),
        b'A'..=b'F' => Some(byte? - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_and_percent_decoding() {
        assert!(has_scheme("https://example.org"));
        assert!(has_scheme("mailto:a@b"));
        assert!(!has_scheme("docs/a.md"));
        assert!(!has_scheme("#anchor"));
        assert_eq!(percent_decode("a%20b.md").unwrap(), "a b.md");
        assert_eq!(percent_decode("ĉ%C4%89").unwrap(), "ĉĉ");
        assert_eq!(percent_decode("100%").unwrap(), "100%");
        assert_eq!(percent_decode("%zz").unwrap(), "%zz");
        assert_eq!(percent_decode("%aĉ").unwrap(), "%aĉ");
        assert_eq!(percent_decode("%").unwrap(), "%");
        assert_eq!(percent_decode("%2").unwrap(), "%2");
        assert_eq!(percent_decode("%2e%2e/x").unwrap(), "../x");
        assert_eq!(percent_decode("a%2Fb").unwrap(), "a/b");
        assert_eq!(percent_decode("a%3Ab").unwrap(), "a:b");
        assert_eq!(percent_decode("a%00b").unwrap(), "a\u{0}b");
        assert!(percent_decode("%ff%fe").is_err());
    }
}
