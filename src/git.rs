// SPDX-License-Identifier: Apache-2.0

//! Git-backed project trees: the index as it stands when the run starts,
//! or one resolved revision. Both are frozen for the duration of the run.
//!
//! Git is invoked as a subprocess through a fixed executable name and an
//! argument vector; nothing is ever interpolated into a shell command, and
//! the inherited environment cannot redirect the repository, its objects,
//! or its configuration, enable replacement objects, or fetch lazily (see
//! [`DROPPED_VARIABLES`]). The kernel never reads the working tree, never
//! writes to the repository, and never materializes a checkout: entries and
//! blob identities are captured once from `ls-files` or `ls-tree`, and blob
//! content is loaded on demand, by object identity, through a long-lived
//! `cat-file --batch` process. Working-tree filters, line-ending
//! conversion, and smudge transformations are never applied; a blob is read
//! exactly as Git stores it.
//!
//! Repository and project roots are distinct. The owning repository is
//! discovered from the project root, the project's prefix within it is
//! determined once, and every exposed path lies beneath that prefix.
//! Linked worktrees, where `.git` is a file, work like any other because
//! Git itself resolves them.
//!
//! What a tree entry retains: its kind (regular file, executable file,
//! symbolic link, directory, submodule gitlink), its object identity, and
//! its size where Git reported one. Object identities are opaque hexadecimal
//! strings of whatever length the repository's hash algorithm produces.
//!
//! Semantics of the index source, the tree a commit would record:
//! staged additions and modifications are present, unstaged modifications
//! and untracked files are absent, staged deletions are absent, a staged
//! rename appears only at its destination, and modes are those of the
//! index. The index file is copied once into a private temporary file and
//! every listing reads that copy, so one capture describes one state even
//! if the live index changes meanwhile. An unmerged entry fails the
//! capture; an intent-to-add entry, which `git commit` would not record,
//! is excluded.
//!
//! Every captured tree carries a deterministic digest over its entries
//! (kind, object identity, path), so that two captures of the same content
//! compare equal whether they came from the index or from a revision.
//!
//! Semantics of the revision source: the name is resolved once, the tree
//! object identity is retained, and that identity is used for the rest of
//! the run even if the branch or tag moves.
//!
//! Symbolic links inside a Git tree resolve only inside the tree they are
//! read from: absolute targets, targets that leave the tree, chains longer
//! than [`MAX_LINK_HOPS`], missing targets, and traversal through a gitlink
//! are errors. The working filesystem is never consulted.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::fmt;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};

use crate::paths::ProjectPath;
use crate::tree::ReadTree;

/// Largest listing (`ls-files`, `ls-tree`, `diff-index`) accepted from Git.
const MAX_LISTING_BYTES: usize = 256 * 1024 * 1024;
/// Largest blob read from Git.
const MAX_BLOB_BYTES: u64 = 256 * 1024 * 1024;
/// Bytes of a Git error stream retained for a diagnostic.
const MAX_STDERR_BYTES: usize = 8 * 1024;
/// Characters of a Git error message shown to the user.
const MAX_MESSAGE_CHARS: usize = 200;
/// Symbolic-link hops followed before a chain is treated as a cycle.
pub const MAX_LINK_HOPS: usize = 40;
/// Environment variables removed from every Git invocation. They redirect
/// the repository, its objects, or its configuration, change discovery,
/// alter pathspec interpretation, or enable tracing to arbitrary files;
/// none of them may influence what a Bearout run reads.
pub const DROPPED_VARIABLES: [&str; 30] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_REPLACE_REF_BASE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_CONFIG",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_SYSTEM",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_PARAMETERS",
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_NOGLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
    "GIT_EXTERNAL_DIFF",
    "GIT_DIFF_OPTS",
    "GIT_TRACE",
    "GIT_TRACE_PACK_ACCESS",
    "GIT_TRACE_PACKET",
    "GIT_TRACE_PERFORMANCE",
    "GIT_TRACE_SETUP",
    "GIT_TRACE_REDACT",
    "GIT_TRACE2",
    "GIT_TRACE2_EVENT",
];

/// Environment variables set on every Git invocation: no opportunistic
/// index writes, no prompts, no replacement objects, no lazy fetching from
/// a promisor remote, flushed batch output, and the C locale for messages.
pub const FIXED_VARIABLES: [(&str, &str); 6] = [
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_NO_REPLACE_OBJECTS", "1"),
    ("GIT_NO_LAZY_FETCH", "1"),
    ("GIT_FLUSH", "1"),
    ("LC_ALL", "C"),
];

/// Blobs at most this large are cached for the run.
const CACHE_ENTRY_BYTES: usize = 4 * 1024 * 1024;
/// Total cached blob bytes per run.
const CACHE_TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// A failure to construct a Git-backed tree. The message is bounded and
/// free of control characters, so it can be shown as a fatal outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitError(String);

impl fmt::Display for GitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GitError {}

/// An object identity: lowercase hexadecimal of whatever length the
/// repository's hash algorithm produces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId(String);

impl ObjectId {
    /// Parse an identity printed by Git.
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.is_empty()
            || !text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!("`{text}` is not an object identity"));
        }
        Ok(Self(text.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The kind of a tree entry, from its Git mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A regular file, mode `100644`.
    File,
    /// An executable file, mode `100755`.
    Executable,
    /// A symbolic link, mode `120000`; its blob holds the target.
    Symlink,
    /// A directory, mode `040000`, explicit in a tree or inferred from
    /// index paths.
    Directory,
    /// A submodule commit, mode `160000`. Never entered.
    Gitlink,
}

impl Kind {
    /// Classify a six-digit octal mode as Git canonicalizes it: any regular
    /// file with an execute bit is executable.
    pub fn from_mode(text: &str) -> Result<Self, String> {
        if text.len() != 6 || !text.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
            return Err(format!("`{text}` is not a Git mode"));
        }
        let mode =
            u32::from_str_radix(text, 8).map_err(|_| format!("`{text}` is not a Git mode"))?;
        match mode & 0o170_000 {
            0o100_000 => Ok(if mode & 0o100 == 0 {
                Self::File
            } else {
                Self::Executable
            }),
            0o120_000 => Ok(Self::Symlink),
            0o040_000 => Ok(Self::Directory),
            0o160_000 => Ok(Self::Gitlink),
            _ => Err(format!("`{text}` is not a Git mode Bearout understands")),
        }
    }
}

/// One captured entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    /// The object identity; `None` only for a directory inferred from index
    /// paths, which has no tree object yet.
    pub object: Option<ObjectId>,
    /// The blob size when Git reported it (`ls-tree -l`); otherwise looked
    /// up on demand.
    pub size: Option<u64>,
}

const ROOT_ENTRY: Entry = Entry {
    kind: Kind::Directory,
    object: None,
    size: None,
};

/// Where the project lives inside its repository.
struct Location {
    /// Repository-relative prefix of the project root, with a trailing `/`
    /// unless the project is the repository root.
    prefix: Vec<u8>,
}

/// The fixed way Bearout runs Git: the `git` executable, an argument
/// vector, the project root as the working directory, every variable in
/// [`DROPPED_VARIABLES`] removed, and every pair in [`FIXED_VARIABLES`]
/// set, so discovery always starts from the project root and the caller's
/// environment cannot redirect what is read. `GIT_INDEX_FILE` is honoured
/// only when it names a regular file, not a symbolic link, whose canonical
/// location is directly inside the discovered repository's own Git
/// directory, so that a partial-commit hook sees the index being committed
/// while a stale, foreign, or redirected value is ignored.
#[derive(Clone)]
struct Git {
    root: PathBuf,
    index_file: Option<OsString>,
}

impl Git {
    fn new(root: &Path) -> Result<Self, GitError> {
        let metadata = std::fs::metadata(root).map_err(|error| {
            GitError(format!("cannot open project {}: {error}", root.display()))
        })?;
        if !metadata.is_dir() {
            return Err(GitError(format!(
                "cannot open project {}: not a directory",
                root.display()
            )));
        }
        let mut git = Self {
            root: root.to_path_buf(),
            index_file: None,
        };
        git.index_file = git.own_index_file();
        Ok(git)
    }

    /// `GIT_INDEX_FILE` when it names a regular file (not a symbolic link)
    /// whose canonical path lies directly inside this repository's
    /// applicable Git directory; made absolute against the process working
    /// directory first. Anything else is ignored.
    fn own_index_file(&self) -> Option<OsString> {
        let value = std::env::var_os("GIT_INDEX_FILE")?;
        let path = PathBuf::from(&value);
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir().ok()?.join(path)
        };
        if !std::fs::symlink_metadata(&path).ok()?.is_file() {
            return None;
        }
        let canonical = std::fs::canonicalize(&path).ok()?;
        let git_dir = self.line(&["rev-parse", "--absolute-git-dir"]).ok()?;
        let git_dir = std::fs::canonicalize(git_dir).ok()?;
        (canonical.parent() == Some(git_dir.as_path())).then(|| canonical.into_os_string())
    }

    /// The same repository, reading the index at `path` instead.
    fn with_index(&self, path: &Path) -> Self {
        Self {
            root: self.root.clone(),
            index_file: Some(path.as_os_str().to_owned()),
        }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in DROPPED_VARIABLES {
            command.env_remove(name);
        }
        for (name, value) in FIXED_VARIABLES {
            command.env(name, value);
        }
        if let Some(index_file) = &self.index_file {
            command.env("GIT_INDEX_FILE", index_file);
        }
        command
    }

    /// Run one command and return its standard output, bounded. A non-zero
    /// exit is an error carrying a sanitized excerpt of standard error.
    fn output(&self, args: &[&str]) -> Result<Vec<u8>, GitError> {
        let mut child = self
            .command(args)
            .spawn()
            .map_err(|error| spawn_error(&error))?;
        // Failure messages name the subcommand, not a leading global flag.
        let name = args
            .iter()
            .find(|arg| !arg.starts_with("--"))
            .copied()
            .unwrap_or("git");
        let args: &[&str] = &[name];
        let stderr = child.stderr.take().expect("stderr is piped");
        let stderr_reader = std::thread::spawn(move || read_bounded(stderr, MAX_STDERR_BYTES));
        let stdout = child.stdout.take().expect("stdout is piped");
        let output = match read_bounded(stdout, MAX_LISTING_BYTES) {
            Ok((output, false)) => Ok(output),
            Ok((_, true)) => Err(format!(
                "git {} produced more than {MAX_LISTING_BYTES} bytes",
                args[0]
            )),
            Err(error) => Err(format!(
                "cannot read the output of git {}: {error}",
                args[0]
            )),
        };
        let output = match output {
            Ok(output) => output,
            Err(message) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitError(message));
            }
        };
        let status = child
            .wait()
            .map_err(|error| GitError(format!("cannot wait for git {}: {error}", args[0])))?;
        let stderr = match stderr_reader.join() {
            Ok(Ok((stderr, _))) => stderr,
            Ok(Err(error)) => format!("error output could not be read: {error}").into_bytes(),
            Err(_) => b"error output could not be read".to_vec(),
        };
        if status.success() {
            Ok(output)
        } else {
            Err(GitError(format!(
                "git {} failed: {}",
                args[0],
                sanitize(&stderr)
            )))
        }
    }

    /// The command's standard output as text with one trailing newline
    /// removed.
    fn line(&self, args: &[&str]) -> Result<String, GitError> {
        let output = self.output(args)?;
        let text = String::from_utf8(output)
            .map_err(|_| GitError(format!("git {} printed invalid UTF-8", args[0])))?;
        Ok(text.trim_end_matches(['\n', '\r']).to_owned())
    }

    fn locate(&self) -> Result<Location, GitError> {
        let output = self.output(&["rev-parse", "--is-inside-work-tree", "--show-prefix"])?;
        let mut lines = output.split(|byte| *byte == b'\n');
        let inside = lines.next().unwrap_or_default();
        if inside != b"true" {
            return Err(GitError(format!(
                "{} is not inside a Git work tree",
                self.root.display()
            )));
        }
        let prefix = lines.next().unwrap_or_default().to_vec();
        if !prefix.is_empty() && !prefix.ends_with(b"/") {
            return Err(GitError(
                "git rev-parse printed an unexpected prefix".to_owned(),
            ));
        }
        Ok(Location { prefix })
    }

    fn object_type(&self, object: &ObjectId) -> Result<String, GitError> {
        self.line(&["cat-file", "-t", object.as_str()])
    }

    /// The tree Git would compare the index against: `HEAD`'s tree, or the
    /// empty tree on an unborn branch.
    fn comparison_base(&self) -> Result<ObjectId, GitError> {
        let head = self.line(&["rev-parse", "--verify", "-q", "HEAD^{tree}"]);
        let text = match head {
            Ok(text) => text,
            Err(_) => self.line(&["hash-object", "-t", "tree", "--stdin"])?,
        };
        ObjectId::parse(&text).map_err(GitError)
    }
}

fn spawn_error(error: &io::Error) -> GitError {
    if error.kind() == io::ErrorKind::NotFound {
        GitError(
            "git is not installed or not on PATH; the index and revision sources require Git"
                .to_owned(),
        )
    } else {
        GitError(format!("cannot run git: {error}"))
    }
}

/// Read up to `limit` bytes. The flag reports whether more were available.
fn read_bounded(reader: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut buffer = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut buffer)?;
    if buffer.len() > limit {
        buffer.truncate(limit);
        Ok((buffer, true))
    } else {
        Ok((buffer, false))
    }
}

/// A private copy of the index file, so that every listing of one capture
/// reads the same bytes. Git treats a nonexistent index file as empty, so
/// a repository without an index yields a path that does not exist. The
/// copy is removed on drop.
struct IndexSnapshot {
    path: PathBuf,
    created: bool,
}

impl IndexSnapshot {
    fn capture(git: &Git) -> Result<Self, GitError> {
        let source = PathBuf::from(git.line(&["rev-parse", "--git-path", "index"])?);
        let source = if source.is_absolute() {
            source
        } else {
            git.root.join(source)
        };
        let bytes = match std::fs::File::open(&source) {
            Ok(file) => match read_bounded(file, MAX_LISTING_BYTES)
                .map_err(|error| GitError(format!("cannot read the index file: {error}")))?
            {
                (bytes, false) => Some(bytes),
                (_, true) => {
                    return Err(GitError(format!(
                        "the index file is larger than {MAX_LISTING_BYTES} bytes"
                    )));
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(GitError(format!("cannot read the index file: {error}"))),
        };
        let base = std::env::temp_dir();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        for attempt in 0..16u32 {
            let path = base.join(format!(
                "bearout-index-{}-{stamp}-{attempt}",
                std::process::id()
            ));
            let Some(bytes) = &bytes else {
                return Ok(Self {
                    path,
                    created: false,
                });
            };
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    let snapshot = Self {
                        path,
                        created: true,
                    };
                    file.write_all(bytes)
                        .and_then(|()| file.sync_data())
                        .map_err(|error| {
                            GitError(format!("cannot write the index snapshot: {error}"))
                        })?;
                    return Ok(snapshot);
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(GitError(format!(
                        "cannot create the index snapshot: {error}"
                    )));
                }
            }
        }
        Err(GitError(
            "cannot create the index snapshot: no free temporary name".to_owned(),
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for IndexSnapshot {
    fn drop(&mut self) {
        if self.created {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// The first meaningful line of a Git error stream, without the `fatal:`
/// or `error:` prefix, control characters, or excess length. Also used for
/// the error streams of external formatters.
pub fn sanitize(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no details");
    let line = line
        .strip_prefix("fatal: ")
        .or_else(|| line.strip_prefix("error: "))
        .unwrap_or(line);
    let cleaned: String = line
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_MESSAGE_CHARS)
        .collect();
    if line.chars().count() > MAX_MESSAGE_CHARS {
        format!("{cleaned}...")
    } else {
        cleaned
    }
}

// ---- listings -----------------------------------------------------------

/// One `ls-files --stage -z` record.
#[derive(Debug, PartialEq, Eq)]
struct IndexRecord {
    kind: Kind,
    object: ObjectId,
    stage: u8,
    path: Vec<u8>,
}

fn parse_ls_files(output: &[u8]) -> Result<Vec<IndexRecord>, String> {
    let mut records = Vec::new();
    for record in output.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "ls-files record without a tab".to_owned())?;
        let head = std::str::from_utf8(&record[..tab])
            .map_err(|_| "ls-files record header is not UTF-8".to_owned())?;
        let fields: Vec<&str> = head.split(' ').collect();
        let [mode, object, stage] = fields[..] else {
            return Err(format!(
                "ls-files record `{head}` has {} fields, expected 3",
                fields.len()
            ));
        };
        records.push(IndexRecord {
            kind: Kind::from_mode(mode)?,
            object: ObjectId::parse(object)?,
            stage: stage
                .parse()
                .ok()
                .filter(|stage| *stage <= 3)
                .ok_or_else(|| format!("`{stage}` is not an index stage"))?,
            path: record[tab + 1..].to_vec(),
        });
    }
    Ok(records)
}

/// One `ls-tree -r -t -l -z` record.
#[derive(Debug, PartialEq, Eq)]
struct TreeRecord {
    kind: Kind,
    object: ObjectId,
    size: Option<u64>,
    path: Vec<u8>,
}

fn parse_ls_tree(output: &[u8]) -> Result<Vec<TreeRecord>, String> {
    let mut records = Vec::new();
    for record in output.split(|byte| *byte == 0).filter(|r| !r.is_empty()) {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "ls-tree record without a tab".to_owned())?;
        let head = std::str::from_utf8(&record[..tab])
            .map_err(|_| "ls-tree record header is not UTF-8".to_owned())?;
        let fields: Vec<&str> = head.split_whitespace().collect();
        let [mode, object_type, object, size] = fields[..] else {
            return Err(format!(
                "ls-tree record `{head}` has {} fields, expected 4",
                fields.len()
            ));
        };
        let kind = Kind::from_mode(mode)?;
        let expected = match kind {
            Kind::Directory => "tree",
            Kind::Gitlink => "commit",
            Kind::File | Kind::Executable | Kind::Symlink => "blob",
        };
        if object_type != expected {
            return Err(format!(
                "ls-tree record `{head}` pairs mode {mode} with object type {object_type}"
            ));
        }
        let size = if size == "-" {
            None
        } else {
            Some(
                size.parse()
                    .map_err(|_| format!("`{size}` is not a size"))?,
            )
        };
        records.push(TreeRecord {
            kind,
            object: ObjectId::parse(object)?,
            size,
            path: record[tab + 1..].to_vec(),
        });
    }
    Ok(records)
}

/// Path to status letter from `diff-index --raw -z` without rename
/// detection, so every record is one header and one path.
fn parse_diff_index(output: &[u8]) -> Result<BTreeMap<Vec<u8>, char>, String> {
    let mut statuses = BTreeMap::new();
    let mut tokens = output.split(|byte| *byte == 0);
    while let Some(header) = tokens.next() {
        if header.is_empty() {
            continue;
        }
        let header = std::str::from_utf8(header)
            .map_err(|_| "diff-index record header is not UTF-8".to_owned())?;
        if !header.starts_with(':') {
            return Err(format!("diff-index record `{header}` is not a raw header"));
        }
        let status = header
            .rsplit(' ')
            .next()
            .and_then(|status| status.chars().next())
            .ok_or_else(|| format!("diff-index record `{header}` has no status"))?;
        let path = tokens
            .next()
            .filter(|path| !path.is_empty())
            .ok_or_else(|| format!("diff-index record `{header}` has no path"))?;
        statuses.insert(path.to_vec(), status);
    }
    Ok(statuses)
}

// ---- the captured tree --------------------------------------------------

/// Entries keyed by project-relative path, plus the directories whose
/// discovery must fail because they hold a name that is not a portable
/// project path.
#[derive(Default)]
struct Entries {
    map: BTreeMap<ProjectPath, Entry>,
    poisoned: BTreeMap<ProjectPath, String>,
}

impl Entries {
    /// A deterministic identity of the captured content: BLAKE3 over one
    /// line per non-directory entry, `<mode> <object> <path>`, in path
    /// order, followed by one line per poisoned directory. Directories are
    /// excluded because the index infers them and a tree lists them, and
    /// the same content must digest equally from either source.
    fn digest(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        for (path, entry) in &self.map {
            let mode = match entry.kind {
                Kind::File => "100644",
                Kind::Executable => "100755",
                Kind::Symlink => "120000",
                Kind::Gitlink => "160000",
                Kind::Directory => continue,
            };
            let object = entry.object.as_ref().map_or("", ObjectId::as_str);
            hasher.update(format!("{mode} {object} {path}\n").as_bytes());
        }
        for (path, problem) in &self.poisoned {
            hasher.update(format!("poisoned {path} {problem}\n").as_bytes());
        }
        format!("blake3:{}", hasher.finalize().to_hex())
    }

    /// Add one entry at a raw Git path. A name that is not valid UTF-8 or
    /// not a portable segment poisons the nearest valid ancestor directory
    /// instead of failing the capture, which mirrors working-tree discovery:
    /// the failure surfaces when that directory is walked.
    fn insert(&mut self, raw: &[u8], entry: Entry) -> Result<(), String> {
        let (path, problem) = match parse_git_path(raw) {
            Ok(path) => (path, None),
            Err((valid, problem)) => (valid, Some(problem)),
        };
        if let Some(problem) = problem {
            self.poisoned.entry(path).or_insert(problem);
            return Ok(());
        }
        for ancestor in path.ancestors() {
            if ancestor == path {
                break;
            }
            match self.map.get(&ancestor) {
                None => {
                    self.map.insert(ancestor, ROOT_ENTRY.clone());
                }
                Some(existing) if existing.kind == Kind::Directory => {}
                Some(_) => {
                    return Err(format!(
                        "`{ancestor}` is listed both as a file and as a directory of `{path}`"
                    ));
                }
            }
        }
        match self.map.get(&path) {
            Some(existing) if existing.kind == Kind::Directory && entry.kind == Kind::Directory => {
                // An explicit tree replaces the inferred directory.
                self.map.insert(path, entry);
            }
            Some(_) => return Err(format!("`{path}` is listed more than once")),
            None => {
                self.map.insert(path, entry);
            }
        }
        Ok(())
    }
}

/// Parse a raw Git path into a project path. On failure, return the longest
/// valid leading directory together with the problem.
fn parse_git_path(raw: &[u8]) -> Result<ProjectPath, (ProjectPath, String)> {
    let mut valid = ProjectPath::root();
    let text = match std::str::from_utf8(raw) {
        Ok(text) => text,
        Err(error) => {
            let prefix = String::from_utf8_lossy(&raw[..error.valid_up_to()]).into_owned();
            let directory = prefix.rsplit_once('/').map_or("", |(head, _)| head);
            for segment in directory.split('/').filter(|segment| !segment.is_empty()) {
                match ProjectPath::parse(segment) {
                    Ok(segment) => valid = valid.join(&segment),
                    Err(_) => break,
                }
            }
            return Err((
                valid,
                format!(
                    "contains an entry whose name is not valid UTF-8: {}",
                    String::from_utf8_lossy(raw)
                ),
            ));
        }
    };
    for segment in text.split('/') {
        match ProjectPath::parse(segment) {
            Ok(parsed) if !parsed.as_str().is_empty() => valid = valid.join(&parsed),
            Ok(_) => {
                return Err((
                    valid,
                    format!(
                        "contains an entry that is not a portable path segment: `{text}` has an empty segment"
                    ),
                ));
            }
            Err(error) => {
                return Err((
                    valid,
                    format!("contains an entry that is not a portable path segment: {error}"),
                ));
            }
        }
    }
    Ok(valid)
}

/// The long-lived `cat-file` processes and the per-run blob cache.
struct Blobs {
    git: Git,
    contents: Option<Batch>,
    info: Option<Batch>,
    cache: HashMap<ObjectId, Arc<[u8]>>,
    cached_bytes: usize,
}

/// One `cat-file --batch` or `--batch-check` process.
struct Batch {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Batch {
    fn spawn(git: &Git, flag: &str) -> io::Result<Self> {
        let mut command = git.command(&["cat-file", flag]);
        command.stdin(Stdio::piped()).stderr(Stdio::null());
        let mut child = command.spawn()?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    /// Send one identity and read the header line: `<object> <type> <size>`.
    fn header(&mut self, object: &ObjectId) -> io::Result<(String, u64)> {
        self.stdin.write_all(object.as_str().as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        let mut line = String::new();
        if self.stdout.read_line(&mut line)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "git cat-file ended unexpectedly",
            ));
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields[..] {
            [id, object_type, size] if id == object.as_str() => {
                let size = size.parse().map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "git cat-file printed an invalid size",
                    )
                })?;
                Ok((object_type.to_owned(), size))
            }
            [id, "missing"] if id == object.as_str() => Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("object {object} is missing from the repository"),
            )),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("git cat-file answered `{}`", line.trim_end()),
            )),
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Blobs {
    fn new(git: Git) -> Self {
        Self {
            git,
            contents: None,
            info: None,
            cache: HashMap::new(),
            cached_bytes: 0,
        }
    }

    fn size(&mut self, object: &ObjectId) -> io::Result<u64> {
        if let Some(bytes) = self.cache.get(object) {
            return Ok(bytes.len() as u64);
        }
        let mut batch = match self.info.take() {
            Some(batch) => batch,
            None => Batch::spawn(&self.git, "--batch-check")
                .map_err(|error| io::Error::new(error.kind(), spawn_error(&error).to_string()))?,
        };
        match batch.header(object) {
            Ok((_, size)) => {
                self.info = Some(batch);
                Ok(size)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.info = Some(batch);
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    fn read(&mut self, object: &ObjectId) -> io::Result<Arc<[u8]>> {
        if let Some(bytes) = self.cache.get(object) {
            return Ok(Arc::clone(bytes));
        }
        let mut batch = match self.contents.take() {
            Some(batch) => batch,
            None => Batch::spawn(&self.git, "--batch")
                .map_err(|error| io::Error::new(error.kind(), spawn_error(&error).to_string()))?,
        };
        let (object_type, size) = match batch.header(object) {
            Ok(header) => header,
            Err(error) => {
                if error.kind() == io::ErrorKind::NotFound {
                    self.contents = Some(batch);
                }
                return Err(error);
            }
        };
        if object_type != "blob" {
            // The content must still be drained, and a non-blob is a
            // programming error in the caller: drop the process instead.
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("object {object} is a {object_type}, not a blob"),
            ));
        }
        if size > MAX_BLOB_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("blob {object} is {size} bytes, above the {MAX_BLOB_BYTES} byte bound"),
            ));
        }
        let mut buffer = vec![0; usize::try_from(size).expect("bounded above")];
        batch.stdout.read_exact(&mut buffer)?;
        let mut newline = [0; 1];
        batch.stdout.read_exact(&mut newline)?;
        self.contents = Some(batch);
        let bytes: Arc<[u8]> = Arc::from(buffer);
        if bytes.len() <= CACHE_ENTRY_BYTES && self.cached_bytes + bytes.len() <= CACHE_TOTAL_BYTES
        {
            self.cached_bytes += bytes.len();
            self.cache.insert(object.clone(), Arc::clone(&bytes));
        }
        Ok(bytes)
    }
}

struct Captured {
    entries: Entries,
    digest: String,
    blobs: Mutex<Blobs>,
}

/// The files of the working directory as Git sees them: tracked files
/// plus untracked files that are not ignored, beneath the project prefix,
/// as project-relative paths in sorted order. Entries Git lists as
/// directories (nested repositories, for instance) are dropped; whether a
/// listed path still exists as a regular file on disk is for the caller
/// to decide against the working tree. A name that is not a portable
/// project path is an error.
pub fn working_files(root: &Path) -> Result<Vec<ProjectPath>, GitError> {
    let git = Git::new(root)?;
    let location = git.locate()?;
    let listing = git.output(&[
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--full-name",
        "--",
        ".",
    ])?;
    let mut files = Vec::new();
    for raw in listing.split(|byte| *byte == 0) {
        if raw.is_empty() || raw.ends_with(b"/") {
            continue;
        }
        let Some(relative) = raw.strip_prefix(&location.prefix[..]) else {
            continue;
        };
        match parse_git_path(relative) {
            Ok(path) => files.push(path),
            Err((_, problem)) => {
                return Err(GitError(format!("the working directory {problem}")));
            }
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// A frozen Git tree, or a directory inside one.
pub struct GitTree {
    captured: Arc<Captured>,
    /// The directory this view is rooted at, relative to the captured tree.
    base: ProjectPath,
}

impl fmt::Debug for GitTree {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitTree")
            .field("base", &self.base)
            .field("entries", &self.captured.entries.map.len())
            .field("poisoned", &self.captured.entries.poisoned)
            .finish_non_exhaustive()
    }
}

impl GitTree {
    /// Capture the Git index of the repository owning `root`, restricted to
    /// the project's prefix. See the module documentation for the exact
    /// semantics; the capture fails on an unmerged entry.
    pub fn index(root: &Path) -> Result<Self, GitError> {
        let git = Git::new(root)?;
        let location = git.locate()?;
        // Every listing below reads one private copy of the index, so the
        // entries and the intent-to-add classification describe one state.
        let snapshot = IndexSnapshot::capture(&git)?;
        let frozen = git.with_index(snapshot.path());
        let listing = frozen.output(&["ls-files", "--stage", "-z", "--full-name", "--", "."])?;
        let records = parse_ls_files(&listing).map_err(GitError)?;

        let unmerged: Vec<String> = records
            .iter()
            .filter(|record| record.stage != 0)
            .map(|record| String::from_utf8_lossy(&record.path).into_owned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if !unmerged.is_empty() {
            let shown: Vec<&str> = unmerged.iter().map(String::as_str).take(3).collect();
            let more = if unmerged.len() > shown.len() {
                format!(" and {} more", unmerged.len() - shown.len())
            } else {
                String::new()
            };
            return Err(GitError(format!(
                "the index has unmerged entries: {}{more}; resolve the conflict before checking the index",
                shown.join(", ")
            )));
        }

        let base = git.comparison_base()?;
        let visible = parse_diff_index(&frozen.output(&[
            "diff-index",
            "--cached",
            "-z",
            "--raw",
            base.as_str(),
            "--",
            ".",
        ])?)
        .map_err(GitError)?;
        let real = parse_diff_index(&frozen.output(&[
            "diff-index",
            "--cached",
            "-z",
            "--raw",
            "--ita-invisible-in-index",
            base.as_str(),
            "--",
            ".",
        ])?)
        .map_err(GitError)?;

        let mut entries = Entries::default();
        for record in records {
            // An intent-to-add entry is one that `--ita-invisible-in-index`
            // treats as absent: deleted relative to the base, or added only
            // when the entry is visible.
            let intent_to_add = matches!(
                (visible.get(&record.path), real.get(&record.path)),
                (_, Some('D')) | (Some('A'), None)
            );
            if intent_to_add {
                continue;
            }
            let relative = record
                .path
                .strip_prefix(&location.prefix[..])
                .ok_or_else(|| {
                    GitError(format!(
                        "git ls-files listed `{}` outside the project prefix",
                        String::from_utf8_lossy(&record.path)
                    ))
                })?;
            entries
                .insert(
                    relative,
                    Entry {
                        kind: record.kind,
                        object: Some(record.object),
                        size: None,
                    },
                )
                .map_err(|error| GitError(format!("the index is inconsistent: {error}")))?;
        }
        drop(snapshot);
        Ok(Self::from_entries(git, entries))
    }

    /// Capture one revision of the repository owning `root`, restricted to
    /// the project's prefix. `revision` is any commit-ish or tree-ish Git
    /// resolves; it is resolved exactly once, and the returned identity is
    /// the tree the run reads. A blob, an unknown name, or a prefix absent
    /// from the revision is an error.
    pub fn revision(root: &Path, revision: &str) -> Result<(Self, ObjectId), GitError> {
        Self::capture_revision(root, revision, false)
    }

    /// Like [`GitTree::revision`], for a comparison baseline: a revision
    /// that predates the project directory yields an empty tree, so that a
    /// wholly added project can be compared. The revision itself must
    /// still resolve.
    pub fn baseline(root: &Path, revision: &str) -> Result<(Self, ObjectId), GitError> {
        Self::capture_revision(root, revision, true)
    }

    fn capture_revision(
        root: &Path,
        revision: &str,
        empty_when_project_absent: bool,
    ) -> Result<(Self, ObjectId), GitError> {
        if revision.is_empty()
            || revision.starts_with('-')
            || revision.chars().any(char::is_control)
        {
            return Err(GitError(format!(
                "`{}` is not a revision name",
                revision
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect::<String>()
            )));
        }
        let git = Git::new(root)?;
        let location = git.locate()?;
        let object = git
            .line(&["rev-parse", "--verify", "-q", "--end-of-options", revision])
            .map_err(|_| GitError(format!("`{revision}` is not a revision of this repository")))?;
        let object = ObjectId::parse(&object).map_err(GitError)?;
        // A well-formed identity passes `rev-parse --verify` without
        // existing; the type lookup is what proves the object is there.
        let object_type = git
            .object_type(&object)
            .map_err(|_| GitError(format!("`{revision}` is not a revision of this repository")))?;
        let tree = match object_type.as_str() {
            "tree" => object,
            "commit" | "tag" => {
                let peeled = git.line(&["rev-parse", "--verify", &format!("{object}^{{tree}}")])?;
                ObjectId::parse(&peeled).map_err(GitError)?
            }
            other => {
                return Err(GitError(format!(
                    "`{revision}` names a {other}, not a commit, tag, or tree"
                )));
            }
        };
        let project_tree = if location.prefix.is_empty() {
            tree.clone()
        } else {
            let prefix = String::from_utf8(location.prefix.clone())
                .map_err(|_| GitError("the project prefix is not valid UTF-8".to_owned()))?;
            let prefix = prefix.trim_end_matches('/');
            // One literal, non-recursive listing of exactly the prefix path.
            // Git proves absence with a successful, empty listing; a present
            // entry carries its mode; and every operational failure, such
            // as an unreadable object, is a non-zero exit and therefore
            // fatal rather than a silently empty history.
            let listing = git.output(&[
                "--literal-pathspecs",
                "ls-tree",
                "-l",
                "-z",
                "--full-tree",
                tree.as_str(),
                "--",
                prefix,
            ])?;
            let records = parse_ls_tree(&listing).map_err(GitError)?;
            match records.as_slice() {
                [] if empty_when_project_absent => {
                    return Ok((Self::from_entries(git, Entries::default()), tree));
                }
                [] => {
                    return Err(GitError(format!(
                        "revision `{revision}` does not contain the project directory `{prefix}`"
                    )));
                }
                [record] if record.path == prefix.as_bytes() => match record.kind {
                    Kind::Directory => record.object.clone(),
                    Kind::File | Kind::Executable => {
                        return Err(GitError(format!(
                            "`{prefix}` is a file, not a directory, in revision `{revision}`"
                        )));
                    }
                    Kind::Symlink => {
                        return Err(GitError(format!(
                            "`{prefix}` is a symbolic link, not a directory, in revision `{revision}`"
                        )));
                    }
                    Kind::Gitlink => {
                        return Err(GitError(format!(
                            "`{prefix}` is a submodule, not a directory, in revision `{revision}`"
                        )));
                    }
                },
                _ => {
                    return Err(GitError(format!(
                        "git ls-tree listed something other than `{prefix}` for revision `{revision}`"
                    )));
                }
            }
        };
        // `--full-tree` lifts the default restriction to the current
        // directory's prefix, which would hide a subtree listed by object.
        let listing = git.output(&[
            "ls-tree",
            "-r",
            "-t",
            "-l",
            "-z",
            "--full-tree",
            project_tree.as_str(),
        ])?;
        let mut entries = Entries::default();
        for record in parse_ls_tree(&listing).map_err(GitError)? {
            entries
                .insert(
                    &record.path,
                    Entry {
                        kind: record.kind,
                        object: Some(record.object),
                        size: record.size,
                    },
                )
                .map_err(|error| {
                    GitError(format!("revision `{revision}` is inconsistent: {error}"))
                })?;
        }
        Ok((Self::from_entries(git, entries), tree))
    }

    fn from_entries(git: Git, entries: Entries) -> Self {
        let digest = entries.digest();
        Self {
            captured: Arc::new(Captured {
                entries,
                digest,
                blobs: Mutex::new(Blobs::new(git)),
            }),
            base: ProjectPath::root(),
        }
    }

    /// The deterministic digest of the whole captured tree; see
    /// [`Entries::digest`]. Independent of the view's base.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.captured.digest
    }

    /// The entry at `path` as captured, without following links.
    #[must_use]
    pub fn entry(&self, path: &ProjectPath) -> Option<&Entry> {
        let full = self.base.join(path);
        if full.as_str().is_empty() {
            return Some(&ROOT_ENTRY);
        }
        self.captured.entries.map.get(&full)
    }

    /// Every captured path beneath this view, in order, with its entry.
    #[cfg(test)]
    pub fn entries(&self) -> impl Iterator<Item = (ProjectPath, &Entry)> {
        self.captured
            .entries
            .map
            .iter()
            .filter(move |(path, _)| self.base.as_str().is_empty() || path.is_within(&self.base))
            .filter_map(move |(path, entry)| {
                path.strip_prefix(&self.base)
                    .filter(|relative| !relative.as_str().is_empty())
                    .map(|relative| (relative, entry))
            })
    }

    fn blob(&self, object: &ObjectId) -> io::Result<Arc<[u8]>> {
        self.captured
            .blobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .read(object)
    }

    fn blob_size(&self, entry: &Entry, object: &ObjectId) -> io::Result<u64> {
        if let Some(size) = entry.size {
            return Ok(size);
        }
        self.captured
            .blobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .size(object)
    }

    /// The target of a symbolic-link entry, resolved lexically against the
    /// link's directory and confined to this view.
    fn link_target(&self, link: &ProjectPath, entry: &Entry) -> io::Result<ProjectPath> {
        let object = entry
            .object
            .as_ref()
            .expect("a symbolic link always has a blob");
        let bytes = self.blob(object)?;
        let target = std::str::from_utf8(&bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("symbolic link `{link}` has a target that is not valid UTF-8"),
            )
        })?;
        if target.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("symbolic link `{link}` has an empty target"),
            ));
        }
        let resolved = ProjectPath::resolve_relative(&link.parent(), target).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("symbolic link `{link}` cannot be followed: {error}"),
            )
        })?;
        if !resolved.is_within(&self.base) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "symbolic link `{link}` targets `{resolved}`, outside the tree it is read from"
                ),
            ));
        }
        Ok(resolved)
    }

    /// Follow `path` from this view's base, resolving symbolic links inside
    /// the view. `Ok(None)` means a component is missing or a file is used
    /// as a directory; `Err` means the path is refused.
    fn resolve(&self, path: &ProjectPath) -> io::Result<Option<(ProjectPath, &Entry)>> {
        let mut pending: std::collections::VecDeque<String> = path
            .as_str()
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_owned)
            .collect();
        let mut current = self.base.clone();
        let mut hops = 0;
        while let Some(segment) = pending.pop_front() {
            let next =
                current.join(&ProjectPath::parse(&segment).expect("segment of a valid path"));
            let Some(entry) = self.captured.entries.map.get(&next) else {
                return Ok(None);
            };
            match entry.kind {
                Kind::Directory => current = next,
                Kind::Gitlink => {
                    if pending.is_empty() {
                        return Ok(Some((next, entry)));
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("`{next}` is a submodule; Bearout never reads through submodules"),
                    ));
                }
                Kind::File | Kind::Executable => {
                    if pending.is_empty() {
                        return Ok(Some((next, entry)));
                    }
                    return Ok(None);
                }
                Kind::Symlink => {
                    hops += 1;
                    if hops > MAX_LINK_HOPS {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!(
                                "`{next}` is reached through more than {MAX_LINK_HOPS} symbolic links; the links may form a cycle"
                            ),
                        ));
                    }
                    let target = self.link_target(&next, entry)?;
                    let relative = target
                        .strip_prefix(&self.base)
                        .expect("confined to the view");
                    let mut restarted: std::collections::VecDeque<String> = relative
                        .as_str()
                        .split('/')
                        .filter(|segment| !segment.is_empty())
                        .map(str::to_owned)
                        .collect();
                    restarted.extend(pending.drain(..));
                    pending = restarted;
                    current = self.base.clone();
                }
            }
        }
        if current.as_str().is_empty() {
            return Ok(Some((current, &ROOT_ENTRY)));
        }
        let entry = self
            .captured
            .entries
            .map
            .get(&current)
            .unwrap_or(&ROOT_ENTRY);
        Ok(Some((current, entry)))
    }

    fn resolved(&self, path: &ProjectPath) -> Option<(ProjectPath, &Entry)> {
        self.resolve(path).ok().flatten()
    }
}

fn not_found(path: &ProjectPath) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("`{path}` does not exist in the tree"),
    )
}

impl ReadTree for GitTree {
    fn read(&self, path: &ProjectPath) -> io::Result<Vec<u8>> {
        let Some((full, entry)) = self.resolve(path)? else {
            return Err(not_found(path));
        };
        match entry.kind {
            Kind::File | Kind::Executable => {
                let object = entry.object.as_ref().expect("a file always has a blob");
                Ok(self.blob(object)?.to_vec())
            }
            Kind::Directory => Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                format!("`{path}` is a directory"),
            )),
            Kind::Gitlink => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("`{full}` is a submodule; Bearout never reads through submodules"),
            )),
            Kind::Symlink => unreachable!("resolution follows links"),
        }
    }

    fn file_len(&self, path: &ProjectPath) -> io::Result<u64> {
        let Some((_, entry)) = self.resolve(path)? else {
            return Err(not_found(path));
        };
        match entry.kind {
            Kind::File | Kind::Executable => {
                let object = entry.object.as_ref().expect("a file always has a blob");
                self.blob_size(entry, object)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("`{path}` is not a file"),
            )),
        }
    }

    fn is_file(&self, path: &ProjectPath) -> bool {
        self.resolved(path)
            .is_some_and(|(_, entry)| matches!(entry.kind, Kind::File | Kind::Executable))
    }

    fn is_dir(&self, path: &ProjectPath) -> bool {
        self.resolved(path)
            .is_some_and(|(_, entry)| entry.kind == Kind::Directory)
    }

    fn exists(&self, path: &ProjectPath) -> bool {
        self.resolved(path).is_some()
    }

    fn symlink_component(&self, path: &ProjectPath) -> io::Result<Option<ProjectPath>> {
        for ancestor in path.ancestors() {
            match self.entry(&ancestor) {
                None => return Ok(None),
                Some(entry) if entry.kind == Kind::Symlink => return Ok(Some(ancestor)),
                Some(_) => {}
            }
        }
        Ok(None)
    }

    fn walk(&self, directory: &ProjectPath) -> io::Result<Vec<ProjectPath>> {
        if let Some(link) = self.symlink_component(directory)? {
            return Err(crate::fs::linked_directory(&link));
        }
        let Some((resolved, entry)) = self.resolve(directory)? else {
            return Err(not_found(directory));
        };
        if entry.kind != Kind::Directory {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("`{directory}` is not a directory"),
            ));
        }
        if let Some((poisoned, problem)) = self
            .captured
            .entries
            .poisoned
            .iter()
            .find(|(poisoned, _)| poisoned.is_within(&resolved))
        {
            let shown = poisoned
                .strip_prefix(&self.base)
                .unwrap_or_else(|| poisoned.clone());
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("`{shown}` {problem}"),
            ));
        }
        let mut found: Vec<ProjectPath> = self
            .captured
            .entries
            .map
            .iter()
            .filter(|(path, entry)| {
                matches!(entry.kind, Kind::File | Kind::Executable)
                    && (resolved.as_str().is_empty() || path.is_within(&resolved))
            })
            .filter_map(|(path, _)| path.strip_prefix(&resolved))
            .map(|relative| directory.join(&relative))
            .collect();
        found.sort();
        Ok(found)
    }

    fn subtree(&self, directory: &ProjectPath) -> io::Result<Arc<dyn ReadTree>> {
        let Some((resolved, entry)) = self.resolve(directory)? else {
            return Err(not_found(directory));
        };
        if entry.kind != Kind::Directory {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                format!("`{directory}` is not a directory"),
            ));
        }
        Ok(Arc::new(Self {
            captured: Arc::clone(&self.captured),
            base: resolved,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_classify_like_git() {
        assert_eq!(Kind::from_mode("100644").unwrap(), Kind::File);
        assert_eq!(Kind::from_mode("100664").unwrap(), Kind::File);
        assert_eq!(Kind::from_mode("100755").unwrap(), Kind::Executable);
        assert_eq!(Kind::from_mode("120000").unwrap(), Kind::Symlink);
        assert_eq!(Kind::from_mode("040000").unwrap(), Kind::Directory);
        assert_eq!(Kind::from_mode("160000").unwrap(), Kind::Gitlink);
        assert!(Kind::from_mode("644").is_err());
        assert!(Kind::from_mode("000000").is_err());
        assert!(Kind::from_mode("10064x").is_err());
    }

    #[test]
    fn object_ids_are_hexadecimal_of_any_length() {
        assert!(ObjectId::parse("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").is_ok());
        assert!(
            ObjectId::parse("473a0f4c3be8a93681a267e3b1e9a7dcda1185436fe141f7749120a303721813")
                .is_ok()
        );
        assert!(ObjectId::parse("").is_err());
        assert!(ObjectId::parse("E69DE29B").is_err());
        assert!(ObjectId::parse("e69de29b\n").is_err());
    }

    #[test]
    fn ls_files_records_parse() {
        let output = b"100644 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 0\tsub/a.md\x00100755 5626abf0f72e58d7a153368ba57db4c673c0e171 2\tb\tc\0";
        let records = parse_ls_files(output).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].kind, Kind::File);
        assert_eq!(records[0].stage, 0);
        assert_eq!(records[0].path, b"sub/a.md");
        assert_eq!(records[1].kind, Kind::Executable);
        assert_eq!(records[1].stage, 2);
        assert_eq!(records[1].path, b"b\tc", "the first tab ends the header");
        assert!(parse_ls_files(b"100644 abc\tx\0").is_err());
        assert!(parse_ls_files(b"100644 abc 7\tx\0").is_err());
        assert!(parse_ls_files(b"100644 abc 0 x\0").is_err());
    }

    #[test]
    fn ls_tree_records_parse() {
        let output = b"040000 tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904       -\tdocs\x00100644 blob 5626abf0f72e58d7a153368ba57db4c673c0e171       4\tdocs/a.md\x00120000 blob e69de29bb2d1d6434b8b29ae775ad8c2e48c5391       0\tlink\x00160000 commit 5626abf0f72e58d7a153368ba57db4c673c0e171       -\tvendor\0";
        let records = parse_ls_tree(output).unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(records[0].kind, Kind::Directory);
        assert_eq!(records[0].size, None);
        assert_eq!(records[1].kind, Kind::File);
        assert_eq!(records[1].size, Some(4));
        assert_eq!(records[2].kind, Kind::Symlink);
        assert_eq!(records[3].kind, Kind::Gitlink);
        assert_eq!(records[3].path, b"vendor");
        assert!(
            parse_ls_tree(b"100644 tree 5626abf0f72e58d7a153368ba57db4c673c0e171 4\tx\0").is_err(),
            "mode and type must agree"
        );
    }

    #[test]
    fn diff_index_records_parse() {
        let output = b":000000 100644 0000000000000000000000000000000000000000 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 A\0new\0:100644 000000 e69de29bb2d1d6434b8b29ae775ad8c2e48c5391 0000000000000000000000000000000000000000 D\0gone\0";
        let statuses = parse_diff_index(output).unwrap();
        assert_eq!(statuses.get(&b"new"[..]), Some(&'A'));
        assert_eq!(statuses.get(&b"gone"[..]), Some(&'D'));
        assert!(parse_diff_index(b"garbage\0x\0").is_err());
        assert!(parse_diff_index(b":000000 100644 0 0 A\0").is_err());
    }

    #[test]
    fn entries_infer_directories_and_poison_bad_names() {
        let file = |kind| Entry {
            kind,
            object: Some(ObjectId::parse("e69de29bb2d1d6434b8b29ae775ad8c2e48c5391").unwrap()),
            size: None,
        };
        let mut entries = Entries::default();
        entries.insert(b"a/b/c.md", file(Kind::File)).unwrap();
        entries.insert(b"a/x:y.md", file(Kind::File)).unwrap();
        entries
            .insert(b"q/bad\\dir/z.md", file(Kind::File))
            .unwrap();
        entries.insert(b"top:bad", file(Kind::File)).unwrap();
        entries.insert(b"u/\xff.md", file(Kind::File)).unwrap();
        assert_eq!(
            entries
                .map
                .get(&ProjectPath::parse("a").unwrap())
                .unwrap()
                .kind,
            Kind::Directory
        );
        assert_eq!(
            entries
                .map
                .get(&ProjectPath::parse("a/b").unwrap())
                .unwrap()
                .kind,
            Kind::Directory
        );
        let poisoned: Vec<&str> = entries.poisoned.keys().map(ProjectPath::as_str).collect();
        assert_eq!(poisoned, ["", "a", "q", "u"]);
        assert!(entries.poisoned[&ProjectPath::root()].contains("`:`"));
        assert!(entries.poisoned[&ProjectPath::parse("u").unwrap()].contains("not valid UTF-8"));
        assert!(
            entries
                .insert(b"a/b", file(Kind::File))
                .unwrap_err()
                .contains("more than once")
        );
        assert!(
            entries
                .insert(b"a/b/c.md/d", file(Kind::File))
                .unwrap_err()
                .contains("both as a file and as a directory")
        );
    }

    #[test]
    fn stderr_is_sanitized() {
        assert_eq!(
            sanitize(b"\nfatal: not a git repository\nmore\n"),
            "not a git repository"
        );
        assert_eq!(sanitize(b"error: x\x07y"), "xy");
        assert_eq!(sanitize(b""), "no details");
        let long = "a".repeat(400);
        assert_eq!(sanitize(long.as_bytes()).len(), MAX_MESSAGE_CHARS + 3);
    }
}

#[cfg(test)]
mod repository_tests {
    //! Tests against synthetic repositories. Entries of every kind are
    //! planted with `update-index --cacheinfo`, so they need no filesystem
    //! symbolic links and run on every platform.

    use std::path::Path;
    use std::process::{Command, Stdio};

    use super::*;

    struct Repo {
        dir: tempfile::TempDir,
    }

    impl Repo {
        fn new() -> Self {
            let repo = Self {
                dir: tempfile::tempdir().expect("temp dir"),
            };
            repo.git(&["init", "-q", "-b", "main"]);
            repo.git(&["config", "commit.gpgsign", "false"]);
            repo.git(&["config", "core.autocrlf", "false"]);
            repo
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn command(&self, args: &[&str]) -> Command {
            let mut command = Command::new("git");
            command
                .args(args)
                .current_dir(self.root())
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env_remove("GIT_INDEX_FILE")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", self.root().join(".no-global-config"))
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.invalid");
            command
        }

        fn git(&self, args: &[&str]) -> String {
            let output = self.command(args).output().expect("run git");
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout)
                .expect("utf-8")
                .trim_end()
                .to_owned()
        }

        fn blob(&self, content: &[u8]) -> String {
            let mut child = self
                .command(&["hash-object", "-w", "--stdin"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .expect("spawn");
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(content)
                .expect("write");
            let output = child.wait_with_output().expect("output");
            assert!(output.status.success());
            String::from_utf8(output.stdout)
                .expect("utf-8")
                .trim()
                .to_owned()
        }

        fn stage(&self, mode: &str, content: &[u8], path: &str) {
            let object = self.blob(content);
            self.git(&[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("{mode},{object},{path}"),
            ]);
        }

        fn commit(&self, message: &str) -> String {
            self.git(&["commit", "-q", "--allow-empty", "-m", message]);
            self.git(&["rev-parse", "HEAD"])
        }
    }

    fn path(text: &str) -> ProjectPath {
        ProjectPath::parse(text).expect("valid test path")
    }

    /// A repository with one entry of every kind.
    fn planted() -> Repo {
        let repo = Repo::new();
        repo.stage("100644", b"plain\n", "docs/a.md");
        repo.stage("100755", b"#!/bin/sh\n", "docs/run.sh");
        repo.stage("120000", b"a.md", "docs/link.md");
        repo.stage("120000", b"link.md", "docs/link2.md");
        repo.stage("120000", b"../docs/a.md", "docs/dotdot.md");
        repo.stage("120000", b"/etc/passwd", "docs/absolute.md");
        repo.stage("120000", b"../../outside.md", "docs/escape.md");
        repo.stage("120000", b"nowhere.md", "docs/dangling.md");
        repo.stage("120000", b"loop-b.md", "docs/loop-a.md");
        repo.stage("120000", b"loop-a.md", "docs/loop-b.md");
        repo.stage("120000", b"sub", "docs/to-dir");
        repo.stage("100644", b"deep\n", "docs/sub/deep.md");
        repo.stage("120000", b"vendor/inside.md", "docs/through-gitlink.md");
        repo.commit("base");
        let head = repo.git(&["rev-parse", "HEAD"]);
        repo.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{head},docs/vendor"),
        ]);
        repo
    }

    #[test]
    fn index_entries_retain_kind_object_and_size() {
        let repo = planted();
        let tree = GitTree::index(repo.root()).unwrap();
        let kinds: Vec<(String, Kind)> = tree
            .entries()
            .map(|(path, entry)| (path.as_str().to_owned(), entry.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("docs".to_owned(), Kind::Directory),
                ("docs/a.md".to_owned(), Kind::File),
                ("docs/absolute.md".to_owned(), Kind::Symlink),
                ("docs/dangling.md".to_owned(), Kind::Symlink),
                ("docs/dotdot.md".to_owned(), Kind::Symlink),
                ("docs/escape.md".to_owned(), Kind::Symlink),
                ("docs/link.md".to_owned(), Kind::Symlink),
                ("docs/link2.md".to_owned(), Kind::Symlink),
                ("docs/loop-a.md".to_owned(), Kind::Symlink),
                ("docs/loop-b.md".to_owned(), Kind::Symlink),
                ("docs/run.sh".to_owned(), Kind::Executable),
                ("docs/sub".to_owned(), Kind::Directory),
                ("docs/sub/deep.md".to_owned(), Kind::File),
                ("docs/through-gitlink.md".to_owned(), Kind::Symlink),
                ("docs/to-dir".to_owned(), Kind::Symlink),
                ("docs/vendor".to_owned(), Kind::Gitlink),
            ]
        );
        let a = tree.entry(&path("docs/a.md")).unwrap();
        assert_eq!(a.object.as_ref().unwrap().as_str(), repo.blob(b"plain\n"));
        assert_eq!(a.size, None, "the index carries no sizes");
        assert_eq!(tree.file_len(&path("docs/a.md")).unwrap(), 6);
        assert_eq!(tree.read(&path("docs/a.md")).unwrap(), b"plain\n");
        assert_eq!(tree.read(&path("docs/run.sh")).unwrap(), b"#!/bin/sh\n");
        assert!(tree.entry(&path("docs/sub")).unwrap().object.is_none());
        assert!(tree.entry(&path("")).is_some());
        assert!(tree.is_dir(&path("")));
    }

    #[test]
    fn revision_entries_carry_tree_objects_and_sizes() {
        let repo = planted();
        let (tree, id) = GitTree::revision(repo.root(), "HEAD").unwrap();
        assert_eq!(id.as_str(), repo.git(&["rev-parse", "HEAD^{tree}"]));
        let docs = tree.entry(&path("docs")).unwrap();
        assert_eq!(docs.kind, Kind::Directory);
        assert_eq!(
            docs.object.as_ref().unwrap().as_str(),
            repo.git(&["rev-parse", "HEAD:docs"])
        );
        assert_eq!(tree.entry(&path("docs/a.md")).unwrap().size, Some(6));
        assert_eq!(
            tree.entry(&path("docs/run.sh")).unwrap().kind,
            Kind::Executable
        );
        assert!(
            tree.entry(&path("docs/vendor")).is_none(),
            "the gitlink was staged after the commit"
        );
    }

    #[test]
    fn symbolic_links_resolve_only_inside_the_tree() {
        let repo = planted();
        for tree in [
            GitTree::index(repo.root()).unwrap(),
            GitTree::revision(repo.root(), "HEAD").unwrap().0,
        ] {
            assert_eq!(tree.read(&path("docs/link.md")).unwrap(), b"plain\n");
            assert_eq!(tree.read(&path("docs/link2.md")).unwrap(), b"plain\n");
            assert_eq!(tree.read(&path("docs/dotdot.md")).unwrap(), b"plain\n");
            assert_eq!(tree.read(&path("docs/to-dir/deep.md")).unwrap(), b"deep\n");
            assert!(tree.is_file(&path("docs/link.md")));
            assert!(tree.is_dir(&path("docs/to-dir")));
            assert_eq!(
                tree.symlink_component(&path("docs/to-dir/deep.md"))
                    .unwrap(),
                Some(path("docs/to-dir"))
            );
            assert_eq!(
                tree.symlink_component(&path("docs/sub/deep.md")).unwrap(),
                None
            );

            let refused = |name: &str, expected: &str| {
                let error = tree.read(&path(name)).unwrap_err();
                assert!(
                    error.to_string().contains(expected),
                    "{name}: {error} (expected {expected:?})"
                );
                assert!(!tree.is_file(&path(name)), "{name}");
                assert!(!tree.exists(&path(name)), "{name}");
            };
            refused("docs/absolute.md", "is absolute");
            refused("docs/escape.md", "leaves the project");
            refused("docs/loop-a.md", "may form a cycle");
            let dangling = tree.read(&path("docs/dangling.md")).unwrap_err();
            assert_eq!(dangling.kind(), io::ErrorKind::NotFound);
            assert!(!tree.exists(&path("docs/dangling.md")));
        }
    }

    #[test]
    fn gitlinks_are_never_entered() {
        let repo = planted();
        let tree = GitTree::index(repo.root()).unwrap();
        assert!(tree.exists(&path("docs/vendor")));
        assert!(!tree.is_dir(&path("docs/vendor")));
        assert!(!tree.is_file(&path("docs/vendor")));
        assert!(!tree.exists(&path("docs/vendor/inside.md")));
        let through = tree.read(&path("docs/through-gitlink.md")).unwrap_err();
        assert!(through.to_string().contains("submodule"), "{through}");
        let direct = tree.read(&path("docs/vendor")).unwrap_err();
        assert!(direct.to_string().contains("submodule"), "{direct}");
        assert!(tree.walk(&path("docs/vendor")).is_err());
        assert!(tree.subtree(&path("docs/vendor")).is_err());
    }

    #[test]
    fn walking_skips_links_and_gitlinks_and_refuses_a_linked_root() {
        let repo = planted();
        let tree = GitTree::index(repo.root()).unwrap();
        let found: Vec<String> = tree
            .walk(&path("docs"))
            .unwrap()
            .iter()
            .map(|p| p.as_str().to_owned())
            .collect();
        assert_eq!(found, ["docs/a.md", "docs/run.sh", "docs/sub/deep.md"]);
        let through_link = tree.walk(&path("docs/to-dir")).unwrap_err();
        assert_eq!(
            through_link.to_string(),
            "`docs/to-dir` is a symbolic link; directories are never walked through links"
        );
        let beneath_link = tree.walk(&path("docs/to-dir/sub")).unwrap_err();
        assert!(
            beneath_link
                .to_string()
                .contains("`docs/to-dir` is a symbolic link")
        );
        assert_eq!(
            tree.walk(&path("docs/missing")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            tree.walk(&path("docs/a.md")).unwrap_err().kind(),
            io::ErrorKind::NotADirectory
        );
    }

    #[test]
    fn subtrees_confine_links_to_their_own_root() {
        let repo = Repo::new();
        repo.stage("100644", b"outer\n", "rules/x.j2");
        repo.stage("100644", b"inner\n", "templates/parts/x.j2");
        repo.stage("120000", b"parts/x.j2", "templates/ok.j2");
        repo.stage("120000", b"../rules/x.j2", "templates/escape.j2");
        repo.stage("120000", b"templates/parts/x.j2", "alias.j2");
        let tree = GitTree::index(repo.root()).unwrap();
        assert_eq!(tree.read(&path("templates/escape.j2")).unwrap(), b"outer\n");
        let templates = tree.subtree(&path("templates")).unwrap();
        assert_eq!(templates.read(&path("ok.j2")).unwrap(), b"inner\n");
        let error = templates.read(&path("escape.j2")).unwrap_err();
        assert!(error.to_string().contains("outside the tree"), "{error}");
        assert!(!templates.exists(&path("escape.j2")));
        assert_eq!(templates.walk(&path("")).unwrap(), [path("parts/x.j2")]);
        assert!(templates.is_dir(&path("")));
        assert!(!templates.exists(&path("rules")));
    }

    #[test]
    fn a_revision_is_resolved_once() {
        let repo = Repo::new();
        repo.stage("100644", b"first\n", "a.md");
        let first = repo.commit("first");
        let (tree, id) = GitTree::revision(repo.root(), "main").unwrap();
        repo.stage("100644", b"second\n", "a.md");
        repo.stage("100644", b"new\n", "b.md");
        let second = repo.commit("second");
        assert_ne!(first, second);
        assert_eq!(repo.git(&["rev-parse", "main"]), second, "the branch moved");
        assert_eq!(tree.read(&path("a.md")).unwrap(), b"first\n");
        assert!(!tree.exists(&path("b.md")));
        assert_eq!(
            id.as_str(),
            repo.git(&["rev-parse", &format!("{first}^{{tree}}")])
        );
        let (later, later_id) = GitTree::revision(repo.root(), "main").unwrap();
        assert_eq!(later.read(&path("a.md")).unwrap(), b"second\n");
        assert_ne!(id, later_id);
    }

    #[test]
    fn revision_blobs_match_git_show() {
        let repo = planted();
        let (tree, id) = GitTree::revision(repo.root(), "HEAD").unwrap();
        let mut compared = 0;
        for (entry_path, entry) in tree.entries() {
            if !matches!(entry.kind, Kind::File | Kind::Executable) {
                continue;
            }
            let output = repo
                .command(&["show", &format!("{id}:{entry_path}")])
                .output()
                .expect("git show");
            assert!(output.status.success());
            assert_eq!(
                tree.read(&entry_path).unwrap(),
                output.stdout,
                "{entry_path}"
            );
            compared += 1;
        }
        assert_eq!(compared, 3);
    }

    #[test]
    fn projects_below_the_root_only_see_their_prefix() {
        let repo = Repo::new();
        repo.stage("100644", b"top\n", "top.md");
        repo.stage("100644", b"inside\n", "packages/docs/content/a.md");
        repo.stage("120000", b"../../../top.md", "packages/docs/content/up.md");
        repo.stage("120000", b"../content/a.md", "packages/docs/rules/side.md");
        repo.commit("layout");
        std::fs::create_dir_all(repo.root().join("packages/docs")).unwrap();
        let project = repo.root().join("packages/docs");
        for tree in [
            GitTree::index(&project).unwrap(),
            GitTree::revision(&project, "HEAD").unwrap().0,
        ] {
            let paths: Vec<String> = tree.entries().map(|(p, _)| p.as_str().to_owned()).collect();
            assert_eq!(
                paths,
                [
                    "content",
                    "content/a.md",
                    "content/up.md",
                    "rules",
                    "rules/side.md"
                ]
            );
            assert!(!tree.exists(&path("top.md")));
            assert!(!tree.exists(&path("packages")));
            assert_eq!(tree.read(&path("rules/side.md")).unwrap(), b"inside\n");
            let error = tree.read(&path("content/up.md")).unwrap_err();
            assert!(error.to_string().contains("leaves the project"), "{error}");
        }
        let error = GitTree::revision(&repo.root().join("packages"), "HEAD:top.md").unwrap_err();
        assert!(error.to_string().contains("names a blob"), "{error}");
    }

    #[test]
    fn a_baseline_goes_empty_only_when_git_proves_the_project_absent() {
        let repo = Repo::new();
        repo.stage("100644", b"top\n", "README");
        let before = repo.commit("before the project");
        repo.stage("100644", b"inside\n", "packages/docs/content/a.md");
        let with = repo.commit("with the project");
        std::fs::create_dir_all(repo.root().join("packages/docs")).unwrap();
        let project = repo.root().join("packages/docs");

        // Genuinely absent: an empty baseline, still an error for a candidate.
        let (tree, id) = GitTree::baseline(&project, &before).unwrap();
        assert_eq!(tree.entries().count(), 0);
        assert_eq!(
            id.as_str(),
            repo.git(&["rev-parse", &format!("{before}^{{tree}}")])
        );
        let error = GitTree::revision(&project, &before).unwrap_err();
        assert!(error.to_string().contains("does not contain"), "{error}");

        // Present: the normal subtree.
        let (tree, _) = GitTree::baseline(&project, &with).unwrap();
        assert_eq!(
            tree.entries()
                .map(|(p, _)| p.as_str().to_owned())
                .collect::<Vec<_>>(),
            ["content", "content/a.md"]
        );

        // A file, then a link, occupying the prefix: fatal either way.
        repo.git(&["rm", "-r", "-q", "--cached", "packages/docs"]);
        repo.stage("100644", b"not a directory\n", "packages/docs");
        let as_file = repo.commit("file at the prefix");
        let error = GitTree::baseline(&project, &as_file).unwrap_err();
        assert!(
            error.to_string().contains("is a file, not a directory"),
            "{error}"
        );
        repo.git(&["rm", "-q", "--cached", "packages/docs"]);
        repo.stage("120000", b"../elsewhere", "packages/docs");
        let as_link = repo.commit("link at the prefix");
        let error = GitTree::baseline(&project, &as_link).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("is a symbolic link, not a directory"),
            "{error}"
        );
        let error = GitTree::revision(&project, &as_link).unwrap_err();
        assert!(error.to_string().contains("symbolic link"), "{error}");

        // An unreadable object on the way to the prefix: fatal, never empty.
        let packages = repo.git(&["rev-parse", &format!("{with}:packages")]);
        let (dir, file) = packages.split_at(2);
        let object = repo.root().join(".git/objects").join(dir).join(file);
        assert!(
            object.is_file(),
            "loose object expected at {}",
            object.display()
        );
        std::fs::remove_file(&object).unwrap();
        let error = GitTree::baseline(&project, &with).unwrap_err();
        assert!(error.to_string().contains("git ls-tree failed"), "{error}");
        let error = GitTree::revision(&project, &with).unwrap_err();
        assert!(error.to_string().contains("git ls-tree failed"), "{error}");
    }

    #[test]
    fn unmerged_entries_fail_closed() {
        let repo = Repo::new();
        repo.stage("100644", b"base\n", "a.md");
        repo.commit("base");
        let base = repo.blob(b"base\n");
        let ours = repo.blob(b"ours\n");
        let theirs = repo.blob(b"theirs\n");
        let info = format!(
            "0 {}\ta.md\n100644 {base} 1\ta.md\n100644 {ours} 2\ta.md\n100644 {theirs} 3\ta.md\n",
            "0".repeat(40)
        );
        let mut child = repo
            .command(&["update-index", "--index-info"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(info.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
        let error = GitTree::index(repo.root()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "the index has unmerged entries: a.md; resolve the conflict before checking the index"
        );
        assert!(GitTree::revision(repo.root(), "HEAD").is_ok());
    }

    #[test]
    fn missing_git_and_missing_projects_are_clean_failures() {
        let error = GitTree::index(Path::new("/definitely/not/here")).unwrap_err();
        assert!(
            error.to_string().starts_with("cannot open project"),
            "{error}"
        );
        let dir = tempfile::tempdir().unwrap();
        let error = GitTree::index(dir.path()).unwrap_err();
        assert!(
            error.to_string().contains("not a git repository"),
            "{error}"
        );
    }
}
