// SPDX-License-Identifier: Apache-2.0

//! Test helpers: a minimal project builder, fixture loading, and a hermetic
//! Git wrapper for the source tests.

#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};

use bearout::{Code, Command, Mode, Options, Report, Source};
use tempfile::TempDir;

/// A throwaway project on disk. The temporary directory is the repository
/// root when Git is initialized; the project root is the same directory or,
/// for [`Project::at`], a directory beneath it.
pub struct Project {
    dir: TempDir,
    root: PathBuf,
}

pub const ENTRY: &str = "bearout.star";

/// The bootstrap most tests start from.
pub const BOOTSTRAP: &str = "version = 1\nentry = \"bearout.star\"\n\n[resources]\nroots = [\"content\"]\n\n[rules]\nroot = \"rules\"\n";

/// The bootstrap with templates and an output root.
pub const BOOTSTRAP_GEN: &str = "version = 1\nentry = \"bearout.star\"\n\n[resources]\nroots = [\"content\"]\n\n[rules]\nroot = \"rules\"\n\n[templates]\nroot = \"templates\"\n\n[outputs]\nroots = [\"generated\"]\nlicense = \"Apache-2.0\"\n";

pub const NOTE_SHAPE: &str = "\"$schema\" = \"https://json-schema.org/draft/2020-12/schema\"\ntype = \"object\"\nadditionalProperties = false\nrequired = [\"title\"]\n\n[properties.title]\ntype = \"string\"\nminLength = 1\n\n[properties.next]\ntype = \"string\"\n\"x-bearout\" = { ref = \"example/test/note@1\" }\n";

impl Project {
    /// A project with the default bootstrap and no files.
    pub fn new() -> Self {
        Self::at("")
    }

    /// A project rooted `relative` beneath the temporary directory, with
    /// the default bootstrap and no files.
    pub fn at(relative: &str) -> Self {
        let dir = tempfile::tempdir().expect("temporary project");
        let root = dir.path().join(relative);
        fs::create_dir_all(&root).expect("project root");
        let project = Self { dir, root };
        project.file("bearout.toml", BOOTSTRAP);
        project
    }

    /// A project with a note schema, one validator that returns nothing, and one note.
    pub fn with_note() -> Self {
        let project = Self::new();
        project.file(
            ENTRY,
            "schema(\"example/test/note@1\", shape = \"note.schema.toml\")\n",
        );
        project.file("rules/note.schema.toml", NOTE_SHAPE);
        project.file("content/note-a.md", "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n# A\n\nBody.\n");
        project
    }

    /// Copy a fixture directory from `tests/fixtures/<name>`.
    pub fn fixture(name: &str) -> Self {
        Self::copied(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
    }

    /// Copy a sample from `samples/<name>`.
    pub fn sample(name: &str) -> Self {
        Self::copied(&samples_dir().join(name))
    }

    fn copied(source: &Path) -> Self {
        let dir = tempfile::tempdir().expect("temporary project");
        copy_dir(source, dir.path());
        let root = dir.path().to_path_buf();
        Self { dir, root }
    }

    /// The project root.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The temporary directory: the repository root once `git_init` ran.
    pub fn repo_path(&self) -> &Path {
        self.dir.path()
    }

    /// Write a file, creating parent directories.
    pub fn file(&self, relative: &str, text: &str) -> &Self {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, text).expect("write file");
        self
    }

    pub fn bytes(&self, relative: &str, bytes: &[u8]) -> &Self {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, bytes).expect("write file");
        self
    }

    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative)).expect("read file")
    }

    pub fn exists(&self, relative: &str) -> bool {
        self.root.join(relative).exists()
    }

    pub fn remove(&self, relative: &str) {
        fs::remove_file(self.root.join(relative)).expect("remove file");
    }

    pub fn check(&self) -> Report {
        bearout::run(&self.root, Command::Check, &Options::default())
    }

    pub fn generate(&self, mode: Mode) -> Report {
        bearout::run(&self.root, Command::Generate(mode), &Options::default())
    }

    pub fn run(&self, command: Command, options: &Options) -> Report {
        bearout::run(&self.root, command, options)
    }

    /// Run `command` reading the project from `source`.
    pub fn run_from(&self, source: Source, command: Command) -> Report {
        bearout::run(
            &self.root,
            command,
            &Options {
                source,
                ..Options::default()
            },
        )
    }

    pub fn check_from(&self, source: Source) -> Report {
        self.run_from(source, Command::Check)
    }

    /// `generate --check` from `source`.
    pub fn verify_from(&self, source: Source) -> Report {
        self.run_from(source, Command::Generate(Mode::Check))
    }

    // ---- Git ---------------------------------------------------------

    /// Initialize a repository at the temporary directory with a `main`
    /// branch and no signing, then return the project.
    pub fn git_init(&self) -> &Self {
        git_run(self.repo_path(), &["init", "-q", "-b", "main"]);
        git_run(self.repo_path(), &["config", "core.autocrlf", "false"]);
        git_run(self.repo_path(), &["config", "commit.gpgsign", "false"]);
        self
    }

    /// Run Git in the project root and return its trimmed standard output.
    pub fn git(&self, args: &[&str]) -> String {
        git_run(&self.root, args)
    }

    /// Run Git in the project root, expecting failure; returns stderr.
    pub fn git_fails(&self, args: &[&str]) -> String {
        let output = git_command(&self.root)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            !output.status.success(),
            "git {args:?} unexpectedly succeeded"
        );
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    /// Stage everything beneath the project root and commit; returns the
    /// commit identity.
    pub fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A", "."]);
        self.git(&["commit", "-q", "--allow-empty", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    /// Write `content` as a blob and return its identity.
    pub fn blob(&self, content: &[u8]) -> String {
        let mut child = git_command(&self.root)
            .args(["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn git");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(content)
            .expect("write blob");
        let output = child.wait_with_output().expect("hash object");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("utf-8")
            .trim()
            .to_owned()
    }

    /// Plant an index entry of any mode at a project-relative path without
    /// touching the working tree: `100644`, `100755`, `120000` (the content
    /// is the link target), or `160000` (a gitlink; the content is ignored
    /// and `HEAD` is used).
    pub fn stage_entry(&self, mode: &str, content: &[u8], path: &str) -> &Self {
        let object = if mode == "160000" {
            self.git(&["rev-parse", "HEAD"])
        } else {
            self.blob(content)
        };
        self.git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("{mode},{object},{path}"),
        ]);
        self
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
}

/// A Git command that ignores the caller's Git environment (a pre-commit
/// hook exports `GIT_DIR` and `GIT_INDEX_FILE`) and every user or system
/// configuration file, so tests behave the same everywhere.
pub fn git_command(cwd: &Path) -> Process {
    let mut command = Process::new("git");
    command
        .current_dir(cwd)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            cwd.join(".bearout-test-no-global-config"),
        )
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
    command
}

pub fn git_run(cwd: &Path, args: &[&str]) -> String {
    let output = git_command(cwd).args(args).output().expect("run git");
    assert!(
        output.status.success(),
        "git {args:?} in {} failed:\n{}",
        cwd.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git printed utf-8")
        .trim_end()
        .to_owned()
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create directory");
    for entry in fs::read_dir(from).expect("read directory") {
        let entry = entry.expect("directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

/// Rendered diagnostics, one per line, in report order.
pub fn lines(report: &Report) -> Vec<String> {
    report.diagnostics.iter().map(ToString::to_string).collect()
}

/// The codes of every diagnostic, in report order.
pub fn codes(report: &Report) -> Vec<Code> {
    report.diagnostics.iter().map(|d| d.code).collect()
}

/// Assert that some diagnostic renders to a line containing `expected`.
pub fn assert_line(report: &Report, expected: &str) {
    let rendered = lines(report);
    assert!(
        rendered.iter().any(|line| line.contains(expected)),
        "expected a diagnostic containing {expected:?}, got:\n{}\nfatal: {:?}",
        rendered.join("\n"),
        report.fatal
    );
}

/// Assert that no diagnostic mentions `unexpected`.
pub fn assert_no_line(report: &Report, unexpected: &str) {
    let rendered = lines(report);
    assert!(
        !rendered.iter().any(|line| line.contains(unexpected)),
        "did not expect a diagnostic containing {unexpected:?}, got:\n{}",
        rendered.join("\n")
    );
}

pub fn assert_clean(report: &Report) {
    assert!(
        report.is_clean(),
        "expected a clean report, got:\n{}\nfatal: {:?}",
        lines(report).join("\n"),
        report.fatal
    );
}

/// Assert a fatal outcome whose message contains `expected`.
pub fn assert_fatal(report: &Report, expected: &str) {
    assert!(
        report
            .fatal
            .as_deref()
            .is_some_and(|message| message.contains(expected)),
        "expected a fatal outcome containing {expected:?}, got fatal {:?} and:\n{}",
        report.fatal,
        lines(report).join("\n")
    );
    assert!(!report.ok);
    assert!(report.diagnostics.is_empty());
    assert!(report.outputs.is_empty());
}

pub fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("samples")
}
