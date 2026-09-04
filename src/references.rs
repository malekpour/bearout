// SPDX-License-Identifier: Apache-2.0

//! Markdown reference checking for resources and schema-less documents
//! alike. A link or image is a document concern: it resolves against the
//! project tree and against the headings and explicit anchors of Markdown
//! files, never against a schema or a resource identifier.
//!
//! Semantics:
//!
//! - a target with a URL scheme is not a local reference;
//! - a relative target resolves from the source document's directory; a
//!   leading `/` means the project root, as repository-hosted Markdown
//!   renders it; `.` and `..` may not leave the project;
//! - a query string is ignored; `%XX` escapes are decoded on bytes and the
//!   result is revalidated as a project path;
//! - `#fragment` alone resolves within the source document; a fragment on
//!   another Markdown file resolves against that file's GFM heading anchors
//!   and explicit `<a id>`/`<a name>` anchors, provided the file is a
//!   structurally valid resource or a parsed schema-less document;
//! - a fragment on an existing Markdown file that is neither is reported:
//!   the anchor cannot be verified, and Bearout does not claim it is valid;
//! - a fragment on a file that failed an earlier phase is not reported
//!   again, since the failure already is;
//! - a fragment on a non-Markdown file or a directory is not interpreted;
//! - an existing ordinary file or directory is a valid link target; an
//!   image must name an existing file;
//! - symbolic links and submodules follow the tree's rules: a link is
//!   followed only inside the tree, a submodule is neither a file nor a
//!   directory.
//!
//! Every independent broken reference is one B011 diagnostic.

use std::collections::BTreeMap;

use crate::bootstrap::MARKDOWN_EXTENSION;
use crate::document::Document;
use crate::envelope::Resource;
use crate::markdown;
use crate::paths::ProjectPath;
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;

/// What the index knows about one Markdown path.
enum Target<'a> {
    /// Parsed; anchors can be verified.
    Verified(&'a markdown::Document),
    /// Selected, but failed an earlier phase; nothing more is reported.
    Unverifiable,
}

/// One source of references.
struct Origin<'a> {
    path: &'a ProjectPath,
    doc: &'a markdown::Document,
}

/// Check every link and image of every structurally valid resource and
/// every parsed document. `resource_paths` and `document_paths` are the
/// discovered sets, so that a file which failed parsing is known to be
/// Markdown that cannot be verified.
pub fn check(
    tree: &dyn ReadTree,
    resources: &[Resource],
    valid: &[bool],
    resource_paths: &[ProjectPath],
    documents: &[Document],
    document_paths: &[ProjectPath],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut index: BTreeMap<&str, Target<'_>> = BTreeMap::new();
    for path in resource_paths.iter().chain(document_paths) {
        index.insert(path.as_str(), Target::Unverifiable);
    }
    for document in documents {
        index.insert(document.path.as_str(), Target::Verified(&document.doc));
    }
    let mut origins = Vec::new();
    for (resource, valid) in resources.iter().zip(valid) {
        if *valid {
            index.insert(resource.path.as_str(), Target::Verified(&resource.doc));
            origins.push(Origin {
                path: &resource.path,
                doc: &resource.doc,
            });
        }
    }
    for document in documents {
        origins.push(Origin {
            path: &document.path,
            doc: &document.doc,
        });
    }
    origins.sort_by(|a, b| a.path.cmp(b.path));

    for origin in origins {
        for link in &origin.doc.links {
            check_link(tree, &index, &origin, &link.target, link.line, diagnostics);
        }
        for image in &origin.doc.images {
            check_image(tree, &origin, &image.target, image.line, diagnostics);
        }
    }
}

/// Split a target into its location, fragment, and nothing else: the
/// query string is dropped.
fn split(target: &str) -> (&str, Option<&str>) {
    let (location, fragment) = match target.split_once('#') {
        Some((location, fragment)) => (location, Some(fragment)),
        None => (target, None),
    };
    let location = location.split_once('?').map_or(location, |(path, _)| path);
    (location, fragment)
}

/// Resolve a decoded location against the origin's directory, or against
/// the project root when it begins with `/`.
fn locate(origin: &ProjectPath, location: &str) -> Result<ProjectPath, String> {
    let decoded = percent_decode(location)?;
    match decoded.strip_prefix('/') {
        Some(rooted) => ProjectPath::resolve_relative(&ProjectPath::root(), rooted),
        None => ProjectPath::resolve_relative(&origin.parent(), &decoded),
    }
}

fn check_link(
    tree: &dyn ReadTree,
    index: &BTreeMap<&str, Target<'_>>,
    origin: &Origin<'_>,
    target: &str,
    line: u32,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if target.is_empty() || has_scheme(target) {
        return;
    }
    let report = |message: String| {
        Diagnostic::new(Code::UnresolvedLink, origin.path.as_str(), message).at_line(Some(line))
    };
    let (location, fragment) = split(target);

    let (path, known) = if location.is_empty() {
        (origin.path.clone(), Some(&Target::Verified(origin.doc)))
    } else {
        let path = match locate(origin.path, location) {
            Ok(path) => path,
            Err(error) => {
                diagnostics.push(report(format!("link `{target}`: {error}")));
                return;
            }
        };
        let known = index.get(path.as_str());
        if known.is_none() && !tree.is_file(&path) && !tree.is_dir(&path) {
            diagnostics.push(report(format!("link `{target}` points at a missing file")));
            return;
        }
        (path, known)
    };

    let Some(fragment) = fragment else {
        return;
    };
    let anchor = match percent_decode(fragment) {
        Ok(anchor) => anchor,
        Err(error) => {
            diagnostics.push(report(format!("link `{target}`: {error}")));
            return;
        }
    };
    match known {
        Some(Target::Verified(doc)) => {
            if !doc.has_anchor(&anchor) {
                diagnostics.push(report(format!(
                    "link `{target}` names anchor `{anchor}`, which `{path}` does not define"
                )));
            }
        }
        Some(Target::Unverifiable) => {}
        None => {
            if path.extension() == Some(MARKDOWN_EXTENSION) && tree.is_file(&path) {
                diagnostics.push(report(format!(
                    "link `{target}` names anchor `{anchor}` in `{path}`, which is not a discovered document; select it in `[documents]` to verify its anchors"
                )));
            }
        }
    }
}

fn check_image(
    tree: &dyn ReadTree,
    origin: &Origin<'_>,
    target: &str,
    line: u32,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if target.is_empty() || has_scheme(target) {
        return;
    }
    let report = |message: String| {
        Diagnostic::new(Code::UnresolvedLink, origin.path.as_str(), message).at_line(Some(line))
    };
    let (location, _) = split(target);
    if location.is_empty() {
        diagnostics.push(report(format!("image `{target}` names no file")));
        return;
    }
    let path = match locate(origin.path, location) {
        Ok(path) => path,
        Err(error) => {
            diagnostics.push(report(format!("image `{target}`: {error}")));
            return;
        }
    };
    if tree.is_file(&path) {
        return;
    }
    if tree.is_dir(&path) {
        diagnostics.push(report(format!(
            "image `{target}` points at a directory, not a file"
        )));
    } else {
        diagnostics.push(report(format!("image `{target}` points at a missing file")));
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

    #[test]
    fn targets_split_and_locate() {
        assert_eq!(split("a.md?x=1#frag"), ("a.md", Some("frag")));
        assert_eq!(
            split("a.md#frag?not-a-query"),
            ("a.md", Some("frag?not-a-query"))
        );
        assert_eq!(split("#only"), ("", Some("only")));
        assert_eq!(split("dir/"), ("dir/", None));
        let origin = ProjectPath::parse("docs/guides/a.md").unwrap();
        assert_eq!(locate(&origin, "../b.md").unwrap().as_str(), "docs/b.md");
        assert_eq!(locate(&origin, "/README.md").unwrap().as_str(), "README.md");
        assert_eq!(locate(&origin, "/").unwrap().as_str(), "");
        assert_eq!(
            locate(&origin, "./x%20y.md").unwrap().as_str(),
            "docs/guides/x y.md"
        );
        assert!(
            locate(&origin, "../../../x")
                .unwrap_err()
                .contains("leaves")
        );
        assert!(locate(&origin, "/../x").unwrap_err().contains("leaves"));
        assert!(locate(&origin, "a%3Ab").unwrap_err().contains("`:`"));
    }
}
