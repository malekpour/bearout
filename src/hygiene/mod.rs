// SPDX-License-Identifier: Apache-2.0

//! Repository hygiene and formatting over the files a project selects.
//!
//! The kernel knows selected paths, their bytes, the effective text
//! properties from `.editorconfig` files of the same tree, and formatter
//! assignments; it assigns no meaning to a file name or extension of its
//! own. Selection is explicit: `[hygiene] scope = "repository"` means every
//! file of the project as Git knows it (the captured index or revision
//! tree, or for the working directory the tracked plus untracked,
//! non-ignored files beneath the project root), `scope = "declared"`
//! means only the listed roots and files. Symbolic links are never
//! followed, submodules are never entered, and every list is sorted.
//!
//! Only the candidate is ever selected; the comparison baseline is
//! history and is neither checked nor formatted.

pub mod editorconfig;
pub mod external;
pub mod selection;
pub mod text;
pub mod write;

use std::cell::Cell;

use crate::bootstrap::{Bootstrap, Limits};
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;

pub use selection::{Selected, Universe, select};

/// The bounds every hygiene read observes: `limits.file_bytes` per file
/// and `limits.hygiene_bytes` in total across selected files,
/// `.editorconfig` files, and formatter support files, so that
/// repository-wide selection cannot grow memory without bound.
pub struct Budget {
    limits: Limits,
    remaining: Cell<u64>,
}

impl Budget {
    #[must_use]
    pub fn new(limits: &Limits) -> Self {
        Self {
            limits: *limits,
            remaining: Cell::new(limits.hygiene_bytes),
        }
    }

    /// The per-file and other limits of the candidate.
    #[must_use]
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Charge `bytes` for reading `what`; exhaustion is fatal.
    pub fn charge(&self, what: &str, bytes: u64) -> Result<(), String> {
        match self.remaining.get().checked_sub(bytes) {
            Some(left) => {
                self.remaining.set(left);
                Ok(())
            }
            None => Err(format!(
                "hygiene inputs exceed `limits.hygiene_bytes` = {} while reading `{what}`",
                self.limits.hygiene_bytes
            )),
        }
    }
}

/// One selected file with its bytes, read exactly once.
#[derive(Debug)]
pub struct Loaded {
    pub selected: Selected,
    pub bytes: Vec<u8>,
}

/// Read every selected file within `limits.file_bytes` and the run's
/// budget, reading no more than the limit allows and charging the bytes
/// actually returned. A file that cannot be read or is too large is B024
/// and is not returned; an exhausted budget is fatal.
pub fn load(
    tree: &dyn ReadTree,
    selected: Vec<Selected>,
    budget: &Budget,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<Loaded>, String> {
    let limits = budget.limits();
    let mut loaded = Vec::new();
    for entry in selected {
        let path = entry.path.as_str();
        match tree.read_bounded(&entry.path, limits.file_bytes) {
            Ok((_, true)) => diagnostics.push(Diagnostic::new(
                Code::FileUnreadable,
                path,
                over_limit(tree, &entry.path, "file", limits.file_bytes),
            )),
            Ok((bytes, false)) => {
                budget.charge(path, bytes.len() as u64)?;
                loaded.push(Loaded {
                    selected: entry,
                    bytes,
                });
            }
            Err(error) => diagnostics.push(Diagnostic::new(
                Code::FileUnreadable,
                path,
                format!("cannot read file: {error}"),
            )),
        }
    }
    Ok(loaded)
}

/// The message for a file above `limits.file_bytes`, with its length when
/// the tree can still report one.
pub fn over_limit(
    tree: &dyn ReadTree,
    path: &crate::paths::ProjectPath,
    what: &str,
    limit: u64,
) -> String {
    match tree.file_len(path) {
        Ok(len) if len > limit => {
            format!("{what} is {len} bytes, above `limits.file_bytes` = {limit}")
        }
        _ => format!("{what} exceeds `limits.file_bytes` = {limit}"),
    }
}

/// Verify every loaded file that has a formatter against that formatter's
/// output, skipping files the text phase could not decode or configure so
/// that nothing cascades from B023 or B025. Formatters run only when the
/// host authorized them; declaring formatters without authorization is
/// fatal, as is a formatter that cannot start or a support file the
/// selected tree lacks. Each difference is B029 and each failed run is
/// B030, in path order.
pub fn check_formatters(
    tree: &dyn ReadTree,
    loaded: &[Loaded],
    decodable: &[bool],
    bootstrap: &Bootstrap,
    budget: &Budget,
    authorized: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), String> {
    let assigned: Vec<&Loaded> = loaded
        .iter()
        .zip(decodable)
        .filter(|(file, decodable)| file.selected.formatter.is_some() && **decodable)
        .map(|(file, _)| file)
        .collect();
    if assigned.is_empty() {
        return Ok(());
    }
    if !authorized {
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
    let mut workdirs: Vec<Option<external::Workdir>> = Vec::new();
    workdirs.resize_with(bootstrap.formatters.len(), || None);
    for file in assigned {
        let index = file.selected.formatter.expect("assigned");
        let formatter = &bootstrap.formatters[index];
        if workdirs[index].is_none() {
            workdirs[index] = Some(external::Workdir::prepare(tree, formatter, budget)?);
        }
        let workdir = workdirs[index].as_ref().expect("prepared");
        match external::run(formatter, workdir, &file.selected.path, &file.bytes) {
            Ok(output) if output == file.bytes => {}
            Ok(_) => diagnostics.push(Diagnostic::new(
                Code::FormatDifference,
                file.selected.path.as_str(),
                format!(
                    "file differs from the output of formatter `{}`; run `bearout format`",
                    formatter.name
                ),
            )),
            Err(external::Failure::Start(detail)) => {
                return Err(format!(
                    "formatter `{}` cannot start: {detail}",
                    formatter.name
                ));
            }
            Err(failure) => diagnostics.push(Diagnostic::new(
                Code::FormatterFailed,
                file.selected.path.as_str(),
                format!("formatter `{}` {failure}", formatter.name),
            )),
        }
    }
    Ok(())
}

/// Native text hygiene over every loaded file, with properties resolved
/// from the same tree. Returns, per loaded file, whether its configuration
/// was usable and its bytes decodable, so later phases can leave the
/// others alone. An exhausted budget is fatal.
pub fn check_text(
    tree: &dyn ReadTree,
    loaded: &[Loaded],
    budget: &Budget,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Vec<bool>, String> {
    let resolver = editorconfig::Resolver::new(tree, budget);
    let mut decodable = Vec::with_capacity(loaded.len());
    for file in loaded {
        match resolver.properties(&file.selected.path) {
            Ok(effective) => {
                let found = text::check(
                    file.selected.path.as_str(),
                    &file.bytes,
                    file.selected.binary,
                    effective,
                );
                decodable.push(!found.iter().any(|d| d.code == Code::Encoding));
                diagnostics.extend(found);
            }
            Err(problems) => {
                decodable.push(false);
                diagnostics.extend(problems);
            }
        }
    }
    diagnostics.extend(resolver.take_diagnostics());
    resolver.fatal()?;
    Ok(decodable)
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Arc;

    use super::*;
    use crate::paths::ProjectPath;

    /// A tree whose reported length disagrees with its content, standing in
    /// for a live file that grows between a length check and a read.
    struct Lying {
        content: Vec<u8>,
    }

    impl ReadTree for Lying {
        fn read(&self, _: &ProjectPath) -> io::Result<Vec<u8>> {
            Ok(self.content.clone())
        }
        fn file_len(&self, _: &ProjectPath) -> io::Result<u64> {
            Ok(5)
        }
        fn is_file(&self, _: &ProjectPath) -> bool {
            true
        }
        fn is_dir(&self, path: &ProjectPath) -> bool {
            path.as_str().is_empty()
        }
        fn exists(&self, _: &ProjectPath) -> bool {
            true
        }
        fn symlink_component(&self, _: &ProjectPath) -> io::Result<Option<ProjectPath>> {
            Ok(None)
        }
        fn walk(&self, _: &ProjectPath) -> io::Result<Vec<ProjectPath>> {
            Ok(Vec::new())
        }
        fn subtree(&self, _: &ProjectPath) -> io::Result<Arc<dyn ReadTree>> {
            Err(io::Error::other("no subtrees"))
        }
    }

    fn selected(path: &str) -> Selected {
        Selected {
            path: ProjectPath::parse(path).unwrap(),
            binary: None,
            formatter: None,
        }
    }

    #[test]
    fn limits_apply_to_the_bytes_actually_read_not_the_reported_length() {
        let tree = Lying {
            content: vec![b'x'; 1000],
        };
        let limits = Limits {
            file_bytes: 100,
            ..Limits::default()
        };
        let budget = Budget::new(&limits);
        let mut diagnostics = Vec::new();
        let loaded = load(&tree, vec![selected("a.txt")], &budget, &mut diagnostics).unwrap();
        assert!(
            loaded.is_empty(),
            "the file is over the limit despite claiming 5 bytes"
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, Code::FileUnreadable);
        assert!(
            diagnostics[0]
                .message
                .contains("exceeds `limits.file_bytes` = 100"),
            "{}",
            diagnostics[0].message
        );

        // Within the file limit, the budget is charged with the real size.
        let limits = Limits {
            file_bytes: 2_000,
            hygiene_bytes: 1_500,
            ..Limits::default()
        };
        let budget = Budget::new(&limits);
        let mut diagnostics = Vec::new();
        let error = load(
            &tree,
            vec![selected("a.txt"), selected("b.txt")],
            &budget,
            &mut diagnostics,
        )
        .unwrap_err();
        assert!(
            error.contains("`limits.hygiene_bytes` = 1500 while reading `b.txt`"),
            "{error}"
        );
    }
}
