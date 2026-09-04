// SPDX-License-Identifier: Apache-2.0

//! Deterministic change facts between the candidate and the baseline over
//! the declared contract surface: the bootstrap, the discovered resources,
//! and the discovered schema-less documents of each side. This is not a
//! repository diff. Each side's surface records, for every file whose
//! bytes were actually read, the classification its own bootstrap gave the
//! path and the BLAKE3 digest of exactly those bytes, so a digest and the
//! parse it accompanies always come from the same read. A file that could
//! not be read has no entry.
//!
//! Paths are compared by name only: a rename is a removal plus an addition,
//! and a path whose classification differs between the sides is a
//! modification even when its bytes are equal. Unchanged paths are omitted.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{Value, json};

use crate::paths::ProjectPath;

/// What a side's bootstrap made of a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    Resource,
    Document,
    Manifest,
}

/// One read file of one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SurfaceEntry {
    pub classification: Classification,
    /// `blake3:` followed by 64 hexadecimal characters, over the bytes read.
    pub digest: String,
    /// Length of those bytes.
    pub bytes: u64,
}

impl SurfaceEntry {
    #[must_use]
    pub fn new(classification: Classification, bytes: &[u8]) -> Self {
        Self {
            classification,
            digest: digest(bytes),
            bytes: bytes.len() as u64,
        }
    }
}

/// Every read file of one side, by project path.
pub type Surface = BTreeMap<ProjectPath, SurfaceEntry>;

/// How a path differs between the sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Added,
    Removed,
    Modified,
}

/// One changed path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub path: ProjectPath,
    pub kind: Kind,
    pub before: Option<SurfaceEntry>,
    pub after: Option<SurfaceEntry>,
}

impl Change {
    /// The JSON view exposed to repository policy.
    #[must_use]
    pub fn view(&self) -> Value {
        json!({
            "path": self.path.as_str(),
            "change": self.kind,
            "before": self.before,
            "after": self.after,
        })
    }
}

/// `blake3:` digest of `bytes`.
#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    format!("blake3:{}", blake3::hash(bytes).to_hex())
}

/// The changes from `before` (the baseline) to `after` (the candidate), in
/// path order. Equal surfaces yield no changes.
#[must_use]
pub fn between(before: &Surface, after: &Surface) -> Vec<Change> {
    let mut paths: Vec<&ProjectPath> = before.keys().chain(after.keys()).collect();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter_map(|path| {
            let (old, new) = (before.get(path), after.get(path));
            let kind = match (old, new) {
                (None, Some(_)) => Kind::Added,
                (Some(_), None) => Kind::Removed,
                (Some(old), Some(new)) if old != new => Kind::Modified,
                _ => return None,
            };
            Some(Change {
                path: path.clone(),
                kind,
                before: old.cloned(),
                after: new.cloned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(entries: &[(&str, Classification, &[u8])]) -> Surface {
        entries
            .iter()
            .map(|(path, classification, bytes)| {
                (
                    ProjectPath::parse(path).unwrap(),
                    SurfaceEntry::new(*classification, bytes),
                )
            })
            .collect()
    }

    #[test]
    fn equal_surfaces_have_no_changes() {
        let a = surface(&[
            ("bearout.toml", Classification::Manifest, b"m"),
            ("records/a.md", Classification::Resource, b"a"),
            ("docs/x.md", Classification::Document, b"x"),
        ]);
        assert!(between(&a, &a).is_empty());
        assert!(between(&Surface::new(), &Surface::new()).is_empty());
    }

    #[test]
    fn additions_removals_modifications_and_reclassifications_sort_by_path() {
        let before = surface(&[
            ("bearout.toml", Classification::Manifest, b"m"),
            ("records/a.md", Classification::Resource, b"a"),
            ("records/gone.md", Classification::Resource, b"g"),
            ("notes/n.md", Classification::Document, b"n"),
        ]);
        let after = surface(&[
            ("bearout.toml", Classification::Manifest, b"m2"),
            ("records/a.md", Classification::Resource, b"a"),
            ("records/new.md", Classification::Resource, b"g"),
            ("notes/n.md", Classification::Resource, b"n"),
        ]);
        let changes = between(&before, &after);
        let summary: Vec<(&str, Kind)> = changes
            .iter()
            .map(|change| (change.path.as_str(), change.kind))
            .collect();
        assert_eq!(
            summary,
            [
                ("bearout.toml", Kind::Modified),
                ("notes/n.md", Kind::Modified),
                ("records/gone.md", Kind::Removed),
                ("records/new.md", Kind::Added),
            ]
        );
        let reclassified = &changes[1];
        assert_eq!(
            reclassified.before.as_ref().unwrap().classification,
            Classification::Document
        );
        assert_eq!(
            reclassified.after.as_ref().unwrap().classification,
            Classification::Resource
        );
        assert_eq!(
            reclassified.before.as_ref().unwrap().digest,
            reclassified.after.as_ref().unwrap().digest,
            "same bytes, different classification"
        );
        assert!(changes[2].after.is_none());
        assert!(changes[3].before.is_none());
        let json = changes[3].view();
        assert_eq!(json["change"], "added");
        assert_eq!(json["before"], Value::Null);
        assert_eq!(json["after"]["classification"], "resource");
        assert_eq!(json["after"]["bytes"], 1);
        assert!(
            json["after"]["digest"]
                .as_str()
                .unwrap()
                .starts_with("blake3:")
        );
    }
}
