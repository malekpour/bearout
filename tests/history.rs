// SPDX-License-Identifier: Apache-2.0

//! Repository history and commit policy: range and pending captures,
//! exact facts, policy authority, finding targets, ordering, and the
//! fatal outcomes that keep incomplete or malformed history from passing.

mod common;

use std::io::Write;
use std::process::Stdio;

use bearout::{Code, HistoryMode, HistoryReport, HistoryTarget, Options, Severity};
use common::Project;
use serde_json::Value;

/// A history check that reports every fact of the view as warnings: one
/// range-wide with the head, base, and count, and one per commit with the
/// commit's whole JSON.
const FACTS: &str = r#"def facts(history):
    findings = [warning(json.encode({"kind": history["kind"], "base": history["base"], "head": history["head"], "count": len(history["commits"])}))]
    for commit in history["commits"]:
        findings.append(warning(json.encode(commit), commit = commit["key"]))
    return findings

history_check("facts", facts)
"#;

/// A project whose policy is `entry`, committed once, with the identity
/// the hardened runner will see configured in the repository.
fn project_with(entry: &str) -> Project {
    let project = Project::new();
    project.file(common::ENTRY, entry);
    project.file(
        "rules/helpers.star",
        "def subject(commit):\n    return commit[\"subject\"]\n",
    );
    project.file("content/.keep", "");
    project.git_init();
    project.git(&["config", "user.name", "Test"]);
    project.git(&["config", "user.email", "test@example.invalid"]);
    project
}

fn facts_project() -> Project {
    let project = project_with(FACTS);
    project.commit_all("chore: initial");
    project
}

fn range(project: &Project, base: Option<&str>, head: Option<&str>) -> HistoryReport {
    bearout::history(
        project.path(),
        &HistoryMode::Range {
            base: base.map(str::to_owned),
            head: head.map(str::to_owned),
        },
        &Options::default(),
    )
}

fn message(project: &Project, file: &std::path::Path) -> HistoryReport {
    bearout::history(
        project.path(),
        &HistoryMode::Message {
            file: file.to_path_buf(),
        },
        &Options::default(),
    )
}

/// The commit facts the `facts` check reported, in report order.
fn commits(report: &HistoryReport) -> Vec<Value> {
    report
        .diagnostics
        .iter()
        .filter(|d| matches!(d.target, HistoryTarget::Commit { .. }))
        .map(|d| {
            let json = d
                .message
                .strip_prefix("history check `facts`: ")
                .unwrap_or_else(|| panic!("not a facts message: {}", d.message));
            serde_json::from_str(json).expect("commit json")
        })
        .collect()
}

/// The range-wide fact the `facts` check reported.
fn range_fact(report: &HistoryReport) -> Value {
    let diagnostic = report
        .diagnostics
        .iter()
        .find(|d| matches!(d.target, HistoryTarget::Range {}))
        .unwrap_or_else(|| panic!("no range fact: {:?}", report.fatal));
    serde_json::from_str(
        diagnostic
            .message
            .strip_prefix("history check `facts`: ")
            .unwrap(),
    )
    .unwrap()
}

#[track_caller]
fn assert_captured(report: &HistoryReport) {
    assert!(
        report.fatal.is_none(),
        "expected the facts to be captured, got fatal {:?}",
        report.fatal
    );
    assert!(
        report
            .diagnostics
            .iter()
            .all(|d| d.code == Code::HistoryWarning),
        "unexpected diagnostics:\n{}",
        lines(report).join("\n")
    );
}

#[track_caller]
fn assert_history_fatal(report: &HistoryReport, expected: &str) {
    assert!(
        report
            .fatal
            .as_deref()
            .is_some_and(|message| message.contains(expected)),
        "expected a fatal outcome containing {expected:?}, got {:?}\n{}",
        report.fatal,
        lines(report).join("\n")
    );
    assert!(!report.ok);
}

fn lines(report: &HistoryReport) -> Vec<String> {
    report.diagnostics.iter().map(ToString::to_string).collect()
}

fn ids(report: &HistoryReport) -> Vec<String> {
    commits(report)
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect()
}

fn subjects(report: &HistoryReport) -> Vec<String> {
    commits(report)
        .iter()
        .map(|c| c["subject"].as_str().unwrap().to_owned())
        .collect()
}

/// Write a commit object verbatim and point `refname` at it.
fn plant_commit(project: &Project, object: &[u8], refname: &str) -> String {
    let mut child = common::git_command(project.path())
        .args([
            "hash-object",
            "-t",
            "commit",
            "-w",
            "--stdin",
            "--literally",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn git");
    child.stdin.take().unwrap().write_all(object).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let id = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    project.git(&["update-ref", refname, &id]);
    id
}

// ---- Git history capture ------------------------------------------------

#[test]
fn head_and_base_resolve_once_and_are_recorded() {
    let project = facts_project();
    let first = project.git(&["rev-parse", "HEAD"]);
    project.file("a.txt", "a\n");
    let second = project.commit_all("feat: a");
    project.git(&["tag", "v1"]);

    // An explicit base and head by name, resolved to full identities.
    let report = range(&project, Some(&first), Some("v1"));
    assert_captured(&report);
    assert_eq!(report.mode, "range");
    let head = report.head.as_ref().unwrap();
    assert_eq!(
        (head.revision.as_str(), head.id.as_str()),
        ("v1", second.as_str())
    );
    let base = report.base.as_ref().unwrap();
    assert_eq!(
        (base.revision.as_str(), base.id.as_str()),
        (first.as_str(), first.as_str())
    );
    assert_eq!(report.commits, 1);
    assert_eq!(ids(&report), [second.as_str()]);
    let fact = range_fact(&report);
    assert_eq!(fact["kind"], "range");
    assert_eq!(fact["head"]["id"], second);
    assert_eq!(fact["base"]["revision"], first);
    assert_eq!(fact["count"], 1);
    // The policy source is the head's tree.
    let source = report.source.as_ref().unwrap();
    assert_eq!(source.kind, "revision");
    assert_eq!(source.revision.as_deref(), Some("v1"));
    assert_eq!(
        source.tree.as_deref(),
        Some(project.git(&["rev-parse", "v1^{tree}"]).as_str())
    );

    // The default head is HEAD, resolved once to the commit it names now.
    let report = range(&project, Some(&first), None);
    assert_eq!(report.head.as_ref().unwrap().revision, "HEAD");
    assert_eq!(report.head.as_ref().unwrap().id, second);
    assert!(range_fact(&report)["base"].is_object());

    // Without a base everything reachable is inspected, oldest first.
    let report = range(&project, None, None);
    assert_captured(&report);
    assert!(report.base.is_none());
    assert!(range_fact(&report)["base"].is_null());
    assert_eq!(ids(&report), [first.clone(), second.clone()]);
    assert_eq!(report.commits, 2);

    // The base itself is excluded; base == head is an empty range.
    let report = range(&project, Some("HEAD"), Some("HEAD"));
    assert_captured(&report);
    assert_eq!(report.commits, 0);
    assert_eq!(range_fact(&report)["count"], 0);
}

#[test]
fn invalid_revisions_are_fatal() {
    let project = facts_project();
    let tree = project.git(&["rev-parse", "HEAD^{tree}"]);
    let blob = project.git(&["rev-parse", "HEAD:bearout.toml"]);
    for (base, head, expected) in [
        (
            None,
            Some("nope"),
            "`nope` is not a revision of this repository",
        ),
        (
            Some("nope"),
            None,
            "`nope` is not a revision of this repository",
        ),
        (None, Some(""), "is not a revision name"),
        (None, Some("--output=x"), "is not a revision name"),
        (None, Some("HEAD\n"), "is not a revision name"),
        (None, Some(tree.as_str()), "names a tree, not a commit"),
        (Some(blob.as_str()), None, "names a blob, not a commit"),
        (
            None,
            Some("0000000000000000000000000000000000000001"),
            "is not a revision of this repository",
        ),
    ] {
        let report = range(&project, base, head);
        assert_history_fatal(&report, expected);
        assert!(report.diagnostics.is_empty());
        assert_eq!(report.commits, 0);
    }
    // An ambiguous abbreviation is refused rather than guessed.
    let a = plant_commit(
        &project,
        b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nfirst\n",
        "refs/tags/planted-a",
    );
    let mut ambiguous = None;
    for salt in 0..20_000u32 {
        let object = format!(
            "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nsalt {salt}\n"
        );
        let mut child = common::git_command(project.path())
            .args(["hash-object", "-t", "commit", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(object.as_bytes())
            .unwrap();
        let id = String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap();
        if id.starts_with(&a[..4]) && id.trim() != a {
            plant_commit(&project, object.as_bytes(), "refs/tags/planted-b");
            ambiguous = Some(a[..4].to_owned());
            break;
        }
    }
    if let Some(prefix) = ambiguous {
        assert_history_fatal(&range(&project, None, Some(&prefix)), "is not a revision");
    }
    // A tag peels to its commit.
    project.git(&["tag", "-a", "-m", "annotated", "release"]);
    let report = range(&project, None, Some("release"));
    assert_eq!(report.head.unwrap().id, project.git(&["rev-parse", "HEAD"]));
    // Options that select a source or a baseline are refused.
    let report = bearout::history(
        project.path(),
        &HistoryMode::Range {
            base: None,
            head: None,
        },
        &Options {
            source: bearout::Source::Index,
            ..Options::default()
        },
    );
    assert_history_fatal(&report, "takes no source selection or comparison baseline");
}

#[test]
fn branches_and_merges_form_the_expected_set_in_deterministic_order() {
    let project = facts_project();
    let base = project.git(&["rev-parse", "HEAD"]);
    // Two branches off the base, one commit each, merged into main.
    project.git(&["checkout", "-q", "-b", "one"]);
    project.file("one.txt", "1\n");
    let one = project.commit_all("feat: one");
    project.git(&["checkout", "-q", "main"]);
    project.git(&["checkout", "-q", "-b", "two"]);
    project.file("two.txt", "2\n");
    let two = project.commit_all("feat: two");
    project.git(&["checkout", "-q", "main"]);
    project.git(&["merge", "-q", "--no-ff", "-m", "merge: one", "one"]);
    let merge_one = project.git(&["rev-parse", "HEAD"]);
    project.git(&["merge", "-q", "--no-ff", "-m", "merge: two", "two"]);
    let merge_two = project.git(&["rev-parse", "HEAD"]);

    let report = range(&project, Some(&base), Some("HEAD"));
    assert_captured(&report);
    assert_eq!(report.commits, 4);
    // Both branch commits are eligible at once: the smaller identity goes
    // first, whatever order they were made in.
    let mut first_two = [one.clone(), two.clone()];
    first_two.sort();
    let order = ids(&report);
    assert_eq!(&order[..2], &first_two[..]);
    assert_eq!(&order[2..], &[merge_one.clone(), merge_two.clone()][..]);
    let facts = commits(&report);
    let merge = &facts[2];
    assert_eq!(merge["merge"], true);
    assert_eq!(merge["subject"], "merge: one");
    assert_eq!(
        merge["parents"].as_array().unwrap().len(),
        2,
        "merges stay visible with their ordered parents"
    );
    assert_eq!(
        merge["parents"][0], base,
        "the first parent is the branch merged into"
    );
    assert_eq!(merge["parents"][1], one);
    assert_eq!(
        merge["change_basis"], base,
        "changes compare against the first parent"
    );
    let changes = merge["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["repository_path"], "one.txt");
    assert_eq!(changes[0]["change"], "added");
    let last = &facts[3];
    assert_eq!(last["change_basis"], merge_one);
    assert_eq!(last["changes"][0]["repository_path"], "two.txt");
    // A branch commit is not a merge and its diagnostic keys are full ids.
    assert_eq!(facts[0]["merge"], false);
    assert_eq!(facts[0]["key"], facts[0]["id"]);
    // Runs are byte-identical.
    let again = range(&project, Some(&base), Some("HEAD"));
    assert_eq!(
        serde_json::to_string(&report).unwrap(),
        serde_json::to_string(&again).unwrap()
    );
    // From a branch: only its own commits.
    let report = range(&project, Some(&base), Some("two"));
    assert_eq!(ids(&report), [two]);
}

#[test]
fn root_commits_compare_against_the_empty_tree() {
    let project = facts_project();
    let report = range(&project, None, None);
    assert_captured(&report);
    let facts = commits(&report);
    let root = &facts[0];
    assert!(root["parents"].as_array().unwrap().is_empty());
    assert!(root["change_basis"].is_null());
    assert_eq!(root["merge"], false);
    let paths: Vec<&str> = root["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["repository_path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "bearout.star",
            "bearout.toml",
            "content/.keep",
            "rules/helpers.star"
        ],
        "sorted by repository path"
    );
    assert!(root["changes"].as_array().unwrap().iter().all(|c| {
        c["change"] == "added" && c["before"].is_null() && c["after"]["kind"] == "file"
    }));
    assert_eq!(root["tree"], project.git(&["rev-parse", "HEAD^{tree}"]));
}

#[test]
fn identities_messages_and_signatures_are_exact() {
    let project = facts_project();
    // Author and committer differ, with exact timestamps and offsets.
    project.file("x.txt", "x\n");
    project.git(&["add", "x.txt"]);
    let status = common::git_command(project.path())
        .env("GIT_AUTHOR_NAME", "Ada  Lovelace")
        .env("GIT_AUTHOR_EMAIL", "Ada@Example.Test")
        .env("GIT_AUTHOR_DATE", "1700000000 +0230")
        .env("GIT_COMMITTER_NAME", "Committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.test")
        .env("GIT_COMMITTER_DATE", "1700000600 -0500")
        .args(["commit", "-q", "--cleanup=verbatim", "-F", "-"])
        .stdin(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .take()
                .unwrap()
                .write_all("feat(scope)!: subjéct ✓\n\nBody paragraph.\n\n\nCo-authored-by: X <x@y>\nSigned-off-by: Ada  Lovelace <Ada@Example.Test>\n".as_bytes())
                .unwrap();
            child.wait()
        })
        .unwrap();
    assert!(status.success());
    let report = range(&project, Some("HEAD~1"), None);
    assert_captured(&report);
    let commit = &commits(&report)[0];
    assert_eq!(
        commit["author"]["name"], "Ada  Lovelace",
        "no whitespace or case folding"
    );
    assert_eq!(commit["author"]["email"], "Ada@Example.Test");
    assert_eq!(commit["author"]["timestamp"], 1_700_000_000);
    assert_eq!(commit["author"]["timezone"], "+0230");
    assert_eq!(commit["committer"]["name"], "Committer");
    assert_eq!(commit["committer"]["timestamp"], 1_700_000_600);
    assert_eq!(commit["committer"]["timezone"], "-0500");
    assert_eq!(commit["subject"], "feat(scope)!: subjéct ✓");
    assert_eq!(
        commit["message"],
        "feat(scope)!: subjéct ✓\n\nBody paragraph.\n\n\nCo-authored-by: X <x@y>\nSigned-off-by: Ada  Lovelace <Ada@Example.Test>\n",
        "blank lines, trailers, Unicode, and the final newline are kept"
    );

    // A signed commit with continuation headers: the signature never
    // reaches the message.
    let signed = format!(
        "tree {}\nparent {}\nauthor S <s@x> 1700000000 +0000\ncommitter S <s@x> 1700000000 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n iQIzBAABCgAdFiEE\n =abcd\n -----END PGP SIGNATURE-----\n\nfix: signed subject\n\nSigned body.\n",
        project.git(&["rev-parse", "HEAD^{tree}"]),
        project.git(&["rev-parse", "HEAD"])
    );
    let signed_id = plant_commit(&project, signed.as_bytes(), "refs/heads/signed");
    let report = range(&project, Some("HEAD"), Some("signed"));
    assert_captured(&report);
    let commit = &commits(&report)[0];
    assert_eq!(commit["id"], signed_id);
    assert_eq!(commit["subject"], "fix: signed subject");
    assert_eq!(commit["message"], "fix: signed subject\n\nSigned body.\n");
    assert!(commit["changes"].as_array().unwrap().is_empty());

    // A mailmap does not rewrite identities.
    project.file(
        ".mailmap",
        "Proper Name <proper@example.test> <Ada@Example.Test>\n",
    );
    project.commit_all("chore: mailmap");
    let report = range(&project, Some("HEAD~2"), Some("HEAD~1"));
    assert_eq!(commits(&report)[0]["author"]["email"], "Ada@Example.Test");
}

#[test]
fn malformed_commit_objects_fail_closed_and_name_the_commit() {
    let project = facts_project();
    let tree = project.git(&["rev-parse", "HEAD^{tree}"]);
    let parent = project.git(&["rev-parse", "HEAD"]);
    let objects: Vec<(Vec<u8>, &str)> = vec![
        (
            {
                let mut object = format!("tree {tree}\nparent {parent}\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nsubject ").into_bytes();
                object.extend_from_slice(b"\xff\xfe\n");
                object
            },
            "is not valid UTF-8",
        ),
        (
            {
                let mut object = format!("tree {tree}\nparent {parent}\nauthor Bad ").into_bytes();
                object.extend_from_slice(b"\xff <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nsubject\n");
                object
            },
            "is not valid UTF-8",
        ),
        (
            format!("tree {tree}\nparent {parent}\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\nencoding ISO-8859-1\n\nsubject\n").into_bytes(),
            "only UTF-8 commits are supported",
        ),
        (
            format!("tree {tree}\nparent {parent}\nauthor A a@x 1 +0000\ncommitter A <a@x> 1 +0000\n\nsubject\n").into_bytes(),
            "author identity",
        ),
        (
            format!("tree {tree}\nparent {parent}\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nsubject\n").into_bytes(),
            "",
        ),
    ];
    for (index, (object, expected)) in objects.iter().enumerate() {
        let refname = format!("refs/heads/planted-{index}");
        let id = plant_commit(&project, object, &refname);
        let report = range(&project, Some("HEAD"), Some(&refname));
        if expected.is_empty() {
            assert_captured(&report);
            continue;
        }
        assert_history_fatal(&report, expected);
        assert!(
            report
                .fatal
                .as_deref()
                .unwrap()
                .contains(&format!("commit {id}")),
            "{:?}",
            report.fatal
        );
    }
}

#[test]
fn every_change_kind_is_represented_exactly() {
    let project = facts_project();
    project.file("kept.txt", "same\n");
    project.file("modified.txt", "before\n");
    project.file("removed.txt", "gone\n");
    project.file("tool.sh", "#!/bin/sh\n");
    project.file("typed", "a file\n");
    project.commit_all("chore: base");
    let base = project.git(&["rev-parse", "HEAD"]);
    project.file("modified.txt", "after\n");
    project.remove("removed.txt");
    project.file("added.txt", "new\n");
    project.git(&["add", "-A", "."]);
    project.git(&["update-index", "--chmod=+x", "tool.sh"]);
    project.git(&["rm", "-q", "--cached", "typed"]);
    project.stage_entry("120000", b"kept.txt", "typed");
    project.stage_entry("120000", b"kept.txt", "link");
    project.stage_entry("160000", b"", "vendor");
    project.git(&["commit", "-q", "-m", "feat: everything"]);
    let report = range(&project, Some(&base), None);
    assert_captured(&report);
    let commit = &commits(&report)[0];
    let changes: Vec<(String, String, Option<String>, Option<String>)> = commit["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["repository_path"].as_str().unwrap().to_owned(),
                c["change"].as_str().unwrap().to_owned(),
                c["before"]["kind"].as_str().map(str::to_owned),
                c["after"]["kind"].as_str().map(str::to_owned),
            )
        })
        .collect();
    let owned = |s: &str| Some(s.to_owned());
    assert_eq!(
        changes,
        [
            (
                "added.txt".to_owned(),
                "added".to_owned(),
                None,
                owned("file")
            ),
            (
                "link".to_owned(),
                "added".to_owned(),
                None,
                owned("symlink")
            ),
            (
                "modified.txt".to_owned(),
                "modified".to_owned(),
                owned("file"),
                owned("file")
            ),
            (
                "removed.txt".to_owned(),
                "removed".to_owned(),
                owned("file"),
                None
            ),
            (
                "tool.sh".to_owned(),
                "modified".to_owned(),
                owned("file"),
                owned("executable")
            ),
            (
                "typed".to_owned(),
                "type-changed".to_owned(),
                owned("file"),
                owned("symlink")
            ),
            (
                "vendor".to_owned(),
                "added".to_owned(),
                None,
                owned("gitlink")
            ),
        ]
    );
    let tool = &commit["changes"][4];
    assert_eq!(tool["before"]["mode"], "100644");
    assert_eq!(tool["after"]["mode"], "100755");
    assert_eq!(
        tool["before"]["object"], tool["after"]["object"],
        "same blob, new mode"
    );
    assert_eq!(commit["changes"][1]["after"]["mode"], "120000");
    assert_eq!(commit["changes"][6]["after"]["mode"], "160000");
    assert_eq!(
        commit["changes"][6]["after"]["object"], base,
        "the gitlink names a commit"
    );
    assert!(
        commit["changes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["project_path"] == c["repository_path"])
    );

    // A rename is a removal plus an addition with the same blob.
    project.git(&["mv", "kept.txt", "renamed.txt"]);
    project.git(&["commit", "-q", "-m", "refactor: rename"]);
    let report = range(&project, Some("HEAD~1"), None);
    let commit = &commits(&report)[0];
    let changes = commit["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0]["repository_path"], "kept.txt");
    assert_eq!(changes[0]["change"], "removed");
    assert_eq!(changes[1]["repository_path"], "renamed.txt");
    assert_eq!(changes[1]["change"], "added");
    assert_eq!(
        changes[0]["before"]["object"],
        changes[1]["after"]["object"]
    );
    // Rename detection through configuration changes nothing.
    project.git(&["config", "diff.renames", "true"]);
    project.git(&["config", "diff.renameLimit", "1000"]);
    let again = range(&project, Some("HEAD~1"), None);
    assert_eq!(commits(&again)[0]["changes"], commit["changes"]);
}

#[test]
fn paths_are_repository_relative_with_project_paths_inside_the_project() {
    let project = Project::at("packages/docs");
    project.file(common::ENTRY, FACTS);
    project.file("rules/helpers.star", "x = 1\n");
    project.file("content/.keep", "");
    project.git_init();
    project.git(&["config", "user.name", "Test"]);
    project.git(&["config", "user.email", "test@example.invalid"]);
    project.commit_all("chore: project");
    // A change outside the project and one inside, in one commit.
    std::fs::write(project.repo_path().join("README.md"), "# top\n").unwrap();
    project.file("content/a.md", "a\n");
    common::git_run(project.repo_path(), &["add", "-A", "."]);
    common::git_run(project.repo_path(), &["commit", "-q", "-m", "feat: both"]);
    let report = range(&project, Some("HEAD~1"), None);
    assert_captured(&report);
    let changes = commits(&report)[0]["changes"].clone();
    assert_eq!(changes[0]["repository_path"], "README.md");
    assert!(changes[0]["project_path"].is_null());
    assert_eq!(changes[1]["repository_path"], "packages/docs/content/a.md");
    assert_eq!(changes[1]["project_path"], "content/a.md");
    // The policy comes from the head tree of the nested project.
    assert_eq!(report.source.as_ref().unwrap().kind, "revision");

    // A non-portable path anywhere in the repository is fatal and names
    // the commit.
    project.stage_entry("100644", b"x", "a:b.txt");
    project.git(&["commit", "-q", "-m", "chore: colon"]);
    let bad = project.git(&["rev-parse", "HEAD"]);
    let report = range(&project, Some("HEAD~1"), None);
    assert_history_fatal(&report, &format!("commit {bad}: the changed paths"));
    assert!(report.fatal.as_deref().unwrap().contains("a:b.txt"));
    assert_eq!(report.fatal, range(&project, Some("HEAD~1"), None).fatal);
}

#[test]
fn shallow_and_partial_clones_never_pass_or_fetch() {
    let origin = facts_project();
    for step in 1..=3 {
        origin.file(&format!("f{step}.txt"), "x\n");
        origin.commit_all(&format!("feat: step {step}"));
    }
    origin.git(&["config", "uploadpack.allowFilter", "true"]);
    let url = format!(
        "file://{}",
        origin.repo_path().canonicalize().unwrap().display()
    );

    // Shallow: the history reachable from the head is cut off.
    let shallow = tempfile::tempdir().unwrap();
    let shallow_path = shallow.path().join("shallow");
    common::git_run(
        shallow.path(),
        &[
            "clone",
            "-q",
            "--depth",
            "2",
            &url,
            shallow_path.to_str().unwrap(),
        ],
    );
    let run = |base: Option<&str>| {
        bearout::history(
            &shallow_path,
            &HistoryMode::Range {
                base: base.map(str::to_owned),
                head: None,
            },
            &Options::default(),
        )
    };
    let report = run(None);
    assert_history_fatal(&report, "is a shallow boundary");
    assert!(
        report
            .fatal
            .as_deref()
            .unwrap()
            .contains("deepen the clone")
    );
    // An explicit range above the boundary has every commit locally.
    let report = run(Some("HEAD~1"));
    assert!(report.fatal.is_none(), "{:?}", report.fatal);
    assert_eq!(report.commits, 1);
    assert_eq!(subjects(&report), ["feat: step 3"]);
    // A range that crosses the boundary is refused: HEAD~1 is the boundary.
    let boundary = common::git_run(&shallow_path, &["rev-parse", "HEAD~1"]);
    let report = bearout::history(
        &shallow_path,
        &HistoryMode::Range {
            base: None,
            head: Some(boundary.clone()),
        },
        &Options::default(),
    );
    assert_history_fatal(&report, &format!("commit {boundary} is a shallow boundary"));

    // Partial: trees are missing and are never fetched to describe them.
    let partial = tempfile::tempdir().unwrap();
    let partial_path = partial.path().join("partial");
    common::git_run(
        partial.path(),
        &[
            "clone",
            "-q",
            "--filter=tree:0",
            "--no-checkout",
            &url,
            partial_path.to_str().unwrap(),
        ],
    );
    let objects_before = std::fs::read_dir(partial_path.join(".git/objects/pack"))
        .unwrap()
        .count();
    let report = bearout::history(
        &partial_path,
        &HistoryMode::Range {
            base: Some("HEAD~1".to_owned()),
            head: None,
        },
        &Options::default(),
    );
    assert!(
        report.fatal.is_some(),
        "missing trees must not be described"
    );
    let objects_after = std::fs::read_dir(partial_path.join(".git/objects/pack"))
        .unwrap()
        .count();
    assert_eq!(objects_before, objects_after, "nothing was fetched");
}

#[test]
fn limits_bound_commits_changes_and_bytes() {
    let project = facts_project();
    for step in 1..=3 {
        project.file(&format!("f{step}.txt"), "x\n");
        project.file(&format!("g{step}.txt"), "y\n");
        project.commit_all(&format!("feat: step {step}"));
    }
    let with_limits = |limits: &str| {
        project.file(
            "bearout.toml",
            &format!("{}\n[limits]\n{limits}\n", common::BOOTSTRAP),
        );
        project.commit_all("chore: limits");
    };
    with_limits("history_commits = 2");
    assert_history_fatal(
        &range(&project, None, None),
        "more than `limits.history_commits` = 2 commit(s)",
    );
    let report = range(&project, Some("HEAD~2"), None);
    assert!(report.fatal.is_none(), "{:?}", report.fatal);
    with_limits("history_changes = 3");
    assert_history_fatal(
        &range(&project, Some("HEAD~3"), None),
        "more than `limits.history_changes` = 3 path(s)",
    );
    with_limits("history_commit_bytes = 200");
    let report = range(&project, Some("HEAD~1"), None);
    assert_history_fatal(&report, "above `limits.history_commit_bytes` = 200");
    assert!(report.fatal.as_deref().unwrap().starts_with("commit "));
    with_limits("history_bytes = 100");
    assert_history_fatal(
        &range(&project, Some("HEAD~1"), None),
        "history inputs exceed `limits.history_bytes` = 100",
    );
}

// ---- pending-message mode -----------------------------------------------

/// The name and email Git would record as the author, asked with the
/// process environment Bearout leaves in place: `GIT_AUTHOR_*` honoured,
/// repository and configuration redirection dropped.
fn pending_author(project: &Project) -> (String, String) {
    let output = std::process::Command::new("git")
        .args(["var", "GIT_AUTHOR_IDENT"])
        .current_dir(project.path())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("GIT_CONFIG_NOSYSTEM")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .output()
        .expect("git var");
    let ident = String::from_utf8(output.stdout).unwrap();
    let (rest, _) = ident.trim_end().rsplit_once(' ').unwrap();
    let (rest, _) = rest.rsplit_once(' ').unwrap();
    let (name, email) = rest.strip_suffix('>').unwrap().split_once(" <").unwrap();
    (name.to_owned(), email.to_owned())
}

/// A message file inside the repository's Git directory.
fn message_file(project: &Project, text: &[u8]) -> std::path::PathBuf {
    let git_dir = project.git(&["rev-parse", "--absolute-git-dir"]);
    let path = std::path::Path::new(&git_dir).join("COMMIT_EDITMSG");
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn the_pending_commit_is_described_from_the_captured_index() {
    let project = facts_project();
    let head = project.git(&["rev-parse", "HEAD"]);
    project.file("staged.txt", "s\n");
    project.git(&["add", "staged.txt"]);
    project.file("unstaged.txt", "u\n");
    let file = message_file(
        &project,
        b"feat: pending\n\n# a comment line\n  \nSigned-off-by: X <x@y>\n",
    );
    let report = message(&project, &file);
    assert_captured(&report);
    assert_eq!(report.mode, "message");
    assert_eq!(report.commits, 1);
    assert!(report.base.is_none() && report.head.is_none());
    let source = report.source.as_ref().unwrap();
    assert_eq!(source.kind, "index");
    assert!(source.tree.is_none());
    let fact = range_fact(&report);
    assert_eq!(fact["kind"], "message");
    assert!(fact["head"].is_null());
    let commit = &commits(&report)[0];
    assert_eq!(commit["key"], "pending");
    assert_eq!(commit["pending"], true);
    assert!(commit["id"].is_null() && commit["tree"].is_null() && commit["committer"].is_null());
    assert_eq!(commit["parents"], serde_json::json!([head]));
    assert_eq!(commit["merge"], false);
    assert_eq!(commit["change_basis"], head);
    assert_eq!(
        commit["message"], "feat: pending\n\n# a comment line\n  \nSigned-off-by: X <x@y>\n",
        "comments, whitespace, and trailers are exactly as supplied"
    );
    assert_eq!(commit["subject"], "feat: pending");
    let changes = commit["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1, "unstaged files are not pending changes");
    assert_eq!(changes[0]["repository_path"], "staged.txt");
    assert_eq!(changes[0]["change"], "added");
    // The author is the one Git would record under the same environment
    // Bearout runs Git in: `GIT_AUTHOR_*` is honoured, configuration
    // redirection is not.
    let ident = pending_author(&project);
    assert_eq!(commit["author"]["name"], ident.0);
    assert_eq!(commit["author"]["email"], ident.1);
    assert_eq!(commit["author"]["timezone"].as_str().unwrap().len(), 5);
    let rendered = report.diagnostics[0].to_string();
    let json = rendered
        .strip_prefix("range:B033[facts]: history check `facts`: ")
        .unwrap_or_else(|| panic!("{rendered}"));
    assert_eq!(serde_json::from_str::<Value>(json).unwrap(), fact);
    assert!(
        report.diagnostics[1]
            .to_string()
            .starts_with("commit pending:B033[facts]: ")
    );
}

#[test]
fn pending_policy_and_state_come_from_the_index_not_the_working_tree() {
    let project =
        project_with("def never(history):\n    return []\nhistory_check(\"never\", never)\n");
    project.commit_all("chore: initial");
    // Stage a policy that reports, then undo it in the working tree only.
    project.file(
        common::ENTRY,
        "def flag(history):\n    return [error(\"staged policy\", commit = \"pending\")]\nhistory_check(\"flag\", flag)\n",
    );
    project.git(&["add", common::ENTRY]);
    project.file(
        common::ENTRY,
        "def never(history):\n    return []\nhistory_check(\"never\", never)\n",
    );
    let file = message_file(&project, b"chore: anything\n");
    let report = message(&project, &file);
    assert!(!report.ok);
    assert_eq!(
        lines(&report),
        ["commit pending:B032[flag]: history check `flag`: staged policy"]
    );
    // An unstaged policy that would report changes nothing either.
    project.git(&["checkout", "-q", "HEAD", "--", common::ENTRY]);
    project.file(
        common::ENTRY,
        "def flag(history):\n    return [error(\"unstaged policy\", commit = \"pending\")]\nhistory_check(\"flag\", flag)\n",
    );
    let report = message(&project, &file);
    assert!(report.ok, "{}", lines(&report).join("\n"));
    // A policy staged with a broken helper is a fatal load, not a pass.
    project.file(common::ENTRY, "this is not starlark\n");
    project.git(&["add", common::ENTRY]);
    let report = message(&project, &file);
    assert_history_fatal(&report, "the repository policy did not load");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == Code::ScriptLoad)
    );
    assert_eq!(
        report.diagnostics[0].target,
        HistoryTarget::Path {
            path: "bearout.star".to_owned()
        }
    );
    // No history check at all is fatal, never a pass.
    project.file(common::ENTRY, "check(\"ordinary\", lambda p: [])\n");
    project.git(&["add", common::ENTRY]);
    assert_history_fatal(&message(&project, &file), "registers no history check");
}

#[test]
fn message_files_are_read_exactly_and_refused_when_unsafe() {
    let project = facts_project();
    // Empty: an input to policy.
    let file = message_file(&project, b"");
    let report = message(&project, &file);
    assert_captured(&report);
    assert_eq!(commits(&report)[0]["message"], "");
    assert_eq!(commits(&report)[0]["subject"], "");
    // Non-UTF-8, NUL, oversized, missing, a directory, outside, linked.
    let file = message_file(&project, b"\xff\xfe");
    assert_history_fatal(&message(&project, &file), "is not valid UTF-8");
    let file = message_file(&project, b"subject\0hidden");
    assert_history_fatal(&message(&project, &file), "contains a NUL byte");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nhistory_commit_bytes = 16\n",
            common::BOOTSTRAP
        ),
    );
    project.git(&["add", "bearout.toml"]);
    let file = message_file(&project, b"a message longer than sixteen bytes\n");
    assert_history_fatal(
        &message(&project, &file),
        "above `limits.history_commit_bytes` = 16",
    );
    project.git(&["checkout", "-q", "HEAD", "--", "bearout.toml"]);
    project.git(&["add", "bearout.toml"]);
    std::fs::remove_file(&file).unwrap();
    assert_history_fatal(&message(&project, &file), "cannot read the message file");
    let git_dir = std::path::PathBuf::from(project.git(&["rev-parse", "--absolute-git-dir"]));
    assert_history_fatal(
        &message(&project, &git_dir.join("hooks")),
        "is not a regular file",
    );
    let outside = tempfile::tempdir().unwrap();
    let elsewhere = outside.path().join("COMMIT_EDITMSG");
    std::fs::write(&elsewhere, b"feat: elsewhere\n").unwrap();
    assert_history_fatal(
        &message(&project, &elsewhere),
        "lies outside the repository's Git directory",
    );
    project.file("MSG", "feat: in the work tree\n");
    assert_history_fatal(
        &message(&project, &project.path().join("MSG")),
        "lies outside the repository's Git directory",
    );
    #[cfg(unix)]
    {
        let link = git_dir.join("LINKED_MSG");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert_history_fatal(&message(&project, &link), "is a symbolic link");
        let inside = message_file(&project, b"feat: real\n");
        let link_inside = git_dir.join("LINK_INSIDE");
        std::os::unix::fs::symlink(&inside, &link_inside).unwrap();
        assert_history_fatal(&message(&project, &link_inside), "is a symbolic link");
    }
}

#[test]
fn pending_parents_cover_normal_merge_and_unborn_states() {
    // Unborn: the policy is staged, nothing is committed.
    let project = project_with(FACTS);
    project.git(&["add", "-A", "."]);
    let file = message_file(&project, b"chore: first\n");
    let report = message(&project, &file);
    assert_captured(&report);
    let commit = &commits(&report)[0];
    assert!(commit["parents"].as_array().unwrap().is_empty());
    assert!(
        commit["change_basis"].is_null(),
        "an unborn branch compares to the empty tree"
    );
    assert_eq!(commit["merge"], false);
    let paths: Vec<&str> = commit["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["repository_path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "bearout.star",
            "bearout.toml",
            "content/.keep",
            "rules/helpers.star"
        ]
    );

    // Normal: one parent.
    project.commit_all("chore: first");
    let head = project.git(&["rev-parse", "HEAD"]);
    let report = message(&project, &file);
    assert_eq!(commits(&report)[0]["parents"], serde_json::json!([head]));

    // Merge in progress: HEAD then MERGE_HEAD, and merge is true.
    project.git(&["checkout", "-q", "-b", "side"]);
    project.file("side.txt", "s\n");
    let side = project.commit_all("feat: side");
    project.git(&["checkout", "-q", "main"]);
    project.file("main.txt", "m\n");
    let main = project.commit_all("feat: main");
    project.git(&["merge", "-q", "--no-commit", "--no-ff", "side"]);
    let report = message(&project, &file);
    assert_captured(&report);
    let commit = &commits(&report)[0];
    assert_eq!(commit["parents"], serde_json::json!([main, side]));
    assert_eq!(commit["merge"], true);
    assert_eq!(commit["change_basis"], main);
    assert_eq!(commit["changes"][0]["repository_path"], "side.txt");
    project.git(&["merge", "--abort"]);
}

#[test]
fn staged_changes_of_every_kind_are_pending_changes() {
    let project = facts_project();
    project.file("gone.txt", "x\n");
    project.file("mode.sh", "#!/bin/sh\n");
    project.commit_all("chore: base");
    project.git(&["rm", "-q", "--cached", "gone.txt"]);
    project.git(&["update-index", "--chmod=+x", "mode.sh"]);
    project.stage_entry("120000", b"mode.sh", "link");
    project.stage_entry("160000", b"", "vendor");
    project.file("new.txt", "n\n");
    project.git(&["add", "new.txt"]);
    // An intent-to-add entry is not what the commit records.
    project.file("intent.txt", "i\n");
    project.git(&["add", "-N", "intent.txt"]);
    let file = message_file(&project, b"feat: staged\n");
    let report = message(&project, &file);
    assert_captured(&report);
    let changes: Vec<(String, String)> = commits(&report)[0]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["repository_path"].as_str().unwrap().to_owned(),
                format!(
                    "{}:{}->{}",
                    c["change"].as_str().unwrap(),
                    c["before"]["kind"].as_str().unwrap_or("-"),
                    c["after"]["kind"].as_str().unwrap_or("-")
                ),
            )
        })
        .collect();
    assert_eq!(
        changes,
        [
            ("gone.txt".to_owned(), "removed:file->-".to_owned()),
            ("link".to_owned(), "added:-->symlink".to_owned()),
            ("mode.sh".to_owned(), "modified:file->executable".to_owned()),
            ("new.txt".to_owned(), "added:-->file".to_owned()),
            ("vendor".to_owned(), "added:-->gitlink".to_owned()),
        ]
    );
}

#[test]
fn linked_worktrees_use_their_own_git_directory_and_index() {
    let project = facts_project();
    let linked = tempfile::tempdir().unwrap();
    let linked_path = linked.path().join("wt");
    project.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "feature",
        linked_path.to_str().unwrap(),
    ]);
    common::git_run(&linked_path, &["config", "user.name", "Test"]);
    // Stage in the linked worktree only.
    std::fs::write(linked_path.join("wt.txt"), "w\n").unwrap();
    common::git_run(&linked_path, &["add", "wt.txt"]);
    let wt_git_dir = common::git_run(&linked_path, &["rev-parse", "--absolute-git-dir"]);
    assert!(wt_git_dir.contains("worktrees"));
    let file = std::path::Path::new(&wt_git_dir).join("COMMIT_EDITMSG");
    std::fs::write(&file, b"feat: worktree\n").unwrap();
    let report = bearout::history(
        &linked_path,
        &HistoryMode::Message { file: file.clone() },
        &Options::default(),
    );
    assert_captured(&report);
    let commit = &commits(&report)[0];
    assert_eq!(commit["changes"][0]["repository_path"], "wt.txt");
    // The main worktree's index has nothing staged, and the main
    // worktree's message file lies outside the linked worktree's own Git
    // directory.
    let main_file = message_file(&project, b"feat: main\n");
    let report = message(&project, &main_file);
    assert!(
        commits(&report)[0]["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let report = bearout::history(
        &linked_path,
        &HistoryMode::Message {
            file: main_file.clone(),
        },
        &Options::default(),
    );
    assert_history_fatal(&report, "lies outside the repository's Git directory");
    // A range from the linked worktree resolves its own HEAD.
    common::git_run(&linked_path, &["commit", "-q", "-m", "feat: on feature"]);
    let report = bearout::history(
        &linked_path,
        &HistoryMode::Range {
            base: Some("main".to_owned()),
            head: None,
        },
        &Options::default(),
    );
    assert_eq!(subjects(&report), ["feat: on feature"]);
}

// ---- policy and findings ------------------------------------------------

#[test]
fn only_history_checks_run_and_only_for_history_commands() {
    let project = project_with(
        "def never(p):\n    return [error(\"ordinary check ran\", resource = \"nothing\")]\ndef flag(history):\n    return [error(\"history ran\", commit = history[\"commits\"][0][\"key\"])]\nschema(\"example/test/note@1\")\ncheck(\"never\", never)\nhistory_check(\"flag\", flag)\ngenerator(\"g\", lambda p: [])\n",
    );
    project.commit_all("chore: initial");
    let report = range(&project, None, None);
    assert!(report.fatal.is_none(), "{:?}", report.fatal);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code, Code::HistoryError);
    assert!(!report.ok);
    // The ordinary pipeline never sees the history check.
    let ordinary = project.check();
    assert!(ordinary.fatal.is_none());
    assert!(
        !ordinary
            .diagnostics
            .iter()
            .any(|d| d.message.contains("history ran"))
    );
    // A history check that would fail the ordinary run does not run there.
    let project =
        project_with("def boom(history):\n    fail(\"never\")\nhistory_check(\"boom\", boom)\n");
    project.commit_all("chore: initial");
    common::assert_clean(&project.check());
    common::assert_clean(&project.check_from(bearout::Source::Index));
}

#[test]
fn errors_warnings_output_failures_and_bad_results_are_reported() {
    let project = project_with(
        "def mixed(history):\n    print(\"seen %d\" % len(history[\"commits\"]))\n    key = history[\"commits\"][0][\"key\"]\n    return [error(\"bad header\", commit = key, line = 1, code = \"header\"), warning(\"long body\", commit = key), warning(\"range note\")]\ndef broken(history):\n    return 3\ndef failing(history):\n    fail(\"policy exploded\")\nhistory_check(\"mixed\", mixed)\nhistory_check(\"broken\", broken)\nhistory_check(\"failing\", failing)\n",
    );
    project.commit_all("chore: initial");
    let report = range(&project, None, None);
    assert!(report.fatal.is_none(), "{:?}", report.fatal);
    let head = project.git(&["rev-parse", "HEAD"]);
    assert_eq!(
        lines(&report),
        [
            "bearout.star:B014: history check `broken` must return a list of findings, found int"
                .to_owned(),
            "bearout.star:B017: history check `mixed` printed: seen 1".to_owned(),
            "bearout.star:8:B013: history check `failing` failed: fail: policy exploded".to_owned(),
            "range:B033[mixed]: history check `mixed`: range note".to_owned(),
            format!("commit {head}:B033[mixed]: history check `mixed`: long body"),
            format!("commit {head}:1:B032[header]: history check `mixed`: bad header"),
        ],
        "script diagnostics by path, then range-wide, then commits in order; line, code, rule, message within each"
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["diagnostics"][5]["commit"], head);
    assert_eq!(json["diagnostics"][5]["line"], 1);
    assert_eq!(json["diagnostics"][5]["rule"], "header");
    assert_eq!(json["diagnostics"][5]["severity"], "error");
    assert!(json["diagnostics"][5].get("path").is_none());
    assert!(json["diagnostics"][3].get("commit").is_none());
    assert!(json["diagnostics"][3].get("path").is_none());
    assert_eq!(json["diagnostics"][0]["path"], "bearout.star");
    assert_eq!(json["diagnostics"][4]["severity"], "warning");
    assert!(json["diagnostics"][4]["line"].is_null());
    assert!(!report.ok, "a warning alone is a finding");
    assert_eq!(
        serde_json::to_string(&report).unwrap(),
        serde_json::to_string(&range(&project, None, None)).unwrap()
    );
    // A single warning fails the run.
    let project = project_with("history_check(\"warn\", lambda h: [warning(\"note\")])\n");
    project.commit_all("chore: initial");
    let report = range(&project, None, None);
    assert!(!report.ok);
    assert_eq!(report.diagnostics[0].severity, Severity::Warning);
    // Cancellation.
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let report = bearout::history(
        project.path(),
        &HistoryMode::Range {
            base: None,
            head: None,
        },
        &Options {
            cancel: Some(cancel),
            ..Options::default()
        },
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == Code::ScriptFailure),
        "{}",
        lines(&report).join("\n")
    );
}

#[test]
fn finding_targets_are_validated_against_the_view() {
    let entry = |body: &str| {
        format!(
            "def t(history):\n    key = history[\"commits\"][0][\"key\"]\n    return [{body}]\nhistory_check(\"t\", t)\n"
        )
    };
    let project = project_with(&entry("error(\"x\", commit = key)"));
    project.commit_all("chore: initial");
    for (body, expected) in [
        (
            "error(\"x\", commit = key, line = 3)".to_owned(),
            "commit {head}:3:B032[t]".to_owned(),
        ),
        (
            "error(\"x\", commit = key, line = 4)".to_owned(),
            "B014: history check `t` finding line 4 is beyond the 3 line(s) of the message"
                .to_owned(),
        ),
        (
            "error(\"x\", commit = \"pending\")".to_owned(),
            "finding names `pending`, which exists only for a pending-message check".to_owned(),
        ),
        (
            "error(\"x\", commit = \"0000000000000000000000000000000000000000\")".to_owned(),
            "finding names unknown commit `0000000000000000000000000000000000000000`".to_owned(),
        ),
        (
            "error(\"x\", line = 1)".to_owned(),
            "a finding line needs a `commit` target".to_owned(),
        ),
        (
            "error(\"x\", resource = \"note-a\")".to_owned(),
            "never names a resource, a document, or a comparison side".to_owned(),
        ),
        (
            "error(\"x\", path = \"README.md\")".to_owned(),
            "never names a resource".to_owned(),
        ),
        (
            "error(\"x\", side = \"baseline\")".to_owned(),
            "never names a resource".to_owned(),
        ),
        (
            "error(\"x\")".to_owned(),
            "range:B032[t]: history check `t`: x".to_owned(),
        ),
    ] {
        project.file(common::ENTRY, &entry(&body));
        project.commit_all("chore: policy\nline two\nline three\n");
        let head = project.git(&["rev-parse", "HEAD"]);
        let expected = expected.replace("{head}", &head);
        let report = range(&project, Some("HEAD~1"), None);
        assert!(report.fatal.is_none(), "{body}: {:?}", report.fatal);
        assert!(
            lines(&report).iter().any(|line| line.contains(&expected)),
            "{body}: expected {expected:?}, got:\n{}",
            lines(&report).join("\n")
        );
    }
    // Targets that are invalid at construction fail inside the script.
    for body in [
        "error(\"x\", commit = key, resource = \"r\")",
        "error(\"x\", commit = key, path = \"p.md\")",
        "error(\"x\", commit = key, side = \"baseline\")",
        "error(\"x\", commit = \"\")",
        "error(\"x\", commit = \"a b\")",
    ] {
        project.file(common::ENTRY, &entry(body));
        project.commit_all("chore: policy");
        let report = range(&project, Some("HEAD~1"), None);
        assert!(
            lines(&report).iter().any(|line| line.contains("B013")),
            "{body}: {}",
            lines(&report).join("\n")
        );
    }
    // Ordinary checks cannot target commits.
    let project = project_with(
        "check(\"c\", lambda p: [error(\"x\", commit = \"pending\")])\nhistory_check(\"h\", lambda h: [])\n",
    );
    let report = project.check();
    assert!(
        report.diagnostics.iter().any(
            |d| d.code == Code::ScriptResult && d.message.contains("only from a history check")
        ),
        "{:?}",
        report.diagnostics
    );
}
