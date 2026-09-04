// SPDX-License-Identifier: Apache-2.0

//! Deterministic selection of the files subject to hygiene and formatting.

use std::collections::BTreeSet;
use std::path::Path;

use crate::bootstrap::{Bootstrap, Formatter, Limits, Scope};
use crate::git;
use crate::paths::ProjectPath;
use crate::tree::ReadTree;

/// Where the repository-wide universe of files comes from.
#[derive(Debug, Clone, Copy)]
pub enum Universe<'a> {
    /// The live working directory at this root: Git lists the tracked and
    /// untracked, non-ignored files, and the tree decides which still exist
    /// as regular files.
    WorkingDirectory(&'a Path),
    /// A captured index or revision: the tree's own regular files.
    Frozen,
}

/// One selected file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub path: ProjectPath,
    /// `Some(true)` when the bootstrap declares the path binary,
    /// `Some(false)` when it declares it text, `None` to decide by content.
    pub binary: Option<bool>,
    /// Index into the bootstrap's formatters, when one is assigned.
    pub formatter: Option<usize>,
}

/// Select the files the bootstrap's `[hygiene]` grant names, sorted by
/// path. An empty result when no grant exists. A repository-wide scope
/// outside a Git repository, a declared root or file that is missing or
/// reached through a link, a path assigned to two formatters, or a count
/// above `limits.files` is a fatal outcome.
pub fn select(
    tree: &dyn ReadTree,
    universe: Universe<'_>,
    bootstrap: &Bootstrap,
    limits: &Limits,
) -> Result<Vec<Selected>, String> {
    let Some(hygiene) = &bootstrap.hygiene else {
        return Ok(Vec::new());
    };
    let mut candidates: BTreeSet<ProjectPath> = BTreeSet::new();
    match hygiene.scope {
        Scope::Repository => match universe {
            Universe::WorkingDirectory(root) => {
                let listed = git::working_files(root).map_err(|error| {
                    format!(
                        "`hygiene.scope = \"repository\"` needs the project inside a Git repository (declare `scope = \"declared\"` with roots and files otherwise): {error}"
                    )
                })?;
                for path in listed {
                    // Listed but deleted, replaced by a directory, or a link:
                    // not a regular file of this working directory.
                    match tree.symlink_component(&path) {
                        Ok(Some(_)) => continue,
                        Ok(None) => {}
                        Err(error) => {
                            return Err(format!("cannot inspect `{path}`: {error}"));
                        }
                    }
                    if tree.is_file(&path) {
                        candidates.insert(path);
                    }
                }
            }
            Universe::Frozen => {
                let walked = tree
                    .walk(&ProjectPath::root())
                    .map_err(|error| format!("cannot list the project files: {error}"))?;
                candidates.extend(walked);
            }
        },
        Scope::Declared => {
            for root in &hygiene.roots {
                if !tree.is_dir(root) {
                    return Err(format!(
                        "hygiene root `{root}` is not a directory inside the project"
                    ));
                }
                let walked = tree
                    .walk(root)
                    .map_err(|error| format!("cannot walk hygiene root `{root}`: {error}"))?;
                candidates.extend(walked);
            }
            for file in &hygiene.files {
                match tree.symlink_component(file) {
                    Ok(None) => {}
                    Ok(Some(link)) => {
                        return Err(format!(
                            "hygiene file `{file}` is reached through the symbolic link `{link}`; files must not be reached through links"
                        ));
                    }
                    Err(error) => {
                        return Err(format!("cannot inspect hygiene file `{file}`: {error}"));
                    }
                }
                if !tree.is_file(file) {
                    return Err(format!(
                        "hygiene file `{file}` is not a file inside the project"
                    ));
                }
                candidates.insert(file.clone());
            }
        }
    }

    let within =
        |path: &ProjectPath, list: &[ProjectPath]| list.iter().any(|entry| path.is_within(entry));
    let mut selected = Vec::new();
    for path in candidates {
        if within(&path, &hygiene.exclude) {
            continue;
        }
        let binary = if within(&path, &hygiene.binary) {
            Some(true)
        } else if within(&path, &hygiene.text) {
            Some(false)
        } else {
            None
        };
        let formatter = assign(&path, &bootstrap.formatters)?;
        selected.push(Selected {
            path,
            binary,
            formatter,
        });
    }
    if selected.len() > limits.files {
        return Err(format!(
            "{} selected files exceed `limits.files` = {}",
            selected.len(),
            limits.files
        ));
    }
    Ok(selected)
}

/// The one formatter assigned to `path`, if any. Two matching formatters
/// are a configuration error.
fn assign(path: &ProjectPath, formatters: &[Formatter]) -> Result<Option<usize>, String> {
    let matching: Vec<usize> = formatters
        .iter()
        .enumerate()
        .filter(|(_, formatter)| {
            (formatter.paths.is_empty() || formatter.paths.iter().any(|root| path.is_within(root)))
                && (formatter.extensions.is_empty()
                    || path.extension().is_some_and(|extension| {
                        formatter.extensions.iter().any(|e| e == extension)
                    }))
        })
        .map(|(index, _)| index)
        .collect();
    match matching.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(*one)),
        [first, second, ..] => Err(format!(
            "`{path}` is assigned to formatters `{}` and `{}`; a file may have at most one formatter",
            formatters[*first].name, formatters[*second].name
        )),
    }
}
