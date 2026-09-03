// SPDX-License-Identifier: Apache-2.0

//! Project-relative paths. Every path the engine reports, compares, or hands
//! to the filesystem capability is a [`ProjectPath`]: forward-slash
//! separated, relative, and free of `.` and `..` segments, so it means the
//! same thing on every platform and cannot leave the project root.
//!
//! Logical paths use `/` everywhere: manifests, Starlark, the state
//! manifest, templates, and reports. A backslash is rejected rather than
//! rewritten, because it is an ordinary filename character on POSIX. Native
//! separators appear only at the filesystem boundary, in [`ProjectPath::to_native`].

use std::fmt;
use std::path::PathBuf;

use serde::Serialize;

/// A normalized project-relative path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProjectPath(String);

impl ProjectPath {
    /// The project root itself.
    #[must_use]
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Parse a path written by a person or a script. Only `/` separates
    /// segments. Absolute paths, drive letters, backslashes, empty
    /// segments, `.`, `..`, and control characters are rejected.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty() {
            return Ok(Self::root());
        }
        if text.starts_with('/') {
            return Err(format!(
                "`{text}` is absolute; paths must be relative to the project"
            ));
        }
        let mut segments = Vec::new();
        for segment in text.split('/') {
            match segment {
                "" => return Err(format!("`{text}` contains an empty path segment")),
                "." | ".." => {
                    return Err(format!(
                        "`{text}` contains `{segment}`; paths must be normalized"
                    ));
                }
                _ => {
                    check_segment(segment).map_err(|error| format!("`{text}`: {error}"))?;
                    segments.push(segment);
                }
            }
        }
        Ok(Self(segments.join("/")))
    }

    /// Resolve a link target written relative to `base_dir`, allowing `.`
    /// and `..` as long as they stay inside the project. Every other
    /// invariant of [`ProjectPath::parse`] holds for the result.
    pub fn resolve_relative(base_dir: &Self, target: &str) -> Result<Self, String> {
        let mut segments: Vec<&str> = if base_dir.0.is_empty() {
            Vec::new()
        } else {
            base_dir.0.split('/').collect()
        };
        if target.starts_with('/') {
            return Err(format!("`{target}` is absolute; links must be relative"));
        }
        for segment in target.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    if segments.pop().is_none() {
                        return Err(format!("`{target}` leaves the project"));
                    }
                }
                other => {
                    check_segment(other).map_err(|error| format!("`{target}`: {error}"))?;
                    segments.push(other);
                }
            }
        }
        Ok(Self(segments.join("/")))
    }

    /// The textual form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The directory containing this path, or the root.
    #[must_use]
    pub fn parent(&self) -> Self {
        match self.0.rfind('/') {
            Some(index) => Self(self.0[..index].to_owned()),
            None => Self::root(),
        }
    }

    /// The final segment.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or("")
    }

    /// The extension after the last `.` of the final segment, if any.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name();
        name.rfind('.')
            .filter(|index| *index > 0)
            .map(|index| &name[index + 1..])
    }

    /// Append one or more segments.
    #[must_use]
    pub fn join(&self, tail: &Self) -> Self {
        match (self.0.is_empty(), tail.0.is_empty()) {
            (true, _) => tail.clone(),
            (_, true) => self.clone(),
            _ => Self(format!("{}/{}", self.0, tail.0)),
        }
    }

    /// Returns `true` when `self` is `ancestor` or lies beneath it.
    #[must_use]
    pub fn is_within(&self, ancestor: &Self) -> bool {
        ancestor.0.is_empty()
            || self.0 == ancestor.0
            || self
                .0
                .strip_prefix(&ancestor.0)
                .is_some_and(|rest| rest.starts_with('/'))
    }

    /// The path relative to `ancestor`, when `self` lies beneath it.
    #[must_use]
    pub fn strip_prefix(&self, ancestor: &Self) -> Option<Self> {
        if ancestor.0.is_empty() {
            return Some(self.clone());
        }
        if self.0 == ancestor.0 {
            return Some(Self::root());
        }
        self.0
            .strip_prefix(&ancestor.0)
            .and_then(|rest| rest.strip_prefix('/'))
            .map(|rest| Self(rest.to_owned()))
    }

    /// The path in the platform's native form, for the filesystem capability.
    #[must_use]
    pub fn to_native(&self) -> PathBuf {
        if self.0.is_empty() {
            PathBuf::from(".")
        } else {
            self.0.split('/').collect()
        }
    }

    /// Every ancestor from the first segment down to the path itself.
    #[must_use]
    pub fn ancestors(&self) -> Vec<Self> {
        let mut result = Vec::new();
        let mut current = String::new();
        for segment in self.0.split('/').filter(|segment| !segment.is_empty()) {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(segment);
            result.push(Self(current.clone()));
        }
        result
    }

    /// A key for collision detection between outputs: Unicode lowercase
    /// folding of the path. This approximates the case-insensitive
    /// comparison of common Windows and macOS filesystems; it is not a full
    /// platform case-folding or normalization model, and two paths that a
    /// filesystem treats as one through Unicode normalization are not
    /// detected.
    #[must_use]
    pub fn fold_key(&self) -> String {
        self.0.to_lowercase()
    }
}

/// One path segment: no separators, no control characters, no `:`.
fn check_segment(segment: &str) -> Result<(), String> {
    if segment.contains('\\') {
        return Err("backslashes are not path separators; use `/`".to_owned());
    }
    if segment.chars().any(char::is_control) {
        return Err("control characters are not allowed in paths".to_owned());
    }
    if segment.contains(':') {
        return Err(
            "`:` is not allowed in paths; drive letters and streams are not portable".to_owned(),
        );
    }
    Ok(())
}

impl fmt::Display for ProjectPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str(".")
        } else {
            f.write_str(&self.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forward_slash_paths_only() {
        assert_eq!(ProjectPath::parse("a/b/c.md").unwrap().as_str(), "a/b/c.md");
        assert_eq!(ProjectPath::parse("").unwrap(), ProjectPath::root());
        assert!(
            ProjectPath::parse("a\\b/c.md")
                .unwrap_err()
                .contains("backslash")
        );
    }

    #[test]
    fn rejects_escapes_and_absolutes() {
        assert!(ProjectPath::parse("/etc/passwd").is_err());
        assert!(ProjectPath::parse("a/../b").is_err());
        assert!(ProjectPath::parse("./a").is_err());
        assert!(ProjectPath::parse("a//b").is_err());
        assert!(ProjectPath::parse("C:\\x").is_err());
        assert!(ProjectPath::parse("a\u{0}b").is_err());
    }

    #[test]
    fn resolves_relative_links_inside_the_project() {
        let base = ProjectPath::parse("docs/guides").unwrap();
        assert_eq!(
            ProjectPath::resolve_relative(&base, "../reference/x.md")
                .unwrap()
                .as_str(),
            "docs/reference/x.md"
        );
        assert_eq!(
            ProjectPath::resolve_relative(&base, "./y.md")
                .unwrap()
                .as_str(),
            "docs/guides/y.md"
        );
        assert!(ProjectPath::resolve_relative(&base, "../../../x").is_err());
        assert!(ProjectPath::resolve_relative(&base, "/x").is_err());
    }

    #[test]
    fn prefix_relations() {
        let root = ProjectPath::parse("generated").unwrap();
        assert!(
            ProjectPath::parse("generated/a.md")
                .unwrap()
                .is_within(&root)
        );
        assert!(
            !ProjectPath::parse("generated-x/a.md")
                .unwrap()
                .is_within(&root)
        );
        assert!(ProjectPath::parse("generated").unwrap().is_within(&root));
        assert_eq!(
            ProjectPath::parse("generated/a/b.md")
                .unwrap()
                .strip_prefix(&root)
                .unwrap()
                .as_str(),
            "a/b.md"
        );
        assert_eq!(
            ProjectPath::parse("a/b.c.md").unwrap().extension(),
            Some("md")
        );
        assert_eq!(ProjectPath::parse("a/b").unwrap().parent().as_str(), "a");
    }
}
