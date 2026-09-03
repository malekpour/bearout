// SPDX-License-Identifier: Apache-2.0

//! The filesystem capability. Every read and write goes through a
//! `cap_std::fs::Dir` opened on the project root, so the kernel cannot reach
//! outside the project even if a path computation is wrong. Symlinks are
//! never followed during discovery and never written through.
//!
//! Writes go through `cap-tempfile`: a uniquely named sibling temporary file
//! opened exclusively (`create_new`, so an existing symlink at that name is
//! an error rather than a target), written through the open handle, and
//! renamed into place only after every byte is written. The temporary file
//! is removed on every failure path by its `Drop`.

use std::io;
use std::path::Path;

use std::io::Write;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use cap_tempfile::TempFile;

use crate::paths::ProjectPath;

/// The project root as a capability.
pub struct ProjectDir {
    dir: Dir,
}

impl ProjectDir {
    /// Open the project root. This is the only place ambient authority is used.
    pub fn open(root: &Path) -> io::Result<Self> {
        let dir = Dir::open_ambient_dir(root, ambient_authority())?;
        Ok(Self { dir })
    }

    pub fn read(&self, path: &ProjectPath) -> io::Result<Vec<u8>> {
        self.dir.read(path.to_native())
    }

    pub fn read_text(&self, path: &ProjectPath) -> io::Result<String> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    #[must_use]
    pub fn is_file(&self, path: &ProjectPath) -> bool {
        self.dir.is_file(path.to_native())
    }

    #[must_use]
    pub fn is_dir(&self, path: &ProjectPath) -> bool {
        path.as_str().is_empty() || self.dir.is_dir(path.to_native())
    }

    #[must_use]
    pub fn exists(&self, path: &ProjectPath) -> bool {
        self.dir.exists(path.to_native())
    }

    /// The size of a file in bytes.
    pub fn file_len(&self, path: &ProjectPath) -> io::Result<u64> {
        Ok(self.dir.metadata(path.to_native())?.len())
    }

    /// The first component of `path`, from the root downward, that is a
    /// symbolic link, if any.
    pub fn symlink_component(&self, path: &ProjectPath) -> io::Result<Option<ProjectPath>> {
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

    /// Every regular file beneath `directory`, sorted, without following
    /// symbolic links. An entry whose name is not valid UTF-8 or not a valid
    /// project path segment is an error, never silently skipped.
    pub fn walk(&self, directory: &ProjectPath) -> io::Result<Vec<ProjectPath>> {
        let mut found = Vec::new();
        self.walk_into(directory, &mut found)?;
        found.sort();
        Ok(found)
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

    /// A capability on a directory beneath the root.
    pub fn subdir(&self, path: &ProjectPath) -> io::Result<Dir> {
        self.dir.open_dir(path.to_native())
    }
}
