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

use crate::bootstrap::{Bootstrap, Limits};
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;

pub use selection::{Selected, Universe, select};

/// One selected file with its bytes, read exactly once.
pub struct Loaded {
    pub selected: Selected,
    pub bytes: Vec<u8>,
}

/// Read every selected file within `limits.file_bytes`. A file that cannot
/// be read or is too large is B024 and is not returned.
pub fn load(
    tree: &dyn ReadTree,
    selected: Vec<Selected>,
    limits: &Limits,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Loaded> {
    let mut loaded = Vec::new();
    for entry in selected {
        let path = entry.path.as_str();
        match tree.file_len(&entry.path) {
            Ok(len) if len > limits.file_bytes => {
                diagnostics.push(Diagnostic::new(
                    Code::FileUnreadable,
                    path,
                    format!(
                        "file is {len} bytes, above `limits.file_bytes` = {}",
                        limits.file_bytes
                    ),
                ));
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                diagnostics.push(Diagnostic::new(
                    Code::FileUnreadable,
                    path,
                    format!("cannot read file: {error}"),
                ));
                continue;
            }
        }
        match tree.read(&entry.path) {
            Ok(bytes) => loaded.push(Loaded {
                selected: entry,
                bytes,
            }),
            Err(error) => diagnostics.push(Diagnostic::new(
                Code::FileUnreadable,
                path,
                format!("cannot read file: {error}"),
            )),
        }
    }
    loaded
}

/// Verify every loaded file that has a formatter against that formatter's
/// output. Formatters run only when the host authorized them; declaring
/// formatters without authorization is fatal, as is a formatter that cannot
/// start or a support file the selected tree lacks. Each difference is
/// B029 and each failed run is B030, in path order.
pub fn check_formatters(
    tree: &dyn ReadTree,
    loaded: &[Loaded],
    bootstrap: &Bootstrap,
    authorized: bool,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), String> {
    let assigned: Vec<&Loaded> = loaded
        .iter()
        .filter(|file| file.selected.formatter.is_some())
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
            workdirs[index] = Some(external::Workdir::prepare(tree, formatter)?);
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
/// from the same tree.
pub fn check_text(tree: &dyn ReadTree, loaded: &[Loaded], diagnostics: &mut Vec<Diagnostic>) {
    let resolver = editorconfig::Resolver::new(tree);
    for file in loaded {
        match resolver.properties(&file.selected.path) {
            Ok(effective) => diagnostics.extend(text::check(
                file.selected.path.as_str(),
                &file.bytes,
                file.selected.binary,
                effective,
            )),
            Err(problems) => diagnostics.extend(problems),
        }
    }
    diagnostics.extend(resolver.take_diagnostics());
}
