// SPDX-License-Identifier: Apache-2.0

//! Repository hygiene: the explicit file selection across sources, native
//! text hygiene, and external formatters.

mod common;

use std::process::Command as Process;

use bearout::{Command, Options, Source};
use common::{Project, assert_clean, assert_fatal};

fn hygiene_bootstrap(body: &str) -> String {
    format!("{}\n[hygiene]\n{body}\n", common::BOOTSTRAP)
}

/// A note project selecting every repository file, with a spread of
/// tracked, untracked, ignored, linked, and nested content.
fn repository_project() -> Project {
    let project = Project::with_note();
    project.file("bearout.toml", &hygiene_bootstrap("scope = \"repository\""));
    project.file("docs/guide.md", "# Guide\n");
    project.file("tools/run.sh", "#!/bin/sh\necho hi\n");
    project.file(".gitignore", "*.log\nbuild/\n");
    project.file("debug.log", "ignored\n");
    project.file("build/out.txt", "ignored\n");
    project
}

fn files_of(report: &bearout::Report) -> usize {
    report.files
}

#[test]
fn no_hygiene_grant_selects_nothing() {
    let project = Project::with_note();
    let report = project.check();
    assert_clean(&report);
    assert_eq!(report.files, 0);
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["files"], 0);
}

#[test]
fn the_working_directory_universe_is_tracked_plus_untracked_non_ignored_files() {
    let project = repository_project();
    project.git_init();
    // Nothing committed yet: every non-ignored file is untracked.
    let report = project.check();
    assert_clean(&report);
    // bearout.toml, bearout.star, rules/note.schema.toml, content/note-a.md,
    // docs/guide.md, tools/run.sh, .gitignore: not debug.log, not build/.
    assert_eq!(files_of(&report), 7);

    project.commit_all("all");
    project.file("notes/untracked.md", "# New\n");
    assert_eq!(
        files_of(&project.check()),
        8,
        "an untracked file joins the universe"
    );
    project.remove("docs/guide.md");
    assert_eq!(
        files_of(&project.check()),
        7,
        "a tracked file deleted from disk is absent"
    );
    assert_eq!(
        files_of(&project.check_from(Source::Index)),
        7,
        "the index still holds the deleted file and lacks the untracked one"
    );
    assert_eq!(
        files_of(&project.check_from(Source::Revision("HEAD".to_owned()))),
        7
    );
}

#[test]
fn git_backed_universes_are_the_captured_trees() {
    let project = repository_project();
    project.git_init();
    project.commit_all("base");
    let committed = files_of(&project.check_from(Source::Revision("HEAD".to_owned())));
    assert_eq!(committed, 7);

    // Staged addition, staged deletion, and a rename, none touching the
    // committed revision.
    project.file("docs/new.md", "# New\n");
    project.git(&["add", "docs/new.md"]);
    project.git(&["rm", "-q", "--cached", "tools/run.sh"]);
    project.git(&["mv", "docs/guide.md", "docs/renamed.md"]);
    let index = files_of(&project.check_from(Source::Index));
    assert_eq!(index, 7, "one added, one removed, one renamed");
    assert_eq!(
        files_of(&project.check_from(Source::Revision("HEAD".to_owned()))),
        7
    );
    // The working directory still has tools/run.sh on disk (untracked now).
    assert_eq!(files_of(&project.check()), 8);

    // Planted entries: a symlink and a gitlink are never files of the
    // selection; an executable is.
    project.stage_entry("120000", b"note-a.md", "content/alias.md");
    project.stage_entry("160000", b"", "vendor");
    project.stage_entry("100755", b"#!/bin/sh\n", "tools/exec.sh");
    assert_eq!(files_of(&project.check_from(Source::Index)), 8);
}

#[test]
fn repository_scope_needs_git_and_declared_scope_does_not() {
    let project = repository_project();
    assert_fatal(
        &project.check(),
        "`hygiene.scope = \"repository\"` needs the project inside a Git repository",
    );
    project.file(
        "bearout.toml",
        &hygiene_bootstrap(
            "scope = \"declared\"\nroots = [\"docs\", \"tools\"]\nfiles = [\"README.md\"]",
        ),
    );
    project.file("README.md", "# Read me\n");
    let report = project.check();
    assert_clean(&report);
    assert_eq!(files_of(&report), 3);
    project.file(
        "bearout.toml",
        &hygiene_bootstrap("scope = \"declared\"\nfiles = [\"MISSING.md\"]"),
    );
    assert_fatal(
        &project.check(),
        "hygiene file `MISSING.md` is not a file inside the project",
    );
    project.file(
        "bearout.toml",
        &hygiene_bootstrap("scope = \"declared\"\nroots = [\"nowhere\"]"),
    );
    assert_fatal(
        &project.check(),
        "hygiene root `nowhere` is not a directory",
    );
}

#[test]
fn exclusions_binary_and_text_lists_refine_the_selection() {
    let project = repository_project();
    project.file(
        "bearout.toml",
        &hygiene_bootstrap(
            "scope = \"repository\"\nexclude = [\"tools\", \"docs/guide.md\"]\nbinary = [\"assets\"]\ntext = [\"assets/readme.txt\"]",
        ),
    );
    project.bytes("assets/logo.bin", b"\x00\x01\x02");
    project.file("assets/readme.txt", "text\n");
    project.git_init();
    let report = project.check();
    assert_clean(&report);
    // bearout.toml, bearout.star, rules/note.schema.toml, content/note-a.md,
    // .gitignore, assets/logo.bin, assets/readme.txt.
    assert_eq!(files_of(&report), 7);
}

#[test]
fn a_formatter_may_claim_each_path_at_most_once() {
    let project = repository_project();
    project.file(
        "bearout.toml",
        &hygiene_bootstrap(
            "scope = \"repository\"\n\n[[formatters]]\nname = \"a\"\ncommand = [\"true\"]\nextensions = [\"md\"]\n\n[[formatters]]\nname = \"b\"\ncommand = [\"true\"]\npaths = [\"docs\"]",
        ),
    );
    project.git_init();
    assert_fatal(
        &project.check(),
        "`docs/guide.md` is assigned to formatters `a` and `b`; a file may have at most one formatter",
    );
}

#[test]
fn selection_is_confined_below_the_repository_root_and_works_in_linked_worktrees() {
    let project = Project::at("packages/docs");
    project.file(
        common::ENTRY,
        "schema(\"example/test/note@1\", shape = \"note.schema.toml\")\n",
    );
    project.file("rules/note.schema.toml", common::NOTE_SHAPE);
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n",
    );
    project.file("bearout.toml", &hygiene_bootstrap("scope = \"repository\""));
    project.git_init();
    std::fs::write(project.repo_path().join("OUTSIDE.md"), "# Outside\n").expect("write");
    common::git_run(project.repo_path(), &["add", "-A", "."]);
    common::git_run(project.repo_path(), &["commit", "-q", "-m", "all"]);
    for source in [
        Source::WorkingDirectory,
        Source::Index,
        Source::Revision("HEAD".to_owned()),
    ] {
        let report = project.check_from(source);
        assert_clean(&report);
        assert_eq!(files_of(&report), 4, "only the project's own files");
    }
    let linked = tempfile::tempdir().expect("worktree dir");
    let linked_path = linked.path().join("wt");
    project.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "feature",
        linked_path.to_str().unwrap(),
    ]);
    let inside = linked_path.join("packages/docs");
    std::fs::write(inside.join("extra.txt"), "x\n").expect("write");
    let report = bearout::run(&inside, Command::Check, &Options::default());
    assert_clean(&report);
    assert_eq!(files_of(&report), 5);
}

#[test]
fn file_limits_and_non_portable_names_fail_closed() {
    let project = repository_project();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nfiles = 3\n",
            hygiene_bootstrap("scope = \"repository\"")
        ),
    );
    project.git_init();
    assert_fatal(&project.check(), "selected files exceed `limits.files` = 3");
    project.file("bearout.toml", &hygiene_bootstrap("scope = \"repository\""));
    project.commit_all("base");
    project.stage_entry("100644", b"x", "notes/a:b.txt");
    assert_fatal(
        &project.check_from(Source::Index),
        "contains an entry that is not a portable path segment",
    );
}

#[test]
fn hostile_git_variables_cannot_redirect_the_working_universe() {
    let project = repository_project();
    project.git_init();
    project.commit_all("base");
    let other = Project::with_note();
    other.git_init();
    other.file("stray.txt", "x\n");
    other.commit_all("other");
    let path = project.path().to_str().unwrap();
    let output = Process::new(env!("CARGO_BIN_EXE_bearout"))
        .args(["--format", "json", "check", path])
        .env("GIT_DIR", other.repo_path().join(".git"))
        .env("GIT_WORK_TREE", other.path())
        .env("GIT_CEILING_DIRECTORIES", project.repo_path())
        .output()
        .expect("run bearout");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["files"], 7);
}

#[test]
fn repeated_selection_is_identical() {
    let project = repository_project();
    project.git_init();
    project.commit_all("base");
    let first = serde_json::to_string(&project.check()).unwrap();
    let second = serde_json::to_string(&project.check()).unwrap();
    assert_eq!(first, second);
}
