// SPDX-License-Identifier: Apache-2.0

//! Git-backed sources: one run never mixes trees. The index source sees
//! exactly what a commit would record; the revision source sees one resolved
//! tree; neither consults the working directory for any input.

mod common;

use std::fs;

use bearout::{Code, Command, Mode, Source};
use common::{ENTRY, Project, assert_clean, assert_fatal, assert_line, assert_no_line, codes};

const TEMPLATE: &str =
    "{% for line in bearout.header %}<!-- {{ line }} -->\n{% endfor %}\n# {{ title }}\n";

const NOTE_A: &str = "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n# A\n\nSee [extra](extra.txt).\n";

/// A generating project whose every kind of input exists: bootstrap, entry
/// module, a loaded module, a shape, a resource, a linked ordinary file, a
/// template, the state manifest, and a generated output. The validator
/// names each resource path in a warning so renames are observable.
fn gen_project(relative: &str) -> Project {
    let project = Project::at(relative);
    project.file("bearout.toml", common::BOOTSTRAP_GEN);
    project.file("rules/note.schema.toml", common::NOTE_SHAPE);
    project.file(
        "rules/lib.star",
        "def title(r):\n    return r[\"fields\"][\"title\"]\n",
    );
    project.file(
        ENTRY,
        "load(\"lib.star\", \"title\")\n\ndef v(r):\n    return [warning(\"seen \" + r[\"path\"])]\n\ndef g(p):\n    return [output(\"page.md.j2\", \"generated/a.md\", context = {\"title\": title(p[\"resources\"][0])})]\n\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\ngenerator(\"pages\", g)\n",
    );
    project.file("templates/page.md.j2", TEMPLATE);
    project.file("content/note-a.md", NOTE_A);
    project.file("content/extra.txt", "plain\n");
    project
}

/// A committed, generated, clean project; returns it with the commit.
fn committed_project(relative: &str) -> (Project, String) {
    let project = gen_project(relative);
    assert_clean(&project.generate(Mode::Write));
    project.git_init();
    let commit = project.commit_all("clean");
    assert_clean(&project.verify_from(Source::WorkingDirectory));
    assert_clean(&project.verify_from(Source::Index));
    assert_clean(&project.verify_from(Source::Revision("HEAD".to_owned())));
    (project, commit)
}

fn revision(name: &str) -> Source {
    Source::Revision(name.to_owned())
}

/// How one input can be broken, and what the break produces.
struct Divergence {
    path: &'static str,
    /// `None` removes the file.
    broken: Option<&'static str>,
    expect: Expect,
}

enum Expect {
    Fatal(&'static str),
    Code(Code),
}

fn assert_expected(report: &bearout::Report, expect: &Expect, label: &str) {
    match expect {
        Expect::Fatal(text) => assert!(
            report.fatal.as_deref().is_some_and(|m| m.contains(text)),
            "{label}: expected fatal containing {text:?}, got {:?}\n{}",
            report.fatal,
            common::lines(report).join("\n")
        ),
        Expect::Code(code) => assert!(
            codes(report).contains(code),
            "{label}: expected {code}, got {:?}\nfatal: {:?}",
            codes(report),
            report.fatal
        ),
    }
}

fn assert_unaffected(report: &bearout::Report, expect: &Expect, label: &str) {
    assert_clean(report);
    if let Expect::Code(code) = expect {
        assert!(
            !codes(report).contains(code),
            "{label}: unexpected {code}: {:?}",
            common::lines(report)
        );
    }
}

const DIVERGENCES: [Divergence; 9] = [
    Divergence {
        path: "bearout.toml",
        broken: Some("version = 1\nentry = \"bearout.star\"\nbogus = 1\n"),
        expect: Expect::Fatal("unknown key `bogus`"),
    },
    Divergence {
        path: ENTRY,
        broken: Some("this is not starlark\n"),
        expect: Expect::Code(Code::ScriptLoad),
    },
    Divergence {
        path: "rules/lib.star",
        broken: Some("def title(r:\n"),
        expect: Expect::Code(Code::ScriptLoad),
    },
    Divergence {
        path: "rules/note.schema.toml",
        broken: Some("not = [toml\n"),
        expect: Expect::Code(Code::ShapeInvalid),
    },
    Divergence {
        path: "content/note-a.md",
        broken: Some(
            "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = 3\n+++\n\n# A\n",
        ),
        expect: Expect::Code(Code::ShapeViolation),
    },
    Divergence {
        path: "templates/page.md.j2",
        broken: Some("# {{ title }} without a header\n"),
        expect: Expect::Code(Code::PlanInvalid),
    },
    Divergence {
        path: "bearout-state.toml",
        broken: Some("version = 2\n"),
        expect: Expect::Code(Code::OutputState),
    },
    Divergence {
        path: "generated/a.md",
        broken: Some("hand edited\n"),
        expect: Expect::Code(Code::OutputState),
    },
    Divergence {
        path: "content/extra.txt",
        broken: None,
        expect: Expect::Code(Code::UnresolvedLink),
    },
];

/// Every input is read from the selected tree and from nothing else:
/// an unstaged edit never reaches the index or a revision, a staged edit
/// reaches the index but not a revision, and a committed edit reaches the
/// revision even after the working directory and index are restored.
fn divergences_are_isolated(relative: &str) {
    let (project, clean) = committed_project(relative);
    for case in &DIVERGENCES {
        let label = format!("{relative:?} {}", case.path);

        // Unstaged: only the working directory sees it.
        match case.broken {
            Some(text) => {
                project.file(case.path, text);
            }
            None => project.remove(case.path),
        }
        assert_expected(
            &project.verify_from(Source::WorkingDirectory),
            &case.expect,
            &format!("{label} unstaged, working"),
        );
        assert_unaffected(
            &project.verify_from(Source::Index),
            &case.expect,
            &format!("{label} unstaged, index"),
        );
        assert_unaffected(
            &project.verify_from(revision("HEAD")),
            &case.expect,
            &format!("{label} unstaged, revision"),
        );

        // Staged: the index sees it, the revision does not.
        project.git(&["add", "-A", "."]);
        assert_expected(
            &project.verify_from(Source::Index),
            &case.expect,
            &format!("{label} staged, index"),
        );
        assert_unaffected(
            &project.verify_from(revision("HEAD")),
            &case.expect,
            &format!("{label} staged, revision"),
        );

        // Committed, then restored: only the revision sees it.
        let broken = project.commit_all("broken");
        project.git(&["reset", "-q", "--hard", &clean]);
        assert_unaffected(
            &project.verify_from(Source::WorkingDirectory),
            &case.expect,
            &format!("{label} restored, working"),
        );
        assert_unaffected(
            &project.verify_from(Source::Index),
            &case.expect,
            &format!("{label} restored, index"),
        );
        assert_expected(
            &project.verify_from(revision(&broken)),
            &case.expect,
            &format!("{label} committed, revision"),
        );
        assert_unaffected(
            &project.verify_from(revision(&clean)),
            &case.expect,
            &format!("{label} clean, revision"),
        );
    }
}

#[test]
fn every_input_comes_from_the_selected_tree() {
    divergences_are_isolated("");
}

#[test]
fn every_input_comes_from_the_selected_tree_below_the_repository_root() {
    divergences_are_isolated("packages/docs");
}

#[test]
fn the_working_directory_is_the_default_and_carries_no_source_identity() {
    let (project, _) = committed_project("");
    let report = project.verify_from(Source::WorkingDirectory);
    assert_clean(&report);
    assert!(report.source.is_none());
    let json = serde_json::to_value(&report).expect("json");
    assert!(json.get("source").is_none(), "{json}");
    assert_eq!(project.generate(Mode::Check).outputs, ["generated/a.md"]);
}

#[test]
fn source_identity_is_recorded_for_git_sources() {
    let (project, commit) = committed_project("");
    let report = project.check_from(Source::Index);
    let source = report.source.as_ref().expect("index source");
    assert_eq!(source.kind, "index");
    assert!(source.revision.is_none() && source.tree.is_none());
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["source"]["kind"], "index");
    assert_eq!(json["source"]["digest"], source.digest);
    assert!(json["source"].get("tree").is_none());

    let report = project.check_from(revision("main"));
    let source = report.source.as_ref().expect("revision source");
    assert_eq!(source.kind, "revision");
    assert_eq!(source.revision.as_deref(), Some("main"));
    let tree = project.git(&["rev-parse", &format!("{commit}^{{tree}}")]);
    assert_eq!(source.tree.as_deref(), Some(tree.as_str()));
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["source"]["tree"], tree);
}

#[test]
fn untracked_files_are_absent_from_the_index() {
    let (project, _) = committed_project("");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = 3\n+++\n",
    );
    let working = project.check_from(Source::WorkingDirectory);
    assert_line(&working, "content/note-b.md:4:B005");
    assert_eq!(working.resources, 2);
    let index = project.check_from(Source::Index);
    assert_clean(&index);
    assert_eq!(index.resources, 1);
    assert_no_line(&index, "note-b");
    let committed = project.check_from(revision("HEAD"));
    assert_clean(&committed);
    assert_eq!(committed.resources, 1);
}

#[test]
fn staged_additions_are_present() {
    let (project, _) = committed_project("");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    project.git(&["add", "content/note-b.md"]);
    let index = project.check_from(Source::Index);
    assert_clean(&index);
    assert_eq!(index.resources, 2);
    assert_line(
        &index,
        "content/note-b.md:B016: schema `example/test/note@1` validate: seen content/note-b.md",
    );
    assert_eq!(project.check_from(revision("HEAD")).resources, 1);
}

#[test]
fn staged_deletions_are_absent() {
    let (project, _) = committed_project("");
    project.git(&["rm", "-q", "--cached", "content/extra.txt"]);
    assert!(
        project.exists("content/extra.txt"),
        "the file stays on disk"
    );
    assert_clean(&project.check_from(Source::WorkingDirectory));
    let index = project.check_from(Source::Index);
    assert_line(
        &index,
        "content/note-a.md:9:B011: link `extra.txt` points at a missing file",
    );
    assert_clean(&project.check_from(revision("HEAD")));

    project.git(&["rm", "-q", "content/note-a.md"]);
    let index = project.check_from(Source::Index);
    assert_eq!(index.resources, 0);
    assert_eq!(project.check_from(revision("HEAD")).resources, 1);
}

#[test]
fn staged_renames_expose_only_the_destination() {
    let (project, _) = committed_project("");
    project.git(&["mv", "content/note-a.md", "content/renamed.md"]);
    let index = project.check_from(Source::Index);
    assert_eq!(index.resources, 1);
    assert_line(&index, "content/renamed.md:B016");
    assert_no_line(&index, "content/note-a.md");
    let committed = project.check_from(revision("HEAD"));
    assert_line(&committed, "content/note-a.md:B016");
    assert_no_line(&committed, "renamed");
}

#[test]
fn a_conflicted_index_fails_closed() {
    let (project, clean) = committed_project("");
    project.git(&["checkout", "-q", "-b", "other"]);
    project.file("content/extra.txt", "theirs\n");
    project.commit_all("theirs");
    project.git(&["checkout", "-q", "main"]);
    project.file("content/extra.txt", "ours\n");
    project.commit_all("ours");
    project.git_fails(&["merge", "-q", "other"]);
    assert!(
        project
            .git(&["ls-files", "--unmerged"])
            .contains("extra.txt")
    );

    let report = project.check_from(Source::Index);
    assert_fatal(&report, "unmerged entries: content/extra.txt");
    assert_fatal(
        &project.verify_from(Source::Index),
        "resolve the conflict before checking the index",
    );
    // Revisions remain readable while the index is unmerged.
    assert_clean(&project.check_from(revision(&clean)));
    project.git(&["merge", "--abort"]);
    assert_clean(&project.check_from(Source::Index));
}

#[test]
fn intent_to_add_entries_are_excluded_like_a_commit_excludes_them() {
    let (project, _) = committed_project("");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    project.git(&["add", "-N", "content/note-b.md"]);
    let index = project.check_from(Source::Index);
    assert_clean(&index);
    assert_eq!(
        index.resources, 1,
        "an intent-to-add entry is not staged content"
    );
    assert_no_line(&index, "note-b");

    // The working-tree file disappearing changes nothing about the index.
    project.remove("content/note-b.md");
    let index = project.check_from(Source::Index);
    assert_clean(&index);
    assert_eq!(index.resources, 1);

    // Once actually staged, the entry is content.
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    project.git(&["add", "content/note-b.md"]);
    assert_eq!(project.check_from(Source::Index).resources, 2);

    // An intent-to-add entry replacing a tracked file is a deletion.
    project.git(&["rm", "-q", "--cached", "content/extra.txt"]);
    project.git(&["add", "-N", "content/extra.txt"]);
    let index = project.check_from(Source::Index);
    assert_line(&index, "B011: link `extra.txt` points at a missing file");
}

#[test]
fn revisions_are_independent_of_the_working_directory_and_index() {
    let (project, commit) = committed_project("");
    project.file("content/note-a.md", "garbage\n");
    project.file("bearout.toml", "broken = true\n");
    project.remove("rules/lib.star");
    project.remove("generated/a.md");
    project.remove("bearout-state.toml");
    fs::remove_dir_all(project.path().join("templates")).expect("remove templates");
    project.git(&["add", "-A", "."]);
    assert!(project.check_from(Source::WorkingDirectory).fatal.is_some());
    assert!(project.check_from(Source::Index).fatal.is_some());
    for name in ["HEAD", "main", &commit, &commit[..12]] {
        let report = project.verify_from(revision(name));
        assert_clean(&report);
        assert_eq!(report.resources, 1);
        assert_eq!(report.outputs, ["generated/a.md"]);
    }
}

#[test]
fn a_named_revision_is_resolved_once_per_run_and_pinned_in_the_report() {
    let (project, first) = committed_project("");
    let before = project.check_from(revision("main"));
    project.file(
        "content/note-a.md",
        NOTE_A.replace("title = \"A\"", "title = 3").as_str(),
    );
    let second = project.commit_all("break on main");
    let after = project.check_from(revision("main"));
    assert_clean(&before);
    assert!(codes(&after).contains(&Code::ShapeViolation));
    let tree = |commit: &str| project.git(&["rev-parse", &format!("{commit}^{{tree}}")]);
    assert_eq!(before.source.unwrap().tree.unwrap(), tree(&first));
    assert_eq!(after.source.unwrap().tree.unwrap(), tree(&second));
    assert_clean(&project.check_from(revision(&first)));
}

#[test]
fn annotated_tags_and_tree_objects_resolve() {
    let (project, commit) = committed_project("");
    project.git(&["tag", "-a", "-m", "release", "v1"]);
    let report = project.check_from(revision("v1"));
    assert_clean(&report);
    let tree = project.git(&["rev-parse", &format!("{commit}^{{tree}}")]);
    assert_eq!(report.source.unwrap().tree.unwrap(), tree);
    assert_clean(&project.check_from(revision(&tree)));
    assert_clean(&project.check_from(revision("v1^{tree}")));
}

#[test]
fn invalid_revisions_fail_cleanly() {
    let (project, _) = committed_project("");
    assert_fatal(
        &project.check_from(revision("no-such-branch")),
        "`no-such-branch` is not a revision of this repository",
    );
    assert_fatal(&project.check_from(revision("")), "is not a revision name");
    assert_fatal(
        &project.check_from(revision("--output=/tmp/x")),
        "is not a revision name",
    );
    assert_fatal(
        &project.check_from(revision("main\n")),
        "is not a revision name",
    );
    assert_fatal(
        &project.check_from(revision("HEAD:bearout.toml")),
        "names a blob",
    );
    assert_fatal(
        &project.check_from(revision(&"0".repeat(40))),
        "is not a revision",
    );
    let report = project.check_from(revision("nope"));
    assert!(report.source.is_none());
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["ok"], false);
    assert!(json["fatal"].as_str().unwrap().contains("nope"));
}

#[test]
fn the_index_and_revisions_can_read_files_deleted_from_the_working_directory() {
    let (project, _) = committed_project("");
    project.remove("content/note-a.md");
    project.remove("rules/lib.star");
    project.remove("templates/page.md.j2");
    project.remove("bearout-state.toml");
    let report = project.verify_from(Source::Index);
    assert_clean(&report);
    assert_eq!(report.resources, 1);
    assert_eq!(report.outputs, ["generated/a.md"]);
    let report = project.verify_from(revision("HEAD"));
    assert_clean(&report);
    assert_eq!(report.outputs, ["generated/a.md"]);
    assert!(project.verify_from(Source::WorkingDirectory).errors() > 0);
}

#[test]
fn a_generated_file_only_in_the_working_directory_is_missing_from_the_index() {
    let project = gen_project("");
    project.git_init();
    project.commit_all("sources only");
    assert_clean(&project.generate(Mode::Write));
    assert!(project.exists("generated/a.md"));
    let report = project.verify_from(Source::Index);
    assert_line(&report, "generated/a.md:B020: generated file is missing");
    assert_line(
        &report,
        "bearout-state.toml:B020: state manifest is missing",
    );
    assert!(report.outputs.is_empty());
    let report = project.verify_from(revision("HEAD"));
    assert_line(&report, "generated/a.md:B020: generated file is missing");
    project.git(&["add", "-A", "."]);
    assert_clean(&project.verify_from(Source::Index));
}

#[test]
fn write_generation_rejects_git_sources_before_reading_anything() {
    let project = Project::new();
    // Not even a repository: the rejection precedes source construction.
    let report = project.run_from(Source::Index, Command::Generate(Mode::Write));
    assert_fatal(&report, "read-only");
    let report = project.run_from(revision("HEAD"), Command::Generate(Mode::Write));
    assert_fatal(&report, "read-only");
    let (project, _) = committed_project("");
    let before = project.read("bearout-state.toml");
    assert_fatal(
        &project.run_from(Source::Index, Command::Generate(Mode::Write)),
        "generation writes to the working directory",
    );
    assert_eq!(project.read("bearout-state.toml"), before);
    assert_eq!(project.git(&["status", "--porcelain"]), "");
}

#[test]
fn outside_a_repository_is_a_fatal_outcome() {
    let project = gen_project("");
    let report = project.check_from(Source::Index);
    assert_fatal(&report, "cannot read the Git index");
    assert!(
        !report.fatal.as_deref().unwrap().contains("bearout.toml"),
        "no bootstrap is read before the source opens: {:?}",
        report.fatal
    );
    assert_fatal(
        &project.check_from(revision("HEAD")),
        "cannot read Git revision",
    );
    let missing = std::path::Path::new("/definitely/not/a/project/anywhere");
    let report = bearout::run(
        missing,
        Command::Check,
        &bearout::Options {
            source: Source::Index,
            ..Default::default()
        },
    );
    assert_fatal(&report, "cannot open project");
}

#[test]
fn projects_below_the_repository_root_see_only_their_prefix() {
    let (project, commit) = committed_project("packages/docs");
    // Sibling content in the repository is invisible to the project.
    fs::create_dir_all(project.repo_path().join("packages/other/content")).expect("dir");
    fs::write(
        project.repo_path().join("packages/other/content/x.md"),
        "+++\nschema = \"example/test/note@1\"\nid = \"x\"\ntitle = 3\n+++\n",
    )
    .expect("write");
    common::git_run(project.repo_path(), &["add", "-A", "."]);
    common::git_run(project.repo_path(), &["commit", "-q", "-m", "sibling"]);
    for source in [Source::Index, revision("HEAD"), revision(&commit)] {
        let report = project.check_from(source);
        assert_clean(&report);
        assert_eq!(report.resources, 1);
    }
    // A revision that predates the project directory is a clean failure.
    project.git(&["checkout", "-q", "--orphan", "empty"]);
    project.git(&["rm", "-rfq", "."]);
    fs::write(project.repo_path().join("README"), "x\n").expect("write");
    common::git_run(project.repo_path(), &["add", "README"]);
    common::git_run(project.repo_path(), &["commit", "-q", "-m", "no project"]);
    assert_fatal(
        &project.check_from(revision("empty")),
        "does not contain the project directory `packages/docs`",
    );
    assert_clean(&project.check_from(revision("main")));
}

#[test]
fn linked_worktrees_have_their_own_index_and_head() {
    let (project, commit) = committed_project("");
    let linked = tempfile::tempdir().expect("worktree dir");
    let linked_path = linked.path().join("wt");
    project.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "feature",
        linked_path.to_str().expect("utf-8 path"),
    ]);
    assert!(
        linked_path.join(".git").is_file(),
        "a linked worktree keeps `.git` as a file"
    );
    let options = |source| bearout::Options {
        source,
        ..Default::default()
    };
    let run = |source| {
        bearout::run(
            &linked_path,
            Command::Generate(Mode::Check),
            &options(source),
        )
    };
    assert_clean(&run(Source::Index));
    assert_clean(&run(revision("HEAD")));
    assert_eq!(
        run(revision("feature")).source.unwrap().tree.unwrap(),
        project.git(&["rev-parse", &format!("{commit}^{{tree}}")])
    );

    // Staging in the linked worktree changes its index only.
    fs::write(
        linked_path.join("content/note-a.md"),
        NOTE_A.replace("title = \"A\"", "title = 3"),
    )
    .expect("write");
    common::git_run(&linked_path, &["add", "content/note-a.md"]);
    assert!(codes(&run(Source::Index)).contains(&Code::ShapeViolation));
    assert_clean(&project.check_from(Source::Index));
    assert_clean(&run(revision("HEAD")));
}

#[test]
fn non_portable_index_paths_fail_deterministically() {
    let (project, _) = committed_project("");
    project.stage_entry("100644", b"x", "content/a:b.md");
    let first = project.check_from(Source::Index);
    assert_fatal(
        &first,
        "cannot walk resource root `content`: `content` contains an entry that is not a portable path segment: `a:b.md`: `:` is not allowed",
    );
    assert_eq!(first.fatal, project.check_from(Source::Index).fatal);
    // Outside the resource roots, discovery never reaches the name.
    project.git(&["rm", "-q", "--cached", "content/a:b.md"]);
    project.stage_entry("100644", b"x", "notes/a:b.md");
    assert_clean(&project.check_from(Source::Index));

    let (project, _) = committed_project("");
    project.stage_entry("100644", b"x", "content/back\\slash.md");
    assert_fatal(&project.check_from(Source::Index), "backslash");
    project.git(&["commit", "-q", "-m", "bad name"]);
    assert_fatal(&project.check_from(revision("HEAD")), "backslash");
}

#[cfg(unix)]
#[test]
fn non_utf8_index_paths_fail_deterministically() {
    use std::os::unix::ffi::OsStrExt;
    let (project, _) = committed_project("");
    let blob = project.blob(b"x");
    let mut spec = std::ffi::OsString::from(format!("100644,{blob},content/bad-"));
    spec.push(std::ffi::OsStr::from_bytes(b"\xff.md"));
    let status = common::git_command(project.path())
        .args(["update-index", "--add", "--cacheinfo"])
        .arg(&spec)
        .status()
        .expect("git");
    assert!(status.success());
    assert_fatal(&project.check_from(Source::Index), "not valid UTF-8");
}

#[test]
fn file_modes_are_retained_and_executables_are_ordinary_files() {
    let (project, _) = committed_project("");
    let note = "+++\nschema = \"example/test/note@1\"\nid = \"note-x\"\ntitle = \"X\"\n+++\n";
    project.stage_entry("100755", note.as_bytes(), "content/note-x.md");
    let report = project.check_from(Source::Index);
    assert_clean(&report);
    assert_eq!(report.resources, 2);
    assert_line(&report, "content/note-x.md:B016");
    assert!(
        project
            .git(&["ls-files", "--stage", "content/note-x.md"])
            .starts_with("100755"),
        "the mode is what the index records"
    );
    project.git(&["commit", "-q", "-m", "executable resource"]);
    assert_eq!(project.check_from(revision("HEAD")).resources, 2);
    assert!(
        project
            .git(&["ls-tree", "HEAD", "content/note-x.md"])
            .starts_with("100755"),
        "the mode survives into the revision"
    );
}

#[test]
fn resource_discovery_skips_symbolic_links() {
    let (project, _) = committed_project("");
    project.stage_entry("120000", b"note-a.md", "content/alias.md");
    let report = project.check_from(Source::Index);
    assert_clean(&report);
    assert_eq!(report.resources, 1);
    assert_no_line(&report, "alias");
    project.git(&["commit", "-q", "-m", "link"]);
    let report = project.check_from(revision("HEAD"));
    assert_clean(&report);
    assert_eq!(report.resources, 1);
}

#[test]
fn rules_and_shapes_are_never_reached_through_links() {
    let (project, _) = committed_project("");
    project.git(&["rm", "-q", "--cached", "rules/lib.star"]);
    project.stage_entry("120000", b"real.star", "rules/lib.star");
    project.stage_entry(
        "100644",
        b"def title(r):\n    return r[\"fields\"][\"title\"]\n",
        "rules/real.star",
    );
    let report = project.check_from(Source::Index);
    assert_line(
        &report,
        "rules/lib.star:B012: `rules/lib.star` is a symbolic link; modules must not be reached through links",
    );
    project.git(&["commit", "-q", "-m", "linked module"]);
    assert_line(
        &project.check_from(revision("HEAD")),
        "rules/lib.star:B012: `rules/lib.star` is a symbolic link",
    );

    let (project, _) = committed_project("");
    project.git(&["rm", "-q", "--cached", "rules/note.schema.toml"]);
    project.stage_entry(
        "120000",
        b"shapes/real.schema.toml",
        "rules/note.schema.toml",
    );
    project.stage_entry(
        "100644",
        common::NOTE_SHAPE.as_bytes(),
        "rules/shapes/real.schema.toml",
    );
    let report = project.check_from(Source::Index);
    assert_line(
        &report,
        "rules/note.schema.toml:B001: cannot read shape for `example/test/note@1`: `rules/note.schema.toml` is a symbolic link; shapes must not be reached through links",
    );
    project.git(&["commit", "-q", "-m", "linked shape"]);
    assert_line(
        &project.check_from(revision("HEAD")),
        "rules/note.schema.toml:B001: cannot read shape",
    );
}

#[cfg(unix)]
#[test]
fn shapes_are_never_reached_through_links_in_the_working_directory() {
    let project = Project::with_note();
    fs::rename(
        project.path().join("rules/note.schema.toml"),
        project.path().join("rules/real.schema.toml"),
    )
    .expect("rename");
    std::os::unix::fs::symlink(
        "real.schema.toml",
        project.path().join("rules/note.schema.toml"),
    )
    .expect("symlink");
    assert_line(
        &project.check(),
        "rules/note.schema.toml:B001: cannot read shape for `example/test/note@1`: `rules/note.schema.toml` is a symbolic link",
    );
}

#[test]
fn template_links_stay_inside_the_templates_root() {
    // Permitted: a link to another template.
    let (project, _) = committed_project("");
    project.git(&["rm", "-q", "--cached", "templates/page.md.j2"]);
    project.stage_entry("120000", b"parts/real.md.j2", "templates/page.md.j2");
    project.stage_entry("100644", TEMPLATE.as_bytes(), "templates/parts/real.md.j2");
    let report = project.verify_from(Source::Index);
    assert_clean(&report);
    assert_eq!(report.outputs, ["generated/a.md"]);
    project.git(&["commit", "-q", "-m", "linked template"]);
    assert_clean(&project.verify_from(revision("HEAD")));

    // Refused: escapes, absolute targets, missing targets, cycles, and
    // links through a submodule, none of which consult the filesystem.
    let cases: [(&[u8], &str); 6] = [
        (b"../rules/note.schema.toml", "escape into the project"),
        (b"../../../../etc/passwd", "escape the project"),
        (b"/etc/passwd", "absolute"),
        (b"missing.md.j2", "missing"),
        (b"loop-b.md.j2", "cycle"),
        (b"vendor/page.md.j2", "submodule"),
    ];
    for (target, label) in cases {
        let (project, _) = committed_project("");
        project.git(&["rm", "-q", "--cached", "templates/page.md.j2"]);
        project.stage_entry("120000", target, "templates/page.md.j2");
        project.stage_entry("120000", b"page.md.j2", "templates/loop-b.md.j2");
        project.stage_entry("160000", b"", "templates/vendor");
        // The escape targets exist in the working directory and the project.
        project.file("templates/page.md.j2", TEMPLATE);
        let report = project.verify_from(Source::Index);
        assert!(report.fatal.is_none(), "{label}: {:?}", report.fatal);
        assert_line(
            &report,
            "bearout.star:B019: generator `pages`: template `page.md.j2` does not exist beneath the templates root",
        );
        assert!(
            codes(&report).iter().all(|code| *code != Code::Delivery),
            "{label}"
        );
        assert!(report.outputs.is_empty(), "{label}");
        project.git(&["commit", "-q", "-m", label]);
        let report = project.verify_from(revision("HEAD"));
        assert_line(&report, "template `page.md.j2` does not exist");
        assert!(report.outputs.is_empty(), "{label} in revision");
    }
}

#[test]
fn links_in_resources_resolve_only_inside_the_tree() {
    let (project, _) = committed_project("");
    // A symbolic link in the resource root is followed for link resolution
    // when it stays inside the tree, and never when it leaves.
    project.stage_entry("120000", b"extra.txt", "content/inside.txt");
    project.stage_entry("120000", b"../../outside.txt", "content/outside.txt");
    project.stage_entry("120000", b"nowhere.txt", "content/dangling.txt");
    fs::write(project.repo_path().join("../outside.txt"), "x").ok();
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n# A\n\n[i](inside.txt) [o](outside.txt) [d](dangling.txt)\n",
    );
    project.git(&["add", "content/note-a.md"]);
    let report = project.check_from(Source::Index);
    assert_no_line(&report, "`inside.txt`");
    assert_line(&report, "B011: link `outside.txt` points at a missing file");
    assert_line(
        &report,
        "B011: link `dangling.txt` points at a missing file",
    );
    assert_eq!(report.errors(), 2);
}

#[test]
fn gitlinks_are_never_traversed() {
    let (project, _) = committed_project("");
    project.stage_entry("160000", b"", "content/vendor");
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n# A\n\n[v](vendor/README.txt)\n",
    );
    project.git(&["add", "content/note-a.md"]);
    // The submodule's checkout, had it one, is not consulted.
    project.file("content/vendor/README.txt", "vendored\n");
    let report = project.check_from(Source::Index);
    assert_eq!(report.resources, 1);
    assert_line(
        &report,
        "B011: link `vendor/README.txt` points at a missing file",
    );
    assert_clean(&project.check_from(Source::WorkingDirectory));

    // A root that is a submodule is not a directory of the project.
    project.file(
        "bearout.toml",
        &common::BOOTSTRAP_GEN
            .replace("roots = [\"content\"]", "roots = [\"content\", \"vendor\"]"),
    );
    project.stage_entry("160000", b"", "vendor");
    project.git(&["add", "bearout.toml"]);
    assert_fatal(
        &project.check_from(Source::Index),
        "resource root `vendor` is not a directory",
    );
}

#[test]
fn repeated_runs_produce_byte_identical_reports() {
    let (project, _) = committed_project("");
    project.file(
        "content/note-a.md",
        NOTE_A.replace("title = \"A\"", "title = 3").as_str(),
    );
    project.git(&["add", "content/note-a.md"]);
    for source in [Source::Index, revision("HEAD"), Source::WorkingDirectory] {
        let first = serde_json::to_string(&project.verify_from(source.clone())).expect("json");
        let second = serde_json::to_string(&project.verify_from(source)).expect("json");
        assert_eq!(first, second);
    }
}

#[test]
fn replacement_objects_are_never_followed() {
    let (project, commit) = committed_project("");
    let original = project.git(&["rev-parse", "HEAD:content/note-a.md"]);
    let replacement = project.blob(NOTE_A.replace("title = \"A\"", "title = 3").as_bytes());
    project.git(&["replace", &original, &replacement]);
    assert!(
        project
            .git(&["cat-file", "-p", &original])
            .contains("title = 3"),
        "plain Git now sees the replacement"
    );
    let before = project.check_from(Source::Index);
    assert_clean(&before);
    assert_clean(&project.check_from(revision(&commit)));
    project.git(&["replace", "-d", &original]);
    let after = project.check_from(Source::Index);
    assert_eq!(
        before.source, after.source,
        "replacement refs do not change the digest"
    );
}

#[test]
fn the_index_digest_identifies_the_captured_content() {
    let (project, commit) = committed_project("");
    let index = project.check_from(Source::Index).source.unwrap();
    let committed = project.check_from(revision(&commit)).source.unwrap();
    assert!(index.digest.starts_with("blake3:") && index.digest.len() == 71);
    assert_eq!(
        index.digest, committed.digest,
        "the same content digests equally from the index and from a revision"
    );
    let json = serde_json::to_value(project.check_from(Source::Index)).expect("json");
    assert_eq!(
        json["source"],
        serde_json::json!({ "kind": "index", "digest": index.digest })
    );

    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    assert_eq!(
        project.check_from(Source::Index).source.unwrap().digest,
        index.digest,
        "an untracked file leaves the index digest alone"
    );
    project.git(&["add", "content/note-b.md"]);
    let staged = project.check_from(Source::Index).source.unwrap().digest;
    assert_ne!(staged, index.digest);
    let next = project.commit_all("note-b");
    assert_eq!(
        project.check_from(revision(&next)).source.unwrap().digest,
        staged
    );
    project.git(&["update-index", "--chmod=+x", "content/note-b.md"]);
    assert_ne!(
        project.check_from(Source::Index).source.unwrap().digest,
        staged,
        "a mode change is content"
    );
}

#[test]
fn a_repository_without_an_index_file_is_an_empty_index() {
    let project = gen_project("");
    project.git_init();
    assert!(!project.repo_path().join(".git/index").exists());
    assert_fatal(
        &project.check_from(Source::Index),
        "cannot read bearout.toml",
    );
}

#[test]
fn index_captures_match_what_git_would_commit() {
    // Differential: the index tree Bearout checks equals the tree
    // `write-tree` records, computed on a copy of the index so that the
    // real index is never written.
    let (project, _) = committed_project("");
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    project.git(&["add", "content/note-b.md"]);
    project.file(
        "content/note-c.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-c\"\ntitle = \"C\"\n+++\n",
    );
    project.git(&["add", "-N", "content/note-c.md"]);
    project.file("content/note-a.md", "unstaged garbage\n");
    let git_dir = project.git(&["rev-parse", "--absolute-git-dir"]);
    let copy = std::path::Path::new(&git_dir).join("bearout-test-index");
    fs::copy(std::path::Path::new(&git_dir).join("index"), &copy).expect("copy index");
    let tree = {
        let output = common::git_command(project.path())
            .env("GIT_INDEX_FILE", &copy)
            .args(["write-tree"])
            .output()
            .expect("write-tree");
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let from_index = project.check_from(Source::Index);
    let from_tree = project.check_from(revision(&tree));
    assert_eq!(from_index.resources, from_tree.resources);
    assert_eq!(common::lines(&from_index), common::lines(&from_tree));
    assert_eq!(
        from_index.resources, 2,
        "note-b staged, note-c intent-to-add only"
    );
    assert_clean(&from_index);
}
