// SPDX-License-Identifier: Apache-2.0

//! Kernel behaviour: envelope parsing, Markdown structure, link resolution,
//! deterministic ordering, fatal outcomes, and the JSON report.

mod common;

use bearout::{Code, Command, Options};
use std::fs;

use common::{Project, assert_clean, assert_line, assert_no_line, codes, lines};

#[test]
fn valid_minimal_fixture_is_clean() {
    let report = Project::fixture("valid-minimal").check();
    assert_clean(&report);
    assert_eq!(report.resources, 2);
    assert!(report.ok);
}

#[test]
fn parser_cases_resolve_unicode_anchors_dates_crlf_and_bom() {
    let project = Project::fixture("parser-cases");
    let report = project.check();
    assert_clean(&report);
    assert_eq!(report.resources, 4);

    let report = project
        .file(
            "content/doc-unicode.md",
            &project
                .read("content/doc-unicode.md")
                .replace("#duplicate-1", "#duplicate-2"),
        )
        .check();
    assert_line(
        &report,
        "content/doc-unicode.md:14:B011: link `#duplicate-2` names anchor `duplicate-2`",
    );
    project.file(
        "content/doc-unicode.md",
        &project
            .read("content/doc-unicode.md")
            .replace("#duplicate-2", "#duplicate-1"),
    );

    let report = project
        .file(
            "content/doc-crlf.md",
            &project
                .read("content/doc-crlf.md")
                .replace("#%C4%89u-vi-parolas", "#cu-vi-parolas"),
        )
        .check();
    assert_line(
        &report,
        "B011: link `doc-unicode.md#cu-vi-parolas` names anchor `cu-vi-parolas`",
    );
}

#[test]
fn native_dates_have_one_textual_form_in_the_report_path() {
    let project = Project::fixture("parser-cases");
    project.file(
        "bearout.star",
        "def v(r):\n    f = r[\"fields\"]\n    if r[\"id\"] != \"doc-dates\":\n        return []\n    expected = {\"date\": \"2026-01-02\", \"time\": \"10:00:00\", \"local\": \"2026-01-02T10:00:00\", \"zulu\": \"2026-01-02T10:00:00Z\", \"offset\": \"2026-01-02T10:00:00+02:00\"}\n    return [error(\"%s is %r\" % (k, f.get(k))) for k in expected if f.get(k) != expected[k]]\n\nschema(\"example/fixture/doc@1\", shape = \"doc.schema.toml\", validate = v)\n",
    );
    let report = project.check();
    assert_clean(&report);
    let report = project
        .file(
            "content/doc-dates.toml",
            "schema = \"example/fixture/doc@1\"\nid = \"doc-dates\"\ndate = 2026-01-03\n",
        )
        .check();
    assert_line(
        &report,
        "B015: schema `example/fixture/doc@1` validate: date is \"2026-01-03\"",
    );
}

#[test]
fn envelope_failures_are_diagnostics_not_fatal() {
    let project = Project::with_note();
    project.file("content/note-b.md", "# missing front matter\n");
    project.file(
        "content/note-c.md",
        "+++\nschema = \"Bad Schema\"\nid = \"note-c\"\n+++\n",
    );
    project.file(
        "content/note-d.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"Note D\"\ntitle = \"D\"\n+++\n",
    );
    project.file(
        "content/note-e.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-e\"\ntitle = [\n+++\n",
    );
    project.file(
        "content/note-f.md",
        "+++\nschema = \"example/other/note@1\"\nid = \"note-f\"\ntitle = \"F\"\n+++\n",
    );
    let report = project.check();
    assert!(report.fatal.is_none());
    assert_line(
        &report,
        "content/note-b.md:B002: resource must begin with TOML front matter",
    );
    assert_line(&report, "content/note-c.md:2:B003: schema `Bad Schema`");
    assert_line(&report, "content/note-d.md:3:B002: identifier `Note D`");
    assert_line(&report, "content/note-e.md:4:B002: invalid front matter");
    assert_line(
        &report,
        "content/note-f.md:B003: schema `example/other/note@1` is not registered",
    );
    assert_eq!(report.resources, 6);
}

#[test]
fn unknown_fields_missing_sections_and_bad_relations_are_reported_once() {
    let project = Project::with_note();
    project.file("content/note-b.md", "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\nnext = \"note-a\"\nextra = 1\n+++\n");
    project.file("content/note-c.md", "+++\nschema = \"example/test/note@1\"\nid = \"note-c\"\ntitle = \"C\"\nnext = \"note-b\"\n+++\n");
    let report = project.check();
    assert_line(
        &report,
        "content/note-b.md:6:B005: Additional properties are not allowed ('extra' was unexpected)",
    );
    // note-b is structurally invalid, but its identifier still resolves: no cascade.
    assert_no_line(&report, "B009");
    assert_eq!(report.errors(), 1);
}

#[test]
fn duplicate_ids_and_typed_relations() {
    let project = Project::with_note();
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"B\"\n+++\n",
    );
    project.file("content/note-c.md", "+++\nschema = \"example/test/note@1\"\nid = \"note-c\"\ntitle = \"C\"\nnext = \"note-zzz\"\n+++\n");
    let report = project.check();
    assert_line(
        &report,
        "content/note-a.md:B008: identifier `note-a` is defined more than once",
    );
    assert_line(
        &report,
        "content/note-b.md:B008: identifier `note-a` is defined more than once",
    );
    assert_line(
        &report,
        "content/note-c.md:5:B009: `next` names `note-zzz`, which nothing defines",
    );
}

#[test]
fn diagnostics_are_sorted_and_deduplicated() {
    let project = Project::with_note();
    project.file(
        "content/note-z.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-z\"\n+++\n",
    );
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\n+++\n",
    );
    let report = project.check();
    let paths: Vec<&str> = report.diagnostics.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["content/note-b.md", "content/note-z.md"]);
    let again = project.check();
    assert_eq!(lines(&report), lines(&again));
}

#[test]
fn fatal_outcomes_keep_the_report_shape() {
    let empty = tempfile::tempdir().expect("dir");
    let report = bearout::run(empty.path(), Command::Check, &Options::default());
    assert!(
        report
            .fatal
            .as_deref()
            .is_some_and(|m| m.contains("bearout.toml"))
    );
    assert!(!report.ok);
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["ok"], false);
    assert!(json["fatal"].is_string());
    assert_eq!(json["diagnostics"], serde_json::json!([]));

    let project = Project::new();
    project.file("bearout.toml", "version = 1\n");
    assert!(
        project
            .check()
            .fatal
            .as_deref()
            .is_some_and(|m| m.contains("`entry` is required"))
    );

    let project = Project::new();
    project.file("bearout.toml", "version = 1\nentry = \"bearout.star\"\n[resources]\nroots = [\"../outside\"]\n[rules]\nroot = \"rules\"\n");
    assert!(
        project
            .check()
            .fatal
            .as_deref()
            .is_some_and(|m| m.contains("normalized"))
    );

    let project = Project::new();
    project.file("bearout.star", "");
    assert!(
        project
            .check()
            .fatal
            .as_deref()
            .is_some_and(|m| m.contains("resource root `content` is not a directory"))
    );

    let missing = std::path::Path::new("/definitely/not/a/project/anywhere");
    let report = bearout::run(missing, Command::Check, &Options::default());
    assert!(report.fatal.is_some());
}

#[test]
fn resource_limits_apply_to_input() {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &format!("{}\n[limits]\nresource_bytes = 40\n", common::BOOTSTRAP),
    );
    let report = project.check();
    assert_line(&report, "content/note-a.md:B001: resource is");
    project.file(
        "bearout.toml",
        &format!("{}\n[limits]\nresources = 1\n", common::BOOTSTRAP),
    );
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    assert!(
        project
            .check()
            .fatal
            .as_deref()
            .is_some_and(|m| m.contains("limits.resources"))
    );
}

#[test]
fn report_paths_use_forward_slashes() {
    let project = Project::with_note();
    project.file(
        "content/nested/deeper/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\n+++\n",
    );
    let report = project.check();
    assert_eq!(codes(&report), [Code::ShapeViolation]);
    assert_eq!(
        report.diagnostics[0].path,
        "content/nested/deeper/note-b.md"
    );
}

#[test]
fn percent_encoded_links_never_panic_and_stay_confined() {
    let project = Project::with_note();
    project.file("content/a b.md", "+++\nschema = \"example/test/note@1\"\nid = \"note-space\"\ntitle = \"S\"\n+++\n\n# Sp\u{e4}ce\n");
    project.file("content/n\u{f6}te-\u{109}.md", "+++\nschema = \"example/test/note@1\"\nid = \"note-unicode\"\ntitle = \"U\"\n+++\n\n# \u{108}u\n");
    let links = [
        ("a%20b.md", None),
        ("n%C3%B6te-%C4%89.md#%C4%89u", None),
        ("%aĉ", Some("points at a missing file")),
        ("a%", Some("points at a missing file")),
        ("%zz", Some("points at a missing file")),
        ("%2e%2e/bearout.toml", None),
        ("..%2F..%2Fetc", Some("leaves the project")),
        ("a%2Fb.md", Some("points at a missing file")),
        ("a%3Ab.md", Some("`:` is not allowed")),
        ("a%00b.md", Some("control characters are not allowed")),
        ("%ff%fe.md", Some("does not decode to valid UTF-8")),
        ("a b.md#sp%C3%A4ce", None),
    ];
    for (target, expected) in links {
        project.file("content/note-a.md", &format!("+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n# A\n\n[l]({target})\n"));
        let report = project.check();
        assert!(report.fatal.is_none(), "{target}: {:?}", report.fatal);
        match expected {
            None => assert_clean(&report),
            Some(text) => {
                assert_line(
                    &report,
                    &format!("content/note-a.md:9:B011: link `{target}`"),
                );
                assert_line(&report, text);
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn discovery_rejects_unportable_names() {
    use std::os::unix::ffi::OsStrExt;
    let project = Project::with_note();
    let name = std::ffi::OsStr::from_bytes(b"bad\xffname.md");
    // A filesystem that refuses non-UTF-8 names (APFS) leaves nothing to
    // discover; the backslash case below still runs there.
    if fs::write(project.path().join("content").join(name), "x").is_ok() {
        let report = project.check();
        assert!(
            report
                .fatal
                .as_deref()
                .is_some_and(|m| m.contains("not valid UTF-8")),
            "{:?}",
            report.fatal
        );
    }

    let project = Project::with_note();
    fs::write(project.path().join("content").join("back\\slash.md"), "x").expect("write");
    let report = project.check();
    assert!(
        report
            .fatal
            .as_deref()
            .is_some_and(|m| m.contains("backslash")),
        "{:?}",
        report.fatal
    );
}

#[test]
fn logical_paths_reject_backslashes_everywhere() {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &common::BOOTSTRAP.replace("roots = [\"content\"]", "roots = [\"content\\\\sub\"]"),
    );
    assert!(
        project
            .check()
            .fatal
            .as_deref()
            .is_some_and(|m| m.contains("backslash"))
    );
    let project = Project::with_note();
    project.file(
        common::ENTRY,
        "schema(\"example/test/note@1\", shape = \"sub\\\\note.schema.toml\")\n",
    );
    let report = project.check();
    assert_line(&report, "backslash");
}
