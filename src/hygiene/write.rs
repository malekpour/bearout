// SPDX-License-Identifier: Apache-2.0

//! The explicit formatting write. Every proposed transformation is computed
//! before the tree changes: native normalization first (encoding mark,
//! line endings, trailing whitespace, final newline, in that order), then
//! the assigned formatter over the normalized bytes. Only existing,
//! selected regular files change; nothing is created or deleted; a
//! symbolic link is never followed or replaced; permissions, the executable
//! bit included, are preserved; a file is replaced only if it still holds
//! the bytes that were read, so a concurrent edit is not lost; each
//! replacement is atomic through the working-directory writer; and a
//! failure part-way undoes every completed replacement from a journal,
//! reporting any restoration failure. Formatting user-owned files is what
//! the command authorizes; the generated-output manifest plays no part.

use super::editorconfig::Resolver;
use super::{Loaded, external, text};
use crate::bootstrap::Bootstrap;
use crate::fs::{WorkingDir, Writer};
use crate::paths::ProjectPath;
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;

/// One file whose bytes should become `after`.
struct Change<'a> {
    path: &'a ProjectPath,
    before: &'a [u8],
    after: Vec<u8>,
}

/// Format every loaded file in place. Returns the paths rewritten, in
/// path order; diagnostics explain every file left alone.
pub fn format(
    working: &WorkingDir,
    loaded: &[Loaded],
    bootstrap: &Bootstrap,
    authorized: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<String>, String> {
    let tree: &dyn ReadTree = working;
    let assigned = loaded.iter().any(|file| file.selected.formatter.is_some());
    if assigned && !authorized {
        return Err(format!(
            "bearout.toml declares formatters ({}), which run as trusted host programs; pass --allow-formatters (library: `Options::allow_formatters`) to run them",
            bootstrap
                .formatters
                .iter()
                .map(|formatter| format!("`{}`", formatter.name))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    // Plan: compute every transformation before touching the tree.
    let resolver = Resolver::new(tree);
    let mut workdirs: Vec<Option<external::Workdir>> = Vec::new();
    workdirs.resize_with(bootstrap.formatters.len(), || None);
    let mut changes: Vec<Change<'_>> = Vec::new();
    for file in loaded {
        let path = &file.selected.path;
        let effective = match resolver.properties(path) {
            Ok(effective) => effective,
            Err(problems) => {
                diagnostics.extend(problems);
                continue;
            }
        };
        let normalized =
            match text::normalize(path.as_str(), &file.bytes, file.selected.binary, effective) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => continue,
                Err(diagnostic) => {
                    diagnostics.push(diagnostic);
                    continue;
                }
            };
        let after = match file.selected.formatter {
            None => normalized,
            Some(index) => {
                let formatter = &bootstrap.formatters[index];
                if workdirs[index].is_none() {
                    workdirs[index] = Some(external::Workdir::prepare(tree, formatter)?);
                }
                let workdir = workdirs[index].as_ref().expect("prepared");
                match external::run(formatter, workdir, path, &normalized) {
                    Ok(output) => output,
                    Err(external::Failure::Start(detail)) => {
                        return Err(format!(
                            "formatter `{}` cannot start: {detail}",
                            formatter.name
                        ));
                    }
                    Err(failure) => {
                        diagnostics.push(Diagnostic::new(
                            Code::FormatterFailed,
                            path.as_str(),
                            format!("formatter `{}` {failure}", formatter.name),
                        ));
                        continue;
                    }
                }
            }
        };
        if after != file.bytes {
            changes.push(Change {
                path,
                before: &file.bytes,
                after,
            });
        }
    }
    diagnostics.extend(resolver.take_diagnostics());

    // Verify: every target is still a regular file holding the bytes read.
    let before = diagnostics.len();
    for change in &changes {
        let refuse =
            |message: String| Diagnostic::new(Code::FormatWrite, change.path.as_str(), message);
        match tree.symlink_component(change.path) {
            Ok(None) => {}
            Ok(Some(link)) => {
                diagnostics.push(refuse(format!(
                    "`{link}` is a symbolic link; files are never formatted through links"
                )));
                continue;
            }
            Err(error) => {
                diagnostics.push(refuse(format!("cannot inspect the file: {error}")));
                continue;
            }
        }
        match tree.read(change.path) {
            Ok(current) if current == change.before => {}
            Ok(_) => diagnostics.push(refuse(
                "file changed after it was read; nothing was written, run again".to_owned(),
            )),
            Err(error) => diagnostics.push(refuse(format!("cannot re-read the file: {error}"))),
        }
    }
    if diagnostics.len() > before {
        return Ok(Vec::new());
    }

    // Deliver, journaled.
    let writer = working.writer();
    let mut journal: Vec<&Change<'_>> = Vec::new();
    let mut failure = None;
    for change in &changes {
        if let Err(error) = writer.replace_preserving(change.path, &change.after) {
            failure = Some(Diagnostic::new(
                Code::FormatWrite,
                change.path.as_str(),
                format!("cannot write the formatted file: {error}"),
            ));
            break;
        }
        journal.push(change);
    }
    match failure {
        None => Ok(changes
            .iter()
            .map(|change| change.path.as_str().to_owned())
            .collect()),
        Some(failure) => {
            diagnostics.push(failure);
            restore(&writer, &journal, diagnostics);
            Ok(Vec::new())
        }
    }
}

/// Undo journaled replacements in reverse order, reporting each failure.
fn restore(writer: &Writer<'_>, journal: &[&Change<'_>], diagnostics: &mut Vec<Diagnostic>) {
    for change in journal.iter().rev() {
        if let Err(error) = writer.replace_preserving(change.path, change.before) {
            diagnostics.push(Diagnostic::new(
                Code::FormatWrite,
                change.path.as_str(),
                format!("could not restore prior content after a failed formatting write: {error}"),
            ));
        }
    }
    if !journal.is_empty() {
        diagnostics.push(Diagnostic::new(
            Code::FormatWrite,
            crate::bootstrap::MANIFEST_NAME,
            format!(
                "formatting failed after {} change(s); prior content was restored where possible",
                journal.len()
            ),
        ));
    }
}
