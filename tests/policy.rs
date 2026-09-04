// SPDX-License-Identifier: Apache-2.0

//! The Starlark runtime: contained loading, limits, the ABI, and phase
//! gating.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use bearout::{Code, Command, Options};
use common::{Project, assert_clean, assert_line, assert_no_line, codes};

fn note_project_with_entry(entry: &str) -> Project {
    let project = Project::with_note();
    project.file(common::ENTRY, entry);
    project
}

#[test]
fn loads_resolve_only_beneath_the_rules_root() {
    let cases = [
        ("load(\"../escape.star\", \"x\")\n", "contains `..`"),
        ("load(\"/abs.star\", \"x\")\n", "is absolute"),
        ("load(\"./same.star\", \"x\")\n", "contains `.`"),
        ("load(\"lib//x.star\", \"x\")\n", "empty path segment"),
        ("load(\"lib/x.txt\", \"x\")\n", "must be a `.star` module"),
        ("load(\"missing.star\", \"x\")\n", "cannot read module"),
    ];
    for (load, expected) in cases {
        let project = note_project_with_entry(&format!(
            "{load}schema(\"example/test/note@1\", shape = \"note.schema.toml\")\n"
        ));
        let report = project.check();
        assert!(codes(&report).contains(&Code::ScriptLoad), "{load}");
        assert_line(&report, expected);
        assert!(report.fatal.is_none());
    }
}

#[test]
fn import_cycles_and_chains_are_reported() {
    let project = note_project_with_entry(
        "load(\"a.star\", \"A\")\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\n",
    );
    project.file("rules/a.star", "load(\"b.star\", \"B\")\nA = B\n");
    project.file("rules/b.star", "load(\"a.star\", \"A\")\nB = 1\n");
    let report = project.check();
    assert_line(
        &report,
        "rules/a.star:B012: import cycle: rules/a.star -> rules/b.star -> rules/a.star",
    );
    assert_line(&report, "(imported via bearout.star -> rules/a.star)");
}

#[cfg(unix)]
#[test]
fn modules_are_not_loaded_through_symlinks() {
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("evil.star"), "X = 1\n").expect("write");
    let project = note_project_with_entry(
        "load(\"lib/evil.star\", \"X\")\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\n",
    );
    std::os::unix::fs::symlink(outside.path(), project.path().join("rules/lib")).expect("symlink");
    let report = project.check();
    assert_line(&report, "`rules/lib` is a symbolic link");
    assert!(codes(&report).contains(&Code::ScriptLoad));
}

#[test]
fn registration_is_only_for_the_entry_module() {
    let project = note_project_with_entry("load(\"reg.star\", \"X\")\n");
    project.file("rules/reg.star", "schema(\"example/test/note@1\")\nX = 1\n");
    let report = project.check();
    assert_line(&report, "rules/reg.star:");
    assert!(
        codes(&report)
            .iter()
            .any(|code| matches!(code, Code::ScriptLoad | Code::ScriptFailure))
    );

    let project = note_project_with_entry(
        "schema(\"example/test/note@1\", shape = \"note.schema.toml\")\nschema(\"example/test/note@1\")\n",
    );
    assert_line(
        &project.check(),
        "schema `example/test/note@1` is registered twice",
    );

    let project = note_project_with_entry("schema(\"Bad\", shape = \"note.schema.toml\")\n");
    assert_line(&project.check(), "schema `Bad`");

    let project = note_project_with_entry("check(\"c\", 42)\n");
    assert_line(&project.check(), "check function must be a function");
}

#[test]
fn malformed_findings_and_outputs_are_rejected() {
    let cases: [(&str, &str); 8] = [
        (
            "return [error(\"m\", cod = \"x\")]",
            "Unexpected parameter named `cod`",
        ),
        ("return [error(\"\")]", "finding message must not be empty"),
        (
            "return [error(\"m\", line = 0)]",
            "finding line must be a positive integer",
        ),
        (
            "return [error(\"m\", line = \"3\")]",
            "Expected type `None | int` but got `str`",
        ),
        ("return [error(\"m\", code = \"Bad Code\")]", "finding code"),
        (
            "return [error(\"m\", resource = \"note-zzz\")]",
            "may only report its own resource",
        ),
        ("return [error(\"m\", line = 999)]", "is beyond the"),
        (
            "return [\"text\"]",
            "list item must be error() or warning(), found string",
        ),
    ];
    for (body, expected) in cases {
        let project = note_project_with_entry(&format!(
            "def v(r):\n    {body}\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n"
        ));
        let report = project.check();
        assert_line(&report, expected);
        assert!(report.fatal.is_none(), "{body}");
        assert!(
            codes(&report).iter().all(|code| matches!(
                code,
                Code::ScriptLoad | Code::ScriptFailure | Code::ScriptResult
            )),
            "{body}: {:?}",
            codes(&report)
        );
    }
}

#[test]
fn check_findings_must_name_a_known_resource() {
    let project = note_project_with_entry(
        "def c(p):\n    return [error(\"m\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ncheck(\"c\", c)\n",
    );
    let report = project.check();
    assert_line(
        &report,
        "bearout.star:B014: check `c` a check finding must name a `resource`",
    );

    let project = note_project_with_entry(
        "def c(p):\n    return [error(\"m\", resource = \"note-zzz\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ncheck(\"c\", c)\n",
    );
    assert_line(
        &project.check(),
        "B014: check `c` finding names unknown resource `note-zzz`",
    );

    let project = note_project_with_entry(
        "def c(p):\n    return [warning(\"w\", resource = \"note-a\", line = 2, code = \"soft\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ncheck(\"c\", c)\n",
    );
    let report = project.check();
    assert!(report.is_clean());
    assert_eq!(
        report.diagnostics[0].to_string(),
        "content/note-a.md:2:B016[soft]: check `c`: w"
    );
}

#[test]
fn prints_become_warnings_and_non_lists_fail() {
    let project = note_project_with_entry(
        "def v(r):\n    print(\"hello\")\n    return []\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    let report = project.check();
    assert!(report.is_clean());
    assert_line(
        &report,
        "bearout.star:B017: schema `example/test/note@1` validate printed: hello",
    );

    let project = note_project_with_entry(
        "def v(r):\n    return None\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    assert_line(
        &project.check(),
        "B014: schema `example/test/note@1` validate must return a list of findings, found NoneType",
    );
}

#[test]
fn views_are_immutable() {
    let project = note_project_with_entry(
        "def v(r):\n    r[\"fields\"][\"title\"] = \"changed\"\n    return []\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    let report = project.check();
    assert!(codes(&report).contains(&Code::ScriptFailure));
    assert_line(&report, "Immutable");
}

#[test]
fn tick_heap_and_stack_limits_are_enforced() {
    let base = common::BOOTSTRAP;
    let cases = [
        (
            "ticks = 5000",
            "def v(r):\n    n = 0\n    for i in range(1000000):\n        n += 1\n    return []\n",
            "5000 ticks has been exceeded",
        ),
        (
            "heap_bytes = 200000",
            "def v(r):\n    xs = []\n    for i in range(100000):\n        xs.append(\"x\" * 100)\n    return []\n",
            "heap",
        ),
        (
            "call_stack = 8",
            "def f(n):\n    return f(n + 1) if n < 1000 else n\ndef v(r):\n    f(0)\n    return []\n",
            "stack",
        ),
    ];
    for (limit, script, expected) in cases {
        let project = Project::with_note();
        project.file("bearout.toml", &format!("{base}\n[limits]\n{limit}\n"));
        project.file(common::ENTRY, &format!("{script}schema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n"));
        let report = project.check();
        assert!(
            codes(&report).contains(&Code::ScriptFailure),
            "{limit}: {:?}",
            common::lines(&report)
        );
        let rendered = common::lines(&report).join("\n").to_lowercase();
        assert!(
            rendered.contains(&expected.to_lowercase()),
            "{limit}: {rendered}"
        );
    }
}

#[test]
fn evaluation_can_be_cancelled() {
    let project = note_project_with_entry(
        "def v(r):\n    n = 0\n    for i in range(100000):\n        n += 1\n    return []\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    let cancel = Arc::new(AtomicBool::new(true));
    let report = project.run(
        Command::Check,
        &Options {
            cancel: Some(Arc::clone(&cancel)),
            ..Options::default()
        },
    );
    assert!(
        codes(&report)
            .iter()
            .any(|code| matches!(code, Code::ScriptFailure | Code::ScriptLoad))
    );
    assert_line(&report, "ancelled");
    cancel.store(false, Ordering::Relaxed);
    assert_clean(&project.run(
        Command::Check,
        &Options {
            cancel: Some(cancel),
            ..Options::default()
        },
    ));
}

#[test]
fn invalid_resources_never_reach_starlark() {
    let project = Project::fixture("invalid-shape");
    let report = project.check();
    assert_line(
        &report,
        "content/note-bad.md:4:B005: `title`: 3 is not of type \"string\"",
    );
    assert_line(&report, "content/note-unparsed.md:B002");
    assert_line(
        &report,
        "content/note-good.md:B015[ran]: schema `example/fixture/note@1` validate: validator ran on note-good",
    );
    assert_no_line(&report, "validator ran on note-bad");
    assert_no_line(&report, "validator ran on note-unparsed");
    // The graph has errors, so the project check does not run at all.
    assert_no_line(&report, "check ran");
    // note-good's relation to the invalid note-bad still resolves.
    assert_no_line(&report, "B009");
}

#[test]
fn checks_and_generators_run_only_on_error_free_projects() {
    let project = Project::with_note();
    project.file("bearout.toml", common::BOOTSTRAP_GEN);
    project.file(
        "templates/t.md.j2",
        "{% for line in bearout.header %}<!-- {{ line }} -->\n{% endfor %}\n# out\n",
    );
    project.file(
        common::ENTRY,
        "def c(p):\n    return [error(\"check ran\", resource = \"note-a\")]\ndef g(p):\n    return [output(\"t.md.j2\", \"generated/out.md\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ncheck(\"c\", c)\ngenerator(\"g\", g)\n",
    );
    let report = project.generate(bearout::Mode::Write);
    assert_line(&report, "check ran");
    assert!(!project.exists("generated/out.md"));
    assert!(report.outputs.is_empty());
}

#[test]
fn lint_findings_are_warnings() {
    let project = note_project_with_entry(
        "def v(r):\n    unused = 1\n    return []\nx = 1\nx = 2\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    let report = project.check();
    assert!(report.is_clean(), "{:?}", common::lines(&report));
    assert!(codes(&report).contains(&Code::ScriptLint));
}

#[test]
fn validators_run_despite_graph_errors_but_checks_do_not() {
    // A validator sees one structurally valid resource; an unresolved
    // relation elsewhere is the kernel's finding, not a reason to withhold
    // per-resource policy. Checks need a whole valid graph and are skipped.
    let project = Project::with_note();
    project.file(
        common::ENTRY,
        "def v(r):\n    return [warning(\"validator ran on \" + r[\"id\"])]\ndef c(p):\n    return [error(\"check ran\", resource = \"note-a\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\ncheck(\"c\", c)\n",
    );
    project.file("content/note-a.md", "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\nnext = \"note-missing\"\n+++\n");
    let report = project.check();
    assert_line(
        &report,
        "content/note-a.md:5:B009: `next` names `note-missing`",
    );
    assert_line(&report, "validator ran on note-a");
    assert_no_line(&report, "check ran");
}
