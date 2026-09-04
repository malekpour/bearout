// SPDX-License-Identifier: Apache-2.0

//! Candidate/baseline comparison: explicit baseline selection, resolution,
//! identity in the report, and the fatal outcomes.

mod common;

use std::process::Command as Process;

use bearout::{Command, Mode, Options, Source};
use common::{Project, assert_clean, assert_fatal};

fn options(source: Source, baseline: &str) -> Options {
    Options {
        source,
        baseline: Some(baseline.to_owned()),
        ..Options::default()
    }
}

/// A committed note project with one later commit on `main`.
fn history() -> (Project, String, String) {
    let project = Project::with_note();
    project.git_init();
    let first = project.commit_all("first");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    let second = project.commit_all("second");
    (project, first, second)
}

#[test]
fn a_baseline_is_resolved_once_and_reported() {
    let (project, first, second) = history();
    for source in [
        Source::WorkingDirectory,
        Source::Index,
        Source::Revision("HEAD".to_owned()),
    ] {
        let report = project.run(Command::Check, &options(source.clone(), "main"));
        assert_clean(&report);
        let baseline = report.baseline.as_ref().expect("baseline identity");
        assert_eq!(baseline.kind, "revision");
        assert_eq!(baseline.revision.as_deref(), Some("main"));
        assert_eq!(
            baseline.tree.as_deref(),
            Some(
                project
                    .git(&["rev-parse", &format!("{second}^{{tree}}")])
                    .as_str()
            )
        );
        assert!(baseline.digest.starts_with("blake3:"));
        let json = serde_json::to_value(&report).expect("json");
        assert_eq!(json["baseline"]["revision"], "main");
        assert_eq!(json["baseline"]["kind"], "revision");
        // The candidate's own identity is untouched by the comparison.
        assert_eq!(report.source.is_some(), source != Source::WorkingDirectory);
    }
    let report = project.run(Command::Check, &options(Source::Index, &first));
    assert_eq!(
        report.baseline.unwrap().tree.unwrap(),
        project.git(&["rev-parse", &format!("{first}^{{tree}}")])
    );
    let report = project.check();
    assert!(report.baseline.is_none());
    let json = serde_json::to_value(&report).expect("json");
    assert!(json.get("baseline").is_none(), "absent when not requested");
}

#[test]
fn invalid_baselines_are_fatal() {
    let (project, _, _) = history();
    for (revision, expected) in [
        ("nope", "`nope` is not a revision of this repository"),
        ("", "is not a revision name"),
        ("--output=x", "is not a revision name"),
        ("main\n", "is not a revision name"),
        ("HEAD:bearout.toml", "names a blob"),
        (&"0".repeat(40), "is not a revision"),
    ] {
        let report = project.run(Command::Check, &options(Source::WorkingDirectory, revision));
        assert_fatal(&report, "cannot read the baseline: ");
        assert_fatal(&report, expected);
        assert!(report.baseline.is_none());
        let json = serde_json::to_value(&report).expect("json");
        assert_eq!(json["ok"], false);
    }
    // Outside any repository, the baseline cannot be opened at all.
    let plain = Project::with_note();
    assert_fatal(
        &plain.run(Command::Check, &options(Source::WorkingDirectory, "HEAD")),
        "cannot read the baseline: git rev-parse failed",
    );
}

#[test]
fn a_baseline_that_predates_the_project_is_an_empty_history() {
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
    project.git_init();
    std::fs::write(project.repo_path().join("README"), "before the project\n").expect("write");
    common::git_run(project.repo_path(), &["add", "README"]);
    common::git_run(project.repo_path(), &["commit", "-q", "-m", "before"]);
    let before = project.git(&["rev-parse", "HEAD"]);
    project.commit_all("project");
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &before));
    assert_clean(&report);
    let baseline = report.baseline.unwrap();
    assert_eq!(
        baseline.tree.unwrap(),
        project.git(&["rev-parse", &format!("{before}^{{tree}}")]),
        "the revision's tree identity is still recorded"
    );
    assert_eq!(
        baseline.digest, "blake3:af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        "the empty tree has one digest"
    );
    // A candidate revision that predates the project stays an error.
    assert_fatal(
        &project.check_from(Source::Revision(before)),
        "does not contain the project directory",
    );
}

#[test]
fn writing_generation_with_a_baseline_needs_the_working_directory() {
    let (project, _, _) = history();
    assert_fatal(
        &project.run(
            Command::Generate(Mode::Write),
            &options(Source::Index, "main"),
        ),
        "read-only",
    );
    // The working directory may generate while comparing; check mode may
    // compare from any candidate.
    let report = project.run(
        Command::Generate(Mode::Write),
        &options(Source::WorkingDirectory, "main"),
    );
    assert_clean(&report);
    assert!(report.baseline.is_some());
    for source in [Source::Index, Source::Revision("main".to_owned())] {
        let report = project.run(Command::Generate(Mode::Check), &options(source, "main"));
        assert_clean(&report);
        assert!(report.baseline.is_some());
    }
}

#[test]
fn the_cli_accepts_a_baseline_with_every_candidate() {
    let (project, first, _) = history();
    let path = project.path().to_str().expect("utf-8 path");
    let run = |args: &[&str]| {
        let output = Process::new(env!("CARGO_BIN_EXE_bearout"))
            .args(args)
            .output()
            .expect("run bearout");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (code, _, stderr) = run(&["check", "--baseline", "main", path]);
    assert_eq!(code, 0, "{stderr}");
    let (code, _, _) = run(&["check", "--index", "--baseline", &first, path]);
    assert_eq!(code, 0);
    let (code, _, _) = run(&["check", "--revision", "HEAD", "--baseline", &first, path]);
    assert_eq!(code, 0);
    let (code, stdout, _) = run(&[
        "--format",
        "json",
        "generate",
        "--check",
        "--index",
        "--baseline",
        "HEAD",
        path,
    ]);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(json["baseline"]["revision"], "HEAD");
    assert_eq!(json["source"]["kind"], "index");

    let (code, stdout, stderr) = run(&["--format", "json", "check", "--baseline", "nope", path]);
    assert_eq!(code, 2);
    assert!(stderr.is_empty());
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert!(
        json["fatal"]
            .as_str()
            .unwrap()
            .contains("cannot read the baseline")
    );
    let (code, _, stderr) = run(&["check", "--baseline", "nope", path]);
    assert_eq!(code, 2);
    assert!(stderr.starts_with("bearout: cannot read the baseline: `nope` is not a revision"));
    let (code, _, _) = run(&["check", "--baseline=--output=x", path]);
    assert_eq!(code, 2);
    let (code, _, _) = run(&["generate", "--index", "--baseline", "main", path]);
    assert_eq!(
        code, 2,
        "write generation with a Git candidate stays refused"
    );
    let (code, stdout, _) = run(&["check", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--baseline <REV>"));
}
