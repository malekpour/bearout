// SPDX-License-Identifier: Apache-2.0

//! A read-only overlay over a selected tree: the virtual candidate a
//! fixture case checks.
//!
//! The overlay holds only what a case's mutations changed: the bytes of
//! written or replaced files, tombstones for deleted files and move
//! sources, and move destinations that read the source's bytes from the
//! unchanged base. Everything else falls through to the base tree, so no
//! repository copy is materialized and nothing is ever written. The
//! overlay implements the same observable semantics as the other sources:
//! sorted walks that never follow links or enter submodules, file and
//! directory existence, subtree confinement, bounded reads, and no
//! ambient filesystem access, since every read goes through the base.
//!
//! Mutations are validated in manifest order before any evaluation, over
//! the virtual state the earlier mutations produced: a path may be touched
//! once per case, a write replaces a regular file or creates one where
//! nothing exists, a delete and a move source must name a regular file of
//! the base, a move destination must not exist, and no touched path may
//! be, lie beneath, or be reached through a symbolic link, a submodule, or
//! a regular file. A write is the only operation that replaces content;
//! every other collision is refused.

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use crate::paths::ProjectPath;
use crate::tree::ReadTree;

/// One mutation of a case, in manifest order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mutation {
    /// Write or replace one regular file with these bytes.
    Write { path: ProjectPath, bytes: Arc<[u8]> },
    /// Delete one regular file of the base.
    Delete { path: ProjectPath },
    /// Move one regular file of the base to a path that does not exist.
    Move { from: ProjectPath, to: ProjectPath },
}

impl Mutation {
    /// Every path the mutation touches.
    fn touched(&self) -> Vec<&ProjectPath> {
        match self {
            Self::Write { path, .. } | Self::Delete { path } => vec![path],
            Self::Move { from, to } => vec![from, to],
        }
    }
}

/// What the overlay records at one path.
#[derive(Debug, Clone)]
enum Entry {
    /// A written or replaced file.
    Bytes(Arc<[u8]>),
    /// A deleted file, or the source of a move.
    Tombstone,
    /// A moved file: its bytes are the base's at the source path.
    Alias(ProjectPath),
}

/// The overlay's changes, shared by every view of it.
struct Changes {
    /// The unchanged base as a whole; aliases resolve against it.
    root: Arc<dyn ReadTree>,
    entries: BTreeMap<ProjectPath, Entry>,
}

/// A read-only view of the base tree with a case's mutations applied.
pub struct Overlay {
    changes: Arc<Changes>,
    /// The base at this view's directory; the whole base for the root
    /// view.
    base: Arc<dyn ReadTree>,
    /// This view's directory within the whole overlay.
    prefix: ProjectPath,
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("entries", &self.changes.entries.len())
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl Overlay {
    /// Validate `mutations` in order against `base` and build the overlay.
    /// Every refusal names the mutation's position (one-based) and path.
    pub fn build(base: Arc<dyn ReadTree>, mutations: &[Mutation]) -> Result<Self, String> {
        let mut entries: BTreeMap<ProjectPath, Entry> = BTreeMap::new();
        let mut touched: Vec<ProjectPath> = Vec::new();
        for (index, mutation) in mutations.iter().enumerate() {
            let position = index + 1;
            let refuse = |message: String| format!("mutation {position}: {message}");
            for path in mutation.touched() {
                if path.as_str().is_empty() {
                    return Err(refuse("the project root cannot be mutated".to_owned()));
                }
                if touched.contains(path) {
                    return Err(refuse(format!(
                        "`{path}` is touched by an earlier mutation of the same case; each path may be mutated once"
                    )));
                }
                for ancestor in path.ancestors() {
                    if ancestor == *path || ancestor.as_str().is_empty() {
                        continue;
                    }
                    let is_file = match entries.get(&ancestor) {
                        Some(Entry::Bytes(_) | Entry::Alias(_)) => true,
                        Some(Entry::Tombstone) => false,
                        None => base.is_file(&ancestor),
                    };
                    if is_file {
                        return Err(refuse(format!(
                            "`{path}` lies beneath `{ancestor}`, which is a file"
                        )));
                    }
                }
                if let Some(link) = base.symlink_component(path).map_err(|error| {
                    refuse(format!(
                        "cannot inspect `{path}` in the source tree: {error}"
                    ))
                })? {
                    return Err(refuse(format!(
                        "`{path}` is or lies beneath the symbolic link `{link}`; mutations never touch links"
                    )));
                }
            }
            let state = |path: &ProjectPath| -> State {
                match entries.get(path) {
                    Some(Entry::Bytes(_) | Entry::Alias(_)) => State::File,
                    Some(Entry::Tombstone) => State::Absent,
                    None if base.is_file(path) => State::File,
                    None => {
                        if base.exists(path) {
                            State::Other
                        } else {
                            State::Absent
                        }
                    }
                }
            };
            match mutation {
                Mutation::Write { path, bytes } => {
                    if state(path) == State::Other {
                        return Err(refuse(format!(
                            "`{path}` exists in the source tree but is not a regular file; only regular files are written"
                        )));
                    }
                    entries.insert(path.clone(), Entry::Bytes(Arc::clone(bytes)));
                }
                Mutation::Delete { path } => {
                    if state(path) != State::File {
                        return Err(refuse(format!(
                            "`{path}` is not a regular file of the source tree; only existing regular files are deleted"
                        )));
                    }
                    entries.insert(path.clone(), Entry::Tombstone);
                }
                Mutation::Move { from, to } => {
                    if from == to {
                        return Err(refuse(format!("`{from}` cannot be moved onto itself")));
                    }
                    if state(from) != State::File {
                        return Err(refuse(format!(
                            "`{from}` is not a regular file of the source tree; only existing regular files are moved"
                        )));
                    }
                    if state(to) != State::Absent {
                        return Err(refuse(format!(
                            "`{to}` already exists; a move never replaces anything"
                        )));
                    }
                    if to.is_within(from) {
                        return Err(refuse(format!("`{to}` lies beneath `{from}`")));
                    }
                    entries.insert(from.clone(), Entry::Tombstone);
                    entries.insert(to.clone(), Entry::Alias(from.clone()));
                }
            }
            touched.extend(mutation.touched().into_iter().cloned());
        }
        Ok(Self {
            changes: Arc::new(Changes {
                root: Arc::clone(&base),
                entries,
            }),
            base,
            prefix: ProjectPath::root(),
        })
    }

    /// The paths the overlay presents as files that the base does not:
    /// written files and move destinations, sorted.
    #[must_use]
    pub fn introduced(&self) -> Vec<ProjectPath> {
        self.changes
            .entries
            .iter()
            .filter(|(_, entry)| matches!(entry, Entry::Bytes(_) | Entry::Alias(_)))
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// The overlay entry at this view's `path`, if any.
    fn entry(&self, path: &ProjectPath) -> Option<&Entry> {
        self.changes.entries.get(&self.prefix.join(path))
    }

    /// Whether some live overlay entry lies strictly beneath this view's
    /// `directory`.
    fn holds_beneath(&self, directory: &ProjectPath) -> bool {
        let full = self.prefix.join(directory);
        self.changes.entries.iter().any(|(path, entry)| {
            matches!(entry, Entry::Bytes(_) | Entry::Alias(_))
                && *path != full
                && (full.as_str().is_empty() || path.is_within(&full))
        })
    }

    fn read_alias(&self, source: &ProjectPath) -> io::Result<Vec<u8>> {
        self.changes.root.read(source)
    }
}

/// A path's state in the virtual tree during validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    File,
    Absent,
    /// A directory, submodule, or anything else that is not a regular
    /// file.
    Other,
}

fn not_found(path: &ProjectPath) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("`{path}` does not exist"))
}

impl ReadTree for Overlay {
    fn read(&self, path: &ProjectPath) -> io::Result<Vec<u8>> {
        match self.entry(path) {
            Some(Entry::Bytes(bytes)) => Ok(bytes.to_vec()),
            Some(Entry::Alias(source)) => self.read_alias(source),
            Some(Entry::Tombstone) => Err(not_found(path)),
            None => self.base.read(path),
        }
    }

    fn read_bounded(&self, path: &ProjectPath, limit: u64) -> io::Result<(Vec<u8>, bool)> {
        match self.entry(path) {
            Some(Entry::Bytes(bytes)) => {
                // Held in memory already; the probe costs nothing extra.
                let over = u64::try_from(bytes.len()).is_ok_and(|len| len > limit);
                if over {
                    let probe = usize::try_from(limit.saturating_add(1)).unwrap_or(usize::MAX);
                    return Ok((bytes[..bytes.len().min(probe)].to_vec(), true));
                }
                Ok((bytes.to_vec(), false))
            }
            Some(Entry::Alias(source)) => self.changes.root.read_bounded(source, limit),
            Some(Entry::Tombstone) => Err(not_found(path)),
            None => self.base.read_bounded(path, limit),
        }
    }

    fn file_len(&self, path: &ProjectPath) -> io::Result<u64> {
        match self.entry(path) {
            Some(Entry::Bytes(bytes)) => Ok(bytes.len() as u64),
            Some(Entry::Alias(source)) => self.changes.root.file_len(source),
            Some(Entry::Tombstone) => Err(not_found(path)),
            None => self.base.file_len(path),
        }
    }

    fn is_file(&self, path: &ProjectPath) -> bool {
        match self.entry(path) {
            Some(Entry::Bytes(_) | Entry::Alias(_)) => true,
            Some(Entry::Tombstone) => false,
            None => self.base.is_file(path),
        }
    }

    fn is_dir(&self, path: &ProjectPath) -> bool {
        match self.entry(path) {
            Some(Entry::Bytes(_) | Entry::Alias(_) | Entry::Tombstone) => false,
            None => self.base.is_dir(path) || self.holds_beneath(path),
        }
    }

    fn exists(&self, path: &ProjectPath) -> bool {
        match self.entry(path) {
            Some(Entry::Bytes(_) | Entry::Alias(_)) => true,
            Some(Entry::Tombstone) => false,
            None => self.base.exists(path) || self.holds_beneath(path),
        }
    }

    fn symlink_component(&self, path: &ProjectPath) -> io::Result<Option<ProjectPath>> {
        // Validation refused every touched path that is or lies beneath a
        // link, and nothing beneath a file exists, so the base's answer is
        // the overlay's.
        self.base.symlink_component(path)
    }

    fn walk(&self, directory: &ProjectPath) -> io::Result<Vec<ProjectPath>> {
        if let Some(link) = self.base.symlink_component(directory)? {
            return Err(crate::fs::linked_directory(&link));
        }
        let mut found = if self.base.is_dir(directory) {
            self.base.walk(directory)?
        } else if self.holds_beneath(directory) {
            Vec::new()
        } else if self.exists(directory) {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("`{directory}` is not a directory"),
            ));
        } else {
            return Err(not_found(directory));
        };
        found.retain(|path| !matches!(self.entry(path), Some(Entry::Tombstone)));
        let full = self.prefix.join(directory);
        for (path, entry) in &self.changes.entries {
            if !matches!(entry, Entry::Bytes(_) | Entry::Alias(_)) {
                continue;
            }
            if (full.as_str().is_empty() || path.is_within(&full))
                && let Some(relative) = path.strip_prefix(&self.prefix)
                && !relative.as_str().is_empty()
            {
                found.push(relative);
            }
        }
        found.sort();
        found.dedup();
        Ok(found)
    }

    fn subtree(&self, directory: &ProjectPath) -> io::Result<Arc<dyn ReadTree>> {
        if directory.as_str().is_empty() {
            return Ok(Arc::new(Self {
                changes: Arc::clone(&self.changes),
                base: Arc::clone(&self.base),
                prefix: self.prefix.clone(),
            }));
        }
        if !self.is_dir(directory) {
            return Err(if self.exists(directory) {
                io::Error::new(
                    io::ErrorKind::NotADirectory,
                    format!("`{directory}` is not a directory"),
                )
            } else {
                not_found(directory)
            });
        }
        // A directory the base holds is viewed through the base's own
        // subtree, so links keep resolving only inside it; one that exists
        // only in the overlay has nothing beneath it in the base.
        let base: Arc<dyn ReadTree> = if self.base.is_dir(directory) {
            self.base.subtree(directory)?
        } else {
            Arc::new(Empty)
        };
        Ok(Arc::new(Self {
            changes: Arc::clone(&self.changes),
            base,
            prefix: self.prefix.join(directory),
        }))
    }
}

/// A tree with nothing in it: the base of an overlay directory that
/// exists only through written files.
struct Empty;

impl ReadTree for Empty {
    fn read(&self, path: &ProjectPath) -> io::Result<Vec<u8>> {
        Err(not_found(path))
    }

    fn read_bounded(&self, path: &ProjectPath, _: u64) -> io::Result<(Vec<u8>, bool)> {
        Err(not_found(path))
    }

    fn file_len(&self, path: &ProjectPath) -> io::Result<u64> {
        Err(not_found(path))
    }

    fn is_file(&self, _: &ProjectPath) -> bool {
        false
    }

    fn is_dir(&self, path: &ProjectPath) -> bool {
        path.as_str().is_empty()
    }

    fn exists(&self, path: &ProjectPath) -> bool {
        path.as_str().is_empty()
    }

    fn symlink_component(&self, _: &ProjectPath) -> io::Result<Option<ProjectPath>> {
        Ok(None)
    }

    fn walk(&self, directory: &ProjectPath) -> io::Result<Vec<ProjectPath>> {
        if directory.as_str().is_empty() {
            Ok(Vec::new())
        } else {
            Err(not_found(directory))
        }
    }

    fn subtree(&self, directory: &ProjectPath) -> io::Result<Arc<dyn ReadTree>> {
        if directory.as_str().is_empty() {
            Ok(Arc::new(Self))
        } else {
            Err(not_found(directory))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::WorkingDir;

    fn path(text: &str) -> ProjectPath {
        ProjectPath::parse(text).unwrap()
    }

    fn write(path: &str, text: &str) -> Mutation {
        Mutation::Write {
            path: ProjectPath::parse(path).unwrap(),
            bytes: Arc::from(text.as_bytes()),
        }
    }

    fn delete(path: &str) -> Mutation {
        Mutation::Delete {
            path: ProjectPath::parse(path).unwrap(),
        }
    }

    fn mv(from: &str, to: &str) -> Mutation {
        Mutation::Move {
            from: ProjectPath::parse(from).unwrap(),
            to: ProjectPath::parse(to).unwrap(),
        }
    }

    /// A working directory with `docs/a.md`, `docs/b.md`, `docs/sub/c.md`,
    /// `top.txt`, and on Unix a link `docs/link.md` to `a.md` and a linked
    /// directory `linked` to `docs`.
    fn base() -> (tempfile::TempDir, Arc<dyn ReadTree>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs/sub")).unwrap();
        std::fs::write(dir.path().join("docs/a.md"), "a\n").unwrap();
        std::fs::write(dir.path().join("docs/b.md"), "b\n").unwrap();
        std::fs::write(dir.path().join("docs/sub/c.md"), "c\n").unwrap();
        std::fs::write(dir.path().join("top.txt"), "top\n").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("a.md", dir.path().join("docs/link.md")).unwrap();
            std::os::unix::fs::symlink("docs", dir.path().join("linked")).unwrap();
        }
        let working = WorkingDir::open(dir.path()).unwrap();
        (dir, Arc::new(working))
    }

    fn strings(paths: &[ProjectPath]) -> Vec<&str> {
        paths.iter().map(ProjectPath::as_str).collect()
    }

    #[test]
    fn writes_deletes_and_moves_are_visible_and_the_base_is_untouched() {
        let (dir, base) = base();
        let overlay = Overlay::build(
            Arc::clone(&base),
            &[
                write("docs/new/d.md", "d\n"),
                write("docs/b.md", "B\n"),
                delete("docs/a.md"),
                mv("docs/sub/c.md", "moved/c.md"),
            ],
        )
        .unwrap();
        assert_eq!(overlay.read(&path("docs/new/d.md")).unwrap(), b"d\n");
        assert_eq!(overlay.read(&path("docs/b.md")).unwrap(), b"B\n");
        assert!(overlay.read(&path("docs/a.md")).is_err());
        assert!(!overlay.exists(&path("docs/a.md")));
        assert_eq!(overlay.read(&path("moved/c.md")).unwrap(), b"c\n");
        assert_eq!(overlay.file_len(&path("moved/c.md")).unwrap(), 2);
        assert!(!overlay.exists(&path("docs/sub/c.md")));
        assert!(
            overlay.is_dir(&path("docs/new")),
            "a written directory exists"
        );
        assert!(overlay.is_dir(&path("moved")));
        assert!(
            overlay.is_dir(&path("docs/sub")),
            "the base directory remains"
        );
        assert!(overlay.exists(&path("moved")));
        assert!(!overlay.is_file(&path("moved")));
        assert_eq!(overlay.read(&path("top.txt")).unwrap(), b"top\n");
        assert_eq!(
            strings(&overlay.walk(&path("docs")).unwrap()),
            ["docs/b.md", "docs/new/d.md"]
        );
        assert_eq!(
            strings(&overlay.walk(&ProjectPath::root()).unwrap()),
            ["docs/b.md", "docs/new/d.md", "moved/c.md", "top.txt"]
        );
        assert_eq!(
            strings(&overlay.walk(&path("moved")).unwrap()),
            ["moved/c.md"]
        );
        assert!(overlay.walk(&path("docs/sub")).unwrap().is_empty());
        assert!(overlay.walk(&path("nowhere")).is_err());
        assert!(overlay.walk(&path("top.txt")).is_err());
        assert_eq!(
            strings(&overlay.introduced()),
            ["docs/b.md", "docs/new/d.md", "moved/c.md"]
        );

        // Nothing on disk changed.
        assert_eq!(std::fs::read(dir.path().join("docs/a.md")).unwrap(), b"a\n");
        assert_eq!(std::fs::read(dir.path().join("docs/b.md")).unwrap(), b"b\n");
        assert!(dir.path().join("docs/sub/c.md").exists());
        assert!(!dir.path().join("docs/new").exists());
        assert!(!dir.path().join("moved").exists());
        assert_eq!(
            strings(&base.walk(&path("docs")).unwrap()),
            ["docs/a.md", "docs/b.md", "docs/sub/c.md"]
        );
    }

    #[test]
    fn bounded_reads_report_the_bytes_pulled() {
        let (_dir, base) = base();
        let overlay = Overlay::build(
            base,
            &[write("big.txt", "0123456789"), mv("docs/a.md", "alias.md")],
        )
        .unwrap();
        let (bytes, over) = overlay.read_bounded(&path("big.txt"), 4).unwrap();
        assert!(over);
        assert_eq!(bytes.len(), 5, "the bound plus one probe byte");
        let (bytes, over) = overlay.read_bounded(&path("big.txt"), 10).unwrap();
        assert!(!over);
        assert_eq!(bytes, b"0123456789");
        let (bytes, over) = overlay.read_bounded(&path("alias.md"), 1).unwrap();
        assert!(over);
        assert_eq!(bytes.len(), 2);
        let (bytes, over) = overlay.read_bounded(&path("top.txt"), 100).unwrap();
        assert!(!over);
        assert_eq!(bytes, b"top\n");
        assert!(overlay.read_bounded(&path("docs/a.md"), 100).is_err());
    }

    #[test]
    fn subtrees_confine_and_carry_the_changes() {
        let (_dir, base) = base();
        let overlay = Overlay::build(
            base,
            &[
                write("docs/new/d.md", "d\n"),
                delete("docs/a.md"),
                mv("top.txt", "docs/top.txt"),
                write("fresh/only.md", "only\n"),
            ],
        )
        .unwrap();
        let docs = overlay.subtree(&path("docs")).unwrap();
        assert_eq!(
            strings(&docs.walk(&ProjectPath::root()).unwrap()),
            ["b.md", "new/d.md", "sub/c.md", "top.txt"]
        );
        assert_eq!(docs.read(&path("top.txt")).unwrap(), b"top\n");
        assert!(!docs.exists(&path("a.md")));
        assert!(docs.is_dir(&path("new")));
        assert_eq!(docs.read(&path("new/d.md")).unwrap(), b"d\n");
        let new = docs.subtree(&path("new")).unwrap();
        assert_eq!(strings(&new.walk(&ProjectPath::root()).unwrap()), ["d.md"]);
        assert!(new.is_dir(&ProjectPath::root()));
        // A directory that exists only through a write has an empty base.
        let fresh = overlay.subtree(&path("fresh")).unwrap();
        assert_eq!(
            strings(&fresh.walk(&ProjectPath::root()).unwrap()),
            ["only.md"]
        );
        assert!(!fresh.exists(&path("b.md")));
        assert!(fresh.subtree(&path("nothing")).is_err());
        assert!(overlay.subtree(&path("docs/a.md")).is_err());
        assert!(overlay.subtree(&path("missing")).is_err());
        assert!(overlay.subtree(&path("docs/b.md")).is_err(), "a file");
    }

    #[cfg(unix)]
    #[test]
    fn links_are_never_touched_or_followed_by_walks() {
        let (_dir, base) = base();
        for (mutation, expected) in [
            (write("docs/link.md", "x"), "symbolic link"),
            (delete("docs/link.md"), "symbolic link"),
            (write("linked/z.md", "x"), "symbolic link"),
            (mv("docs/a.md", "linked/a.md"), "symbolic link"),
            (delete("linked"), "symbolic link"),
        ] {
            let error = Overlay::build(Arc::clone(&base), &[mutation]).unwrap_err();
            assert!(error.contains(expected), "{error}");
        }
        let overlay = Overlay::build(base, &[write("docs/z.md", "z\n")]).unwrap();
        assert!(overlay.walk(&path("linked")).is_err());
        assert_eq!(
            overlay.symlink_component(&path("linked/z.md")).unwrap(),
            Some(path("linked"))
        );
        assert_eq!(
            strings(&overlay.walk(&path("docs")).unwrap()),
            ["docs/a.md", "docs/b.md", "docs/sub/c.md", "docs/z.md"],
            "the link is not walked"
        );
    }

    #[test]
    fn conflicting_and_impossible_mutations_are_refused_in_order() {
        let (_dir, base) = base();
        for (mutations, expected) in [
            (vec![write("", "x")], "project root"),
            (vec![write("docs", "x")], "not a regular file"),
            (vec![delete("docs")], "not a regular file"),
            (vec![delete("docs/missing.md")], "not a regular file"),
            (
                vec![delete("docs/a.md"), delete("docs/a.md")],
                "mutation 2: `docs/a.md` is touched",
            ),
            (
                vec![delete("docs/a.md"), write("docs/a.md", "x")],
                "mutation 2",
            ),
            (
                vec![write("docs/a.md", "x"), mv("docs/a.md", "x.md")],
                "mutation 2",
            ),
            (vec![mv("docs/a.md", "docs/b.md")], "already exists"),
            (vec![mv("docs/a.md", "docs")], "already exists"),
            (vec![mv("docs/a.md", "docs/a.md")], "onto itself"),
            (vec![mv("docs/missing.md", "x.md")], "not a regular file"),
            (vec![mv("docs", "elsewhere")], "not a regular file"),
            (vec![write("top.txt/inner.md", "x")], "which is a file"),
            (
                vec![write("new.md", "x"), write("new.md/inner.md", "y")],
                "which is a file",
            ),
            (vec![mv("docs/a.md", "top.txt/a.md")], "which is a file"),
            (
                vec![write("x.md", "x"), mv("docs/a.md", "x.md")],
                "mutation 2: `x.md` is touched",
            ),
            (
                vec![mv("docs/a.md", "moved.md"), delete("moved.md")],
                "mutation 2: `moved.md` is touched",
            ),
        ] {
            let error = Overlay::build(Arc::clone(&base), &mutations).unwrap_err();
            assert!(error.contains(expected), "{mutations:?}: {error}");
        }
        // A deleted path cannot be a move destination in the same case,
        // but a written file can be replaced only by itself.
        assert!(Overlay::build(Arc::clone(&base), &[write("docs/a.md", "1")]).is_ok());
        assert!(Overlay::build(Arc::clone(&base), &[mv("docs/a.md", "docs/sub/a.md")]).is_ok());
        assert!(
            Overlay::build(
                Arc::clone(&base),
                &[delete("docs/a.md"), write("docs/z.md", "z")]
            )
            .is_ok()
        );
    }

    #[test]
    fn each_overlay_starts_from_the_same_base() {
        let (_dir, base) = base();
        let first = Overlay::build(Arc::clone(&base), &[delete("docs/a.md")]).unwrap();
        let second = Overlay::build(Arc::clone(&base), &[write("docs/a.md", "A\n")]).unwrap();
        assert!(!first.exists(&path("docs/a.md")));
        assert_eq!(second.read(&path("docs/a.md")).unwrap(), b"A\n");
        assert_eq!(base.read(&path("docs/a.md")).unwrap(), b"a\n");
        assert!(first.introduced().is_empty());
    }
}
