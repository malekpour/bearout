// SPDX-License-Identifier: Apache-2.0

//! The immutable history view repository history checks receive.
//!
//! One dict: `kind` (`range` or `message`), `base` and `head` (each
//! `{revision, id}` or `None`), and `commits`, oldest first, each with
//! `key` (the full identity, or `pending`), `id`, `pending`, `tree`,
//! ordered `parents`, the derived `merge`, raw `author` and `committer`
//! identities (`{name, email, timestamp, timezone}`, the committer `None`
//! for a pending commit), the exact `message`, its first line as
//! `subject`, `changes` sorted by repository path, and `change_basis`.
//! Nothing here interprets a message: headers, trailers, sign-offs,
//! breaking-change markers, and autosquash prefixes are the repository
//! policy's to read with ordinary string operations.

use serde_json::{Value, json};
use starlark::values::OwnedFrozenValue;

use super::capture::{Change, Commit, EntrySide, History, Identity};
use crate::git::Kind;
use crate::policy::views::freeze_json;

fn identity_json(identity: &Identity) -> Value {
    json!({
        "name": identity.name,
        "email": identity.email,
        "timestamp": identity.timestamp,
        "timezone": identity.timezone,
    })
}

fn side_json(side: &EntrySide) -> Value {
    json!({
        "mode": side.mode,
        "object": side.object.as_str(),
        "kind": match side.kind {
            Kind::File => "file",
            Kind::Executable => "executable",
            Kind::Symlink => "symlink",
            Kind::Gitlink => "gitlink",
            Kind::Directory => "directory",
        },
    })
}

fn change_json(change: &Change) -> Value {
    json!({
        "repository_path": change.repository_path,
        "project_path": change.project_path,
        "change": change.change.as_str(),
        "before": change.before.as_ref().map(side_json),
        "after": change.after.as_ref().map(side_json),
    })
}

fn commit_json(commit: &Commit) -> Value {
    json!({
        "key": commit.key,
        "id": commit.id.as_ref().map(ToString::to_string),
        "pending": commit.pending,
        "tree": commit.tree.as_ref().map(ToString::to_string),
        "parents": commit.parents.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "merge": commit.is_merge(),
        "author": identity_json(&commit.author),
        "committer": commit.committer.as_ref().map(identity_json),
        "message": commit.message,
        "subject": commit.subject(),
        "changes": commit.changes.iter().map(change_json).collect::<Vec<_>>(),
        "change_basis": commit.change_basis.as_ref().map(ToString::to_string),
    })
}

/// The view as JSON, the source of both the Starlark value and any
/// machine rendering.
#[must_use]
pub fn json(history: &History) -> Value {
    let reference = |reference: &super::capture::Reference| json!({ "revision": reference.revision, "id": reference.id.as_str() });
    json!({
        "kind": history.mode.as_str(),
        "base": history.base.as_ref().map(reference),
        "head": history.head.as_ref().map(reference),
        "commits": history.commits.iter().map(commit_json).collect::<Vec<_>>(),
    })
}

/// The frozen Starlark value of the view.
pub fn frozen(history: &History) -> Result<OwnedFrozenValue, String> {
    freeze_json(&json(history)).map_err(|error| format!("cannot build the history view: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ObjectId;
    use crate::history::capture::{ChangeKind, Mode, Reference, parse_identity};

    #[test]
    fn the_view_carries_every_fact() {
        let id = ObjectId::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        let parent = ObjectId::parse("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
        let history = History {
            mode: Mode::Range,
            base: Some(Reference {
                revision: "main".to_owned(),
                id: parent.clone(),
            }),
            head: Some(Reference {
                revision: "HEAD".to_owned(),
                id: id.clone(),
            }),
            commits: vec![Commit {
                key: id.to_string(),
                id: Some(id.clone()),
                pending: false,
                tree: Some(parent.clone()),
                parents: vec![parent.clone(), id.clone()],
                author: parse_identity("A <a@x> 1 +0100").unwrap(),
                committer: Some(parse_identity("C <c@x> 2 -0200").unwrap()),
                message: "subject\n\nbody\n".to_owned(),
                changes: vec![Change {
                    repository_path: "pkg/a.md".to_owned(),
                    project_path: Some("a.md".to_owned()),
                    change: ChangeKind::Modified,
                    before: Some(EntrySide {
                        mode: "100644".to_owned(),
                        object: parent.clone(),
                        kind: Kind::File,
                    }),
                    after: Some(EntrySide {
                        mode: "100755".to_owned(),
                        object: id.clone(),
                        kind: Kind::Executable,
                    }),
                }],
                change_basis: Some(parent.clone()),
            }],
        };
        let view = json(&history);
        assert_eq!(view["kind"], "range");
        assert_eq!(view["base"]["revision"], "main");
        assert_eq!(view["head"]["id"], id.as_str());
        let commit = &view["commits"][0];
        assert_eq!(commit["key"], id.as_str());
        assert_eq!(commit["merge"], true);
        assert_eq!(commit["parents"].as_array().unwrap().len(), 2);
        assert_eq!(commit["author"]["timezone"], "+0100");
        assert_eq!(commit["committer"]["timestamp"], 2);
        assert_eq!(commit["subject"], "subject");
        assert_eq!(commit["message"], "subject\n\nbody\n");
        assert_eq!(commit["change_basis"], parent.as_str());
        let change = &commit["changes"][0];
        assert_eq!(change["change"], "modified");
        assert_eq!(change["before"]["kind"], "file");
        assert_eq!(change["after"]["mode"], "100755");
        assert_eq!(change["project_path"], "a.md");
        assert!(frozen(&history).is_ok());

        let pending = History {
            mode: Mode::Message,
            base: None,
            head: None,
            commits: vec![Commit {
                key: "pending".to_owned(),
                id: None,
                pending: true,
                tree: None,
                parents: Vec::new(),
                author: parse_identity("A <a@x> 1 +0000").unwrap(),
                committer: None,
                message: String::new(),
                changes: Vec::new(),
                change_basis: None,
            }],
        };
        let view = json(&pending);
        assert_eq!(view["kind"], "message");
        assert!(view["base"].is_null() && view["head"].is_null());
        let commit = &view["commits"][0];
        assert!(commit["id"].is_null());
        assert!(commit["committer"].is_null());
        assert!(commit["tree"].is_null());
        assert_eq!(commit["pending"], true);
        assert_eq!(commit["merge"], false);
        assert_eq!(commit["subject"], "");
    }
}
