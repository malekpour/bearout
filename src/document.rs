// SPDX-License-Identifier: Apache-2.0

//! Schema-less Markdown documents: files selected by the bootstrap's
//! `[documents]` grant that carry Markdown structure but no envelope,
//! schema, identifier, shape, or relations. They are never turned into
//! resources, and a resource never becomes a document: a path selected as
//! both is processed once, as a resource.

use crate::bootstrap::{Bootstrap, MARKDOWN_EXTENSION};
use crate::markdown;
use crate::paths::ProjectPath;
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;

/// One schema-less document.
#[derive(Debug, Clone)]
pub struct Document {
    /// Project-relative path.
    pub path: ProjectPath,
    /// The whole text, with a leading UTF-8 byte-order mark removed.
    pub text: String,
    /// Number of lines in the file.
    pub line_count: u32,
    /// Markdown structure: sections, explicit anchors, blocks, links, and
    /// images, from the same parser resources use.
    pub doc: markdown::Document,
}

impl Document {
    /// The JSON view exposed to repository policy.
    #[must_use]
    pub fn view(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path.as_str(),
            "text": self.text,
            "line_count": self.line_count,
            "sections": self.doc.sections,
            "anchors": self.doc.anchors,
            "links": self.doc.links,
            "images": self.doc.images,
        })
    }
}

/// Discover the schema-less documents the bootstrap selects: every Markdown
/// file beneath a document root (never following links or entering
/// submodules) plus every listed file, sorted and deduplicated, minus the
/// paths that resource discovery already claimed. A missing or linked
/// declared file, a root that is not a directory, or a count above
/// `limits.documents` is a fatal outcome.
pub fn discover(
    tree: &dyn ReadTree,
    bootstrap: &Bootstrap,
    resources: &[ProjectPath],
) -> Result<Vec<ProjectPath>, String> {
    let mut found = std::collections::BTreeSet::new();
    for root in &bootstrap.document_roots {
        if !tree.is_dir(root) {
            return Err(format!(
                "document root `{root}` is not a directory inside the project"
            ));
        }
        let walked = tree
            .walk(root)
            .map_err(|error| format!("cannot walk document root `{root}`: {error}"))?;
        found.extend(
            walked
                .into_iter()
                .filter(|path| path.extension() == Some(MARKDOWN_EXTENSION)),
        );
    }
    for file in &bootstrap.document_files {
        match tree.symlink_component(file) {
            Ok(None) => {}
            Ok(Some(link)) => {
                return Err(format!(
                    "document `{file}` is reached through the symbolic link `{link}`; documents must not be reached through links"
                ));
            }
            Err(error) => return Err(format!("cannot inspect document `{file}`: {error}")),
        }
        if !tree.is_file(file) {
            return Err(format!(
                "document `{file}` is not a file inside the project"
            ));
        }
        found.insert(file.clone());
    }
    for resource in resources {
        found.remove(resource);
    }
    if found.len() > bootstrap.limits.documents {
        return Err(format!(
            "{} documents exceed `limits.documents` = {}",
            found.len(),
            bootstrap.limits.documents
        ));
    }
    Ok(found.into_iter().collect())
}

/// Read and parse one document. Every failure is a B022 diagnostic.
pub fn read(
    tree: &dyn ReadTree,
    bootstrap: &Bootstrap,
    path: &ProjectPath,
) -> Result<Document, Diagnostic> {
    let report =
        |message: String| Diagnostic::new(Code::DocumentUnreadable, path.as_str(), message);
    let len = tree
        .file_len(path)
        .map_err(|error| report(format!("cannot read document: {error}")))?;
    if len > bootstrap.limits.document_bytes {
        return Err(report(format!(
            "document is {len} bytes, above `limits.document_bytes` = {}",
            bootstrap.limits.document_bytes
        )));
    }
    let bytes = tree
        .read(path)
        .map_err(|error| report(format!("cannot read document: {error}")))?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|error| report(format!("document is not valid UTF-8: {error}")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let line_count = u32::try_from(text.split_inclusive('\n').count()).unwrap_or(u32::MAX);
    Ok(Document {
        path: path.clone(),
        text: text.to_owned(),
        line_count,
        doc: markdown::parse(text, 1),
    })
}
