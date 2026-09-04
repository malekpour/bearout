// SPDX-License-Identifier: Apache-2.0

//! The contract fixture runner: declaration, mutations through the
//! overlay, structured expectation matching, outcomes, sources, limits,
//! isolation, and the boundary between assertion failures and fixture
//! infrastructure failures.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use bearout::{Code, Command, Mode, Options, Outcome, Side, Source, TestReport};
use common::{Project, assert_clean};

const FIXTURE_FILE: &str = "contract-tests/notes.test.toml";

/// A note policy with a validator (`bad-title` error, `warn-title`
/// warning, `line-two` error at line 2) and a comparison-aware check
/// (`deleted` on the baseline side, `moved` warning).
const ENTRY: &str = r#"def validate(r):
    findings = []
    title = r["fields"]["title"]
    if title == "BAD":
        findings.append(error("title is BAD", code = "bad-title"))
    if title.startswith("warn"):
        findings.append(warning("title warns", code = "warn-title"))
    if title == "LINE":
        findings.append(error("line two", code = "line-two", line = 2))
    return findings

def protect(p):
    comparison = p["comparison"]
    if comparison == None:
        return []
    findings = []
    for old in comparison["baseline"]["resources"]:
        new = p["by_id"].get(old["id"])
        if new == None:
            findings.append(error("record `%s` was deleted" % old["id"], resource = old["id"], side = "baseline", code = "deleted"))
        elif new["path"] != old["path"]:
            findings.append(warning("moved from `%s`" % old["path"], resource = old["id"], code = "moved"))
    findings.append(warning("baseline holds %d record(s); %d change(s)" % (len(comparison["baseline"]["resources"]), len(comparison["changes"])), resource = "note-b", code = "facts"))
    return findings

schema("example/test/note@1", shape = "note.schema.toml", validate = validate)
check("protect", protect)
"#;

fn bootstrap(extra: &str) -> String {
    format!(
        "{}\n[fixtures]\nfiles = [\"{FIXTURE_FILE}\"]\n{extra}",
        common::BOOTSTRAP
    )
}

fn note(id: &str, title: &str) -> String {
    format!(
        "+++\nschema = \"example/test/note@1\"\nid = \"{id}\"\ntitle = \"{title}\"\n+++\n\n# {title}\n"
    )
}

/// The project every test starts from: two notes, the policy above, and
/// the fixture grant naming one file that each test writes.
fn project() -> Project {
    let project = Project::with_note();
    project.file("bearout.toml", &bootstrap(""));
    project.file(common::ENTRY, ENTRY);
    project.file("content/note-a.md", &note("note-a", "A"));
    project.file("content/note-b.md", &note("note-b", "B"));
    project
}

fn write_case(name: &str, body: &str) -> String {
    format!("[[cases]]\nname = \"{name}\"\n{body}\n")
}

/// A case that writes one note with `title` and expects `expect`.
fn note_case(name: &str, title: &str, expect: &str) -> String {
    write_case(
        name,
        &format!(
            "expect = \"{expect}\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n",
            note("note-c", title)
        ),
    )
}

fn test(project: &Project) -> TestReport {
    bearout::test(project.path(), &Options::default())
}

fn test_from(project: &Project, source: Source) -> TestReport {
    bearout::test(
        project.path(),
        &Options {
            source,
            ..Options::default()
        },
    )
}

fn names(report: &TestReport) -> Vec<(&str, bool)> {
    report
        .cases
        .iter()
        .map(|case| (case.name.as_str(), case.passed))
        .collect()
}

#[track_caller]
fn assert_suite_ok(report: &TestReport) {
    assert!(
        report.ok,
        "expected every case to pass:\n{}",
        serde_json::to_string_pretty(report).unwrap()
    );
    assert_eq!(report.failed, 0);
    assert_eq!(report.passed, report.total);
}

#[track_caller]
fn assert_suite_fatal(report: &TestReport, expected: &str) {
    assert!(
        report
            .fatal
            .as_deref()
            .is_some_and(|message| message.contains(expected)),
        "expected a fatal suite containing {expected:?}, got {:?}",
        report.fatal
    );
    assert!(!report.ok);
    assert!(report.cases.is_empty(), "a fatal suite reports no case");
    assert_eq!((report.total, report.passed, report.failed), (0, 0, 0));
}

/// Every regular file beneath `root` with its bytes.
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, found: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).expect("dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let kind = entry.file_type().expect("type");
            if kind.is_dir() {
                walk(&path, root, found);
            } else if kind.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .expect("beneath root")
                    .to_string_lossy()
                    .replace('\\', "/");
                found.insert(relative, fs::read(&path).expect("read"));
            }
        }
    }
    let mut found = BTreeMap::new();
    walk(root, root, &mut found);
    found
}

#[test]
fn clean_and_expected_diagnostic_cases_pass() {
    let project = project();
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}",
            note_case("a clean addition stays clean", "C", "clean"),
            write_case(
                "a bad title is reported",
                &format!(
                    "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-c.md\"\nrule = \"bad-title\"\nseverity = \"error\"\n",
                    note("note-c", "BAD")
                )
            ),
            write_case("no mutation at all is the source itself", "expect = \"clean\"\n"),
        ),
    );
    let report = test(&project);
    assert_suite_ok(&report);
    assert_eq!(report.total, 3);
    assert_eq!(
        names(&report),
        [
            ("a clean addition stays clean", true),
            ("a bad title is reported", true),
            ("no mutation at all is the source itself", true),
        ]
    );
    let case = &report.cases[1];
    assert_eq!(case.expected, Outcome::Diagnostics);
    assert_eq!(case.actual, Outcome::Diagnostics);
    assert!(case.missing.is_empty() && case.unexpected.is_empty());
    assert_eq!(case.file, FIXTURE_FILE);
    assert!(
        report.source.is_none(),
        "the working directory has no identity"
    );
}

#[test]
fn missing_and_unexpected_diagnostics_fail_a_case() {
    let project = project();
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}{}",
            // Expects a diagnostic; the candidate is clean.
            write_case(
                "expected but absent",
                &format!(
                    "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B015\"\nrule = \"bad-title\"\n",
                    note("note-c", "C")
                )
            ),
            // Expects clean; the candidate reports.
            note_case("clean but reported", "BAD", "clean"),
            // Two reported, one expected, exact matching.
            write_case(
                "one of two expected",
                &format!(
                    "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.mutations]]\nwrite = \"content/note-d.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-c.md\"\n",
                    note("note-c", "BAD"),
                    note("note-d", "BAD")
                )
            ),
            // One reported, two expected.
            write_case(
                "two expected of one",
                &format!(
                    "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-c.md\"\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-d.md\"\n",
                    note("note-c", "BAD")
                )
            ),
        ),
    );
    let report = test(&project);
    assert!(!report.ok);
    assert!(report.fatal.is_none());
    assert_eq!((report.total, report.passed, report.failed), (4, 0, 4));

    let absent = &report.cases[0];
    assert_eq!(
        (absent.expected, absent.actual),
        (Outcome::Diagnostics, Outcome::Clean)
    );
    assert_eq!(absent.missing.len(), 1);
    assert_eq!(absent.missing[0].to_string(), "B015 rule=bad-title");
    assert!(absent.unexpected.is_empty());

    let reported = &report.cases[1];
    assert_eq!(
        (reported.expected, reported.actual),
        (Outcome::Clean, Outcome::Diagnostics)
    );
    assert!(reported.missing.is_empty());
    assert_eq!(reported.unexpected.len(), 1);
    assert_eq!(
        reported.unexpected[0].to_string(),
        "content/note-c.md:B015[bad-title]: schema `example/test/note@1` validate: title is BAD"
    );

    let one_of_two = &report.cases[2];
    assert!(one_of_two.missing.is_empty());
    assert_eq!(one_of_two.unexpected.len(), 1);
    assert_eq!(one_of_two.unexpected[0].path(), Some("content/note-d.md"));

    let two_of_one = &report.cases[3];
    assert_eq!(two_of_one.missing.len(), 1);
    assert_eq!(
        two_of_one.missing[0].path.as_deref(),
        Some("content/note-d.md")
    );
    assert!(two_of_one.unexpected.is_empty());
}

#[test]
fn contains_matching_permits_unrelated_diagnostics_and_exact_does_not() {
    let project = project();
    let body = |matching: &str| {
        format!(
            "expect = \"diagnostics\"\nmatch = \"{matching}\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.mutations]]\nwrite = \"content/note-d.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-c.md\"\n",
            note("note-c", "BAD"),
            note("note-d", "warn me")
        )
    };
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}",
            write_case("contains", &body("contains")),
            write_case("exact", &body("exact"))
        ),
    );
    let report = test(&project);
    assert_eq!(names(&report), [("contains", true), ("exact", false)]);
    assert!(
        report.cases[0].unexpected.is_empty(),
        "allowed extras are not listed"
    );
    assert_eq!(report.cases[1].unexpected.len(), 1);
    assert_eq!(report.cases[1].unexpected[0].code(), Code::PolicyWarning);
    // A warning alone is a diagnostics outcome, never clean.
    project.file(FIXTURE_FILE, &note_case("warned", "warn me", "clean"));
    let report = test(&project);
    assert_eq!(report.cases[0].actual, Outcome::Diagnostics);
    assert!(!report.cases[0].passed);
}

#[test]
fn duplicate_diagnostics_are_matched_as_a_multiset() {
    let project = project();
    let case = |name: &str, expectations: usize| {
        let mut body = format!(
            "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.mutations]]\nwrite = \"content/note-d.md\"\ncontent = '''{}'''\n",
            note("note-c", "BAD"),
            note("note-d", "BAD")
        );
        for _ in 0..expectations {
            body.push_str("[[cases.diagnostics]]\ncode = \"B015\"\nrule = \"bad-title\"\n");
        }
        write_case(name, &body)
    };
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}",
            case("once", 1),
            case("twice", 2),
            case("thrice", 3)
        ),
    );
    let report = test(&project);
    assert_eq!(
        names(&report),
        [("once", false), ("twice", true), ("thrice", false)]
    );
    assert_eq!(
        report.cases[0].unexpected.len(),
        1,
        "the second diagnostic is unexpected"
    );
    assert_eq!(
        report.cases[2].missing.len(),
        1,
        "the third expectation is missing"
    );
}

#[test]
fn every_structured_field_is_matched() {
    let project = project();
    let line_case = |name: &str, fields: &str| {
        write_case(
            name,
            &format!(
                "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B015\"\n{fields}\n",
                note("note-c", "LINE")
            ),
        )
    };
    let baseline_case = |name: &str, fields: &str| {
        write_case(
            name,
            &format!(
                "expect = \"diagnostics\"\nmatch = \"contains\"\nbaseline = true\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.diagnostics]]\ncode = \"B015\"\n{fields}\n"
            ),
        )
    };
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}{}{}{}{}{}{}{}{}",
            line_case("line", "line = 2"),
            line_case("wrong line", "line = 3"),
            line_case("rule", "rule = \"line-two\""),
            line_case("wrong rule", "rule = \"bad-title\""),
            line_case("severity", "severity = \"error\""),
            line_case(
                "message",
                "message = \"schema `example/test/note@1` validate: line two\""
            ),
            line_case("wrong message", "message = \"line two\""),
            line_case("wrong path", "path = \"content/note-d.md\""),
            baseline_case(
                "side",
                "side = \"baseline\"\npath = \"content/note-a.md\"\nrule = \"deleted\""
            ),
            baseline_case(
                "wrong side",
                "side = \"candidate\"\npath = \"content/note-a.md\""
            ),
            baseline_case("unspecified side", "path = \"content/note-a.md\""),
        ),
    );
    let report = test(&project);
    assert_eq!(
        names(&report),
        [
            ("line", true),
            ("wrong line", false),
            ("rule", true),
            ("wrong rule", false),
            ("severity", true),
            ("message", true),
            ("wrong message", false),
            ("wrong path", false),
            ("side", true),
            ("wrong side", false),
            ("unspecified side", true),
        ]
    );
    assert_eq!(report.cases[0].file, FIXTURE_FILE);
    let wrong_line = &report.cases[1];
    assert_eq!(wrong_line.missing[0].line, Some(3));
    assert_eq!(wrong_line.unexpected[0].line(), Some(2));
    // A severity that contradicts its code is refused when parsing.
    project.file(
        FIXTURE_FILE,
        &line_case("contradiction", "severity = \"warning\""),
    );
    assert_suite_fatal(
        &test(&project),
        "`severity` contradicts `code`: B015 is always an error",
    );
    // A warning code with a warning severity is fine.
    project.file(
        FIXTURE_FILE,
        &write_case(
            "warning",
            &format!(
                "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B016\"\nseverity = \"warning\"\nrule = \"warn-title\"\n",
                note("note-c", "warn")
            ),
        ),
    );
    assert_suite_ok(&test(&project));
}

#[test]
fn expected_and_unexpected_fatal_outcomes() {
    let project = project();
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}{}{}",
            write_case(
                "expected fatal",
                "expect = \"fatal\"\n[[cases.mutations]]\ndelete = \"bearout.star\"\n"
            ),
            write_case(
                "expected fatal with text",
                "expect = \"fatal\"\nfatal = \"entry module `bearout.star` is not a file\"\n[[cases.mutations]]\ndelete = \"bearout.star\"\n"
            ),
            write_case(
                "expected fatal with wrong text",
                "expect = \"fatal\"\nfatal = \"something else\"\n[[cases.mutations]]\ndelete = \"bearout.star\"\n"
            ),
            write_case(
                "unexpected fatal",
                "expect = \"clean\"\n[[cases.mutations]]\ndelete = \"bearout.star\"\n"
            ),
            note_case("fatal expected but clean", "C", "fatal"),
        ),
    );
    let report = test(&project);
    assert_eq!(
        names(&report),
        [
            ("expected fatal", true),
            ("expected fatal with text", true),
            ("expected fatal with wrong text", false),
            ("unexpected fatal", false),
            ("fatal expected but clean", false),
        ]
    );
    let wrong = &report.cases[2];
    assert_eq!(wrong.expected_fatal.as_deref(), Some("something else"));
    assert!(wrong.fatal.as_deref().unwrap().contains("entry module"));
    let unexpected = &report.cases[3];
    assert_eq!(
        (unexpected.expected, unexpected.actual),
        (Outcome::Clean, Outcome::Fatal)
    );
    assert!(unexpected.fatal.is_some());
    assert_eq!(report.cases[4].actual, Outcome::Clean);
    assert!(report.cases[4].fatal.is_none());
    // A mutated bootstrap that no longer parses is an observable fatal
    // outcome of the candidate, not a suite failure.
    project.file(
        FIXTURE_FILE,
        &write_case(
            "broken bootstrap",
            "expect = \"fatal\"\nfatal = \"unsupported manifest version 2\"\n[[cases.mutations]]\nwrite = \"bearout.toml\"\ncontent = \"version = 2\\n\"\n",
        ),
    );
    assert_suite_ok(&test(&project));
}

#[test]
fn malformed_fixtures_are_fatal_for_the_whole_suite() {
    let project = project();
    let good = note_case("good", "C", "clean");
    for (body, expected) in [
        ("[[cases]\nname = 1\n", "not valid TOML"),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases]]\nname = \"y\"\nexpect = \"clean\"\n[[cases]]\nname = \"x\"\nexpect = \"clean\"\n",
            "case `x` in `contract-tests/notes.test.toml` repeats the name",
        ),
        ("", "`[[cases]]` is required"),
        ("cases = []\n", "must declare at least one case"),
        ("cases = 3\n", "`cases` must be an array of tables"),
        (
            "[[cases]]\nexpect = \"clean\"\n",
            "case 1: `name` is required",
        ),
        (
            "[[cases]]\nname = \"\"\nexpect = \"clean\"\n",
            "`name` must be non-empty",
        ),
        (
            "[[cases]]\nname = \" x\"\nexpect = \"clean\"\n",
            "`name` must be non-empty",
        ),
        (
            "[[cases]]\nname = \"x\\ty\"\nexpect = \"clean\"\n",
            "control characters",
        ),
        ("[[cases]]\nname = \"x\"\n", "`expect` is required"),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"ok\"\n",
            "`expect` must be `clean`, `diagnostics`, or `fatal`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\nmatch = \"exact\"\n",
            "`match` applies only",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\nmatch = \"any\"\n",
            "`match` must be `exact` or `contains`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n",
            "needs at least one `[[cases.diagnostics]]`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\ndiagnostics = []\n",
            "needs at least one `[[cases.diagnostics]]`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.diagnostics]]\ncode = \"B015\"\n",
            "`diagnostics` applies only",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\nfatal = \"y\"\n",
            "`fatal` applies only",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"fatal\"\nfatal = \"\"\n",
            "`fatal` must be a non-empty",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\nbaseline = \"yes\"\n",
            "`baseline` must be a boolean",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\nextra = 1\n",
            "unknown key `extra`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\n",
            "exactly one of `write`, `delete`, or `move`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nwrite = \"a\"\ndelete = \"b\"\ncontent = \"\"\n",
            "mutually exclusive",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nwrite = \"a\"\n",
            "needs exactly one of `content` or `payload`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nwrite = \"a\"\ncontent = \"\"\npayload = \"p\"\n",
            "`content` and `payload` are mutually exclusive",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nwrite = \"a\"\ncontent = \"\"\nto = \"b\"\n",
            "`to` does not apply to `write`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\ndelete = \"a\"\ncontent = \"\"\n",
            "`content` does not apply to `delete`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nmove = \"a\"\n",
            "`move` needs `to`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nmove = \"a\"\nto = \"\"\n",
            "`to` must not be the project root",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nwrite = \"/etc/passwd\"\ncontent = \"\"\n",
            "`write`: `/etc/passwd` is absolute",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nwrite = \"../x\"\ncontent = \"\"\n",
            "must be normalized",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\nwrite = \"a\"\ncontent = \"\"\nshell = \"rm\"\n",
            "unknown key `shell`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\nmutations = [1]\n",
            "`mutations` must be an array of tables",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B999\"\n",
            "`code` `B999` is not a Bearout diagnostic code",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[[cases.diagnostics]]\npath = \"a\"\n",
            "diagnostic 1: `code` is required",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B015\"\nline = 0\n",
            "`line` must be a positive integer",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B015\"\nside = \"left\"\n",
            "`side` must be `candidate` or `baseline`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B015\"\nseverity = \"fatal\"\n",
            "`severity` must be `error` or `warning`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B015\"\nrule = \"\"\n",
            "`rule` must be non-empty",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B015\"\ntext = \"y\"\n",
            "unknown key `text`",
        ),
        (
            "[[cases]]\nname = \"same\"\nexpect = \"clean\"\n[[cases]]\nname = \"same\"\nexpect = \"clean\"\n",
            "case `same` in `contract-tests/notes.test.toml` repeats the name",
        ),
    ] {
        project.file(FIXTURE_FILE, body);
        let report = test(&project);
        assert_suite_fatal(&report, expected);
        assert!(
            report
                .fatal
                .as_deref()
                .unwrap()
                .contains("fixture file `contract-tests/notes.test.toml`")
                || expected.starts_with("case `"),
            "{:?}",
            report.fatal
        );
    }
    project.file(FIXTURE_FILE, &good);
    assert_suite_ok(&test(&project));
    // Inline arrays of inline tables are the same vocabulary.
    project.file(
        FIXTURE_FILE,
        "cases = [{ name = \"inline\", expect = \"clean\", mutations = [{ delete = \"content/note-b.md\" }] }]\n",
    );
    assert_suite_ok(&test(&project));
    // A missing, linked, non-UTF-8, or absent fixture file.
    project.remove(FIXTURE_FILE);
    assert_suite_fatal(
        &test(&project),
        "fixture file `contract-tests/notes.test.toml` is not a file",
    );
    project.bytes(FIXTURE_FILE, b"\xff\xfe");
    assert_suite_fatal(&test(&project), "is not valid UTF-8");
    #[cfg(unix)]
    {
        project.remove(FIXTURE_FILE);
        project.file("contract-tests/real.toml", &good);
        std::os::unix::fs::symlink("real.toml", project.path().join(FIXTURE_FILE)).unwrap();
        assert_suite_fatal(
            &test(&project),
            "reached through the symbolic link `contract-tests/notes.test.toml`",
        );
    }
    // No grant at all.
    project.file("bearout.toml", common::BOOTSTRAP);
    assert_suite_fatal(
        &test(&project),
        "bearout.toml declares no `[fixtures]`; there is nothing to test",
    );
    // A comparison baseline is not an option of the runner.
    project.file("bearout.toml", &bootstrap(""));
    project.file(FIXTURE_FILE, &good);
    let report = bearout::test(
        project.path(),
        &Options {
            baseline: Some("HEAD".to_owned()),
            ..Options::default()
        },
    );
    assert_suite_fatal(&report, "takes no comparison baseline");
    // An unopenable source.
    let report = test_from(&project, Source::Index);
    assert_suite_fatal(&report, "cannot read the Git index");
}

#[test]
fn write_delete_and_move_are_observed_by_the_policy() {
    let project = project();
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}{}",
            // Written content is what the validator sees.
            write_case(
                "write",
                &format!(
                    "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-a.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-a.md\"\nrule = \"bad-title\"\n",
                    note("note-a", "BAD")
                )
            ),
            // A deleted resource is gone: a reference to it dangles.
            write_case(
                "delete",
                &format!(
                    "expect = \"diagnostics\"\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B009\"\npath = \"content/note-c.md\"\n",
                    note("note-c", "C").replace("+++\n\n", "next = \"note-a\"\n+++\n\n")
                )
            ),
            // A moved resource keeps its id and its content at the new path.
            write_case(
                "move",
                &format!(
                    "expect = \"diagnostics\"\n[[cases.mutations]]\nmove = \"content/note-a.md\"\nto = \"content/archive/note-a.md\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-c.md\"\nrule = \"bad-title\"\n",
                    note("note-c", "BAD").replace("+++\n\n", "next = \"note-a\"\n+++\n\n")
                )
            ),
            // A move out of the resource root removes the resource.
            write_case(
                "move out",
                &format!(
                    "expect = \"diagnostics\"\n[[cases.mutations]]\nmove = \"content/note-a.md\"\nto = \"attic/note-a.md\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n[[cases.diagnostics]]\ncode = \"B009\"\npath = \"content/note-c.md\"\n",
                    note("note-c", "C").replace("+++\n\n", "next = \"note-a\"\n+++\n\n")
                )
            ),
        ),
    );
    assert_suite_ok(&test(&project));
}

#[test]
fn conflicting_and_escaping_mutations_fail_closed() {
    let project = project();
    for (mutations, expected) in [
        (
            "[[cases.mutations]]\ndelete = \"content/missing.md\"\n",
            "mutation 1: `content/missing.md` is not a regular file",
        ),
        (
            "[[cases.mutations]]\ndelete = \"content\"\n",
            "not a regular file",
        ),
        (
            "[[cases.mutations]]\nwrite = \"content\"\ncontent = \"\"\n",
            "is not a regular file; only regular files are written",
        ),
        (
            "[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.mutations]]\nwrite = \"content/note-a.md\"\ncontent = \"\"\n",
            "mutation 2: `content/note-a.md` is touched by an earlier mutation",
        ),
        (
            "[[cases.mutations]]\nmove = \"content/note-a.md\"\nto = \"content/note-b.md\"\n",
            "`content/note-b.md` already exists; a move never replaces anything",
        ),
        (
            "[[cases.mutations]]\nmove = \"content/note-a.md\"\nto = \"content/note-a.md\"\n",
            "onto itself",
        ),
        (
            "[[cases.mutations]]\nwrite = \"content/note-a.md/x.md\"\ncontent = \"\"\n",
            "lies beneath `content/note-a.md`, which is a file",
        ),
        (
            "[[cases.mutations]]\nwrite = \"new.md\"\ncontent = \"\"\n[[cases.mutations]]\nmove = \"content/note-a.md\"\nto = \"new.md\"\n",
            "mutation 2: `new.md` is touched",
        ),
    ] {
        project.file(
            FIXTURE_FILE,
            &format!(
                "{}{}",
                note_case("fine", "C", "clean"),
                write_case("bad", &format!("expect = \"clean\"\n{mutations}"))
            ),
        );
        let report = test(&project);
        assert_suite_fatal(&report, expected);
        assert!(report.fatal.as_deref().unwrap().starts_with("case `bad`: "));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("note-a.md", project.path().join("content/link.md")).unwrap();
        std::os::unix::fs::symlink("content", project.path().join("linked")).unwrap();
        for (mutations, expected) in [
            (
                "[[cases.mutations]]\ndelete = \"content/link.md\"\n",
                "`content/link.md` is or lies beneath the symbolic link `content/link.md`",
            ),
            (
                "[[cases.mutations]]\nwrite = \"linked/x.md\"\ncontent = \"\"\n",
                "lies beneath the symbolic link `linked`",
            ),
            (
                "[[cases.mutations]]\nmove = \"content/note-a.md\"\nto = \"linked/z.md\"\n",
                "symbolic link `linked`",
            ),
        ] {
            project.file(
                FIXTURE_FILE,
                &write_case("bad", &format!("expect = \"clean\"\n{mutations}")),
            );
            assert_suite_fatal(&test(&project), expected);
        }
    }
}

#[test]
fn payloads_come_from_the_selected_tree_and_never_through_links() {
    let project = project();
    project.file("contract-tests/payloads/note-c.md", &note("note-c", "BAD"));
    let case = |payload: &str| {
        write_case(
            "payload",
            &format!(
                "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\npayload = \"{payload}\"\n[[cases.diagnostics]]\ncode = \"B015\"\nrule = \"bad-title\"\n"
            ),
        )
    };
    project.file(FIXTURE_FILE, &case("contract-tests/payloads/note-c.md"));
    assert_suite_ok(&test(&project));
    // Payload paths are project-relative, never relative to the fixture.
    project.file(FIXTURE_FILE, &case("payloads/note-c.md"));
    assert_suite_fatal(
        &test(&project),
        "payload of case `payload` mutation 1 `payloads/note-c.md` is not a file inside the selected tree",
    );
    project.file(FIXTURE_FILE, &case("contract-tests/payloads"));
    assert_suite_fatal(&test(&project), "is not a file inside the selected tree");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            "note-c.md",
            project.path().join("contract-tests/payloads/link.md"),
        )
        .unwrap();
        project.file(FIXTURE_FILE, &case("contract-tests/payloads/link.md"));
        assert_suite_fatal(
            &test(&project),
            "reached through the symbolic link `contract-tests/payloads/link.md`",
        );
    }
    // Payload bytes are exactly the file's bytes.
    project.bytes("contract-tests/payloads/raw.bin", b"\x00\x01raw");
    project.file(
        FIXTURE_FILE,
        &write_case(
            "raw",
            "expect = \"clean\"\n[[cases.mutations]]\nwrite = \"attic/raw.bin\"\npayload = \"contract-tests/payloads/raw.bin\"\n",
        ),
    );
    assert_suite_ok(&test(&project));
}

#[test]
fn no_mutation_touches_the_working_directory_and_cases_are_isolated() {
    let project = project();
    project.file("contract-tests/payloads/note-c.md", &note("note-c", "C"));
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}{}",
            write_case(
                "first deletes",
                "expect = \"clean\"\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.mutations]]\nmove = \"content/note-b.md\"\nto = \"content/moved/note-b.md\"\n"
            ),
            // If the deletion leaked, `next = "note-a"` would be B009.
            write_case(
                "second still sees note-a",
                &format!(
                    "expect = \"clean\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n",
                    note("note-c", "C").replace("+++\n\n", "next = \"note-a\"\n+++\n\n")
                )
            ),
            // If the write leaked, note-c would already exist and the
            // policy would see two of it (B008), and the move would show
            // as a move.
            write_case(
                "third sees the original layout",
                "expect = \"diagnostics\"\nbaseline = true\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\npayload = \"contract-tests/payloads/note-c.md\"\n[[cases.diagnostics]]\ncode = \"B016\"\nrule = \"facts\"\nmessage = \"check `protect`: baseline holds 2 record(s); 1 change(s)\"\n"
            ),
            write_case(
                "fourth rewrites the bootstrap",
                "expect = \"fatal\"\n[[cases.mutations]]\nwrite = \"bearout.toml\"\ncontent = \"broken\"\n"
            ),
        ),
    );
    let before = snapshot(project.path());
    let report = test(&project);
    assert_suite_ok(&report);
    assert_eq!(report.total, 4);
    assert_eq!(
        snapshot(project.path()),
        before,
        "the working directory is untouched"
    );
    assert!(!project.path().join("content/moved").exists());
    // The same holds when the suite fails or is fatal.
    project.file(FIXTURE_FILE, &note_case("wrong", "BAD", "clean"));
    let before = snapshot(project.path());
    assert!(!test(&project).ok);
    assert_eq!(snapshot(project.path()), before);
    project.file(FIXTURE_FILE, "[[cases]\n");
    let before = snapshot(project.path());
    assert!(test(&project).fatal.is_some());
    assert_eq!(snapshot(project.path()), before);
}

#[test]
fn a_baseline_case_compares_against_the_unmodified_source() {
    let project = project();
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}{}",
            write_case(
                "deleting a record is reported on the baseline side",
                "expect = \"diagnostics\"\nbaseline = true\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.diagnostics]]\ncode = \"B015\"\nside = \"baseline\"\npath = \"content/note-a.md\"\nrule = \"deleted\"\nmessage = \"check `protect`: record `note-a` was deleted\"\n[[cases.diagnostics]]\ncode = \"B016\"\nrule = \"facts\"\nmessage = \"check `protect`: baseline holds 2 record(s); 1 change(s)\"\n"
            ),
            write_case(
                "moving a record is a warning",
                "expect = \"diagnostics\"\nbaseline = true\n[[cases.mutations]]\nmove = \"content/note-b.md\"\nto = \"content/moved/note-b.md\"\n[[cases.diagnostics]]\ncode = \"B016\"\npath = \"content/moved/note-b.md\"\nrule = \"moved\"\nmessage = \"check `protect`: moved from `content/note-b.md`\"\n[[cases.diagnostics]]\ncode = \"B016\"\nrule = \"facts\"\nmessage = \"check `protect`: baseline holds 2 record(s); 2 change(s)\"\n"
            ),
            write_case(
                "no mutation means no change",
                "expect = \"diagnostics\"\nbaseline = true\n[[cases.diagnostics]]\ncode = \"B016\"\nrule = \"facts\"\nmessage = \"check `protect`: baseline holds 2 record(s); 0 change(s)\"\n"
            ),
            // Without a baseline the comparison is None and the check is
            // inactive.
            write_case(
                "without a baseline the deletion is free",
                "expect = \"clean\"\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n"
            ),
        ),
    );
    let report = test(&project);
    assert_suite_ok(&report);
    assert_eq!(report.total, 4);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["cases"][0]["passed"], true);
    assert_eq!(json["cases"][0]["expected"], "diagnostics");
    // A wrong side does not match, and the diagnostic keeps its side.
    project.file(
        FIXTURE_FILE,
        &write_case(
            "wrong side",
            "expect = \"diagnostics\"\nmatch = \"contains\"\nbaseline = true\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.diagnostics]]\ncode = \"B015\"\nside = \"candidate\"\nrule = \"deleted\"\n",
        ),
    );
    let report = test(&project);
    assert!(!report.ok);
    assert_eq!(report.cases[0].missing[0].side, Some(Side::Candidate));
    assert!(
        report.cases[0].unexpected.is_empty(),
        "contains lists no extras"
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["cases"][0]["missing"][0]["side"], "candidate");
}

#[test]
fn the_suite_reads_from_the_working_directory_index_or_revision() {
    let project = project();
    project.file("contract-tests/payloads/note-c.md", &note("note-c", "BAD"));
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}",
            write_case(
                "payload",
                "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\npayload = \"contract-tests/payloads/note-c.md\"\n[[cases.diagnostics]]\ncode = \"B015\"\nrule = \"bad-title\"\n"
            ),
            write_case(
                "baseline",
                "expect = \"diagnostics\"\nbaseline = true\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.diagnostics]]\ncode = \"B015\"\nside = \"baseline\"\nrule = \"deleted\"\n[[cases.diagnostics]]\ncode = \"B016\"\nrule = \"facts\"\n"
            ),
        ),
    );
    project.git_init();
    let commit = project.commit_all("suite");
    for source in [
        Source::WorkingDirectory,
        Source::Index,
        Source::Revision("HEAD".to_owned()),
    ] {
        let report = test_from(&project, source.clone());
        assert_suite_ok(&report);
        assert_eq!(report.total, 2);
        match &source {
            Source::WorkingDirectory => assert!(report.source.is_none()),
            Source::Index => assert_eq!(report.source.as_ref().unwrap().kind, "index"),
            Source::Revision(_) => {
                let info = report.source.as_ref().unwrap();
                assert_eq!(info.kind, "revision");
                assert_eq!(info.revision.as_deref(), Some("HEAD"));
                assert_eq!(
                    info.tree.as_deref(),
                    Some(
                        project
                            .git(&["rev-parse", &format!("{commit}^{{tree}}")])
                            .as_str()
                    )
                );
            }
        }
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json.get("source").is_some(),
            source != Source::WorkingDirectory
        );
    }

    // An unstaged correction cannot hide a broken staged fixture: stage a
    // case that is wrong, then fix the file on disk without staging.
    project.file(FIXTURE_FILE, &note_case("wrong", "BAD", "clean"));
    project.git(&["add", FIXTURE_FILE]);
    project.file(
        FIXTURE_FILE,
        &note_case("right", "BAD", "diagnostics").replace(
            "expect = \"diagnostics\"\n",
            "expect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B015\"\n",
        ),
    );
    assert_suite_ok(&test(&project));
    let staged = test_from(&project, Source::Index);
    assert!(!staged.ok);
    assert_eq!(names(&staged), [("wrong", false)]);
    assert_suite_ok(&test_from(&project, Source::Revision("HEAD".to_owned())));

    // An untracked payload cannot satisfy an index fixture.
    project.git(&["checkout", "-q", "--", FIXTURE_FILE]);
    project.git(&["add", FIXTURE_FILE]);
    project.file(
        FIXTURE_FILE,
        &write_case(
            "untracked payload",
            "expect = \"clean\"\n[[cases.mutations]]\nwrite = \"content/note-d.md\"\npayload = \"contract-tests/payloads/note-d.md\"\n",
        ),
    );
    project.file("contract-tests/payloads/note-d.md", &note("note-d", "D"));
    project.git(&["add", FIXTURE_FILE]);
    assert_suite_ok(&test(&project));
    assert_suite_fatal(
        &test_from(&project, Source::Index),
        "payload of case `untracked payload` mutation 1 `contract-tests/payloads/note-d.md` is not a file inside the selected tree",
    );
    // Staging the payload makes the index suite whole.
    project.git(&["add", "contract-tests/payloads/note-d.md"]);
    assert_suite_ok(&test_from(&project, Source::Index));
}

#[test]
fn projects_below_the_repository_root_and_linked_worktrees_work() {
    let project = Project::at("nested/project");
    project.file("bearout.toml", &bootstrap(""));
    project.file(common::ENTRY, ENTRY);
    project.file("rules/note.schema.toml", common::NOTE_SHAPE);
    project.file("content/note-a.md", &note("note-a", "A"));
    project.file("content/note-b.md", &note("note-b", "B"));
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}",
            note_case("bad", "BAD", "diagnostics").replace(
                "expect = \"diagnostics\"\n",
                "expect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-c.md\"\n",
            ),
            write_case(
                "baseline",
                "expect = \"diagnostics\"\nbaseline = true\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.diagnostics]]\ncode = \"B015\"\nside = \"baseline\"\npath = \"content/note-a.md\"\n[[cases.diagnostics]]\ncode = \"B016\"\nrule = \"facts\"\n"
            ),
        ),
    );
    project.git_init();
    project.commit_all("nested");
    for source in [
        Source::WorkingDirectory,
        Source::Index,
        Source::Revision("HEAD".to_owned()),
    ] {
        assert_suite_ok(&test_from(&project, source));
    }
    // A linked worktree has its own index and HEAD.
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
    let nested = linked_path.join("nested/project");
    let run = |source| {
        bearout::test(
            &nested,
            &Options {
                source,
                ..Options::default()
            },
        )
    };
    assert_suite_ok(&run(Source::WorkingDirectory));
    assert_suite_ok(&run(Source::Index));
    assert_suite_ok(&run(Source::Revision("feature".to_owned())));
    // Break the fixture in the linked worktree's index only.
    fs::write(
        nested.join(FIXTURE_FILE),
        note_case("wrong", "BAD", "clean"),
    )
    .expect("write");
    common::git_run(&nested, &["add", FIXTURE_FILE]);
    assert!(!run(Source::Index).ok);
    assert_suite_ok(&test_from(&project, Source::Index));
    assert_suite_ok(&run(Source::Revision("HEAD".to_owned())));
}

#[test]
fn fixture_limits_bound_the_suite() {
    let project = project();
    let two_cases = format!(
        "{}{}",
        note_case("one", "C", "clean"),
        note_case("two", "D", "clean")
    );
    project.file(FIXTURE_FILE, &two_cases);
    project.file("bearout.toml", &bootstrap("[limits]\nfixture_cases = 1\n"));
    assert_suite_fatal(
        &test(&project),
        "2 fixture cases exceed `limits.fixture_cases` = 1",
    );
    project.file(
        "bearout.toml",
        &bootstrap("[limits]\nfixture_mutations = 1\n"),
    );
    assert_suite_fatal(
        &test(&project),
        "2 fixture mutations exceed `limits.fixture_mutations` = 1",
    );
    project.file(
        "bearout.toml",
        &bootstrap("[limits]\nfixture_cases = 2\nfixture_mutations = 2\n"),
    );
    assert_suite_ok(&test(&project));

    // The byte budget covers fixture files and payloads together, and a
    // payload is read within what remains.
    project.file("contract-tests/payloads/big.md", &"x".repeat(500));
    let with_payload = format!(
        "{two_cases}{}",
        write_case(
            "payload",
            "expect = \"clean\"\n[[cases.mutations]]\nwrite = \"attic/big.md\"\npayload = \"contract-tests/payloads/big.md\"\n"
        )
    );
    project.file(FIXTURE_FILE, &with_payload);
    let fixture_len = with_payload.len() as u64;
    project.file(
        "bearout.toml",
        &bootstrap(&format!("[limits]\nfixture_bytes = {}\n", fixture_len - 1)),
    );
    assert_suite_fatal(
        &test(&project),
        "fixture inputs exceed `limits.fixture_bytes`",
    );
    let report = test(&project);
    assert!(
        report
            .fatal
            .as_deref()
            .unwrap()
            .contains("while reading fixture file `contract-tests/notes.test.toml`")
    );
    project.file(
        "bearout.toml",
        &bootstrap(&format!(
            "[limits]\nfixture_bytes = {}\n",
            fixture_len + 499
        )),
    );
    let report = test(&project);
    assert!(report.fatal.as_deref().unwrap().contains(
        "while reading payload of case `payload` mutation 1 `contract-tests/payloads/big.md`"
    ));
    project.file(
        "bearout.toml",
        &bootstrap(&format!(
            "[limits]\nfixture_bytes = {}\n",
            fixture_len + 500
        )),
    );
    assert_suite_ok(&test(&project));
}

#[test]
fn reports_are_identical_across_runs_and_ordered() {
    let project = project();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[fixtures]\nfiles = [\"contract-tests/z.test.toml\", \"contract-tests/a.test.toml\"]\n",
            common::BOOTSTRAP
        ),
    );
    project.file(
        "contract-tests/z.test.toml",
        &format!(
            "{}{}",
            note_case("z first", "BAD", "clean"),
            note_case("z second", "C", "clean")
        ),
    );
    project.file(
        "contract-tests/a.test.toml",
        &format!(
            "{}{}",
            write_case(
                "a first",
                &format!(
                    "expect = \"clean\"\n[[cases.mutations]]\nwrite = \"content/note-d.md\"\ncontent = '''{}'''\n[[cases.mutations]]\nwrite = \"content/note-c.md\"\ncontent = '''{}'''\n",
                    note("note-d", "BAD"),
                    note("note-c", "BAD")
                )
            ),
            note_case("a second", "C", "clean")
        ),
    );
    let first = test(&project);
    let second = test(&project);
    assert_eq!(
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    assert_eq!(
        names(&first),
        [
            ("a first", false),
            ("a second", true),
            ("z first", false),
            ("z second", true),
        ],
        "fixture files sorted, cases in file order"
    );
    assert_eq!((first.total, first.passed, first.failed), (4, 2, 2));
    let unexpected: Vec<&str> = first.cases[0]
        .unexpected
        .iter()
        .map(|d| d.path().unwrap())
        .collect();
    assert_eq!(
        unexpected,
        ["content/note-c.md", "content/note-d.md"],
        "diagnostics in report order"
    );
    assert_eq!(first.cases[0].file, "contract-tests/a.test.toml");
    assert_eq!(first.cases[2].file, "contract-tests/z.test.toml");
}

#[test]
fn formatters_stay_unauthorized_unless_allowed() {
    let project = project();
    let formatter = env!("CARGO_BIN_EXE_bearout-fixture-formatter").replace('\\', "/");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{formatter}\", \"upper\"]\nextensions = [\"txt\"]\n",
            bootstrap("")
        ),
    );
    project.file("src/a.txt", "UPPER\n");
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}",
            note_case("clean", "C", "clean"),
            write_case(
                "formatter difference",
                "expect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"src/b.txt\"\ncontent = \"lower\\n\"\n[[cases.diagnostics]]\ncode = \"B029\"\npath = \"src/b.txt\"\n"
            ),
        ),
    );
    assert_suite_fatal(
        &test(&project),
        "bearout.toml declares formatters (`fixture`), which run as trusted host programs; fixture cases check with them only under --allow-formatters",
    );
    let report = bearout::test(
        project.path(),
        &Options {
            allow_formatters: true,
            ..Options::default()
        },
    );
    assert_suite_ok(&report);
    assert_eq!(report.total, 2);
    // A case whose mutation declares formatters is an observable fatal
    // outcome of that candidate, never a silent authorization.
    project.file("bearout.toml", &bootstrap(""));
    project.file(
        FIXTURE_FILE,
        &write_case(
            "mutated bootstrap declares formatters",
            &format!(
                "expect = \"fatal\"\nfatal = \"pass --allow-formatters\"\n[[cases.mutations]]\nwrite = \"bearout.toml\"\ncontent = '''{}\n[hygiene]\nscope = \"declared\"\nroots = [\"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{formatter}\", \"upper\"]\nextensions = [\"txt\"]\n'''\n",
                bootstrap("")
            ),
        ),
    );
    assert_suite_ok(&test(&project));
}

#[test]
fn every_case_is_validated_before_any_case_runs() {
    let project = project();
    let formatter = env!("CARGO_BIN_EXE_bearout-fixture-formatter").replace('\\', "/");
    let marker = project.path().join("formatter-ran");
    let marker_text = marker.to_str().unwrap().replace('\\', "/");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{formatter}\", \"touch\", \"{marker_text}\"]\nextensions = [\"txt\"]\n",
            bootstrap("")
        ),
    );
    project.file("src/a.txt", "text\n");
    let first = note_case("first runs the formatter", "C", "clean");
    let invalid = write_case(
        "later case is invalid",
        "expect = \"clean\"\n[[cases.mutations]]\ndelete = \"content/missing.md\"\n",
    );
    let authorized = Options {
        allow_formatters: true,
        ..Options::default()
    };
    project.file(FIXTURE_FILE, &format!("{first}{invalid}"));
    let report = bearout::test(project.path(), &authorized);
    assert_suite_fatal(&report, "case `later case is invalid`: mutation 1");
    assert!(
        !marker.exists(),
        "the first case's formatter ran before the suite was validated"
    );
    // A suite whose every case is valid runs the formatter as authorized.
    project.file(FIXTURE_FILE, &first);
    assert_suite_ok(&bearout::test(project.path(), &authorized));
    assert!(marker.exists());
    // Without the formatter, the same ordering holds for the evaluator:
    // a fatal suite reports no case at all.
    project.file("bearout.toml", &bootstrap(""));
    project.file(FIXTURE_FILE, &format!("{first}{invalid}"));
    let report = test(&project);
    assert_suite_fatal(&report, "case `later case is invalid`");
    assert!(report.cases.is_empty());
}

#[test]
fn check_generate_and_format_never_execute_fixtures() {
    let project = project();
    // A fixture that would be fatal to run, and one that would fail.
    project.file(FIXTURE_FILE, "[[cases]\nbroken");
    assert_clean(&project.check());
    assert_clean(&project.generate(Mode::Check));
    assert_clean(&project.run(Command::Format, &Options::default()));
    project.file(FIXTURE_FILE, &note_case("wrong", "BAD", "clean"));
    let report = project.check();
    assert_clean(&report);
    assert_eq!(report.resources, 2, "the fixture file is not a resource");
    assert!(!test(&project).ok);
    // A fixture file inside a resource root is refused by the bootstrap.
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[fixtures]\nfiles = [\"content/x.toml\"]\n",
            common::BOOTSTRAP
        ),
    );
    let report = project.check();
    assert!(
        report
            .fatal
            .as_deref()
            .unwrap()
            .contains("lies beneath resource root `content`")
    );
}

// ---- history cases ------------------------------------------------------

/// A sample-style commit policy: a `<type>: <summary>` header from a
/// short allow-list, a sign-off naming the author, and merge and
/// autosquash exemptions, every one of them this policy's own decision.
const COMMIT_POLICY: &str = r#"TYPES = ["feat", "fix", "chore"]

def commit_policy(history):
    findings = []
    for commit in history["commits"]:
        key = commit["key"]
        if commit["merge"]:
            continue
        subject = commit["subject"]
        if subject.startswith("fixup! ") or subject.startswith("squash! "):
            findings.append(warning("autosquash commit awaits its rebase", commit = key, code = "autosquash"))
            continue
        head, sep, summary = subject.partition(": ")
        if sep == "" or head not in TYPES or summary.strip() == "":
            findings.append(error("subject must be `<type>: <summary>` with a known type", commit = key, line = 1, code = "header"))
        author = commit["author"]
        expected = "Signed-off-by: %s <%s>" % (author["name"], author["email"])
        if expected not in commit["message"].split("\n"):
            findings.append(error("missing `%s`" % expected, commit = key, code = "sign-off"))
    return findings

history_check("commit-policy", commit_policy)
"#;

fn history_project() -> Project {
    let project = Project::with_note();
    project.file("bearout.toml", &bootstrap(""));
    project.file(common::ENTRY, COMMIT_POLICY);
    project
}

fn history_case(name: &str, body: &str, expectations: &str) -> String {
    format!(
        "[[cases]]\nname = \"{name}\"\n{body}\n[cases.history]\nkind = \"message\"\nauthor_name = \"Example Author\"\nauthor_email = \"author@example.test\"\n{expectations}\n"
    )
}

#[test]
fn pending_message_cases_exercise_the_history_policy() {
    let project = history_project();
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}{}{}{}{}",
            history_case(
                "a conventional signed message passes",
                "expect = \"clean\"",
                "message = \"feat: add a capability\\n\\nSigned-off-by: Example Author <author@example.test>\\n\""
            ),
            history_case(
                "a bad header is caught",
                "expect = \"diagnostics\"\nmatch = \"exact\"",
                "message = \"added stuff\\n\\nSigned-off-by: Example Author <author@example.test>\\n\"\n\n[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"pending\"\nline = 1\nrule = \"header\""
            ),
            history_case(
                "a missing sign-off is caught",
                "expect = \"diagnostics\"",
                "message = \"fix: something\\n\"\n\n[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"pending\"\nrule = \"sign-off\"\nmessage = \"history check `commit-policy`: missing `Signed-off-by: Example Author <author@example.test>`\""
            ),
            history_case(
                "a mismatched sign-off is caught",
                "expect = \"diagnostics\"",
                "message = \"fix: something\\n\\nSigned-off-by: Someone Else <else@example.test>\\n\"\n\n[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"pending\"\nrule = \"sign-off\""
            ),
            history_case(
                "a merge is exempt by this policy",
                "expect = \"clean\"",
                "message = \"Merge branch 'topic'\\n\"\nparents = [\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\", \"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\"]\nmerge = true"
            ),
            history_case(
                "an autosquash commit is a warning by this policy",
                "expect = \"diagnostics\"",
                "message = \"fixup! feat: add a capability\\n\"\n\n[[cases.diagnostics]]\ncode = \"B033\"\nseverity = \"warning\"\ncommit = \"pending\"\nrule = \"autosquash\""
            ),
            history_case(
                "both problems at once, in order",
                "expect = \"diagnostics\"",
                "message = \"nope\"\nauthor_timestamp = 1700000000\nauthor_timezone = \"+0200\"\n\n[[cases.diagnostics]]\ncode = \"B032\"\nrule = \"sign-off\"\n\n[[cases.diagnostics]]\ncode = \"B032\"\nrule = \"header\"\nline = 1"
            ),
        ),
    );
    let report = test(&project);
    assert_suite_ok(&report);
    assert_eq!(report.total, 7);
    // The same suite passes without any Git repository: history cases make
    // no Git call.
    assert!(!project.path().join(".git").exists());

    // Expectations match structurally, and exact matching still refuses
    // extras: a case expecting only the header finding fails on the
    // sign-off finding, and lists it.
    project.file(
        FIXTURE_FILE,
        &history_case(
            "only the header",
            "expect = \"diagnostics\"",
            "message = \"nope\"\n\n[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"pending\"\nrule = \"header\"",
        ),
    );
    let report = test(&project);
    assert!(!report.ok);
    let case = &report.cases[0];
    assert_eq!(case.unexpected.len(), 1);
    assert_eq!(case.unexpected[0].commit(), Some("pending"));
    assert_eq!(case.unexpected[0].rule(), Some("sign-off"));
    assert_eq!(
        case.unexpected[0].to_string(),
        "commit pending:B032[sign-off]: history check `commit-policy`: missing `Signed-off-by: Example Author <author@example.test>`"
    );
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["cases"][0]["unexpected"][0]["commit"], "pending");
    assert!(json["cases"][0]["unexpected"][0].get("path").is_none());
    // A wrong commit key in an expectation is simply missing.
    project.file(
        FIXTURE_FILE,
        &history_case(
            "wrong key",
            "expect = \"diagnostics\"\nmatch = \"contains\"",
            "message = \"nope\"\n\n[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"0000000000000000000000000000000000000000\"\nrule = \"header\"",
        ),
    );
    let report = test(&project);
    assert!(!report.ok);
    assert_eq!(report.cases[0].missing.len(), 1);
    assert_eq!(
        report.cases[0].missing[0].to_string(),
        "B032 commit=0000000000000000000000000000000000000000 rule=header"
    );
}

#[test]
fn history_cases_run_only_history_checks_and_no_programs() {
    let project = history_project();
    let formatter = env!("CARGO_BIN_EXE_bearout-fixture-formatter").replace('\\', "/");
    let marker = project.path().join("formatter-ran");
    let marker_text = marker.to_str().unwrap().replace('\\', "/");
    // An ordinary check that would fail, a validator that would fail, and
    // a formatter that would leave a marker: none of them runs for a
    // history case.
    project.file(
        common::ENTRY,
        &format!(
            "{COMMIT_POLICY}\ndef never(p):\n    return [error(\"ordinary check ran\", resource = \"note-a\")]\ndef bad(r):\n    return [error(\"validator ran\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = bad)\ncheck(\"never\", never)\n"
        ),
    );
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{formatter}\", \"touch\", \"{marker_text}\"]\nextensions = [\"txt\"]\n",
            bootstrap("")
        ),
    );
    project.file("src/a.txt", "text\n");
    project.file(
        FIXTURE_FILE,
        &history_case(
            "history only",
            "expect = \"clean\"",
            "message = \"feat: x\\n\\nSigned-off-by: Example Author <author@example.test>\\n\"",
        ),
    );
    let report = bearout::test(
        project.path(),
        &Options {
            allow_formatters: true,
            ..Options::default()
        },
    );
    assert_suite_ok(&report);
    assert!(!marker.exists(), "the formatter ran for a history case");
    // A mutation case in the same suite still runs the whole pipeline.
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}",
            history_case(
                "history only",
                "expect = \"clean\"",
                "message = \"feat: x\\n\\nSigned-off-by: Example Author <author@example.test>\\n\""
            ),
            write_case(
                "mutation",
                "expect = \"diagnostics\"\nmatch = \"contains\"\n[[cases.mutations]]\nwrite = \"src/b.txt\"\ncontent = \"b\\n\"\n[[cases.diagnostics]]\ncode = \"B015\"\n"
            ),
        ),
    );
    let report = bearout::test(
        project.path(),
        &Options {
            allow_formatters: true,
            ..Options::default()
        },
    );
    assert_suite_ok(&report);
    assert!(marker.exists());

    // Without a history check, a history case is a fatal outcome of that
    // case, never a pass; a policy that does not load is fatal too.
    project.file("bearout.toml", &bootstrap(""));
    project.file(
        common::ENTRY,
        "schema(\"example/test/note@1\", shape = \"note.schema.toml\")\n",
    );
    project.file(
        FIXTURE_FILE,
        &history_case(
            "no history check",
            "expect = \"clean\"",
            "message = \"feat: x\\n\"",
        ),
    );
    let report = test(&project);
    assert!(!report.ok);
    assert_eq!(report.cases[0].actual, Outcome::Fatal);
    assert!(
        report.cases[0]
            .fatal
            .as_deref()
            .unwrap()
            .contains("registers no history check")
    );
    project.file(
        FIXTURE_FILE,
        &history_case(
            "no history check expected",
            "expect = \"fatal\"\nfatal = \"registers no history check\"",
            "message = \"feat: x\\n\"",
        ),
    );
    assert_suite_ok(&test(&project));
    project.file(common::ENTRY, "this is not starlark\n");
    project.file(
        FIXTURE_FILE,
        &history_case(
            "broken policy",
            "expect = \"fatal\"\nfatal = \"did not load\"",
            "message = \"feat: x\\n\"",
        ),
    );
    assert_suite_ok(&test(&project));
}

#[test]
fn malformed_history_cases_are_fatal_for_the_suite() {
    let project = history_project();
    for (body, expected) in [
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\n",
            "history: `kind` is required",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"range\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\n",
            "`kind` must be `message`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\n",
            "`message` is required",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_email = \"a@b\"\n",
            "`author_name` is required",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\n",
            "`author_email` is required",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a<b\"\n",
            "author identity",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\nauthor_timestamp = 1\nauthor_timezone = \"UTC\"\n",
            "invalid timezone",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\nauthor_timestamp = \"now\"\n",
            "`author_timestamp` must be an integer",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\nparents = [\"abc\"]\n",
            "is not a full commit identity",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\nmerge = true\n",
            "`merge = true` contradicts the 0 `parents` given",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\nscript = \"x\"\n",
            "unknown key `script`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\nbaseline = true\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\n",
            "`history` and `baseline` are mutually exclusive",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\n",
            "`history` and `mutations` are mutually exclusive",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\n[[cases.diagnostics]]\ncode = \"B032\"\nside = \"baseline\"\n",
            "`side` does not apply to a history case",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[cases.history]\nkind = \"message\"\nmessage = \"m\"\nauthor_name = \"a\"\nauthor_email = \"a@b\"\n[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"pending\"\npath = \"x\"\n",
            "`commit` is exclusive with `path` and `side`",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"diagnostics\"\n[[cases.mutations]]\ndelete = \"content/note-a.md\"\n[[cases.diagnostics]]\ncode = \"B015\"\ncommit = \"pending\"\n",
            "`commit` applies only to a history case",
        ),
        (
            "[[cases]]\nname = \"x\"\nexpect = \"clean\"\nhistory = 3\n",
            "`history` must be a table",
        ),
    ] {
        project.file(FIXTURE_FILE, body);
        assert_suite_fatal(&test(&project), expected);
    }
    // The message is bounded by the history limit.
    project.file(
        "bearout.toml",
        &bootstrap("[limits]\nhistory_commit_bytes = 8\n"),
    );
    project.file(
        FIXTURE_FILE,
        &history_case(
            "long",
            "expect = \"clean\"",
            "message = \"feat: far too long\"",
        ),
    );
    assert_suite_fatal(&test(&project), "above `limits.history_commit_bytes` = 8");
}

#[test]
fn history_fixture_messages_keep_line_semantics_and_identity_shape() {
    let project = history_project();
    project.file(
        common::ENTRY,
        "def t(history):\n    commit = history[\"commits\"][0]\n    author = commit[\"author\"]\n    findings = [error(\"last\", commit = \"pending\", line = 2, code = \"last\")]\n    if author[\"timestamp\"] == None and author[\"timezone\"] == None:\n        findings.append(warning(\"no time\", commit = \"pending\", code = \"no-time\"))\n    else:\n        findings.append(warning(\"%d %s\" % (author[\"timestamp\"], author[\"timezone\"]), commit = \"pending\", code = \"time\"))\n    findings.append(warning(commit[\"subject\"], commit = \"pending\", code = \"subject\"))\n    return findings\nhistory_check(\"t\", t)\n",
    );
    let case = |name: &str, message: &str, extra: &str, expectations: &str| {
        format!(
            "[[cases]]\nname = \"{name}\"\nexpect = \"diagnostics\"\n[cases.history]\nkind = \"message\"\nmessage = \"{message}\"\nauthor_name = \"A\"\nauthor_email = \"a@example.test\"\n{extra}\n{expectations}\n"
        )
    };
    let time_free =
        "[[cases.diagnostics]]\ncode = \"B033\"\ncommit = \"pending\"\nrule = \"no-time\"\n";
    project.file(
        FIXTURE_FILE,
        &format!(
            "{}{}{}{}",
            case(
                "crlf lines and no invented time",
                "subject\\r\\nsecond\\r\\n",
                "",
                &format!("[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"pending\"\nline = 2\nrule = \"last\"\n{time_free}[[cases.diagnostics]]\ncode = \"B033\"\ncommit = \"pending\"\nrule = \"subject\"\nmessage = \"history check `t`: subject\"\n")
            ),
            case(
                "cr only lines",
                "subject\\rsecond",
                "",
                &format!("[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"pending\"\nline = 2\nrule = \"last\"\n{time_free}[[cases.diagnostics]]\ncode = \"B033\"\ncommit = \"pending\"\nrule = \"subject\"\nmessage = \"history check `t`: subject\"\n")
            ),
            case(
                "one line refuses line two",
                "subject\\r\\n",
                "",
                &format!("[[cases.diagnostics]]\ncode = \"B014\"\npath = \"bearout.star\"\n{time_free}[[cases.diagnostics]]\ncode = \"B033\"\ncommit = \"pending\"\nrule = \"subject\"\n")
            ),
            case(
                "explicit synthetic time",
                "subject\\nsecond",
                "author_timestamp = 1700000000\nauthor_timezone = \"+0200\"",
                "[[cases.diagnostics]]\ncode = \"B032\"\ncommit = \"pending\"\nline = 2\nrule = \"last\"\n[[cases.diagnostics]]\ncode = \"B033\"\ncommit = \"pending\"\nrule = \"time\"\nmessage = \"history check `t`: 1700000000 +0200\"\n[[cases.diagnostics]]\ncode = \"B033\"\ncommit = \"pending\"\nrule = \"subject\"\n"
            ),
        ),
    );
    assert_suite_ok(&test(&project));
    // A timestamp without a timezone, or the reverse, is refused.
    for extra in ["author_timestamp = 5", "author_timezone = \"+0000\""] {
        project.file(FIXTURE_FILE, &case("half", "x", extra, ""));
        assert_suite_fatal(&test(&project), "given together or not at all");
    }
    // A message within the per-commit limit but above the total budget.
    project.file(
        "bearout.toml",
        &bootstrap("[limits]\nhistory_commit_bytes = 100\nhistory_bytes = 8\n"),
    );
    project.file(
        FIXTURE_FILE,
        &case(
            "too big for the budget",
            "feat: twenty bytes\\n",
            "",
            "[[cases.diagnostics]]\ncode = \"B032\"\n",
        ),
    );
    assert_suite_fatal(&test(&project), "above `limits.history_bytes` = 8");
}
