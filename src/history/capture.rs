// SPDX-License-Identifier: Apache-2.0

//! Exact Git history facts, captured through the hardened Git runner.
//!
//! Two captures exist. A **range** resolves a head and an optional base
//! exactly once, takes the commits reachable from the head but not from
//! the base (every commit reachable from the head without a base), reads
//! each raw commit object through one bounded `cat-file --batch` process,
//! and lists each commit's changes against its first parent, or against
//! the empty tree for a root commit, without rename detection. A
//! **pending** capture describes the commit a `commit-msg` hook is about
//! to make: the exact message file, the author identity Git would use, the
//! parents (`HEAD` and any `MERGE_HEAD`), and the staged changes of one
//! captured index against `HEAD`.
//!
//! Facts are raw: identities are the commit object's own, without
//! `.mailmap`; messages are byte-exact UTF-8; parents are ordered; merges,
//! fixups, reverts, and root commits are all present. The commit set is
//! Git reachability; the order is Bearout's, oldest first, with the full
//! object identity breaking ties among simultaneously eligible commits.
//! Everything read is charged against `limits.history_bytes`, a commit
//! object larger than `limits.history_commit_bytes` is rejected from its
//! announced size before it is loaded, and a shallow boundary inside the
//! set is refused rather than described as complete history.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::bootstrap::Limits;
use crate::git::{self, Batch, Git, GitTree, IndexSnapshot, Kind, ObjectId};
use crate::report::SourceInfo;

/// Which capture a history describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Committed history: a range of commits.
    Range,
    /// One pending commit from a message file and the captured index.
    Message,
}

impl Mode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Range => "range",
            Self::Message => "message",
        }
    }
}

/// A revision as supplied and as resolved, exactly once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub revision: String,
    pub id: ObjectId,
}

/// A raw Git identity line: `Name <email> <timestamp> <timezone>`. A
/// committed identity carries its exact time; a pending identity has
/// none, because Git would only invent the current clock for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
    pub timestamp: Option<i64>,
    pub timezone: Option<String>,
}

/// One side of a changed path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntrySide {
    /// The six-digit octal mode as Git printed it.
    pub mode: String,
    pub object: ObjectId,
    pub kind: Kind,
}

/// How a path changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    TypeChanged,
}

impl ChangeKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::Modified => "modified",
            Self::TypeChanged => "type-changed",
        }
    }
}

/// One changed path of a commit, relative to its change basis.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_field_names)]
pub struct Change {
    /// The path from the repository root.
    pub repository_path: String,
    /// The same path from the Bearout project root, when it lies inside
    /// the project.
    pub project_path: Option<String>,
    pub change: ChangeKind,
    pub before: Option<EntrySide>,
    pub after: Option<EntrySide>,
}

/// One commit of the history: committed, or pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The full object identity, or `pending`.
    pub key: String,
    pub id: Option<ObjectId>,
    pub pending: bool,
    pub tree: Option<ObjectId>,
    pub parents: Vec<ObjectId>,
    pub author: Identity,
    /// Absent for a pending commit, whose committer Git decides later.
    pub committer: Option<Identity>,
    /// The exact message.
    pub message: String,
    pub changes: Vec<Change>,
    /// What the changes are relative to: the first parent, or `None` for
    /// a root commit or an unborn branch, meaning the empty tree.
    pub change_basis: Option<ObjectId>,
}

impl Commit {
    /// `true` when the commit has more than one parent.
    #[must_use]
    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }

    /// The first logical line of the message, without its terminator.
    #[must_use]
    pub fn subject(&self) -> &str {
        logical_lines(&self.message)
            .first()
            .copied()
            .unwrap_or_default()
    }

    /// The number of logical lines a message target may name.
    #[must_use]
    pub fn line_count(&self) -> u32 {
        u32::try_from(logical_lines(&self.message).len()).unwrap_or(u32::MAX)
    }
}

/// The logical lines of a message, with CRLF, LF, and a lone CR all
/// ending a line, as Bearout's text handling reads them; a terminator at
/// the very end closes the last line rather than opening an empty one.
/// The message itself is never changed.
#[must_use]
pub fn logical_lines(message: &str) -> Vec<&str> {
    let bytes = message.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < bytes.len() {
        let width = match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => 2,
            b'\r' | b'\n' => 1,
            _ => {
                index += 1;
                continue;
            }
        };
        lines.push(&message[start..index]);
        index += width;
        start = index;
    }
    if start < bytes.len() {
        lines.push(&message[start..]);
    }
    lines
}

/// The captured facts of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    pub mode: Mode,
    pub base: Option<Reference>,
    pub head: Option<Reference>,
    /// Oldest first; see the module documentation for the order.
    pub commits: Vec<Commit>,
}

/// A source opened for a capture: revisions resolved exactly once, the
/// policy tree, and how the report identifies it. The facts are read
/// afterwards by [`Opened::capture`], under the limits of the bootstrap
/// that tree holds, so a name is never resolved twice.
pub struct Opened {
    git: Git,
    prefix: Vec<u8>,
    mode: Mode,
    base: Option<Reference>,
    head: Option<Reference>,
    /// The captured index of a pending commit, kept until its staged
    /// changes are listed from the same snapshot.
    snapshot: Option<IndexSnapshot>,
    message_file: Option<PathBuf>,
    pub policy_tree: GitTree,
    pub source: SourceInfo,
}

/// Bytes read for the facts, charged against `limits.history_bytes`.
struct Budget {
    limit: u64,
    remaining: u64,
}

impl Budget {
    fn new(limits: &Limits) -> Self {
        Self {
            limit: limits.history_bytes,
            remaining: limits.history_bytes,
        }
    }

    fn charge(&mut self, bytes: u64, what: &str) -> Result<(), String> {
        match self.remaining.checked_sub(bytes) {
            Some(left) => {
                self.remaining = left;
                Ok(())
            }
            None => Err(self.exhausted(what)),
        }
    }

    fn exhausted(&self, what: &str) -> String {
        format!(
            "history inputs exceed `limits.history_bytes` = {} while reading {what}",
            self.limit
        )
    }

    /// The most one listing may pull now.
    fn listing_limit(&self) -> usize {
        usize::try_from(self.remaining).unwrap_or(usize::MAX)
    }

    /// Run one listing within what remains and charge it.
    fn listing(&mut self, git: &Git, args: &[&str], what: &str) -> Result<Vec<u8>, String> {
        let output = git
            .output_within(args, self.listing_limit())
            .map_err(|error| format!("cannot list {what}: {error}"))?;
        self.charge(output.len() as u64, what)?;
        Ok(output)
    }
}

/// Reject a revision name Git might read as an option or that carries
/// control characters, before it reaches the command line.
fn check_revision_name(revision: &str) -> Result<(), String> {
    if revision.is_empty() || revision.starts_with('-') || revision.chars().any(char::is_control) {
        return Err(format!(
            "`{}` is not a revision name",
            revision
                .chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
        ));
    }
    Ok(())
}

/// Resolve `revision` exactly once to a commit: a commit, or a tag that
/// peels to one. A tree, a blob, an unknown or ambiguous name, and an
/// identity no object bears are errors.
fn resolve_commit(git: &Git, revision: &str) -> Result<ObjectId, String> {
    check_revision_name(revision)?;
    let object = git
        .line(&["rev-parse", "--verify", "-q", "--end-of-options", revision])
        .map_err(|_| format!("`{revision}` is not a revision of this repository"))?;
    let object = ObjectId::parse(&object)?;
    let object_type = git
        .object_type(&object)
        .map_err(|_| format!("`{revision}` is not a revision of this repository"))?;
    match object_type.as_str() {
        "commit" => Ok(object),
        "tag" => {
            let peeled = git
                .line(&[
                    "rev-parse",
                    "--verify",
                    "-q",
                    &format!("{object}^{{commit}}"),
                ])
                .map_err(|_| format!("`{revision}` is a tag that does not point at a commit"))?;
            ObjectId::parse(&peeled)
        }
        other => Err(format!("`{revision}` names a {other}, not a commit")),
    }
}

/// Open a range: resolve `head` and `base` exactly once and read the
/// policy tree of the resolved head.
pub fn range(root: &Path, base: Option<&str>, head: &str) -> Result<Opened, String> {
    let git = Git::new(root).map_err(|error| error.to_string())?;
    let location = git.locate().map_err(|error| error.to_string())?;
    let head_id = resolve_commit(&git, head)?;
    let base = match base {
        Some(base) => Some(Reference {
            revision: base.to_owned(),
            id: resolve_commit(&git, base)?,
        }),
        None => None,
    };
    let (policy_tree, tree_id) = GitTree::revision(root, head_id.as_str())
        .map_err(|error| format!("cannot read the head tree: {error}"))?;
    let source = SourceInfo {
        kind: "revision".to_owned(),
        revision: Some(head.to_owned()),
        tree: Some(tree_id.to_string()),
        digest: policy_tree.digest().to_owned(),
    };
    Ok(Opened {
        git,
        prefix: location.prefix,
        mode: Mode::Range,
        base,
        head: Some(Reference {
            revision: head.to_owned(),
            id: head_id,
        }),
        snapshot: None,
        message_file: None,
        policy_tree,
        source,
    })
}

/// Open a pending commit: capture the index once, as the policy tree and
/// as the source of the staged changes.
pub fn pending(root: &Path, message_file: &Path) -> Result<Opened, String> {
    let git = Git::new(root).map_err(|error| error.to_string())?;
    let location = git.locate().map_err(|error| error.to_string())?;
    let snapshot = IndexSnapshot::capture(&git).map_err(|error| error.to_string())?;
    let policy_tree = GitTree::index_from(&git, &snapshot)
        .map_err(|error| format!("cannot read the Git index: {error}"))?;
    let source = SourceInfo {
        kind: "index".to_owned(),
        revision: None,
        tree: None,
        digest: policy_tree.digest().to_owned(),
    };
    Ok(Opened {
        git,
        prefix: location.prefix,
        mode: Mode::Message,
        base: None,
        head: None,
        snapshot: Some(snapshot),
        message_file: Some(message_file.to_path_buf()),
        policy_tree,
        source,
    })
}

impl Opened {
    /// Read the facts under `limits`. The policy tree stays available
    /// afterwards; a pending commit's index snapshot is released.
    pub fn capture(&mut self, limits: &Limits) -> Result<History, String> {
        match self.mode {
            Mode::Range => self.capture_range(limits),
            Mode::Message => self.capture_pending(limits),
        }
    }

    /// The commits reachable from the head but not from the base.
    fn capture_range(&self, limits: &Limits) -> Result<History, String> {
        let git = &self.git;
        let head = self.head.as_ref().expect("a range has a head");
        let head_id = &head.id;
        let base_id = self.base.as_ref().map(|base| &base.id);
        let mut budget = Budget::new(limits);
        let shallow = shallow_boundaries(git, &mut budget)?;
        // The set: Git reachability, one listing, bounded by the commit limit
        // plus one so that an over-limit range is detected without listing
        // everything reachable.
        let max_count = limits.history_commits.saturating_add(1).to_string();
        let mut args = vec!["rev-list", "--max-count", &max_count, head_id.as_str()];
        let excluded = base_id.map(|id| format!("^{id}"));
        if let Some(excluded) = &excluded {
            args.push(excluded);
        }
        let listing = budget.listing(git, &args, "the commit range")?;
        let listing = String::from_utf8(listing)
            .map_err(|_| "git rev-list printed invalid UTF-8".to_owned())?;
        let mut ids = Vec::new();
        for line in listing.lines() {
            ids.push(ObjectId::parse(line.trim())?);
        }
        if ids.len() > limits.history_commits {
            return Err(format!(
                "the range holds more than `limits.history_commits` = {} commit(s)",
                limits.history_commits
            ));
        }
        let in_set: BTreeSet<&ObjectId> = ids.iter().collect();
        for id in &ids {
            if shallow.contains(id) {
                return Err(format!(
                    "commit {id} is a shallow boundary: the history reachable from `{}` is incomplete in this repository; deepen the clone or give a base above the boundary",
                    head.revision
                ));
            }
        }

        // Raw commit objects, each rejected from its announced size before it
        // is loaded.
        let mut objects: HashMap<ObjectId, RawCommit> = HashMap::new();
        if !ids.is_empty() {
            let mut batch = Batch::spawn(git, "--batch")
                .map_err(|error| git::spawn_error(&error).to_string())?;
            for id in &ids {
                let raw = read_commit(&mut batch, id, limits, &mut budget)?;
                objects.insert(id.clone(), raw);
            }
        }

        // Oldest first: a commit is eligible once every parent inside the set
        // is emitted; the smallest identity among the eligible goes first.
        let mut pending_parents: BTreeMap<&ObjectId, usize> = BTreeMap::new();
        let mut children: HashMap<&ObjectId, Vec<&ObjectId>> = HashMap::new();
        for id in &ids {
            let parents = &objects[id].parents;
            let inside = parents
                .iter()
                .filter(|parent| in_set.contains(parent))
                .count();
            pending_parents.insert(id, inside);
            for parent in parents {
                if let Some(parent) = in_set.get(parent) {
                    children.entry(parent).or_default().push(id);
                }
            }
        }
        let mut eligible: BTreeSet<&ObjectId> = pending_parents
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut order: Vec<&ObjectId> = Vec::with_capacity(ids.len());
        while let Some(next) = eligible.pop_first() {
            order.push(next);
            for child in children.get(next).into_iter().flatten() {
                let count = pending_parents
                    .get_mut(child)
                    .expect("every child is in the set");
                *count -= 1;
                if *count == 0 {
                    eligible.insert(child);
                }
            }
        }
        if order.len() != ids.len() {
            return Err(
                "the commit range is not acyclic; Git printed an inconsistent history".to_owned(),
            );
        }

        // Changes against the first parent, or the empty tree for a root.
        let empty_tree = empty_tree(git)?;
        let mut remaining_changes = limits.history_changes;
        let mut commits = Vec::with_capacity(order.len());
        for id in order {
            let raw = objects.remove(id).expect("read above");
            let basis = raw.parents.first().cloned();
            let listing = budget.listing(
                git,
                &[
                    "diff-tree",
                    "-r",
                    "-z",
                    "--raw",
                    "--full-index",
                    "--no-renames",
                    "--no-color",
                    "--ignore-submodules=none",
                    basis.as_ref().unwrap_or(&empty_tree).as_str(),
                    id.as_str(),
                ],
                &format!("the changes of commit {id}"),
            )?;
            let changes = parse_changes(&listing, &self.prefix)
                .map_err(|problem| format!("commit {id}: {problem}"))?;
            remaining_changes = remaining_changes
                .checked_sub(changes.len())
                .ok_or_else(|| {
                    format!(
                        "the range changes more than `limits.history_changes` = {} path(s)",
                        limits.history_changes
                    )
                })?;
            commits.push(Commit {
                key: id.to_string(),
                id: Some(id.clone()),
                pending: false,
                tree: Some(raw.tree),
                parents: raw.parents,
                author: raw.author,
                committer: Some(raw.committer),
                message: raw.message,
                changes,
                change_basis: basis,
            });
        }

        Ok(History {
            mode: Mode::Range,
            base: self.base.clone(),
            head: self.head.clone(),
            commits,
        })
    }
}

/// The commit `HEAD` names, or `None` only when Git positively proves an
/// unborn branch: `HEAD` is a symbolic reference to a branch that does
/// not exist yet. A `HEAD` that names an object which is missing or not
/// a commit, a detached `HEAD` that cannot be read, and every other
/// failure are errors.
fn pending_head(git: &Git) -> Result<Option<ObjectId>, String> {
    if let Ok(text) = git.line(&["rev-parse", "--verify", "-q", "HEAD"]) {
        let id = ObjectId::parse(&text)?;
        return match git.object_type(&id) {
            Ok(object_type) if object_type == "commit" => Ok(Some(id)),
            Ok(other) => Err(format!("HEAD names {id}, a {other} rather than a commit")),
            Err(_) => Err(format!(
                "HEAD names {id}, which is not an object of this repository"
            )),
        };
    }
    let target = git
        .line(&["symbolic-ref", "-q", "HEAD"])
        .map_err(|error| format!("cannot resolve HEAD: {error}"))?;
    if target.is_empty() || !target.starts_with("refs/") {
        return Err(format!(
            "cannot resolve HEAD: `{target}` is not a reference"
        ));
    }
    // Absence is proven only by exit 1 of `show-ref --verify`; exit 0 is
    // a reference that exists, and anything else (malformed packed
    // references, an unreadable store) is an operational failure. A
    // malformed loose reference is then excluded by asking `rev-parse` as
    // well: it must also answer exit 1 and say nothing, since Git warns
    // about a broken reference it ignores.
    let shown = git
        .probe(&["show-ref", "--verify", "-q", "--", &target])
        .map_err(|error| format!("cannot resolve HEAD: {error}"))?;
    match shown.status {
        Some(0) => {
            return Err(format!(
                "cannot resolve HEAD: `{target}` exists but does not name a commit"
            ));
        }
        Some(1) => {}
        _ => return Err(format!("cannot resolve HEAD: {}", shown.failure())),
    }
    let parsed = git
        .probe(&["rev-parse", "--verify", "-q", "--end-of-options", &target])
        .map_err(|error| format!("cannot resolve HEAD: {error}"))?;
    match parsed.status {
        Some(1) if parsed.stderr_empty => Ok(None),
        Some(1) => Err(format!(
            "cannot resolve HEAD: `{target}` is not readable as a reference: {}",
            parsed.stderr
        )),
        Some(0) => Err(format!(
            "cannot resolve HEAD: `{target}` exists but does not name a commit"
        )),
        _ => Err(format!("cannot resolve HEAD: {}", parsed.failure())),
    }
}

/// The width of a full object identity in this repository's object
/// format, so that an abbreviation is never accepted as an identity.
fn object_id_width(git: &Git) -> Result<usize, String> {
    match git
        .line(&["rev-parse", "--show-object-format"])
        .map_err(|error| format!("cannot determine the object format: {error}"))?
        .as_str()
    {
        "sha1" => Ok(40),
        "sha256" => Ok(64),
        other => Err(format!("unsupported object format `{other}`")),
    }
}

/// The parents a merge in progress adds: every line of `MERGE_HEAD`,
/// which, when it exists, must be a regular readable file holding full
/// identities of commits this repository has. Nothing about it is
/// ignored silently.
fn merge_heads(git: &Git, budget: &mut Budget) -> Result<Vec<ObjectId>, String> {
    let merge_head = git_path(git, "MERGE_HEAD")?;
    let metadata = match std::fs::symlink_metadata(&merge_head) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot inspect MERGE_HEAD: {error}")),
    };
    if metadata.is_symlink() {
        return Err(
            "MERGE_HEAD is a symbolic link; merge state is read only from a regular file"
                .to_owned(),
        );
    }
    if !metadata.is_file() {
        return Err("MERGE_HEAD is not a regular file".to_owned());
    }
    let bytes = read_file_within(&merge_head, budget.listing_limit(), "MERGE_HEAD")?;
    budget.charge(bytes.len() as u64, "MERGE_HEAD")?;
    let text = String::from_utf8(bytes).map_err(|_| "MERGE_HEAD is not valid UTF-8".to_owned())?;
    let width = object_id_width(git)?;
    let mut parents = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        if line.len() != width {
            return Err(format!(
                "MERGE_HEAD holds `{line}`, which is not a full commit identity of this repository's object format"
            ));
        }
        let id = ObjectId::parse(line).map_err(|error| format!("MERGE_HEAD: {error}"))?;
        match git.object_type(&id) {
            Ok(object_type) if object_type == "commit" => {}
            Ok(other) => {
                return Err(format!(
                    "MERGE_HEAD names {id}, a {other} rather than a commit"
                ));
            }
            Err(_) => {
                return Err(format!(
                    "MERGE_HEAD names {id}, which is not an object of this repository"
                ));
            }
        }
        parents.push(id);
    }
    if parents.is_empty() {
        return Err("MERGE_HEAD exists but names no commit".to_owned());
    }
    Ok(parents)
}

/// The commits at which this repository's history is cut off, from the
/// `shallow` file of its Git directory; empty for a complete repository.
fn shallow_boundaries(git: &Git, budget: &mut Budget) -> Result<BTreeSet<ObjectId>, String> {
    let shallow = git
        .line(&["rev-parse", "--is-shallow-repository"])
        .map_err(|error| error.to_string())?;
    if shallow != "true" {
        return Ok(BTreeSet::new());
    }
    let path = git_path(git, "shallow")?;
    let bytes = read_file_within(&path, budget.listing_limit(), "the shallow boundary list")?;
    budget.charge(bytes.len() as u64, "the shallow boundary list")?;
    let text = String::from_utf8(bytes)
        .map_err(|_| "the shallow boundary list is not valid UTF-8".to_owned())?;
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ObjectId::parse)
        .collect()
}

/// A path of the repository's Git directory, absolute. `--git-path`
/// answers relative to the directory Git ran in, the project root.
fn git_path(git: &Git, name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(
        git.line(&["rev-parse", "--git-path", name])
            .map_err(|error| error.to_string())?,
    );
    Ok(if path.is_absolute() {
        path
    } else {
        git.root().join(path)
    })
}

/// Read a file within `limit` bytes; more is an error, and nothing above
/// the limit plus one probe byte is ever held.
fn read_file_within(path: &Path, limit: usize, what: &str) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| format!("cannot read {what}: {error}"))?;
    match git::read_bounded(file, limit) {
        Ok((bytes, false)) => Ok(bytes),
        Ok((_, true)) => Err(format!("{what} is larger than {limit} bytes")),
        Err(error) => Err(format!("cannot read {what}: {error}")),
    }
}

/// The identity of the empty tree in this repository's hash algorithm.
fn empty_tree(git: &Git) -> Result<ObjectId, String> {
    let text = git
        .line(&["hash-object", "-t", "tree", "--stdin"])
        .map_err(|error| error.to_string())?;
    ObjectId::parse(&text)
}

/// A parsed commit object.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawCommit {
    tree: ObjectId,
    parents: Vec<ObjectId>,
    author: Identity,
    committer: Identity,
    message: String,
}

/// Read one commit object through the batch process: its announced size
/// must be within `limits.history_commit_bytes` and the remaining budget
/// before a byte of it is loaded.
fn read_commit(
    batch: &mut Batch,
    id: &ObjectId,
    limits: &Limits,
    budget: &mut Budget,
) -> Result<RawCommit, String> {
    let (object_type, size) = batch
        .header(id)
        .map_err(|error| format!("cannot read commit {id}: {error}"))?;
    if object_type != "commit" {
        return Err(format!("object {id} is a {object_type}, not a commit"));
    }
    if size > limits.history_commit_bytes {
        return Err(format!(
            "commit {id} is {size} bytes, above `limits.history_commit_bytes` = {}",
            limits.history_commit_bytes
        ));
    }
    if size > budget.remaining {
        return Err(budget.exhausted(&format!("commit {id}")));
    }
    let mut bytes = vec![0; usize::try_from(size).expect("bounded above")];
    let mut newline = [0; 1];
    batch
        .stdout
        .read_exact(&mut bytes)
        .and_then(|()| batch.stdout.read_exact(&mut newline))
        .map_err(|error| format!("cannot read commit {id}: {error}"))?;
    budget.charge(size, &format!("commit {id}"))?;
    parse_commit(&bytes).map_err(|problem| format!("commit {id}: {problem}"))
}

/// Parse a raw commit object: headers up to the first blank line, with
/// continuation lines (a leading space) belonging to the header above them
/// as in signatures and merge tags, then the exact message.
fn parse_commit(bytes: &[u8]) -> Result<RawCommit, String> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| "commit object is not valid UTF-8".to_owned())?;
    let (headers, message) = match text.split_once("\n\n") {
        Some((headers, message)) => (headers, message),
        None => (text.strip_suffix('\n').unwrap_or(text), ""),
    };
    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = None;
    let mut committer = None;
    let mut current: Option<(String, String)> = None;
    let mut collected: Vec<(String, String)> = Vec::new();
    for line in headers.split('\n') {
        if let Some(rest) = line.strip_prefix(' ') {
            match &mut current {
                Some((_, value)) => {
                    value.push('\n');
                    value.push_str(rest);
                }
                None => return Err("commit header begins with a continuation line".to_owned()),
            }
            continue;
        }
        if let Some(header) = current.take() {
            collected.push(header);
        }
        let (key, value) = line
            .split_once(' ')
            .ok_or_else(|| format!("commit header `{line}` has no value"))?;
        current = Some((key.to_owned(), value.to_owned()));
    }
    if let Some(header) = current.take() {
        collected.push(header);
    }
    for (key, value) in collected {
        match key.as_str() {
            "tree" => {
                if tree.is_some() {
                    return Err("commit names two trees".to_owned());
                }
                tree = Some(ObjectId::parse(&value)?);
            }
            "parent" => parents.push(ObjectId::parse(&value)?),
            "author" => {
                if author.is_some() {
                    return Err("commit names two authors".to_owned());
                }
                author =
                    Some(parse_identity(&value).map_err(|problem| format!("author {problem}"))?);
            }
            "committer" => {
                if committer.is_some() {
                    return Err("commit names two committers".to_owned());
                }
                committer =
                    Some(parse_identity(&value).map_err(|problem| format!("committer {problem}"))?);
            }
            "encoding" if !value.eq_ignore_ascii_case("utf-8") => {
                return Err(format!(
                    "commit declares encoding `{value}`; only UTF-8 commits are supported"
                ));
            }
            // Signatures, merge tags, and unknown headers carry no fact
            // the view exposes.
            _ => {}
        }
    }
    Ok(RawCommit {
        tree: tree.ok_or_else(|| "commit has no tree".to_owned())?,
        parents,
        author: author.ok_or_else(|| "commit has no author".to_owned())?,
        committer: committer.ok_or_else(|| "commit has no committer".to_owned())?,
        message: message.to_owned(),
    })
}

/// Parse `Name <email> <timestamp> <timezone>` exactly as Git wrote it: no
/// mailmap, no case folding, no whitespace changes inside the name.
pub fn parse_identity(text: &str) -> Result<Identity, String> {
    let (rest, timezone) = text
        .rsplit_once(' ')
        .ok_or_else(|| format!("identity `{text}` has no timezone"))?;
    let valid_zone = timezone.len() == 5
        && (timezone.starts_with('+') || timezone.starts_with('-'))
        && timezone[1..].bytes().all(|b| b.is_ascii_digit());
    if !valid_zone {
        return Err(format!(
            "identity `{text}` has an invalid timezone `{timezone}`"
        ));
    }
    let (rest, timestamp) = rest
        .rsplit_once(' ')
        .ok_or_else(|| format!("identity `{text}` has no timestamp"))?;
    let timestamp: i64 = timestamp
        .parse()
        .map_err(|_| format!("identity `{text}` has an invalid timestamp `{timestamp}`"))?;
    let rest = rest
        .strip_suffix('>')
        .ok_or_else(|| format!("identity `{text}` has no email"))?;
    let (name, email) = rest
        .split_once(" <")
        .or_else(|| rest.strip_prefix('<').map(|email| ("", email)))
        .ok_or_else(|| format!("identity `{text}` has no email"))?;
    if email.contains('<') || email.contains('>') || name.contains('<') || name.contains('>') {
        return Err(format!("identity `{text}` is malformed"));
    }
    Ok(Identity {
        name: name.to_owned(),
        email: email.to_owned(),
        timestamp: Some(timestamp),
        timezone: Some(timezone.to_owned()),
    })
}

/// A fixed, obviously synthetic author date for asking Git which name
/// and email it would record, so that the answer never depends on the
/// host clock; the date itself is discarded.
const SYNTHETIC_AUTHOR_DATE: &str = "@0 +0000";

/// Parse `--raw -z --full-index --no-renames` records into changes sorted
/// by repository path. Every path must be UTF-8 and portable.
fn parse_changes(listing: &[u8], prefix: &[u8]) -> Result<Vec<Change>, String> {
    let mut changes = Vec::new();
    let mut tokens = listing.split(|byte| *byte == 0);
    while let Some(header) = tokens.next() {
        if header.is_empty() {
            continue;
        }
        let header = std::str::from_utf8(header)
            .map_err(|_| "a change record header is not UTF-8".to_owned())?;
        let fields: Vec<&str> = header
            .strip_prefix(':')
            .ok_or_else(|| format!("change record `{header}` is not a raw header"))?
            .split(' ')
            .collect();
        let [old_mode, new_mode, old_object, new_object, status] = fields[..] else {
            return Err(format!(
                "change record `{header}` has {} fields, expected 5",
                fields.len()
            ));
        };
        let raw_path = tokens
            .next()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| format!("change record `{header}` has no path"))?;
        let repository_path = match git::parse_git_path(raw_path) {
            Ok(path) => path,
            Err((_, problem)) => return Err(format!("the changed paths {problem}")),
        };
        let side = |mode: &str, object: &str| -> Result<Option<EntrySide>, String> {
            if mode == "000000" {
                return Ok(None);
            }
            Ok(Some(EntrySide {
                mode: mode.to_owned(),
                object: ObjectId::parse(object)?,
                kind: Kind::from_mode(mode)?,
            }))
        };
        let before = side(old_mode, old_object)?;
        let after = side(new_mode, new_object)?;
        let change = match (status.chars().next(), &before, &after) {
            (Some('A'), None, Some(_)) => ChangeKind::Added,
            (Some('D'), Some(_), None) => ChangeKind::Removed,
            (Some('M'), Some(_), Some(_)) => ChangeKind::Modified,
            (Some('T'), Some(_), Some(_)) => ChangeKind::TypeChanged,
            _ => {
                return Err(format!(
                    "change record `{header}` pairs status `{status}` with modes {old_mode} and {new_mode}"
                ));
            }
        };
        let project_path = raw_path
            .strip_prefix(prefix)
            .filter(|_| prefix.is_empty() || raw_path.len() > prefix.len())
            .map(|relative| String::from_utf8_lossy(relative).into_owned());
        changes.push(Change {
            repository_path: repository_path.as_str().to_owned(),
            project_path,
            change,
            before,
            after,
        });
    }
    changes.sort_by(|a, b| a.repository_path.cmp(&b.repository_path));
    for pair in changes.windows(2) {
        if pair[0].repository_path == pair[1].repository_path {
            return Err(format!(
                "`{}` is listed twice among the changes",
                pair[0].repository_path
            ));
        }
    }
    Ok(changes)
}

impl Opened {
    /// The pending commit: the message at the named file, the author Git
    /// would use, the parents, and the staged changes of the captured
    /// index.
    fn capture_pending(&mut self, limits: &Limits) -> Result<History, String> {
        let git = &self.git;
        let message_file = self
            .message_file
            .as_deref()
            .expect("a pending commit has a message file");
        let snapshot = self
            .snapshot
            .take()
            .expect("a pending commit has a snapshot");
        let mut budget = Budget::new(limits);
        // The message: exactly the named file, a regular file inside this
        // repository's own Git directory, bounded before it is read.
        let git_dir = std::fs::canonicalize(
            git.line(&["rev-parse", "--absolute-git-dir"])
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("cannot resolve the Git directory: {error}"))?;
        let message_file = if message_file.is_absolute() {
            message_file.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|error| format!("cannot resolve the message file: {error}"))?
                .join(message_file)
        };
        let shown = message_file.display().to_string();
        let metadata = std::fs::symlink_metadata(&message_file)
            .map_err(|error| format!("cannot read the message file `{shown}`: {error}"))?;
        if metadata.is_symlink() {
            return Err(format!(
                "the message file `{shown}` is a symbolic link; the message is read only from a regular file"
            ));
        }
        if !metadata.is_file() {
            return Err(format!("the message file `{shown}` is not a regular file"));
        }
        let canonical = std::fs::canonicalize(&message_file)
            .map_err(|error| format!("cannot resolve the message file `{shown}`: {error}"))?;
        if !canonical.starts_with(&git_dir) {
            return Err(format!(
                "the message file `{shown}` lies outside the repository's Git directory {}",
                git_dir.display()
            ));
        }
        if metadata.len() > limits.history_commit_bytes {
            return Err(format!(
                "the message file `{shown}` is {} bytes, above `limits.history_commit_bytes` = {}",
                metadata.len(),
                limits.history_commit_bytes
            ));
        }
        let limit = usize::try_from(limits.history_commit_bytes.min(budget.remaining))
            .unwrap_or(usize::MAX);
        let bytes = read_file_within(&canonical, limit, &format!("the message file `{shown}`"))?;
        budget.charge(bytes.len() as u64, "the message file")?;
        if bytes.contains(&0) {
            return Err(format!("the message file `{shown}` contains a NUL byte"));
        }
        let message = String::from_utf8(bytes)
            .map_err(|_| format!("the message file `{shown}` is not valid UTF-8"))?;

        // The staged changes of the captured index against HEAD's tree, or
        // the empty tree on an unborn branch.
        let frozen = git.with_index(snapshot.path());
        let head = pending_head(git)?;
        let basis = match &head {
            Some(head) => git
                .line(&["rev-parse", "--verify", "-q", &format!("{head}^{{tree}}")])
                .map_err(|error| error.to_string())
                .and_then(|text| ObjectId::parse(&text))?,
            None => empty_tree(git)?,
        };
        let listing = budget.listing(
            &frozen,
            &[
                "diff-index",
                "--cached",
                "-z",
                "--raw",
                "--full-index",
                "--no-renames",
                "--no-color",
                "--ignore-submodules=none",
                "--ita-invisible-in-index",
                basis.as_str(),
            ],
            "the staged changes",
        )?;
        let changes = parse_changes(&listing, &self.prefix)
            .map_err(|problem| format!("the pending commit: {problem}"))?;
        if changes.len() > limits.history_changes {
            return Err(format!(
                "the pending commit changes more than `limits.history_changes` = {} path(s)",
                limits.history_changes
            ));
        }

        // Parents: HEAD, then every MERGE_HEAD of a merge in progress.
        let mut parents = Vec::new();
        parents.extend(head.clone());
        parents.extend(merge_heads(git, &mut budget)?);

        // The name and email Git would record, asked with a fixed author
        // date so the host clock never reaches the view; the date is
        // discarded and the pending identity carries no time.
        let author = git
            .with_variable("GIT_AUTHOR_DATE", SYNTHETIC_AUTHOR_DATE)
            .line(&["var", "GIT_AUTHOR_IDENT"])
            .map_err(|error| format!("cannot determine the pending author: {error}"))?;
        let author =
            parse_identity(&author).map_err(|problem| format!("pending author {problem}"))?;
        let author = Identity {
            name: author.name,
            email: author.email,
            timestamp: None,
            timezone: None,
        };

        Ok(History {
            mode: Mode::Message,
            base: None,
            head: None,
            commits: vec![Commit {
                key: "pending".to_owned(),
                id: None,
                pending: true,
                tree: None,
                parents,
                author,
                committer: None,
                message,
                changes,
                change_basis: head,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_parse_exactly() {
        let identity = parse_identity("Ada  Lovelace <ada@example.test> 1700000000 +0200").unwrap();
        assert_eq!(identity.name, "Ada  Lovelace", "inner whitespace is kept");
        assert_eq!(identity.email, "ada@example.test");
        assert_eq!(identity.timestamp, Some(1_700_000_000));
        assert_eq!(identity.timezone.as_deref(), Some("+0200"));
        let negative = parse_identity("x <x@y> -5 -0930").unwrap();
        assert_eq!(
            (negative.timestamp, negative.timezone.as_deref()),
            (Some(-5), Some("-0930"))
        );
        let empty = parse_identity("<nobody@example.test> 0 +0000").unwrap();
        assert_eq!(empty.name, "");
        for bad in [
            "",
            "Name",
            "Name <a@b>",
            "Name <a@b> 12",
            "Name <a@b> 12 0200",
            "Name <a@b> 12 +02",
            "Name <a@b> x +0200",
            "Name a@b 12 +0200",
            "Na<me <a@b> 12 +0200",
            "Name <a<@b> 12 +0200",
        ] {
            assert!(parse_identity(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn commit_objects_parse_with_continuation_headers() {
        let object = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nparent aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nparent bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\nauthor A <a@x> 1 +0000\ncommitter C <c@x> 2 -0100\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n iQEzBAABCAAdFiEE\n -----END PGP SIGNATURE-----\n\nSubject line\n\nBody with  spaces.\n\nSigned-off-by: A <a@x>\n";
        let raw = parse_commit(object.as_bytes()).unwrap();
        assert_eq!(
            raw.tree.as_str(),
            "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        );
        assert_eq!(raw.parents.len(), 2);
        assert_eq!(raw.author.name, "A");
        assert_eq!(raw.committer.email, "c@x");
        assert_eq!(
            raw.message, "Subject line\n\nBody with  spaces.\n\nSigned-off-by: A <a@x>\n",
            "signature lines never reach the message"
        );
        // No message at all.
        let bare = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@x> 1 +0000\ncommitter C <c@x> 2 -0100\n";
        assert_eq!(parse_commit(bare.as_bytes()).unwrap().message, "");
        // An explicit UTF-8 encoding is fine; anything else is refused.
        let encoded = format!("{}encoding UTF-8\n\nx", &bare[..bare.len()]);
        assert!(parse_commit(encoded.as_bytes()).is_ok());
        let latin = format!("{bare}encoding ISO-8859-1\n\nx");
        assert!(
            parse_commit(latin.as_bytes())
                .unwrap_err()
                .contains("ISO-8859-1")
        );
        for bad in [
            &b"\xff\xfe"[..],
            b" continuation first\n\nmsg",
            b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@x> 1 +0000\n\nmsg",
            b"tree zz\nauthor A <a@x> 1 +0000\ncommitter C <c@x> 2 -0100\n\nmsg",
            b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor broken\ncommitter C <c@x> 2 -0100\n\nmsg",
            b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\ntree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@x> 1 +0000\ncommitter C <c@x> 2 -0100\n\nmsg",
            b"novalue\n\nmsg",
        ] {
            assert!(parse_commit(bad).is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn change_records_parse_and_sort() {
        let listing = b":100644 100755 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb M\0pkg/tool.sh\0:000000 120000 0000000000000000000000000000000000000000 cccccccccccccccccccccccccccccccccccccccc A\0link\0:100644 000000 dddddddddddddddddddddddddddddddddddddddd 0000000000000000000000000000000000000000 D\0pkg/old.md\0:100644 120000 eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee ffffffffffffffffffffffffffffffffffffffff T\0pkg/sub/typed\0:000000 160000 0000000000000000000000000000000000000000 1111111111111111111111111111111111111111 A\0vendor\0";
        let changes = parse_changes(listing, b"pkg/").unwrap();
        let summary: Vec<(&str, Option<&str>, &str)> = changes
            .iter()
            .map(|c| {
                (
                    c.repository_path.as_str(),
                    c.project_path.as_deref(),
                    c.change.as_str(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("link", None, "added"),
                ("pkg/old.md", Some("old.md"), "removed"),
                ("pkg/sub/typed", Some("sub/typed"), "type-changed"),
                ("pkg/tool.sh", Some("tool.sh"), "modified"),
                ("vendor", None, "added"),
            ]
        );
        let tool = &changes[3];
        assert_eq!(tool.before.as_ref().unwrap().kind, Kind::File);
        assert_eq!(tool.after.as_ref().unwrap().kind, Kind::Executable);
        assert_eq!(tool.after.as_ref().unwrap().mode, "100755");
        assert_eq!(changes[0].after.as_ref().unwrap().kind, Kind::Symlink);
        assert!(changes[0].before.is_none());
        assert_eq!(changes[4].after.as_ref().unwrap().kind, Kind::Gitlink);
        assert!(changes[1].after.is_none());
        // At the repository root every path is also a project path.
        let root = parse_changes(listing, b"").unwrap();
        assert_eq!(root[0].project_path.as_deref(), Some("link"));
        assert_eq!(root[3].project_path.as_deref(), Some("pkg/tool.sh"));
        // Malformed records.
        for bad in [
            &b"100644 100755 a b M\0x\0"[..],
            b":100644 100755 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb M\0",
            b":100644 100755 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb R100\0a\0b\0",
            b":100644 000000 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0000000000000000000000000000000000000000 M\0x\0",
            b":100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb M\0a:b\0",
            b":100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb M\0\xff\0",
            b":100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb M\0x\0:100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb M\0x\0",
        ] {
            assert!(parse_changes(bad, b"").is_err(), "{:?}", String::from_utf8_lossy(bad));
        }
        assert!(parse_changes(b"", b"pkg/").unwrap().is_empty());
    }

    #[test]
    fn subjects_and_line_counts_follow_the_message() {
        let commit = |message: &str| Commit {
            key: "pending".to_owned(),
            id: None,
            pending: true,
            tree: None,
            parents: Vec::new(),
            author: parse_identity("A <a@x> 1 +0000").unwrap(),
            committer: None,
            message: message.to_owned(),
            changes: Vec::new(),
            change_basis: None,
        };
        assert_eq!(commit("").line_count(), 0);
        assert_eq!(commit("").subject(), "");
        assert_eq!(commit("one").line_count(), 1);
        assert_eq!(commit("one\n").line_count(), 1);
        assert_eq!(commit("one\n\nthree\n").line_count(), 3);
        assert_eq!(commit("one\n\nthree\n\n").line_count(), 4);
        assert_eq!(commit("subject\nbody").subject(), "subject");
        // CRLF and a lone CR end lines too; the message keeps its bytes.
        let crlf = commit("subject\r\n\r\nbody\r\n");
        assert_eq!(crlf.subject(), "subject");
        assert_eq!(crlf.line_count(), 3);
        assert_eq!(crlf.message, "subject\r\n\r\nbody\r\n");
        let cr = commit("subject\rsecond\rthird");
        assert_eq!(cr.subject(), "subject");
        assert_eq!(cr.line_count(), 3);
        assert_eq!(logical_lines("a\r\n\rb\n\nc"), ["a", "", "b", "", "c"]);
        assert_eq!(logical_lines("\n"), [""]);
        assert_eq!(logical_lines("\r\n\r\n"), ["", ""]);
    }
}
