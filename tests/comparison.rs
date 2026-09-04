// SPDX-License-Identifier: Apache-2.0

//! Candidate/baseline comparison: explicit baseline selection, resolution,
//! identity in the report, and the fatal outcomes.

mod common;

use std::process::Command as Process;

use bearout::{Code, Command, Mode, Options, Source};
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

// ---- historical projection --------------------------------------------------

use common::{assert_line, assert_no_line, codes, lines};

/// The entry module of a check that reports what the comparison view holds.
const INSPECT: &str = concat!(
    "def inspect(p):\n",
    "    c = p[\"comparison\"]\n",
    "    if c == None:\n",
    "        return [warning(\"no comparison\", resource = \"note-a\")]\n",
    "    b = c[\"baseline\"]\n",
    "    return [warning(\"baseline %s ids %s docs %s\" % (b[\"revision\"], \",\".join(sorted(b[\"by_id\"].keys())), \",\".join([d[\"path\"] for d in b[\"documents\"]])), resource = \"note-a\")]\n",
    "schema(\"example/test/note@1\", shape = \"note.schema.toml\")\n",
    "check(\"inspect\", inspect)\n",
);

fn inspecting_project() -> Project {
    let project = Project::with_note();
    project.file(common::ENTRY, INSPECT);
    project
}

#[test]
fn the_comparison_view_is_none_without_a_baseline_and_holds_history_with_one() {
    let project = inspecting_project();
    project.git_init();
    let first = project.commit_all("first");
    assert_line(
        &project.check(),
        "content/note-a.md:B016: check `inspect`: no comparison",
    );

    project.file(
        "bearout.toml",
        &format!(
            "{}\n[documents]\nfiles = [\"NOTES.md\"]\n",
            common::BOOTSTRAP
        ),
    );
    project.file("NOTES.md", "# Notes\n");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    let second = project.commit_all("second");
    project.remove("content/note-a.md");
    project.file(
        "content/note-c.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"moved\"\n+++\n",
    );

    // Each side is classified by its own manifest: the first commit selected
    // no documents and held only note-a; the second holds both notes and
    // the document. The candidate is what is on disk now.
    for (source, expected) in [
        (
            Source::WorkingDirectory,
            "baseline main ids note-a,note-b docs NOTES.md",
        ),
        (
            Source::Index,
            "baseline main ids note-a,note-b docs NOTES.md",
        ),
    ] {
        let report = project.run(Command::Check, &options(source, "main"));
        assert_clean(&report);
        assert_line(&report, expected);
    }
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &first));
    assert_clean(&report);
    assert_line(
        &report,
        "baseline first-commit ids note-a docs "
            .replace("first-commit", &first)
            .as_str(),
    );
    assert_line(
        &project.run(
            Command::Check,
            &options(Source::Revision(first.clone()), &second),
        ),
        "baseline",
    );
    // Identical candidate bytes give the same view whichever source they
    // come from.
    let from_working = project.run(Command::Check, &options(Source::WorkingDirectory, &first));
    project.git(&["add", "-A", "."]);
    let staged = project.run(Command::Check, &options(Source::Index, &first));
    assert_eq!(lines(&staged), lines(&from_working));
    assert_eq!(
        serde_json::to_string(&staged.baseline).unwrap(),
        serde_json::to_string(&from_working.baseline).unwrap()
    );
}

#[test]
fn a_missing_historical_manifest_is_an_empty_project_and_a_malformed_one_is_fatal() {
    let project = inspecting_project();
    project.git_init();
    std::fs::remove_file(project.path().join("bearout.toml")).expect("remove");
    let before = project.commit_all("no manifest");
    project.file("bearout.toml", common::BOOTSTRAP);
    project.commit_all("manifest");
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &before));
    assert_clean(&report);
    assert_line(&report, &format!("baseline {before} ids  docs "));

    project.file("bearout.toml", "version = 1\nentry = \"bearout.star\"\n[resources]\nroots = [\"content\"]\n[rules]\nroot = \"rules\"\nbogus = 1\n");
    let broken = project.commit_all("broken manifest");
    project.file("bearout.toml", common::BOOTSTRAP);
    project.commit_all("fixed");
    assert_fatal(
        &project.run(Command::Check, &options(Source::WorkingDirectory, &broken)),
        &format!("baseline `{broken}`: bearout.toml is not usable: unknown key `rules.bogus`"),
    );
}

#[test]
fn roots_and_documents_present_on_one_side_only_stay_visible() {
    let project = inspecting_project();
    project.file(
        "bearout.toml",
        "version = 1\nentry = \"bearout.star\"\n[resources]\nroots = [\"content\", \"archive\"]\n[rules]\nroot = \"rules\"\n[documents]\nfiles = [\"OLD.md\"]\n",
    );
    project.file(
        "archive/note-old.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-old\"\ntitle = \"Old\"\n+++\n",
    );
    project.file("OLD.md", "# Old\n");
    project.git_init();
    let old = project.commit_all("old layout");
    // The candidate drops the archive root and the document, and adds a new
    // root with a new resource and a new document.
    project.file(
        "bearout.toml",
        "version = 1\nentry = \"bearout.star\"\n[resources]\nroots = [\"content\", \"fresh\"]\n[rules]\nroot = \"rules\"\n[documents]\nfiles = [\"NEW.md\"]\n",
    );
    std::fs::remove_dir_all(project.path().join("archive")).expect("remove");
    project.remove("OLD.md");
    project.file(
        "fresh/note-new.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-new\"\ntitle = \"New\"\n+++\n",
    );
    project.file("NEW.md", "# New\n");
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &old));
    assert_clean(&report);
    assert_eq!(report.resources, 2);
    assert_eq!(report.documents, 1);
    assert_line(
        &report,
        &format!("baseline {old} ids note-a,note-old docs OLD.md"),
    );
}

#[test]
fn baseline_problems_are_diagnostics_on_the_baseline_side() {
    let project = inspecting_project();
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\nnext = \"note-a\"\n+++\n",
    );
    project.file(
        "content/note-c.md",
        "+++\nschema = \"example/test/other@1\"\nid = \"note-c\"\ntitle = \"C\"\n+++\n",
    );
    project.file(
        "rules/other.schema.toml",
        "\"$schema\" = \"https://json-schema.org/draft/2020-12/schema\"\ntype = \"object\"\n",
    );
    project.file(
        common::ENTRY,
        &format!("{INSPECT}schema(\"example/test/other@1\", shape = \"other.schema.toml\")\n"),
    );
    project.git_init();
    let old = project.commit_all("old");

    // The candidate drops the `other` schema, tightens the note shape so
    // that `next` is no longer allowed, and duplicates nothing itself.
    project.file(common::ENTRY, INSPECT);
    project.remove("rules/other.schema.toml");
    project.file(
        "rules/note.schema.toml",
        &common::NOTE_SHAPE.replace(
            "[properties.next]\ntype = \"string\"\n\"x-bearout\" = { ref = \"example/test/note@1\" }\n",
            "",
        ),
    );
    project.remove("content/note-c.md");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    assert_clean(&project.check());
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &old));
    assert!(report.fatal.is_none(), "{:?}", report.fatal);
    assert_eq!(
        lines(&report),
        [
            "baseline:content/note-b.md:5:B005: Additional properties are not allowed ('next' was unexpected)",
            "baseline:content/note-c.md:B003: schema `example/test/other@1` is not registered by the current policy; comparison interprets history with the candidate's schemas, so the policy must keep every schema its baseline uses",
        ]
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.side == bearout::Side::Baseline)
    );
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["diagnostics"][0]["side"], "baseline");
    assert_eq!(json["diagnostics"][0]["path"], "content/note-b.md");
    assert_eq!(report.errors(), 2, "baseline problems fail the run");
    assert_no_line(&report, "check `inspect`");

    // A candidate diagnostic carries no side in JSON and no text prefix.
    let plain = project.check();
    let json = serde_json::to_value(&plain).expect("json");
    assert_eq!(json["diagnostics"][0]["code"], "B016");
    assert!(json["diagnostics"][0].get("side").is_none());
    assert!(lines(&plain)[0].starts_with("content/note-a.md:"));
}

#[test]
fn duplicate_historical_ids_and_unreadable_history_are_reported_on_the_baseline() {
    let project = inspecting_project();
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"B\"\n+++\n",
    );
    project.file("content/note-c.md", "# not a resource\n");
    project.git_init();
    let old = project.commit_all("old");
    project.remove("content/note-b.md");
    project.remove("content/note-c.md");
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &old));
    assert_eq!(
        codes(&report),
        [Code::DuplicateId, Code::DuplicateId, Code::Envelope],
        "sorted by path within the baseline side"
    );
    assert_line(
        &report,
        "baseline:content/note-a.md:B008: identifier `note-a` is defined more than once",
    );
    assert_line(
        &report,
        "baseline:content/note-c.md:B002: resource must begin with TOML front matter",
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.side == bearout::Side::Baseline)
    );
    // Candidate findings sort before baseline findings.
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = 3\n+++\n",
    );
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &old));
    let sides: Vec<bearout::Side> = report.diagnostics.iter().map(|d| d.side).collect();
    assert_eq!(sides[0], bearout::Side::Candidate);
    assert!(sides[1..].iter().all(|s| *s == bearout::Side::Baseline));
    assert!(lines(&report)[0].starts_with("content/note-a.md:4:B005"));
}

#[test]
fn baseline_symlinks_and_gitlinks_stay_confined() {
    let project = inspecting_project();
    project.git_init();
    project.commit_all("clean");
    project.stage_entry("120000", b"note-a.md", "content/alias.md");
    project.stage_entry("160000", b"", "content/vendor");
    project.git(&["commit", "-q", "-m", "links"]);
    let linked = project.git(&["rev-parse", "HEAD"]);
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &linked));
    assert_clean(&report);
    assert_line(&report, &format!("baseline {linked} ids note-a docs "));

    // A historical resource root that is a symbolic link is refused, and
    // the run names the baseline.
    let object = project.git(&["rev-parse", "HEAD:content/note-a.md"]);
    project.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("100644,{object},real/note-a.md"),
    ]);
    project.git(&["rm", "-r", "-q", "--cached", "content"]);
    project.stage_entry("120000", b"real", "content");
    project.git(&["commit", "-q", "-m", "linked root"]);
    let bad = project.git(&["rev-parse", "HEAD"]);
    assert_fatal(
        &project.run(Command::Check, &options(Source::WorkingDirectory, &bad)),
        &format!(
            "baseline `{bad}`: cannot walk resource root `content`: `content` is a symbolic link"
        ),
    );
}

#[test]
fn baselines_work_below_the_repository_root_and_in_linked_worktrees() {
    let project = Project::at("packages/docs");
    project.file(common::ENTRY, INSPECT);
    project.file("rules/note.schema.toml", common::NOTE_SHAPE);
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n",
    );
    project.git_init();
    let first = project.commit_all("first");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    project.commit_all("second");
    let report = project.run(Command::Check, &options(Source::Index, &first));
    assert_clean(&report);
    assert_line(&report, &format!("baseline {first} ids note-a docs "));

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
    std::fs::write(
        inside.join("content/note-c.md"),
        "+++\nschema = \"example/test/note@1\"\nid = \"note-c\"\ntitle = \"C\"\n+++\n",
    )
    .expect("write");
    let report = bearout::run(
        &inside,
        Command::Check,
        &options(Source::WorkingDirectory, "main"),
    );
    assert_clean(&report);
    assert_eq!(report.resources, 3);
    assert_line(&report, "baseline main ids note-a,note-b docs ");
}

// ---- change facts -----------------------------------------------------------

/// A check that lists every change fact as `path:change:before>after`.
const CHANGES: &str = concat!(
    "def facts(p):\n",
    "    c = p[\"comparison\"]\n",
    "    if c == None:\n",
    "        return []\n",
    "    out = []\n",
    "    for ch in c[\"changes\"]:\n",
    "        b = ch[\"before\"][\"classification\"] + \"/\" + str(ch[\"before\"][\"bytes\"]) if ch[\"before\"] != None else \"-\"\n",
    "        a = ch[\"after\"][\"classification\"] + \"/\" + str(ch[\"after\"][\"bytes\"]) if ch[\"after\"] != None else \"-\"\n",
    "        out.append(\"%s:%s:%s>%s\" % (ch[\"path\"], ch[\"change\"], b, a))\n",
    "    return [warning(\"changes \" + \" \".join(out), resource = \"note-a\")]\n",
    "schema(\"example/test/note@1\", shape = \"note.schema.toml\")\n",
    "check(\"facts\", facts)\n",
);

#[test]
fn change_facts_cover_the_contract_surface_and_nothing_else() {
    let project = Project::with_note();
    project.file(common::ENTRY, CHANGES);
    project.file(
        "bearout.toml",
        &format!("{}\n[documents]\nroots = [\"notes\"]\n", common::BOOTSTRAP),
    );
    project.file("notes/keep.md", "# Keep\n");
    project.file("notes/gone.md", "# Gone\n");
    project.file("notes/edit.md", "# Edit\n");
    project.file(
        "content/note-old.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-old\"\ntitle = \"Old\"\n+++\n",
    );
    project.file("unrelated.txt", "not part of the contract surface\n");
    project.git_init();
    let old = project.commit_all("old");

    // Equal snapshots: no change facts, from every candidate source.
    for source in [
        Source::WorkingDirectory,
        Source::Index,
        Source::Revision("HEAD".to_owned()),
    ] {
        let report = project.run(Command::Check, &options(source, &old));
        assert_clean(&report);
        assert_line(&report, "check `facts`: changes ");
        assert!(
            lines(&report)[0].ends_with("changes "),
            "{:?}",
            lines(&report)
        );
    }

    // Edit, add, remove, rename, and touch something outside the surface.
    project.file("notes/edit.md", "# Edited\n");
    project.file("notes/new.md", "# New\n");
    project.remove("notes/gone.md");
    project.git(&["mv", "content/note-old.md", "content/note-renamed.md"]);
    project.file("unrelated.txt", "still not part of it\n");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[documents]\nroots = [\"notes\"]\n# a comment\n",
            common::BOOTSTRAP
        ),
    );
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &old));
    assert_clean(&report);
    assert_line(
        &report,
        "changes bearout.toml:modified:manifest/123>manifest/135 content/note-old.md:removed:resource/69>- content/note-renamed.md:added:->resource/69 notes/edit.md:modified:document/7>document/9 notes/gone.md:removed:document/7>- notes/new.md:added:->document/6",
    );
    // The same facts through the index once staged, and identical reports
    // on repeated runs.
    project.git(&["add", "-A", "."]);
    let first = project.run(Command::Check, &options(Source::Index, &old));
    let second = project.run(Command::Check, &options(Source::Index, &old));
    assert_eq!(lines(&first), lines(&report));
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
}

#[test]
fn reclassification_and_a_missing_manifest_show_in_the_facts() {
    let project = Project::with_note();
    project.file(common::ENTRY, CHANGES);
    project.file(
        "bearout.toml",
        &format!("{}\n[documents]\nroots = [\"notes\"]\n", common::BOOTSTRAP),
    );
    project.file(
        "notes/n.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-n\"\ntitle = \"N\"\n+++\n",
    );
    project.git_init();
    let as_document = project.commit_all("as document");
    // The same bytes, now selected as a resource root instead.
    project.file(
        "bearout.toml",
        &common::BOOTSTRAP.replace("roots = [\"content\"]", "roots = [\"content\", \"notes\"]"),
    );
    let report = project.run(
        Command::Check,
        &options(Source::WorkingDirectory, &as_document),
    );
    assert_clean(&report);
    assert_eq!(report.resources, 2);
    assert_line(&report, "notes/n.md:modified:document/65>resource/65");

    // Against a revision with no bearout.toml, everything is added, the
    // manifest included.
    std::fs::remove_file(project.path().join("bearout.toml")).expect("remove");
    let empty = project.commit_all("no manifest");
    project.file(
        "bearout.toml",
        &common::BOOTSTRAP.replace("roots = [\"content\"]", "roots = [\"content\", \"notes\"]"),
    );
    let report = project.run(Command::Check, &options(Source::WorkingDirectory, &empty));
    assert_clean(&report);
    assert_line(&report, "changes bearout.toml:added:->manifest/");
    assert_line(&report, "content/note-a.md:added:->resource/");
    assert_line(&report, "notes/n.md:added:->resource/65");
}

// ---- findings against either side -------------------------------------------

/// A history in which `note-b` and the document `OLD.md` exist only in the
/// baseline, `note-a` moved to another path, and `note-c` is new.
fn moved_history() -> (Project, String) {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[documents]\nfiles = [\"OLD.md\", \"NOTES.md\"]\n",
            common::BOOTSTRAP
        ),
    );
    project.file("OLD.md", "# Old\n\nline three\n");
    project.file("NOTES.md", "# Notes\n");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    project.git_init();
    let old = project.commit_all("old");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[documents]\nfiles = [\"NOTES.md\"]\n",
            common::BOOTSTRAP
        ),
    );
    project.remove("OLD.md");
    project.remove("content/note-b.md");
    project.remove("content/note-a.md");
    project.file(
        "content/moved/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n# A\n",
    );
    project.file(
        "content/note-c.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-c\"\ntitle = \"C\"\n+++\n",
    );
    (project, old)
}

fn check_returning(body: &str) -> String {
    format!(
        "def c(p):\n    return [{body}]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ncheck(\"c\", c)\n"
    )
}

#[test]
fn checks_target_either_side_explicitly() {
    let (project, old) = moved_history();
    let accepted: [(&str, &str); 6] = [
        (
            "error(\"deleted\", resource = \"note-b\", side = \"baseline\", code = \"protected-record-deleted\")",
            "baseline:content/note-b.md:B015[protected-record-deleted]: check `c`: deleted",
        ),
        (
            "error(\"removed\", path = \"OLD.md\", side = \"baseline\", line = 3)",
            "baseline:OLD.md:3:B015: check `c`: removed",
        ),
        (
            "warning(\"was here\", resource = \"note-a\", side = \"baseline\")",
            "baseline:content/note-a.md:B016: check `c`: was here",
        ),
        (
            "warning(\"is here\", resource = \"note-a\", side = \"candidate\", line = 7)",
            "content/moved/note-a.md:7:B016: check `c`: is here",
        ),
        (
            "warning(\"new\", resource = \"note-c\")",
            "content/note-c.md:B016: check `c`: new",
        ),
        (
            "warning(\"kept\", path = \"NOTES.md\", side = \"candidate\")",
            "NOTES.md:B016: check `c`: kept",
        ),
    ];
    for (body, expected) in accepted {
        project.file(common::ENTRY, &check_returning(body));
        let report = project.run(Command::Check, &options(Source::WorkingDirectory, &old));
        assert!(report.fatal.is_none(), "{body}: {:?}", report.fatal);
        assert_eq!(lines(&report), [expected], "{body}");
        let json = serde_json::to_value(&report).expect("json");
        let diagnostic = &json["diagnostics"][0];
        if expected.starts_with("baseline:") {
            assert_eq!(diagnostic["side"], "baseline", "{body}");
            assert_eq!(report.diagnostics[0].side, bearout::Side::Baseline);
        } else {
            assert!(diagnostic.get("side").is_none(), "{body}");
        }
    }
}

#[test]
fn invalid_side_targets_stay_b014() {
    let (project, old) = moved_history();
    let rejected: [(&str, &str); 9] = [
        (
            "error(\"m\", resource = \"note-b\")",
            "check `c` finding names unknown resource `note-b`",
        ),
        (
            "error(\"m\", resource = \"note-c\", side = \"baseline\")",
            "check `c` finding names unknown baseline resource `note-c`",
        ),
        (
            "error(\"m\", path = \"OLD.md\")",
            "check `c` finding names unknown document `OLD.md`",
        ),
        (
            "error(\"m\", path = \"content/note-b.md\", side = \"baseline\")",
            "check `c` finding names unknown baseline document `content/note-b.md`",
        ),
        (
            "error(\"m\", resource = \"note-a\", side = \"baseline\", line = 10)",
            "check `c` finding line 10 is beyond the 9 line(s) of baseline `note-a`",
        ),
        (
            "error(\"m\", resource = \"note-a\", side = \"candidate\", line = 8)",
            "check `c` finding line 8 is beyond the 7 line(s) of `note-a`",
        ),
        (
            "error(\"m\", resource = \"note-a\", side = \"history\")",
            "finding side must be \"candidate\" or \"baseline\", found \"history\"",
        ),
        (
            "error(\"m\", resource = \"note-a\", path = \"NOTES.md\", side = \"baseline\")",
            "a finding names either a `resource` or a `path`, not both",
        ),
        (
            "error(\"m\", side = \"baseline\")",
            "check `c` a check finding must name a `resource` or a `path`",
        ),
    ];
    for (body, expected) in rejected {
        project.file(common::ENTRY, &check_returning(body));
        let report = project.run(Command::Check, &options(Source::WorkingDirectory, &old));
        assert_line(&report, expected);
        assert!(
            codes(&report)
                .iter()
                .all(|code| matches!(code, Code::ScriptResult | Code::ScriptFailure)),
            "{body}: {:?}",
            codes(&report)
        );
        assert!(
            report
                .diagnostics
                .iter()
                .all(|d| d.side == bearout::Side::Candidate)
        );
    }

    // Without a comparison the baseline side does not exist.
    project.file(
        common::ENTRY,
        &check_returning("error(\"m\", resource = \"note-a\", side = \"baseline\")"),
    );
    assert_line(
        &project.check(),
        "check `c` a finding may name the baseline side only when a comparison baseline was given",
    );

    // Validators never reach the baseline.
    project.file(
        common::ENTRY,
        "def v(r):\n    return [error(\"m\", side = \"baseline\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    assert_line(
        &project.run(Command::Check, &options(Source::WorkingDirectory, &old)),
        "validate a validator may only report its own candidate resource `note-a`, not the baseline",
    );
    project.file(
        common::ENTRY,
        "def v(r):\n    return [error(\"m\", resource = \"note-b\", side = \"baseline\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    assert_line(
        &project.run(Command::Check, &options(Source::WorkingDirectory, &old)),
        "not the baseline",
    );
}
