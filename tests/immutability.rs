// SPDX-License-Identifier: Apache-2.0

//! Policy-defined immutability on the `decision-records` sample: the
//! repository's own rule interprets the comparison view, and Bearout
//! supplies the two trees, the change facts, and the finding targets.

mod common;

use bearout::{Command, Mode, Options, Source};
use common::{Project, assert_clean, assert_line, assert_no_line, lines};

fn compare(source: Source, baseline: &str) -> Options {
    Options {
        source,
        baseline: Some(baseline.to_owned()),
        ..Options::default()
    }
}

/// The sample, committed once on `main`.
fn committed_sample() -> (Project, String) {
    let project = Project::sample("decision-records");
    project.git_init();
    let commit = project.commit_all("sample");
    (project, commit)
}

const RECORD_0001_RULING: &str =
    "text = \"An accepted record changes only through a new record that supersedes it.\"";

#[test]
fn without_a_baseline_the_comparison_aware_check_is_inactive() {
    let (project, _) = committed_sample();
    project.file(
        "records/decision-0001.md",
        &project
            .read("records/decision-0001.md")
            .replace(RECORD_0001_RULING, "text = \"Rewritten.\""),
    );
    assert_clean(&project.check());
    assert_clean(&project.check_from(Source::Index));
}

#[test]
fn an_unchanged_candidate_is_clean_and_yields_no_change_facts() {
    let (project, commit) = committed_sample();
    for source in [
        Source::WorkingDirectory,
        Source::Index,
        Source::Revision("HEAD".to_owned()),
    ] {
        let report = project.run(Command::Generate(Mode::Check), &compare(source, &commit));
        assert_clean(&report);
        assert!(report.diagnostics.is_empty(), "{:?}", lines(&report));
        assert_eq!(report.outputs, ["generated/decision-index.md"]);
    }
    // The change list really is empty, seen through a facts-printing check.
    project.file(
        "bearout.star",
        "def facts(p):\n    return [warning(\"facts %d\" % len(p[\"comparison\"][\"changes\"]), resource = \"decision-0001\")]\nschema(\"example/decision-records/decision@1\", shape = \"decision.schema.toml\")\ncheck(\"facts\", facts)\n",
    );
    assert_line(
        &project.run(Command::Check, &compare(Source::WorkingDirectory, &commit)),
        "facts 0",
    );
}

#[test]
fn new_records_are_free_and_protected_content_is_not() {
    let (project, commit) = committed_sample();
    // A new proposed record: the policy allows it.
    project.file(
        "records/decision-0006.md",
        "+++\nschema = \"example/decision-records/decision@1\"\nid = \"decision-0006\"\ntitle = \"Archive rejected records after a year\"\nstatus = \"proposed\"\ndate = \"2026-09-04\"\n+++\n\n# Archive rejected records after a year\n\n## Context\n\nRejected records accumulate.\n",
    );
    assert_clean(&project.run(Command::Check, &compare(Source::WorkingDirectory, &commit)));
    project.remove("records/decision-0006.md");

    // A protected field.
    let original = project.read("records/decision-0001.md");
    project.file(
        "records/decision-0001.md",
        &original.replace("date = \"2026-08-20\"", "date = \"2026-08-21\""),
    );
    let report = project.run(Command::Check, &compare(Source::WorkingDirectory, &commit));
    assert_eq!(
        lines(&report),
        [
            "records/decision-0001.md:B015[protected-field]: check `protected-records-are-immutable`: protected record changed its date; only `title`, relations, and the Context section may be corrected"
        ]
    );

    // A protected fragment.
    project.file(
        "records/decision-0001.md",
        &original.replace(RECORD_0001_RULING, "text = \"Rewritten.\""),
    );
    let report = project.run(Command::Check, &compare(Source::WorkingDirectory, &commit));
    assert_line(
        &report,
        "records/decision-0001.md:B015[protected-rulings]: check `protected-records-are-immutable`: protected record changed its rulings",
    );
    assert_eq!(report.errors(), 1);

    // A protected status change, and the one permitted transition.
    project.file(
        "records/decision-0005.md",
        &format!(
            "{}\n## Decision\n\nAccepted after all.\n\n## Rulings\n\n### decision-0005-ruling-01\n\n```toml bearout=ruling\nid = \"decision-0005-ruling-01\"\ntext = \"Reopened.\"\n```\n",
            project
                .read("records/decision-0005.md")
                .replace("status = \"rejected\"", "status = \"accepted\"")
        ),
    );
    let report = project.run(Command::Check, &compare(Source::WorkingDirectory, &commit));
    assert_line(
        &report,
        "records/decision-0005.md:B015[protected-status]: check `protected-records-are-immutable`: protected record changed status from `rejected` to `accepted`",
    );

    // Metadata-only corrections are clean: title and Context.
    project.file(
        "records/decision-0005.md",
        &project.git(&["show", &format!("{commit}:records/decision-0005.md")]),
    );
    project.file(
        "records/decision-0001.md",
        &original
            .replace(
                "title = \"Decision records get citable rulings\"",
                "title = \"Decision records get citable rulings (corrected)\"",
            )
            .replace(
                "A decision that cannot be cited cannot be relied on.",
                "A decision that cannot be cited cannot be relied upon.",
            ),
    );
    assert_clean(&project.run(Command::Check, &compare(Source::WorkingDirectory, &commit)));
}

#[test]
fn deleting_or_moving_a_protected_record_is_the_policys_call() {
    let (project, commit) = committed_sample();
    project.remove("records/decision-0005.md");
    let report = project.run(Command::Check, &compare(Source::WorkingDirectory, &commit));
    assert_eq!(
        lines(&report),
        [
            "baseline:records/decision-0005.md:B015[protected-record-deleted]: check `protected-records-are-immutable`: protected record `decision-0005` was deleted; supersede it with a new record instead"
        ]
    );
    assert_eq!(report.diagnostics[0].side, bearout::Side::Baseline);
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["diagnostics"][0]["side"], "baseline");
    assert_eq!(json["diagnostics"][0]["path"], "records/decision-0005.md");

    // Moved within its directory, so its relative links still resolve:
    // both views hold the record, and the policy warns.
    project.git(&["checkout", "-q", "--", "records/decision-0005.md"]);
    project.git(&[
        "mv",
        "records/decision-0005.md",
        "records/decision-0005-rejected.md",
    ]);
    let report = project.run(Command::Check, &compare(Source::WorkingDirectory, &commit));
    assert_clean(&report);
    assert_eq!(
        lines(&report),
        [
            "records/decision-0005-rejected.md:B016[protected-record-moved]: check `protected-records-are-immutable`: protected record moved from `records/decision-0005.md`"
        ]
    );
    // A move that must rewrite a link inside the protected Decision section
    // is caught by the same rule.
    project.git(&[
        "mv",
        "records/decision-0005-rejected.md",
        "records/decision-0005.md",
    ]);
    std::fs::create_dir_all(project.path().join("records/archive")).expect("dir");
    project.git(&[
        "mv",
        "records/decision-0005.md",
        "records/archive/decision-0005.md",
    ]);
    project.file(
        "records/archive/decision-0005.md",
        &project
            .read("records/archive/decision-0005.md")
            .replace("](decision-0001.md#", "](../decision-0001.md#"),
    );
    let report = project.run(Command::Check, &compare(Source::WorkingDirectory, &commit));
    assert_line(
        &report,
        "records/archive/decision-0005.md:B015[protected-decision]",
    );
    assert_line(
        &report,
        "records/archive/decision-0005.md:B016[protected-record-moved]",
    );
    project.git(&[
        "mv",
        "records/archive/decision-0005.md",
        "records/decision-0005.md",
    ]);
    project.git(&["checkout", "-q", "--", "records/decision-0005.md"]);
    // Generation still verifies clean against the moved layout.
    assert_clean(&project.run(
        Command::Generate(Mode::Write),
        &compare(Source::WorkingDirectory, &commit),
    ));
}

#[test]
fn schema_less_documents_are_compared_too() {
    let (project, commit) = committed_sample();
    // Modified: facts only, nothing the policy objects to.
    project.file(
        "README.md",
        &format!("{}\nAn added line.\n", project.read("README.md")),
    );
    assert_clean(&project.run(Command::Check, &compare(Source::WorkingDirectory, &commit)));
    // Added: select another document.
    project.file("NOTES.md", "# Notes\n");
    project.file(
        "bearout.toml",
        &project.read("bearout.toml").replace(
            "files = [\"README.md\"]",
            "files = [\"README.md\", \"NOTES.md\"]",
        ),
    );
    let report = project.run(Command::Check, &compare(Source::WorkingDirectory, &commit));
    assert_clean(&report);
    assert_eq!(report.documents, 2);
    // Removed from the selection: reported against the baseline.
    project.file(
        "bearout.toml",
        &project.read("bearout.toml").replace(
            "files = [\"README.md\", \"NOTES.md\"]",
            "files = [\"NOTES.md\"]",
        ),
    );
    let report = project.run(Command::Check, &compare(Source::WorkingDirectory, &commit));
    assert_eq!(
        lines(&report),
        [
            "baseline:README.md:B015[document-removed]: check `protected-records-are-immutable`: document `README.md` was removed from the selection"
        ]
    );
}

#[test]
fn staged_violations_show_through_the_index_and_repairs_cannot_hide_them() {
    let (project, commit) = committed_sample();
    let original = project.read("records/decision-0001.md");
    project.file(
        "records/decision-0001.md",
        &original.replace(RECORD_0001_RULING, "text = \"Rewritten.\""),
    );
    project.git(&["add", "records/decision-0001.md"]);
    project.file("records/decision-0001.md", &original);
    assert_clean(&project.run(Command::Check, &compare(Source::WorkingDirectory, &commit)));
    let report = project.run(Command::Check, &compare(Source::Index, &commit));
    assert_line(&report, "records/decision-0001.md:B015[protected-rulings]");
    assert_eq!(report.errors(), 1);

    // Revision candidate against revision baseline, with the branch moved
    // after the baseline name was resolved.
    project.git(&["commit", "-q", "-m", "rewrite a ruling"]);
    let rewritten = project.git(&["rev-parse", "HEAD"]);
    let report = project.run(
        Command::Check,
        &compare(Source::Revision(rewritten.clone()), "main~1"),
    );
    assert_line(&report, "B015[protected-rulings]");
    assert_eq!(
        report.baseline.as_ref().unwrap().revision.as_deref(),
        Some("main~1")
    );
    assert_eq!(
        report.baseline.unwrap().tree.unwrap(),
        project.git(&["rev-parse", &format!("{commit}^{{tree}}")])
    );
    project.git(&["checkout", "-q", "--", "records/decision-0001.md"]);
    let pinned = project.run(Command::Check, &compare(Source::WorkingDirectory, "main"));
    assert_clean(&pinned);
    project.file("records/decision-0001.md", &original);
    project.commit_all("restore");
    let later = project.run(Command::Check, &compare(Source::WorkingDirectory, "main"));
    assert_ne!(
        pinned.baseline.as_ref().unwrap().tree,
        later.baseline.as_ref().unwrap().tree,
        "each run resolves the name once, at its start"
    );
    assert_eq!(
        project.git(&["rev-parse", "HEAD~1^{tree}"]),
        project.git(&["rev-parse", &format!("{rewritten}^{{tree}}")])
    );
    assert_no_line(&later, "B015");
}

#[test]
fn reports_are_identical_across_runs() {
    let (project, commit) = committed_sample();
    project.remove("records/decision-0005.md");
    project.file(
        "records/decision-0001.md",
        &project
            .read("records/decision-0001.md")
            .replace(RECORD_0001_RULING, "text = \"Rewritten.\""),
    );
    let first = project.run(
        Command::Generate(Mode::Check),
        &compare(Source::WorkingDirectory, &commit),
    );
    let second = project.run(
        Command::Generate(Mode::Check),
        &compare(Source::WorkingDirectory, &commit),
    );
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert_eq!(first.errors(), 2);
    let sides: Vec<bearout::Side> = first.diagnostics.iter().map(|d| d.side).collect();
    assert_eq!(sides, [bearout::Side::Candidate, bearout::Side::Baseline]);
}
