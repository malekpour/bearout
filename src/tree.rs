// SPDX-License-Identifier: Apache-2.0

//! The read-only view of a project tree.
//!
//! Every phase of a run except delivery reads the project through
//! [`ReadTree`]. Two implementations exist:
//!
//! - [`crate::fs::WorkingDir`] reads the live working directory through a
//!   `cap-std` capability. It may change concurrently with the run; Bearout
//!   makes no snapshot of it.
//! - [`crate::git::GitTree`] reads a frozen Git tree: the index as captured
//!   at the start of the run, or one resolved revision. Its paths, modes, and
//!   object identities cannot change during the run, whatever happens to the
//!   working directory, the index, or the branch afterwards.
//!
//! The interface deliberately carries no write or delete operation. Writes
//! go through the separate delivery capability in [`crate::fs`], which only
//! the working directory provides.
//!
//! Semantics shared by every implementation:
//!
//! - paths are [`ProjectPath`]s relative to the tree's root;
//! - `read`, `read_text`, `file_len`, `is_file`, `is_dir`, and `exists`
//!   follow symbolic links, but only inside the tree; a link that leaves it
//!   is an error, never a read of something outside;
//! - `walk` never follows or reports symbolic links, never descends into a
//!   submodule, and fails on a name that is not a portable project path;
//! - `symlink_component` inspects the path literally, without following.

use std::io;
use std::sync::Arc;

use crate::paths::ProjectPath;

/// Read-only access to one project tree.
pub trait ReadTree: Send + Sync {
    /// The bytes of a regular file.
    fn read(&self, path: &ProjectPath) -> io::Result<Vec<u8>>;

    /// The UTF-8 text of a regular file. Invalid UTF-8 is
    /// [`io::ErrorKind::InvalidData`].
    fn read_text(&self, path: &ProjectPath) -> io::Result<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// The size of a regular file in bytes.
    fn file_len(&self, path: &ProjectPath) -> io::Result<u64>;

    /// `true` when `path` names a regular file, following links.
    fn is_file(&self, path: &ProjectPath) -> bool;

    /// `true` when `path` names a directory, following links. The root is a
    /// directory.
    fn is_dir(&self, path: &ProjectPath) -> bool;

    /// `true` when `path` names anything, following links.
    fn exists(&self, path: &ProjectPath) -> bool;

    /// The first component of `path`, from the root downward, that is a
    /// symbolic link, if any. Missing components end the search.
    fn symlink_component(&self, path: &ProjectPath) -> io::Result<Option<ProjectPath>>;

    /// Every regular file beneath `directory`, sorted, without following
    /// symbolic links or entering submodules. An entry whose name is not
    /// valid UTF-8 or not a portable project path segment is an error,
    /// never silently skipped.
    fn walk(&self, directory: &ProjectPath) -> io::Result<Vec<ProjectPath>>;

    /// A tree rooted at `directory`. Paths are then relative to that
    /// directory and symbolic links resolve only inside it.
    fn subtree(&self, directory: &ProjectPath) -> io::Result<Arc<dyn ReadTree>>;
}
