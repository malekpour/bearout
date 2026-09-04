// SPDX-License-Identifier: Apache-2.0

//! The explicit formatting write. Every proposed transformation is computed
//! before the tree changes: native normalization first (encoding mark,
//! line endings, trailing whitespace, final newline, in that order), then
//! the assigned formatter over the normalized bytes. Only existing,
//! selected regular files change; nothing is created or deleted; a
//! symbolic link is never followed or replaced; permissions, the executable
//! bit included, are preserved; a file is replaced only if, immediately
//! before the replacement, it still holds the bytes that were read; each
//! replacement is atomic through the working-directory writer; and a
//! failure part-way undoes every completed replacement from a journal,
//! restoring a file only while it still holds the bytes Bearout wrote and
//! reporting every refusal and failure. Formatting user-owned files is
//! what the command authorizes; the generated-output manifest plays no
//! part.
//!
//! The concurrent-edit protection is best-effort conflict detection, not
//! an atomic compare-and-swap: a read precedes each replacement, and an
//! edit landing in the moment between them can still be overwritten.

use super::editorconfig::Resolver;
use super::{Budget, Loaded, external, text};
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
    budget: &Budget,
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
    let resolver = Resolver::new(tree, budget);
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
                    workdirs[index] = Some(external::Workdir::prepare(tree, formatter, budget)?);
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
    resolver.fatal()?;

    // Preflight: every target is a regular file, not reached through a
    // link, still holding the bytes read. Nothing is written if any fails.
    let before = diagnostics.len();
    for change in &changes {
        if let Err(diagnostic) = verify(tree, change.path, change.before) {
            diagnostics.push(diagnostic);
        }
    }
    if diagnostics.len() > before {
        return Ok(Vec::new());
    }

    // Deliver, journaled, revalidating each file immediately before it is
    // replaced; the first failure ends delivery and restores the journal.
    let writer = working.writer();
    let mut journal: Vec<&Change<'_>> = Vec::new();
    let mut failure = None;
    for change in &changes {
        if let Err(diagnostic) = verify(tree, change.path, change.before) {
            failure = Some(diagnostic);
            break;
        }
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
            restore(tree, &writer, &journal, diagnostics);
            Ok(Vec::new())
        }
    }
}

/// Refuse a target that is reached through a link, cannot be read, or no
/// longer holds `expected`.
fn verify(tree: &dyn ReadTree, path: &ProjectPath, expected: &[u8]) -> Result<(), Diagnostic> {
    let refuse = |message: String| Diagnostic::new(Code::FormatWrite, path.as_str(), message);
    match tree.symlink_component(path) {
        Ok(None) => {}
        Ok(Some(link)) => {
            return Err(refuse(format!(
                "`{link}` is a symbolic link; files are never formatted through links"
            )));
        }
        Err(error) => return Err(refuse(format!("cannot inspect the file: {error}"))),
    }
    match tree.read(path) {
        Ok(current) if current == expected => Ok(()),
        Ok(_) => Err(refuse(
            "file changed after it was read; nothing was written, run again".to_owned(),
        )),
        Err(error) => Err(refuse(format!("cannot re-read the file: {error}"))),
    }
}

/// Undo journaled replacements in reverse order. A file is restored only
/// while it still holds the bytes Bearout wrote; one that changed since
/// is kept as it is, and the refusal is reported, so a concurrent edit is
/// never overwritten by the rollback.
fn restore(
    tree: &dyn ReadTree,
    writer: &Writer<'_>,
    journal: &[&Change<'_>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut restored = 0;
    for change in journal.iter().rev() {
        let report =
            |message: String| Diagnostic::new(Code::FormatWrite, change.path.as_str(), message);
        match tree.read(change.path) {
            Ok(current) if current == change.after => {
                match writer.replace_preserving(change.path, change.before) {
                    Ok(()) => restored += 1,
                    Err(error) => diagnostics.push(report(format!(
                        "could not restore prior content after a failed formatting write: {error}"
                    ))),
                }
            }
            Ok(_) => diagnostics.push(report(
                "restoration refused: the file changed after Bearout formatted it, so its current content is kept"
                    .to_owned(),
            )),
            Err(error) => diagnostics.push(report(format!(
                "could not inspect the file to restore it: {error}"
            ))),
        }
    }
    if !journal.is_empty() {
        diagnostics.push(Diagnostic::new(
            Code::FormatWrite,
            crate::bootstrap::MANIFEST_NAME,
            format!(
                "formatting failed after {} change(s); {restored} restored to prior content",
                journal.len()
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A concurrent edit of an already formatted file survives the rollback
    /// of a later failure: only files still holding Bearout's bytes are
    /// restored, and the refusal is reported.
    #[test]
    fn rollback_keeps_a_concurrent_edit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a formatted\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"b edited meanwhile\n").unwrap();
        let working = WorkingDir::open(dir.path()).unwrap();
        let a = ProjectPath::parse("a.txt").unwrap();
        let b = ProjectPath::parse("b.txt").unwrap();
        let changes = [
            Change {
                path: &a,
                before: b"a original\n",
                after: b"a formatted\n".to_vec(),
            },
            Change {
                path: &b,
                before: b"b original\n",
                after: b"b formatted\n".to_vec(),
            },
        ];
        let journal: Vec<&Change<'_>> = changes.iter().collect();
        let mut diagnostics = Vec::new();
        restore(&working, &working.writer(), &journal, &mut diagnostics);
        assert_eq!(
            std::fs::read(dir.path().join("a.txt")).unwrap(),
            b"a original\n"
        );
        assert_eq!(
            std::fs::read(dir.path().join("b.txt")).unwrap(),
            b"b edited meanwhile\n",
            "the concurrent edit is kept"
        );
        let rendered: Vec<String> = diagnostics.iter().map(ToString::to_string).collect();
        assert_eq!(
            rendered,
            [
                "b.txt:B031: restoration refused: the file changed after Bearout formatted it, so its current content is kept",
                "bearout.toml:B031: formatting failed after 2 change(s); 1 restored to prior content",
            ]
        );
    }
}
