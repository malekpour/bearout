// SPDX-License-Identifier: Apache-2.0

//! Test helpers: a minimal project builder and fixture loading.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use bearout::{Code, Command, Mode, Options, Report};
use tempfile::TempDir;

/// A throwaway project on disk.
pub struct Project {
    dir: TempDir,
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
        let dir = tempfile::tempdir().expect("temporary project");
        let project = Self { dir };
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
        let dir = tempfile::tempdir().expect("temporary project");
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        copy_dir(&source, dir.path());
        Self { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Write a file, creating parent directories.
    pub fn file(&self, relative: &str, text: &str) -> &Self {
        let path = self.dir.path().join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, text).expect("write file");
        self
    }

    pub fn bytes(&self, relative: &str, bytes: &[u8]) -> &Self {
        let path = self.dir.path().join(relative);
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, bytes).expect("write file");
        self
    }

    pub fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.dir.path().join(relative)).expect("read file")
    }

    pub fn exists(&self, relative: &str) -> bool {
        self.dir.path().join(relative).exists()
    }

    pub fn remove(&self, relative: &str) {
        fs::remove_file(self.dir.path().join(relative)).expect("remove file");
    }

    pub fn check(&self) -> Report {
        bearout::run(self.dir.path(), Command::Check, &Options::default())
    }

    pub fn generate(&self, mode: Mode) -> Report {
        bearout::run(
            self.dir.path(),
            Command::Generate(mode),
            &Options::default(),
        )
    }

    pub fn run(&self, command: Command, options: &Options) -> Report {
        bearout::run(self.dir.path(), command, options)
    }
}

impl Default for Project {
    fn default() -> Self {
        Self::new()
    }
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
        "expected a diagnostic containing {expected:?}, got:\n{}",
        rendered.join("\n")
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

pub fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("samples")
}
