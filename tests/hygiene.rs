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

// ---- native text hygiene --------------------------------------------------

use common::{assert_line, assert_no_line, codes, lines};

const STRICT: &str = "root = true\n\n[*]\ncharset = utf-8\nend_of_line = lf\ninsert_final_newline = true\ntrim_trailing_whitespace = true\n\n[*.md]\ntrim_trailing_whitespace = false\n";

/// A declared-scope project over `text/` with a strict `.editorconfig`.
fn text_project() -> Project {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &hygiene_bootstrap("scope = \"declared\"\nroots = [\"text\"]"),
    );
    project.file(".editorconfig", STRICT);
    project.file("text/clean.txt", "one\ntwo\n");
    project
}

#[test]
fn line_endings_final_newlines_and_trailing_whitespace_are_reported_once_per_file() {
    let project = text_project();
    project.file("text/crlf.txt", "a\r\nb\r\n");
    project.file("text/cr.txt", "a\rb\r");
    project.file("text/missing.txt", "no newline");
    project.file("text/extra.txt", "a\n\n\n");
    project.file("text/trailing.txt", "a  \nb\t\nc\n");
    project.file("text/hard-breaks.md", "line one  \nline two\n");
    project.file("text/empty.txt", "");
    let report = project.check();
    assert_eq!(
        lines(&report),
        [
            "text/cr.txt:1:B026: line ends with cr; `end_of_line = lf` requires lf (and 1 more line)",
            "text/crlf.txt:1:B026: line ends with crlf; `end_of_line = lf` requires lf (and 1 more line)",
            "text/extra.txt:1:B027: file ends with 2 blank line(s); `insert_final_newline = true` requires exactly one final newline",
            "text/missing.txt:1:B027: file does not end with a newline; `insert_final_newline = true` requires exactly one",
            "text/trailing.txt:1:B028: line ends with whitespace; `trim_trailing_whitespace = true` forbids it (and 1 more line)",
        ]
    );
    assert_eq!(report.files, 8);
    assert_eq!(lines(&project.check()), lines(&report));
}

#[test]
fn encoding_rules_come_from_charset_and_binary_files_are_skipped() {
    let project = text_project();
    project.bytes("text/latin1.txt", b"caf\xe9\n");
    project.bytes("text/bom.txt", b"\xEF\xBB\xBFa\n");
    project.bytes("text/image.bin", b"\x00\x01  \r\n");
    project.file("text/bom-required.dat", "plain\n");
    project.file("text/.editorconfig", "[*.dat]\ncharset = utf-8-bom\n");
    let report = project.check();
    assert_eq!(
        lines(&report),
        [
            "text/bom-required.dat:B025: file has no byte-order mark; `charset = utf-8-bom` requires one",
            "text/bom.txt:B025: file begins with a byte-order mark; `charset = utf-8` forbids one",
            "text/latin1.txt:1:B025: file is not valid UTF-8: invalid utf-8 sequence of 1 bytes from index 3",
        ]
    );
    // The bootstrap overrides content sniffing both ways.
    project.file(
        "bearout.toml",
        &hygiene_bootstrap(
            "scope = \"declared\"\nroots = [\"text\"]\nbinary = [\"text/latin1.txt\"]\ntext = [\"text/image.bin\"]",
        ),
    );
    let report = project.check();
    assert_no_line(&report, "latin1");
    assert_line(&report, "text/image.bin:1:B026: line ends with crlf");
    assert_line(&report, "text/image.bin:1:B028");
}

#[test]
fn editorconfig_precedence_root_unset_and_unsupported_values() {
    let project = text_project();
    project.file(
        "text/nested/.editorconfig",
        "[*.txt]\ninsert_final_newline = unset\n",
    );
    project.file("text/nested/loose.txt", "no newline");
    project.file(
        "text/isolated/.editorconfig",
        "root = true\r\n\r\n[*]\r\nend_of_line = crlf\r\n",
    );
    project.file("text/isolated/win.txt", "a\r\nb\r\n  ");
    project.file(
        "text/odd/.editorconfig",
        "[*.txt]\ncharset = latin1\n\n[*.cfg]\nend_of_line = native\n",
    );
    project.file("text/odd/legacy.txt", "x\n");
    project.file("text/odd/tool.cfg", "x\n");
    project.file("text/odd/fine.md", "x\n");
    let report = project.check();
    assert_eq!(
        lines(&report),
        [
            "text/odd/legacy.txt:B023: `charset = latin1` is not a value Bearout can enforce; remove the property, set it to `unset`, or exclude the file from the selection",
            "text/odd/tool.cfg:B023: `end_of_line = native` is not a value Bearout can enforce; remove the property, set it to `unset`, or exclude the file from the selection",
        ]
    );
    // `unset` removed the outer requirement; `root = true` hid the outer
    // file entirely, so trailing whitespace went unchecked there.
    assert_no_line(&report, "loose.txt");
    assert_no_line(&report, "win.txt");
    // Pattern precedence: a later, more specific section wins.
    project.file(
        "text/.editorconfig",
        "[*]\ntrim_trailing_whitespace = true\n\n[*.md]\ntrim_trailing_whitespace = false\n\n[keep-*.md]\ntrim_trailing_whitespace = true\n",
    );
    project.file("text/keep-tidy.md", "a  \n");
    project.file("text/loose.md", "a  \n");
    let report = project.check();
    assert_line(&report, "text/keep-tidy.md:1:B028");
    assert_no_line(&report, "text/loose.md");
}

#[test]
fn an_unusable_editorconfig_is_reported_once_and_stops_checks_beneath_it() {
    let project = text_project();
    project.file(
        "text/broken/.editorconfig",
        "[*\ngarbage line without equals\n",
    );
    project.file("text/broken/a.txt", "no newline");
    project.file("text/broken/b.txt", "trailing  \n");
    let report = project.check();
    assert_eq!(
        lines(&report),
        [
            "text/broken/.editorconfig:B023: `.editorconfig` has a line that is neither a section header, a property, nor a comment"
        ]
    );
}

#[test]
fn size_limits_stop_a_file_without_cascading() {
    let project = text_project();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nfile_bytes = 160\n",
            hygiene_bootstrap("scope = \"declared\"\nroots = [\"text\"]")
        ),
    );
    project.file("text/big.txt", &format!("{}  \r\n", "x".repeat(200)));
    let report = project.check();
    assert_eq!(
        lines(&report),
        ["text/big.txt:B024: file is 204 bytes, above `limits.file_bytes` = 160"]
    );
    assert!(
        codes(&report)
            .iter()
            .all(|code| *code == bearout::Code::FileUnreadable)
    );
}

#[test]
fn the_selected_trees_editorconfig_governs_git_backed_checks() {
    let project = text_project();
    project.git_init();
    project.commit_all("strict");
    // Relax the rules on disk only; the index still says strict.
    project.file(".editorconfig", "root = true\n\n[*]\ncharset = utf-8\n");
    project.file("text/trailing.txt", "a  \n");
    project.git(&["add", "text/trailing.txt"]);
    assert_clean(&project.check());
    let index = project.check_from(Source::Index);
    assert_eq!(
        lines(&index),
        [
            "text/trailing.txt:1:B028: line ends with whitespace; `trim_trailing_whitespace = true` forbids it"
        ]
    );
    // Stage the relaxed rules: the index is now clean while HEAD is strict
    // and lacks the file entirely.
    project.git(&["add", ".editorconfig"]);
    assert_clean(&project.check_from(Source::Index));
    assert_clean(&project.check_from(Source::Revision("HEAD".to_owned())));
    // An unstaged repair cannot hide a staged violation.
    project.git(&["checkout", "-q", "HEAD", "--", ".editorconfig"]);
    project.git(&["add", ".editorconfig"]);
    project.file("text/trailing.txt", "a\n");
    assert_clean(&project.check());
    assert_line(
        &project.check_from(Source::Index),
        "text/trailing.txt:1:B028",
    );
}

// ---- external formatters --------------------------------------------------

use bearout::Mode;

/// The deterministic formatter stand-in, a feature-gated binary of this
/// crate. Run the tests with `--all-features`, as the pinned tasks do.
fn fixture_formatter() -> String {
    env!("CARGO_BIN_EXE_bearout-fixture-formatter").replace('\\', "/")
}

fn formatter_bootstrap(command_tail: &str, extra: &str) -> String {
    format!(
        "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{}\"{}]\nextensions = [\"txt\"]\n{extra}\n",
        common::BOOTSTRAP,
        fixture_formatter(),
        command_tail
    )
}

fn allowed(source: Source) -> Options {
    Options {
        source,
        allow_formatters: true,
        ..Options::default()
    }
}

fn formatter_project(command_tail: &str, extra: &str) -> Project {
    let project = Project::with_note();
    project.file("bearout.toml", &formatter_bootstrap(command_tail, extra));
    project.file("src/a.txt", "ALREADY\n");
    project.file("src/b.txt", "lower\n");
    project.file("src/ignored.md", "not assigned\n");
    project
}

#[test]
fn formatters_need_explicit_authorization() {
    let project = formatter_project(", \"upper\"", "");
    assert_fatal(
        &project.check(),
        "bearout.toml declares formatters (`fixture`), which run as trusted host programs; pass --allow-formatters",
    );
    let report = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert!(report.fatal.is_none(), "{:?}", report.fatal);
    // A selection without any assigned file needs no authorization.
    project.remove("src/a.txt");
    project.remove("src/b.txt");
    assert_clean(&project.check());
}

#[test]
fn unchanged_and_changed_output_are_told_apart_in_path_order() {
    let project = formatter_project(", \"upper\"", "");
    project.file("src/c.txt", "MIXED case\n");
    let report = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_eq!(
        lines(&report),
        [
            "src/b.txt:B029: file differs from the output of formatter `fixture`; run `bearout format`",
            "src/c.txt:B029: file differs from the output of formatter `fixture`; run `bearout format`",
        ]
    );
    let again = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_eq!(lines(&again), lines(&report));
    let echo = formatter_project(", \"echo\"", "");
    assert_clean(&echo.run(Command::Check, &allowed(Source::WorkingDirectory)));
}

#[test]
fn the_project_relative_path_is_substituted() {
    let project = formatter_project(", \"name\", \"{path}\"", "");
    project.file("src/a.txt", "name=src/a.txt\nbody\n");
    project.file("src/b.txt", "name=src/other.txt\nbody\n");
    let report = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_eq!(
        lines(&report),
        [
            "src/b.txt:B029: file differs from the output of formatter `fixture`; run `bearout format`"
        ]
    );
}

#[test]
fn support_files_come_from_the_selected_tree() {
    let project = formatter_project(", \"config\", \"tool.cfg\"", "support = [\"tool.cfg\"]");
    project.file("tool.cfg", "committed config\n");
    project.file("src/a.txt", "committed config\nbody\n");
    project.file("src/b.txt", "committed config\nbody\n");
    project.git_init();
    project.commit_all("base");
    assert_clean(&project.run(Command::Check, &allowed(Source::WorkingDirectory)));
    // A different configuration on disk does not govern the index.
    project.file("tool.cfg", "edited config\n");
    let working = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_eq!(working.errors(), 2, "{:?}", lines(&working));
    assert_clean(&project.run(Command::Check, &allowed(Source::Index)));
    assert_clean(&project.run(
        Command::Check,
        &allowed(Source::Revision("HEAD".to_owned())),
    ));
    // A staged configuration governs the index even before it is on disk.
    project.file("tool.cfg", "staged config\n");
    project.git(&["add", "tool.cfg"]);
    project.file("tool.cfg", "committed config\n");
    assert_clean(&project.run(Command::Check, &allowed(Source::WorkingDirectory)));
    assert_eq!(
        project
            .run(Command::Check, &allowed(Source::Index))
            .errors(),
        2
    );
    // A missing support file is fatal and names it.
    project.file(
        "bearout.toml",
        &formatter_bootstrap(", \"config\", \"tool.cfg\"", "support = [\"absent.cfg\"]"),
    );
    assert_fatal(
        &project.run(Command::Check, &allowed(Source::WorkingDirectory)),
        "cannot read support file `absent.cfg` of formatter `fixture` from the selected tree",
    );
}

#[test]
fn failures_are_bounded_sanitized_and_fail_closed() {
    let project = formatter_project(", \"fail\"", "");
    let report = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_eq!(
        lines(&report),
        [
            "src/a.txt:B030: formatter `fixture` exited with status 3: formatter says no",
            "src/b.txt:B030: formatter `fixture` exited with status 3: formatter says no",
        ]
    );

    let missing = Project::with_note();
    missing.file(
        "bearout.toml",
        &format!("{}\n[hygiene]\nscope = \"declared\"\nroots = [\"src\"]\n\n[[formatters]]\nname = \"gone\"\ncommand = [\"bearout-no-such-formatter-executable\"]\n", common::BOOTSTRAP),
    );
    missing.file("src/a.txt", "x\n");
    assert_fatal(
        &missing.run(Command::Check, &allowed(Source::WorkingDirectory)),
        "formatter `gone` cannot start: `bearout-no-such-formatter-executable` is not installed or not on PATH",
    );

    let slow = formatter_project(", \"sleep\", \"5\"", "timeout = 1");
    slow.remove("src/b.txt");
    let started = std::time::Instant::now();
    let report = slow.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(4),
        "killed at the timeout"
    );
    assert_eq!(
        lines(&report),
        ["src/a.txt:B030: formatter `fixture` timed out after 1 s"]
    );

    let spew = formatter_project(", \"spew\", \"3000000\"", "");
    spew.remove("src/b.txt");
    let report = spew.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_line(
        &report,
        "src/a.txt:B030: formatter `fixture` produced more than",
    );
    assert_eq!(report.errors(), 1);

    let noisy = formatter_project(", \"stderr\", \"200000\"", "");
    noisy.remove("src/b.txt");
    assert_clean(&noisy.run(Command::Check, &allowed(Source::WorkingDirectory)));

    let crash = formatter_project(", \"crash\"", "");
    crash.remove("src/b.txt");
    let report = crash.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_eq!(report.errors(), 1);
    assert!(
        lines(&report)[0].starts_with("src/a.txt:B030: formatter `fixture` "),
        "{:?}",
        lines(&report)
    );
}

#[test]
fn formatters_run_confined_and_leave_the_repository_untouched() {
    let project = formatter_project(", \"cache\"", "");
    project.git_init();
    project.commit_all("base");
    assert_clean(&project.run(Command::Check, &allowed(Source::WorkingDirectory)));
    assert_eq!(
        project.git(&["status", "--porcelain"]),
        "",
        "no cache file in the repository"
    );
    assert!(!project.path().join(".fixture-cache").exists());
    assert!(!project.path().join("src/.fixture-cache").exists());

    let env = formatter_project(", \"env\"", "");
    env.remove("src/b.txt");
    env.file("src/a.txt", "NO_COLOR=1 TERM=dumb\n");
    assert_clean(&env.run(Command::Check, &allowed(Source::WorkingDirectory)));
}

#[test]
fn starlark_cannot_invoke_a_formatter() {
    let project = formatter_project(", \"upper\"", "");
    project.file(
        common::ENTRY,
        "def c(p):\n    return [warning(str(dir(p)), resource = \"note-a\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ncheck(\"c\", c)\n",
    );
    project.file("src/b.txt", "LOWER\n");
    let report = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_clean(&report);
    let rendered = lines(&report).join("\n");
    assert!(!rendered.contains("format"), "{rendered}");
    assert!(!rendered.contains("exec"), "{rendered}");
    project.file(
        common::ENTRY,
        "def c(p):\n    return [warning(str(format), resource = \"note-a\")]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ncheck(\"c\", c)\n",
    );
    let report = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert!(codes(&report).iter().any(|code| matches!(
        code,
        bearout::Code::ScriptLoad | bearout::Code::ScriptFailure
    )));
}

#[test]
fn generate_check_and_write_include_formatter_verification() {
    let project = formatter_project(", \"upper\"", "");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[outputs]\nroots = [\"generated\"]\n",
            formatter_bootstrap(", \"upper\"", "")
        ),
    );
    let report = project.run(
        Command::Generate(Mode::Check),
        &allowed(Source::WorkingDirectory),
    );
    assert_line(&report, "src/b.txt:B029");
    assert!(
        report.outputs.is_empty(),
        "a hygiene error blocks generation"
    );
    let report = project.run(
        Command::Generate(Mode::Write),
        &allowed(Source::WorkingDirectory),
    );
    assert_line(&report, "src/b.txt:B029");
    assert_eq!(
        project.read("src/b.txt"),
        "lower\n",
        "generation never rewrites sources"
    );
}

// ---- formatting writes ----------------------------------------------------

fn format_options() -> Options {
    Options {
        allow_formatters: true,
        ..Options::default()
    }
}

/// Every file under the project with its bytes.
fn snapshot(project: &Project) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .expect("dir")
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let name = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((name, std::fs::read(&path).expect("read")));
            }
        }
    }
    let mut out = Vec::new();
    walk(project.path(), project.path(), &mut out);
    out
}

fn leftovers(project: &Project, dir: &str) -> Vec<String> {
    std::fs::read_dir(project.path().join(dir))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.contains("tmp") || name.starts_with(".bearout"))
                .collect()
        })
        .unwrap_or_default()
}

/// A project whose `text/` files need native fixes and whose `src/*.txt`
/// files need the uppercasing fixture.
fn writable_project() -> Project {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"text\", \"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{}\", \"upper\"]\npaths = [\"src\"]\nextensions = [\"txt\"]\n",
            common::BOOTSTRAP,
            fixture_formatter()
        ),
    );
    project.file(".editorconfig", STRICT);
    project.file("text/messy.txt", "a  \r\nb\t\r\n\r\n\r\n");
    project.file("text/clean.txt", "fine\n");
    project.file("text/hard.md", "line  \nnext\n");
    project.file("src/lower.txt", "lower\n");
    project.file("src/lower.rs", "not assigned  \n");
    project.file("outside.txt", "not selected  \n");
    project
}

#[test]
fn check_writes_nothing_and_format_rewrites_only_selected_files() {
    let project = writable_project();
    let before = snapshot(&project);
    let report = project.run(Command::Check, &format_options());
    assert_eq!(report.errors(), 5, "{:?}", lines(&report));
    assert_eq!(snapshot(&project), before, "check mode never writes");

    let report = project.run(Command::Format, &format_options());
    assert_clean(&report);
    assert_eq!(
        report.formatted,
        ["src/lower.rs", "src/lower.txt", "text/messy.txt"]
    );
    assert_eq!(project.read("text/messy.txt"), "a\nb\n");
    assert_eq!(project.read("src/lower.txt"), "LOWER\n");
    assert_eq!(
        project.read("src/lower.rs"),
        "not assigned\n",
        "native rules still apply without a formatter"
    );
    assert_eq!(
        project.read("text/hard.md"),
        "line  \nnext\n",
        "hard breaks kept where trimming is off"
    );
    assert_eq!(
        project.read("outside.txt"),
        "not selected  \n",
        "unselected files are untouched"
    );
    assert_eq!(project.read("text/clean.txt"), "fine\n");
    assert!(leftovers(&project, "text").is_empty() && leftovers(&project, "src").is_empty());

    // A second run changes nothing, and the check is clean.
    let again = project.run(Command::Format, &format_options());
    assert_clean(&again);
    assert!(again.formatted.is_empty());
    assert_clean(&project.run(Command::Check, &format_options()));
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["formatted"][2], "text/messy.txt");
}

#[test]
fn native_normalization_precedes_the_formatter() {
    let project = writable_project();
    // The fixture uppercases whatever it receives: trailing whitespace and
    // CRLF are gone before it runs, so the result satisfies both.
    project.file("src/lower.txt", "lower  \r\nmore\r\n\r\n");
    assert_clean(&project.run(Command::Format, &format_options()));
    assert_eq!(project.read("src/lower.txt"), "LOWER\nMORE\n");
}

#[test]
fn git_backed_sources_and_baselines_cannot_be_formatted() {
    let project = writable_project();
    project.git_init();
    project.commit_all("base");
    let before = snapshot(&project);
    for source in [Source::Index, Source::Revision("HEAD".to_owned())] {
        let report = project.run(
            Command::Format,
            &Options {
                source,
                allow_formatters: true,
                ..Options::default()
            },
        );
        assert_fatal(&report, "formatting writes to the working directory");
    }
    let report = project.run(
        Command::Format,
        &Options {
            baseline: Some("HEAD".to_owned()),
            allow_formatters: true,
            ..Options::default()
        },
    );
    assert_fatal(&report, "formatting never touches a comparison baseline");
    assert_eq!(snapshot(&project), before);
    assert_fatal(
        &project.run(Command::Format, &Options::default()),
        "pass --allow-formatters",
    );
    assert_eq!(snapshot(&project), before);
}

#[test]
fn files_that_cannot_be_formatted_are_reported_and_left_alone() {
    let project = writable_project();
    project.bytes("text/latin1.txt", b"caf\xe9  \n");
    project.bytes("text/blob.txt", b"\x00\x01  \n");
    let report = project.run(Command::Format, &format_options());
    assert_eq!(
        lines(&report),
        [
            "text/latin1.txt:B025: file is not valid UTF-8 and cannot be formatted: invalid utf-8 sequence of 1 bytes from index 3"
        ]
    );
    assert_eq!(
        std::fs::read(project.path().join("text/latin1.txt")).unwrap(),
        b"caf\xe9  \n"
    );
    assert_eq!(
        std::fs::read(project.path().join("text/blob.txt")).unwrap(),
        b"\x00\x01  \n",
        "binary files are never rewritten"
    );
    assert_eq!(
        report.formatted,
        ["src/lower.rs", "src/lower.txt", "text/messy.txt"],
        "other files are still formatted"
    );
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_refused_and_permissions_are_preserved() {
    use std::os::unix::fs::PermissionsExt;
    let project = writable_project();
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("victim.txt"), "victim  \n").expect("write");
    std::os::unix::fs::symlink(
        outside.path().join("victim.txt"),
        project.path().join("text/link.txt"),
    )
    .expect("symlink");
    std::fs::set_permissions(
        project.path().join("src/lower.txt"),
        std::fs::Permissions::from_mode(0o755),
    )
    .expect("chmod");
    let report = project.run(Command::Format, &format_options());
    assert_clean(&report);
    assert_eq!(
        std::fs::read_to_string(outside.path().join("victim.txt")).unwrap(),
        "victim  \n",
        "the link target is untouched"
    );
    assert!(
        std::fs::symlink_metadata(project.path().join("text/link.txt"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(project.read("src/lower.txt"), "LOWER\n");
    let mode = std::fs::metadata(project.path().join("src/lower.txt"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o755, "the executable bit survives");

    // A declared file that is a link is refused outright.
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nfiles = [\"text/link.txt\"]\n",
            common::BOOTSTRAP
        ),
    );
    assert_fatal(
        &project.run(Command::Format, &format_options()),
        "reached through the symbolic link",
    );
}

#[test]
fn a_concurrent_edit_is_detected_and_nothing_is_written() {
    let project = writable_project();
    // A "formatter" that edits the target on disk while formatting it.
    let target = project.path().join("src/lower.txt");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"text\", \"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{}\", \"edit\", \"{}\"]\npaths = [\"src\"]\nextensions = [\"txt\"]\n",
            common::BOOTSTRAP,
            fixture_formatter(),
            target.to_str().unwrap().replace('\\', "/")
        ),
    );
    let before = snapshot(&project);
    let report = project.run(Command::Format, &format_options());
    assert_line(
        &report,
        "src/lower.txt:B031: file changed after it was read; nothing was written, run again",
    );
    assert!(report.formatted.is_empty());
    let mut expected = before;
    for (name, bytes) in &mut expected {
        if name == "src/lower.txt" {
            *bytes = b"edited meanwhile\n".to_vec();
        }
    }
    assert_eq!(
        snapshot(&project),
        expected,
        "only the concurrent edit itself changed the tree"
    );
}

#[cfg(unix)]
#[test]
fn a_partial_failure_restores_prior_content() {
    use std::os::unix::fs::PermissionsExt;
    if std::fs::read_to_string("/proc/self/status").is_ok_and(|s| s.contains("Uid:\t0\t")) {
        eprintln!("skipped: running as root, permission checks do not apply");
        return;
    }
    let project = writable_project();
    project.file("text/locked/late.txt", "late  \n");
    let locked = project.path().join("text/locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let before = snapshot(&project);
    let report = project.run(Command::Format, &format_options());
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).expect("chmod back");
    assert_line(
        &report,
        "text/locked/late.txt:B031: cannot write the formatted file",
    );
    assert_line(
        &report,
        "bearout.toml:B031: formatting failed after 2 change(s); 2 restored to prior content",
    );
    assert!(report.formatted.is_empty());
    assert_eq!(
        snapshot(&project),
        before,
        "every completed replacement was undone"
    );
    assert!(leftovers(&project, "text").is_empty());
    assert!(leftovers(&project, "src").is_empty());
}

// ---- bounded inputs and phase gating --------------------------------------

#[test]
fn configuration_and_support_files_are_bounded_and_never_linked() {
    // An oversized `.editorconfig` is reported once and suspends checks
    // beneath it, from every source.
    let project = text_project();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nfile_bytes = 160\n",
            hygiene_bootstrap("scope = \"declared\"\nroots = [\"text\"]")
        ),
    );
    project.file(
        "text/big/.editorconfig",
        &format!("[*]\n# {}\ninsert_final_newline = true\n", "x".repeat(280)),
    );
    project.file("text/big/a.txt", "no newline");
    project.git_init();
    project.commit_all("base");
    for source in [
        Source::WorkingDirectory,
        Source::Index,
        Source::Revision("HEAD".to_owned()),
    ] {
        let report = project.check_from(source);
        // The oversized configuration is reported as configuration (B023)
        // and, being a selected file itself, as an oversized file (B024);
        // nothing beneath it is checked.
        assert_eq!(
            lines(&report),
            [
                "text/big/.editorconfig:B023: `.editorconfig` is 315 bytes, above `limits.file_bytes` = 160",
                "text/big/.editorconfig:B024: file is 315 bytes, above `limits.file_bytes` = 160",
            ]
        );
    }

    // A linked `.editorconfig` is refused, in Git trees and on disk.
    let project = text_project();
    project.file("text/linked/real.ini", "[*]\ninsert_final_newline = true\n");
    project.file("text/linked/a.txt", "no newline");
    project.git_init();
    project.commit_all("base");
    project.stage_entry("120000", b"real.ini", "text/linked/.editorconfig");
    let report = project.check_from(Source::Index);
    assert_eq!(
        lines(&report),
        [
            "text/linked/.editorconfig:B023: `.editorconfig` is reached through the symbolic link `text/linked/.editorconfig`; configuration is never read through links"
        ]
    );
    project.git(&["commit", "-q", "-m", "linked config"]);
    assert_line(
        &project.check_from(Source::Revision("HEAD".to_owned())),
        "text/linked/.editorconfig:B023: `.editorconfig` is reached through the symbolic link",
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("real.ini", project.path().join("text/linked/.editorconfig"))
            .expect("symlink");
        assert_line(
            &project.check(),
            "text/linked/.editorconfig:B023: `.editorconfig` is reached through the symbolic link",
        );
    }

    // Oversized and linked support files are fatal and name the file.
    let project = formatter_project(", \"config\", \"tool.cfg\"", "support = [\"tool.cfg\"]");
    project.file("tool.cfg", &"c".repeat(100));
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nfile_bytes = 50\n",
            formatter_bootstrap(", \"config\", \"tool.cfg\"", "support = [\"tool.cfg\"]")
        ),
    );
    project.file("src/a.txt", "small\n");
    project.remove("src/b.txt");
    assert_fatal(
        &project.run(Command::Check, &allowed(Source::WorkingDirectory)),
        "support file `tool.cfg` of formatter `fixture` is 100 bytes, above `limits.file_bytes` = 50",
    );
    project.file(
        "bearout.toml",
        &formatter_bootstrap(", \"config\", \"tool.cfg\"", "support = [\"tool.cfg\"]"),
    );
    project.git_init();
    project.commit_all("base");
    project.stage_entry("120000", b"src/a.txt", "tool.cfg");
    assert_fatal(
        &project.run(Command::Check, &allowed(Source::Index)),
        "support file `tool.cfg` of formatter `fixture` is reached through the symbolic link `tool.cfg`",
    );
}

#[test]
fn the_total_hygiene_budget_is_fatal_when_exhausted() {
    let project = text_project();
    project.file("text/one.txt", "1234567890\n");
    project.file("text/two.txt", "1234567890\n");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nhygiene_bytes = 100\n",
            hygiene_bootstrap("scope = \"declared\"\nroots = [\"text\"]")
        ),
    );
    assert_fatal(
        &project.check(),
        "hygiene inputs exceed `limits.hygiene_bytes` = 100 while reading `",
    );
    // Configuration files count too.
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nhygiene_bytes = 30\n",
            hygiene_bootstrap("scope = \"declared\"\nfiles = [\"text/one.txt\"]")
        ),
    );
    assert_fatal(&project.check(), "while reading `.editorconfig`");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nhygiene_bytes = 100000\n",
            hygiene_bootstrap("scope = \"declared\"\nroots = [\"text\"]")
        ),
    );
    assert_clean(&project.check());
}

#[test]
fn formatters_never_see_undecodable_or_unconfigured_files() {
    let project = formatter_project(", \"upper\"", "");
    project.file(
        ".editorconfig",
        "root = true\n\n[*]\ncharset = utf-8\n\n[weird.txt]\ncharset = latin1\n",
    );
    project.bytes("src/a.txt", b"caf\xe9\n");
    project.file("src/weird.txt", "x\n");
    project.file("src/b.txt", "lower\n");
    project.file("src/broken/.editorconfig", "[*\nnot a property\n");
    project.file("src/broken/c.txt", "lower\n");
    let report = project.run(Command::Check, &allowed(Source::WorkingDirectory));
    assert_eq!(
        lines(&report),
        [
            "src/a.txt:1:B025: file is not valid UTF-8: invalid utf-8 sequence of 1 bytes from index 3",
            "src/b.txt:B029: file differs from the output of formatter `fixture`; run `bearout format`",
            "src/broken/.editorconfig:B023: `.editorconfig` has a line that is neither a section header, a property, nor a comment",
            "src/weird.txt:B023: `charset = latin1` is not a value Bearout can enforce; remove the property, set it to `unset`, or exclude the file from the selection",
        ]
    );
    // The same gating governs the write: nothing undecodable or unconfigured
    // reaches the formatter or the tree.
    let report = project.run(Command::Format, &format_options());
    assert_eq!(report.formatted, ["src/b.txt"]);
    assert_eq!(
        std::fs::read(project.path().join("src/a.txt")).unwrap(),
        b"caf\xe9\n"
    );
    assert_eq!(project.read("src/weird.txt"), "x\n");
    assert_eq!(project.read("src/broken/c.txt"), "lower\n");
}

#[test]
fn repeated_oversized_files_exhaust_the_total_budget() {
    // Each rejected read still costs its probe (the limit plus one byte),
    // so oversized files alone can exhaust the total, and the boundary
    // reached is the one reported.
    let project = text_project();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nfile_bytes = 100\nhygiene_bytes = 400\n",
            hygiene_bootstrap("scope = \"declared\"\nroots = [\"big\"]")
        ),
    );
    project.remove(".editorconfig");
    for name in ["a", "b", "c", "d"] {
        project.file(&format!("big/{name}.txt"), &"x".repeat(150));
    }
    // 101 + 101 + 101 = 303 charged; the fourth read is bounded to the 97
    // bytes left, pulls 98, and the budget is what stops it.
    assert_fatal(
        &project.check(),
        "hygiene inputs exceed `limits.hygiene_bytes` = 400 while reading `big/d.txt`",
    );
    // With room for every probe, each file is its own B024 instead.
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\nfile_bytes = 100\nhygiene_bytes = 1000\n",
            hygiene_bootstrap("scope = \"declared\"\nroots = [\"big\"]")
        ),
    );
    let report = project.check();
    assert_eq!(report.errors(), 4);
    assert!(
        codes(&report)
            .iter()
            .all(|code| *code == bearout::Code::FileUnreadable)
    );
    assert_line(
        &report,
        "big/a.txt:B024: file is 150 bytes, above `limits.file_bytes` = 100",
    );
}

#[test]
fn exact_exhaustion_then_a_nonempty_file_is_fatal_from_every_source() {
    // Once the budget is spent to the byte, the next nonempty file reaches
    // the budget boundary, not the file limit, and it does so identically
    // whether the tree pulled a probe byte (the working directory) or
    // decided from the recorded size without loading anything (Git).
    let project = text_project();
    let bootstrap = |hygiene_bytes: u32| {
        format!(
            "{}\n[limits]\nfile_bytes = 100\nhygiene_bytes = {hygiene_bytes}\n",
            hygiene_bootstrap("scope = \"declared\"\nroots = [\"big\"]")
        )
    };
    project.file("bearout.toml", &bootstrap(100));
    project.remove(".editorconfig");
    project.file("big/a.txt", &format!("{}\n", "x".repeat(99)));
    project.file("big/b.txt", "y\n");
    project.git_init();
    project.commit_all("base");
    let sources = [
        Source::WorkingDirectory,
        Source::Index,
        Source::Revision("HEAD".to_owned()),
    ];
    for source in &sources {
        assert_fatal(
            &project.check_from(source.clone()),
            "hygiene inputs exceed `limits.hygiene_bytes` = 100 while reading `big/b.txt`",
        );
    }
    // With exactly enough for both, the budget bounds the second read at
    // its own size and the run is clean from every source.
    project.file("bearout.toml", &bootstrap(102));
    project.commit_all("room");
    for source in &sources {
        assert_clean(&project.check_from(source.clone()));
    }
}
