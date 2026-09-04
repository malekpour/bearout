// SPDX-License-Identifier: Apache-2.0

//! The working-directory capability. Every read of the working directory
//! goes through a `cap_std::fs::Dir` opened on the project root, so the
//! kernel cannot reach outside the project even if a path computation is
//! wrong. Symlinks are never followed during discovery and never written
//! through.
//!
//! [`WorkingDir`] is the live filesystem: it may change while a run reads
//! it, and Bearout makes no snapshot. It is also the only source that can
//! hand out a [`Writer`], the delivery capability through which generation
//! changes the tree.
//!
//! Writes go through `cap-tempfile`: a uniquely named sibling temporary file
//! opened exclusively (`create_new`, so an existing symlink at that name is
//! an error rather than a target), written through the open handle, and
//! renamed into place only after every byte is written. The temporary file
//! is removed on every failure path by its `Drop`.

use std::io;
use std::path::Path;
use std::sync::Arc;

use std::io::Write;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use cap_tempfile::TempFile;

use crate::paths::ProjectPath;
use crate::tree::ReadTree;

/// The project root of the working directory as a capability.
pub struct WorkingDir {
    dir: Dir,
}

/// The delivery capability: the only handle through which Bearout writes or
/// removes a file. It exists only for the working directory.
pub struct Writer<'a> {
    dir: &'a Dir,
}

impl WorkingDir {
    /// Open the project root. This is the only place ambient authority is
    /// used for the working directory.
    pub fn open(root: &Path) -> io::Result<Self> {
        let dir = Dir::open_ambient_dir(root, ambient_authority())?;
        Ok(Self { dir })
    }

    /// The capability to change this working directory.
    #[must_use]
    pub fn writer(&self) -> Writer<'_> {
        Writer { dir: &self.dir }
    }

    fn walk_into(&self, directory: &ProjectPath, found: &mut Vec<ProjectPath>) -> io::Result<()> {
        let mut entries = Vec::new();
        for entry in self.dir.read_dir(directory.to_native())? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "`{directory}` contains an entry whose name is not valid UTF-8: {}",
                        name.to_string_lossy()
                    ),
                ));
            };
            let file_type = entry.file_type()?;
            entries.push((name.to_owned(), file_type));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, file_type) in entries {
            let segment = ProjectPath::parse(&name).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("`{directory}` contains an entry that is not a portable path segment: {error}"),
                )
            })?;
            let path = directory.join(&segment);
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                self.walk_into(&path, found)?;
            } else if file_type.is_file() {
                found.push(path);
            }
        }
        Ok(())
    }
}

impl ReadTree for WorkingDir {
    fn read(&self, path: &ProjectPath) -> io::Result<Vec<u8>> {
        self.dir.read(path.to_native())
    }

    fn read_bounded(&self, path: &ProjectPath, limit: u64) -> io::Result<(Vec<u8>, bool)> {
        use std::io::Read;
        let file = self.dir.open(path.to_native())?;
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
        let over = u64::try_from(bytes.len()).is_ok_and(|len| len > limit);
        Ok((bytes, over))
    }

    fn file_len(&self, path: &ProjectPath) -> io::Result<u64> {
        Ok(self.dir.metadata(path.to_native())?.len())
    }

    fn is_file(&self, path: &ProjectPath) -> bool {
        self.dir.is_file(path.to_native())
    }

    fn is_dir(&self, path: &ProjectPath) -> bool {
        path.as_str().is_empty() || self.dir.is_dir(path.to_native())
    }

    fn exists(&self, path: &ProjectPath) -> bool {
        self.dir.exists(path.to_native())
    }

    fn symlink_component(&self, path: &ProjectPath) -> io::Result<Option<ProjectPath>> {
        for ancestor in path.ancestors() {
            match self.dir.symlink_metadata(ancestor.to_native()) {
                Ok(metadata) if metadata.is_symlink() => return Ok(Some(ancestor)),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    fn walk(&self, directory: &ProjectPath) -> io::Result<Vec<ProjectPath>> {
        if let Some(link) = self.symlink_component(directory)? {
            return Err(linked_directory(&link));
        }
        let mut found = Vec::new();
        self.walk_into(directory, &mut found)?;
        found.sort();
        Ok(found)
    }

    fn subtree(&self, directory: &ProjectPath) -> io::Result<Arc<dyn ReadTree>> {
        let dir = if directory.as_str().is_empty() {
            self.dir.try_clone()?
        } else {
            self.dir.open_dir(directory.to_native())?
        };
        Ok(Arc::new(Self { dir }))
    }
}

/// The error for walking a directory reached through a symbolic link.
pub fn linked_directory(link: &ProjectPath) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("`{link}` is a symbolic link; directories are never walked through links"),
    )
}

impl Writer<'_> {
    /// Write `bytes` to `path` atomically: an exclusively created, uniquely
    /// named temporary file in the same directory receives every byte
    /// through its open handle and is then renamed over `path`. Readers see
    /// either the old content or the new content, never a partial file, and
    /// a symbolic link is never followed or installed. On failure the
    /// temporary file is removed and `path` is untouched.
    pub fn write_atomic(&self, path: &ProjectPath, bytes: &[u8]) -> io::Result<()> {
        let parent = path.parent();
        if !parent.as_str().is_empty() {
            self.dir.create_dir_all(parent.to_native())?;
        }
        let directory = if parent.as_str().is_empty() {
            self.dir.try_clone()?
        } else {
            self.dir.open_dir(parent.to_native())?
        };
        let mut temp = TempFile::new(&directory)?;
        temp.write_all(bytes)?;
        temp.as_file_mut().sync_data()?;
        temp.replace(path.file_name())
    }

    pub fn remove_file(&self, path: &ProjectPath) -> io::Result<()> {
        self.dir.remove_file(path.to_native())
    }

    /// Replace an existing regular file atomically, keeping its
    /// permissions (the executable bit included). The file must exist.
    pub fn replace_preserving(&self, path: &ProjectPath, bytes: &[u8]) -> io::Result<()> {
        let permissions = self.dir.metadata(path.to_native())?.permissions();
        let parent = path.parent();
        let directory = if parent.as_str().is_empty() {
            self.dir.try_clone()?
        } else {
            self.dir.open_dir(parent.to_native())?
        };
        let mut temp = TempFile::new(&directory)?;
        temp.write_all(bytes)?;
        temp.as_file_mut().set_permissions(permissions)?;
        temp.as_file_mut().sync_data()?;
        temp.replace(path.file_name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_reads_never_read_more_than_the_bound_plus_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.txt"), vec![b'x'; 10_000]).unwrap();
        std::fs::write(dir.path().join("small.txt"), b"tiny").unwrap();
        let working = WorkingDir::open(dir.path()).unwrap();
        let big = ProjectPath::parse("big.txt").unwrap();
        let (bytes, over) = working.read_bounded(&big, 100).unwrap();
        assert!(over);
        assert_eq!(
            bytes.len(),
            101,
            "the bound plus one probe byte, nothing more"
        );
        let (bytes, over) = working.read_bounded(&big, 10_000).unwrap();
        assert!(!over);
        assert_eq!(bytes.len(), 10_000);
        let small = ProjectPath::parse("small.txt").unwrap();
        let (bytes, over) = working.read_bounded(&small, 100).unwrap();
        assert!(!over);
        assert_eq!(bytes, b"tiny");
        assert!(
            working
                .read_bounded(&ProjectPath::parse("missing").unwrap(), 1)
                .is_err()
        );
    }
}
