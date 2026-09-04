// SPDX-License-Identifier: Apache-2.0

//! CLI smoke tests: exit codes, JSON output for every outcome, and the
//! source flags.

mod common;

use std::process::Command;

use common::Project;

fn bearout(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_bearout"))
        .args(args)
        .output()
        .expect("run bearout");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn exit_codes_follow_the_outcome() {
    let project = Project::fixture("valid-minimal");
    let path = project.path().to_str().expect("utf-8 path");
    let (code, stdout, _) = bearout(&["check", path]);
    assert_eq!(code, 0);
    assert!(stdout.contains("checked 2 resource(s): clean"));

    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/fixture/note@1\"\nid = \"note-b\"\ntitle = \"B\"\n+++\n",
    );
    let (code, _, stderr) = bearout(&["check", path]);
    assert_eq!(code, 1);
    assert!(stderr.contains("content/note-b.md:B015[empty-body]"));

    let empty = tempfile::tempdir().expect("dir");
    let (code, _, stderr) = bearout(&["check", empty.path().to_str().expect("path")]);
    assert_eq!(code, 2);
    assert!(stderr.starts_with("bearout: cannot read bearout.toml"));
}

#[test]
fn json_is_valid_for_every_outcome() {
    let project = Project::fixture("valid-minimal");
    let path = project.path().to_str().expect("utf-8 path");
    let (code, stdout, _) = bearout(&["--format", "json", "check", path]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(code, 0);
    assert_eq!(json["ok"], true);
    assert_eq!(json["resources"], 2);

    let empty = tempfile::tempdir().expect("dir");
    let (code, stdout, _) = bearout(&[
        "--format",
        "json",
        "check",
        empty.path().to_str().expect("path"),
    ]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json on fatal");
    assert_eq!(code, 2);
    assert_eq!(json["ok"], false);
    assert!(
        json["fatal"]
            .as_str()
            .expect("fatal message")
            .contains("bearout.toml")
    );

    project.file("bearout.toml", "version = \"one\"\n");
    let (code, stdout, _) = bearout(&["--format", "json", "generate", "--check", path]);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("valid json on invalid manifest");
    assert_eq!(code, 2);
    assert!(json["fatal"].as_str().expect("fatal").contains("version"));

    project.file("bearout.toml", "version = 1\nentry = \"bearout.star\"\n[resources]\nroots = [\"content\"]\n[rules]\nroot = \"rules\"\n[outputs]\nroots = [\"content/out\"]\n");
    let (code, stdout, _) = bearout(&["--format", "json", "check", path]);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("valid json on path boundary");
    assert_eq!(code, 2);
    assert!(json["fatal"].as_str().expect("fatal").contains("overlap"));

    project.file("bearout.toml", "version = 1\nentry = \"bearout.star\"\n[resources]\nroots = [\"content\"]\n[rules]\nroot = \"rules\"\n");
    project.file("bearout.star", "this is not starlark\n");
    let (code, stdout, _) = bearout(&["--format", "json", "check", path]);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("valid json on script failure");
    assert_eq!(code, 1);
    assert_eq!(json["diagnostics"][0]["code"], "B012");
    assert_eq!(json["diagnostics"][0]["severity"], "error");
}

// ---- source selection ---------------------------------------------------

/// A committed generating project for the source flags.
fn committed_project() -> Project {
    committed_project_at("")
}

/// A committed generating project rooted `relative` beneath the repository.
fn committed_project_at(relative: &str) -> Project {
    let project = Project::at(relative);
    project.file("rules/note.schema.toml", common::NOTE_SHAPE);
    project.file("content/note-a.md", "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n# A\n\nBody.\n");
    project.file("bearout.toml", common::BOOTSTRAP_GEN);
    project.file(
        "templates/page.md.j2",
        "{% for line in bearout.header %}<!-- {{ line }} -->\n{% endfor %}\n# {{ title }}\n",
    );
    project.file(
        common::ENTRY,
        "def g(p):\n    return [output(\"page.md.j2\", \"generated/a.md\", context = {\"title\": \"A\"})]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ngenerator(\"pages\", g)\n",
    );
    assert!(bearout::generate(project.path(), bearout::Mode::Write).is_clean());
    project.git_init();
    project.commit_all("clean");
    project
}

#[test]
fn source_flags_select_the_tree_and_keep_exit_codes() {
    let project = committed_project();
    let path = project.path().to_str().expect("utf-8 path");
    let broken = "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = 3\n+++\n";

    // Unstaged breakage: working directory fails, index and revision pass.
    project.file("content/note-a.md", broken);
    let (code, _, stderr) = bearout(&["check", path]);
    assert_eq!(code, 1, "{stderr}");
    let (code, stdout, stderr) = bearout(&["check", "--index", path]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("checked 1 resource(s): clean"));
    let (code, stdout, _) = bearout(&["generate", "--check", "--revision", "HEAD", path]);
    assert_eq!(code, 0);
    assert!(stdout.contains("1 output(s) verified"));

    // Staged breakage: index fails with a contract diagnostic, revision passes.
    project.git(&["add", "content/note-a.md"]);
    let (code, _, stderr) = bearout(&["check", "--index", path]);
    assert_eq!(code, 1);
    assert!(stderr.contains("content/note-a.md:4:B005"), "{stderr}");
    let (code, _, _) = bearout(&["check", "--revision", "main", path]);
    assert_eq!(code, 0);

    // Path may follow or precede the flags.
    let (code, _, _) = bearout(&["check", path, "--index"]);
    assert_eq!(code, 1);
}

#[test]
fn source_failures_are_fatal_with_valid_json() {
    let project = committed_project();
    let path = project.path().to_str().expect("utf-8 path");

    let (code, stdout, stderr) =
        bearout(&["--format", "json", "check", "--revision", "nope", path]);
    assert_eq!(code, 2);
    assert!(stderr.is_empty(), "{stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["ok"], false);
    assert!(
        json["fatal"]
            .as_str()
            .unwrap()
            .contains("`nope` is not a revision")
    );
    assert!(json.get("source").is_none());

    let (code, _, stderr) = bearout(&["check", "--revision", "nope", path]);
    assert_eq!(code, 2);
    assert_eq!(
        stderr.trim(),
        "bearout: cannot read Git revision: `nope` is not a revision of this repository"
    );

    let outside = tempfile::tempdir().expect("dir");
    let (code, stdout, _) = bearout(&[
        "--format",
        "json",
        "check",
        "--index",
        outside.path().to_str().expect("path"),
    ]);
    assert_eq!(code, 2);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let fatal = json["fatal"].as_str().unwrap();
    assert!(
        fatal.starts_with("cannot read the Git index: git rev-parse failed: "),
        "{fatal}"
    );
    assert!(!fatal.contains('\n'));

    // A revision spelled with a leading dash never reaches Git as an option.
    let (code, stdout, _) = bearout(&["--format", "json", "check", "--revision=--output=x", path]);
    assert_eq!(code, 2);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert!(
        json["fatal"]
            .as_str()
            .unwrap()
            .contains("is not a revision name")
    );
    assert!(!project.exists("x"));
}

#[test]
fn json_reports_carry_the_git_source() {
    let project = committed_project();
    let path = project.path().to_str().expect("utf-8 path");
    let (code, stdout, _) = bearout(&["--format", "json", "generate", "--check", "--index", path]);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["source"]["kind"], "index");
    assert!(
        json["source"]["digest"]
            .as_str()
            .unwrap()
            .starts_with("blake3:")
    );
    assert!(json["source"].get("revision").is_none());
    assert_eq!(json["outputs"], serde_json::json!(["generated/a.md"]));

    let (code, stdout, _) = bearout(&["--format", "json", "check", "--revision", "main", path]);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["source"]["kind"], "revision");
    assert_eq!(json["source"]["revision"], "main");
    assert_eq!(
        json["source"]["tree"],
        project.git(&["rev-parse", "HEAD^{tree}"])
    );

    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = 3\n+++\n",
    );
    project.git(&["add", "content/note-a.md"]);
    let (code, stdout, _) = bearout(&["--format", "json", "check", "--index", path]);
    assert_eq!(code, 1);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json on diagnostics");
    assert_eq!(json["source"]["kind"], "index");
    assert_eq!(json["diagnostics"][0]["code"], "B005");
}

#[test]
fn write_generation_and_conflicting_flags_are_invocation_errors() {
    let project = committed_project();
    let path = project.path().to_str().expect("utf-8 path");
    let state = project.read("bearout-state.toml");

    let (code, _, stderr) = bearout(&["generate", "--index", path]);
    assert_eq!(code, 2);
    assert!(stderr.contains("read-only"), "{stderr}");
    let (code, stdout, _) = bearout(&["--format", "json", "generate", "--revision", "main", path]);
    assert_eq!(code, 2);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert!(json["fatal"].as_str().unwrap().contains("read-only"));
    assert_eq!(project.read("bearout-state.toml"), state);

    let (code, _, stderr) = bearout(&["check", "--index", "--revision", "main", path]);
    assert_eq!(code, 2);
    assert!(stderr.contains("cannot be used with"), "{stderr}");
    let (code, _, _) = bearout(&["check", "--revision", path]);
    assert_eq!(code, 2);
}

#[test]
fn help_text_documents_the_sources() {
    let (code, stdout, _) = bearout(&["check", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--index"));
    assert!(stdout.contains("--revision <REV>"));
    assert!(stdout.contains("Git index"));
    let (code, stdout, _) = bearout(&["generate", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Required with --index or --revision"));
}

#[test]
fn git_index_file_is_honoured_only_inside_the_own_repository() {
    let project = committed_project();
    let path = project.path().to_str().expect("utf-8 path");
    let git_dir = project.git(&["rev-parse", "--absolute-git-dir"]);
    let temp_index = std::path::Path::new(&git_dir).join("bearout-test-index");
    std::fs::copy(std::path::Path::new(&git_dir).join("index"), &temp_index).expect("copy");
    // Stage a breaking change into the temporary index only.
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = 3\n+++\n",
    );
    let status = common::git_command(project.path())
        .env("GIT_INDEX_FILE", &temp_index)
        .args(["add", "content/note-a.md"])
        .status()
        .expect("git add");
    assert!(status.success());
    let run = |index: Option<&std::path::Path>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bearout"));
        command
            .args(["check", "--index", path])
            .env_remove("GIT_DIR");
        match index {
            Some(index) => command.env("GIT_INDEX_FILE", index),
            None => command.env_remove("GIT_INDEX_FILE"),
        };
        let output = command.output().expect("run bearout");
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    let (code, _) = run(None);
    assert_eq!(code, 0, "the real index is clean");
    let (code, stderr) = run(Some(&temp_index));
    assert_eq!(code, 1, "the hook's index is checked: {stderr}");
    assert!(stderr.contains("B005"));

    let foreign = tempfile::tempdir().expect("foreign");
    let foreign_index = foreign.path().join("index");
    std::fs::copy(&temp_index, &foreign_index).expect("copy");
    let (code, _) = run(Some(&foreign_index));
    assert_eq!(code, 0, "an index outside the repository is ignored");

    // A path that names the Git directory lexically but resolves elsewhere.
    let sideways = project.repo_path().join("sideways-index");
    std::fs::copy(&temp_index, &sideways).expect("copy");
    let dotdot = std::path::Path::new(&git_dir)
        .join("..")
        .join("sideways-index");
    let (code, _) = run(Some(&dotdot));
    assert_eq!(code, 0, "`..` out of the Git directory is ignored");

    // A directory is not an index file.
    let (code, _) = run(Some(std::path::Path::new(&git_dir)));
    assert_eq!(code, 0, "a directory is ignored");

    // A nonexistent file is ignored rather than treated as an empty index.
    let (code, _) = run(Some(&std::path::Path::new(&git_dir).join("absent-index")));
    assert_eq!(code, 0, "a missing file is ignored");

    #[cfg(unix)]
    {
        let link = std::path::Path::new(&git_dir).join("linked-index");
        std::os::unix::fs::symlink(&temp_index, &link).expect("symlink");
        let (code, _) = run(Some(&link));
        assert_eq!(
            code, 0,
            "a symbolic link inside the Git directory is ignored"
        );
        let outward = std::path::Path::new(&git_dir).join("outward-index");
        std::os::unix::fs::symlink(&foreign_index, &outward).expect("symlink");
        let (code, _) = run(Some(&outward));
        assert_eq!(
            code, 0,
            "a symbolic link leaving the Git directory is ignored"
        );
    }
}

#[test]
fn hostile_git_environment_variables_cannot_redirect_a_run() {
    let project = committed_project_at("packages/docs");
    let path = project.path().to_str().expect("utf-8 path");
    let other = committed_project();
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = 3\n+++\n",
    );
    // Stage the breakage into the other repository's index only, so that a
    // run redirected there would report B005.
    std::fs::copy(
        project.path().join("content/note-a.md"),
        other.path().join("content/note-a.md"),
    )
    .expect("copy");
    other.git(&["add", "content/note-a.md"]);
    let other_git_dir = other.git(&["rev-parse", "--absolute-git-dir"]);
    let bogus = tempfile::tempdir().expect("bogus");
    let global = bogus.path().join("gitconfig");
    std::fs::write(&global, "[core]\n\tbare = true\n").expect("config");

    let hostile = |command: &mut Command| {
        command
            .env("GIT_DIR", &other_git_dir)
            .env("GIT_WORK_TREE", other.path())
            .env(
                "GIT_INDEX_FILE",
                std::path::Path::new(&other_git_dir).join("index"),
            )
            .env("GIT_COMMON_DIR", &other_git_dir)
            .env("GIT_OBJECT_DIRECTORY", bogus.path().join("objects"))
            .env(
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                bogus.path().join("alternates"),
            )
            .env("GIT_NAMESPACE", "hostile")
            .env("GIT_CEILING_DIRECTORIES", project.repo_path())
            .env("GIT_CONFIG_GLOBAL", &global)
            .env("GIT_CONFIG_SYSTEM", &global)
            .env("GIT_CONFIG_PARAMETERS", "'core.bare=true'")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.bare")
            .env("GIT_CONFIG_VALUE_0", "true")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .env("GIT_ICASE_PATHSPECS", "1")
            .env("GIT_TRACE", bogus.path().join("trace").to_str().unwrap())
            .env("GIT_TRACE2", bogus.path().join("trace2").to_str().unwrap());
    };

    // The environment really is hostile: plain Git cannot even find the
    // project's repository under it.
    let mut probe = Command::new("git");
    probe
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(project.path());
    hostile(&mut probe);
    let probe = probe.output().expect("git");
    assert_ne!(
        String::from_utf8_lossy(&probe.stdout).trim(),
        project
            .repo_path()
            .canonicalize()
            .unwrap()
            .to_string_lossy(),
        "the probe should not resolve to the project's repository"
    );
    // The probe itself honours the tracing variables; Bearout's Git must not.
    let _ = std::fs::remove_file(bogus.path().join("trace"));
    let _ = std::fs::remove_file(bogus.path().join("trace2"));

    let index: &[&str] = &["check", "--index", path];
    let head: &[&str] = &["check", "--revision", "HEAD", path];
    for args in [index, head] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_bearout"));
        command.args(args);
        hostile(&mut command);
        let output = command.output().expect("run bearout");
        assert_eq!(
            output.status.code(),
            Some(0),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("checked 1 resource(s): clean"));
    }
    assert!(
        !bogus.path().join("trace").exists(),
        "tracing to a file is disabled"
    );
    assert!(!bogus.path().join("trace2").exists());
}

#[test]
fn index_snapshots_leave_no_temporary_files_behind() {
    let project = committed_project();
    let path = project.path().to_str().expect("utf-8 path");
    let leftovers = |pid: u32| {
        std::fs::read_dir(std::env::temp_dir())
            .expect("temp dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("bearout-index-{pid}-"))
            })
            .count()
    };
    let check: &[&str] = &["check", "--index", path];
    let verify: &[&str] = &["generate", "--check", "--index", path];
    for args in [check, verify] {
        let child = Command::new(env!("CARGO_BIN_EXE_bearout"))
            .args(args)
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn bearout");
        let pid = child.id();
        let status = child.wait_with_output().expect("wait").status;
        assert_eq!(status.code(), Some(0), "{args:?}");
        assert_eq!(leftovers(pid), 0, "{args:?} left an index snapshot behind");
    }
}

#[test]
fn document_counts_appear_in_text_and_json() {
    let project = Project::with_note();
    let path = project.path().to_str().expect("utf-8 path");
    let (code, stdout, _) = bearout(&["check", path]);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), "checked 1 resource(s): clean");
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[documents]\nfiles = [\"README.md\", \"NOTES.md\"]\n",
            common::BOOTSTRAP
        ),
    );
    project.file("README.md", "# Read me\n\n[notes](NOTES.md#notes)\n");
    project.file("NOTES.md", "# Notes\n");
    let (code, stdout, _) = bearout(&["check", path]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout.trim(),
        "checked 1 resource(s) and 2 document(s): clean"
    );
    let (_, stdout, _) = bearout(&["--format", "json", "check", path]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(json["documents"], 2);
    project.file("README.md", "# Read me\n\n[notes](NOTES.md#missing)\n");
    let (code, _, stderr) = bearout(&["check", path]);
    assert_eq!(code, 1);
    assert!(stderr.contains("README.md:3:B011"), "{stderr}");
    assert!(
        stderr.contains("checked 1 resource(s) and 2 document(s): 1 error(s)"),
        "{stderr}"
    );
}

// ---- hygiene and formatting -------------------------------------------------

fn fixture_formatter() -> String {
    env!("CARGO_BIN_EXE_bearout-fixture-formatter").replace('\\', "/")
}

/// A project selecting `text/` with a strict `.editorconfig` and an
/// uppercasing formatter over `src/*.txt`.
fn hygiene_project() -> Project {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"text\", \"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{}\", \"upper\"]\npaths = [\"src\"]\nextensions = [\"txt\"]\n",
            common::BOOTSTRAP,
            fixture_formatter()
        ),
    );
    project.file(".editorconfig", "root = true\n\n[*]\nend_of_line = lf\ninsert_final_newline = true\ntrim_trailing_whitespace = true\n");
    project.file("text/messy.txt", "a  \r\n");
    project.file("src/lower.txt", "lower\n");
    project
}

#[test]
fn hygiene_exit_codes_and_json_cover_every_outcome() {
    let project = hygiene_project();
    let path = project.path().to_str().expect("utf-8 path");

    // Differences: exit 1, with the codes in JSON.
    let (code, stdout, _) = bearout(&["--format", "json", "--allow-formatters", "check", path]);
    assert_eq!(code, 1);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    let codes: Vec<&str> = json["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes, ["B029", "B026", "B028"]);
    assert_eq!(json["files"], 2);
    let (code, _, stderr) = bearout(&["--allow-formatters", "check", path]);
    assert_eq!(code, 1);
    assert!(stderr.contains("text/messy.txt:1:B026"), "{stderr}");
    assert!(
        stderr.contains("checked 1 resource(s): 3 error(s)"),
        "{stderr}"
    );

    // Unauthorized formatters and a missing executable: exit 2, JSON valid.
    let (code, stdout, _) = bearout(&["--format", "json", "check", path]);
    assert_eq!(code, 2);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert!(
        json["fatal"]
            .as_str()
            .unwrap()
            .contains("--allow-formatters")
    );
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"src\"]\n\n[[formatters]]\nname = \"gone\"\ncommand = [\"bearout-no-such-executable\"]\n",
            common::BOOTSTRAP
        ),
    );
    let (code, stdout, _) = bearout(&["--format", "json", "--allow-formatters", "check", path]);
    assert_eq!(code, 2);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert!(json["fatal"].as_str().unwrap().contains("cannot start"));
    // A malformed hygiene declaration: exit 2.
    project.file(
        "bearout.toml",
        &format!("{}\n[hygiene]\nscope = \"everything\"\n", common::BOOTSTRAP),
    );
    let (code, _, stderr) = bearout(&["check", path]);
    assert_eq!(code, 2);
    assert!(stderr.contains("hygiene.scope"), "{stderr}");
    // A repository-wide selection outside Git: exit 2.
    project.file(
        "bearout.toml",
        &format!("{}\n[hygiene]\nscope = \"repository\"\n", common::BOOTSTRAP),
    );
    let (code, _, stderr) = bearout(&["check", path]);
    assert_eq!(code, 2);
    assert!(
        stderr.contains("needs the project inside a Git repository"),
        "{stderr}"
    );
}

#[test]
fn format_writes_then_checks_clean_and_reports_in_text_and_json() {
    let project = hygiene_project();
    let path = project.path().to_str().expect("utf-8 path");
    let (code, _, stderr) = bearout(&["format", path]);
    assert_eq!(code, 2, "formatters need authorization for writes too");
    assert!(stderr.contains("--allow-formatters"));
    assert_eq!(project.read("text/messy.txt"), "a  \r\n");

    let (code, stdout, stderr) = bearout(&["--allow-formatters", "format", path]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout,
        "formatted src/lower.txt\nformatted text/messy.txt\nformatted 2 of 2 selected file(s)\n"
    );
    assert_eq!(project.read("text/messy.txt"), "a\n");
    assert_eq!(project.read("src/lower.txt"), "LOWER\n");
    let (code, stdout, _) = bearout(&["--allow-formatters", "check", path]);
    assert_eq!(code, 0);
    assert!(stdout.contains("checked 1 resource(s): clean"));
    let (code, stdout, _) = bearout(&["--format", "json", "--allow-formatters", "format", path]);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(json["formatted"], serde_json::json!([]));
    assert_eq!(json["ok"], true);

    // A format that cannot rewrite a file: exit 1 with B031/B030 and valid JSON.
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[hygiene]\nscope = \"declared\"\nroots = [\"src\"]\n\n[[formatters]]\nname = \"fixture\"\ncommand = [\"{}\", \"fail\"]\nextensions = [\"txt\"]\n",
            common::BOOTSTRAP,
            fixture_formatter()
        ),
    );
    let (code, stdout, stderr) =
        bearout(&["--format", "json", "--allow-formatters", "format", path]);
    assert_eq!(code, 1, "{stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("json");
    assert_eq!(json["diagnostics"][0]["code"], "B030");
    let (code, _, _) = bearout(&["--allow-formatters", "format", "--index", path]);
    assert_eq!(code, 2, "format takes no source flags");
    let (code, stdout, _) = bearout(&["format", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Working directory only"));
    let (code, stdout, _) = bearout(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--allow-formatters"));
}

#[test]
fn formatter_working_directories_leave_no_trace() {
    let project = hygiene_project();
    let path = project.path().to_str().expect("utf-8 path");
    let leftovers = |pid: u32| {
        std::fs::read_dir(std::env::temp_dir())
            .expect("temp dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&format!("bearout-format-{pid}-"))
            })
            .count()
    };
    for args in [
        vec!["--allow-formatters", "check", path],
        vec!["--allow-formatters", "format", path],
    ] {
        let child = Command::new(env!("CARGO_BIN_EXE_bearout"))
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn bearout");
        let pid = child.id();
        let status = child.wait_with_output().expect("wait").status;
        assert!(status.code().is_some_and(|code| code < 2), "{args:?}");
        assert_eq!(
            leftovers(pid),
            0,
            "{args:?} left a working directory behind"
        );
    }
    assert!(!project.path().join(".fixture-cache").exists());
}

// ---- contract fixtures --------------------------------------------------

/// A committed note project declaring one fixture file with a passing
/// case, a failing case, and a fatal-expecting case.
fn fixture_project() -> Project {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[fixtures]\nfiles = [\"contract-tests/notes.test.toml\"]\n",
            common::BOOTSTRAP
        ),
    );
    project.file(
        common::ENTRY,
        "def v(r):\n    if r[\"fields\"][\"title\"] == \"BAD\":\n        return [error(\"title is BAD\", code = \"bad-title\")]\n    return []\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    project.file(
        "contract-tests/notes.test.toml",
        "[[cases]]\nname = \"a good note is clean\"\nexpect = \"clean\"\n[[cases.mutations]]\nwrite = \"content/note-b.md\"\ncontent = \"+++\\nschema = \\\"example/test/note@1\\\"\\nid = \\\"note-b\\\"\\ntitle = \\\"B\\\"\\n+++\\n\"\n\n[[cases]]\nname = \"a bad note is reported\"\nexpect = \"diagnostics\"\n[[cases.mutations]]\nwrite = \"content/note-b.md\"\ncontent = \"+++\\nschema = \\\"example/test/note@1\\\"\\nid = \\\"note-b\\\"\\ntitle = \\\"BAD\\\"\\n+++\\n\"\n[[cases.diagnostics]]\ncode = \"B015\"\npath = \"content/note-b.md\"\nrule = \"bad-title\"\n\n[[cases]]\nname = \"no entry module is fatal\"\nexpect = \"fatal\"\nfatal = \"entry module\"\n[[cases.mutations]]\ndelete = \"bearout.star\"\n",
    );
    project.git_init();
    project.commit_all("fixtures");
    project
}

#[test]
fn test_exit_codes_and_output_cover_every_outcome() {
    let project = fixture_project();
    let path = project.path().to_str().expect("utf-8 path");

    // Every case passes: 0, one line per case, the summary on stdout.
    let (code, stdout, stderr) = bearout(&["test", path]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout,
        "ok   a good note is clean (contract-tests/notes.test.toml)\nok   a bad note is reported (contract-tests/notes.test.toml)\nok   no entry module is fatal (contract-tests/notes.test.toml)\ntested 3 case(s): 3 passed, 0 failed\n"
    );
    assert!(stderr.is_empty());
    let again = bearout(&["test", path]);
    assert_eq!((code, stdout.clone(), stderr), again, "byte-identical text");
    let (code, json_first, _) = bearout(&["--format", "json", "test", path]);
    assert_eq!(code, 0);
    let json: serde_json::Value = serde_json::from_str(&json_first).expect("valid json");
    assert_eq!(json["ok"], true);
    assert_eq!(json["total"], 3);
    assert_eq!(json["passed"], 3);
    assert_eq!(json["failed"], 0);
    assert_eq!(json["cases"][1]["name"], "a bad note is reported");
    assert_eq!(json["cases"][1]["expected"], "diagnostics");
    assert_eq!(json["cases"][1]["actual"], "diagnostics");
    assert_eq!(
        json["cases"][2]["fatal"]
            .as_str()
            .map(|m| m.contains("entry module")),
        Some(true)
    );
    assert!(json.get("source").is_none());
    assert_eq!(
        json_first,
        bearout(&["--format", "json", "test", path]).1,
        "byte-identical json"
    );

    // The same suite from the index and a revision, with source identity.
    for args in [
        vec!["test", "--index", path],
        vec!["test", "--revision", "HEAD", path],
    ] {
        let (code, _, stderr) = bearout(&args);
        assert_eq!(code, 0, "{args:?}: {stderr}");
    }
    let (_, stdout, _) = bearout(&["--format", "json", "test", "--revision", "main", path]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["source"]["kind"], "revision");
    assert_eq!(json["source"]["revision"], "main");
    let (_, stdout, _) = bearout(&["--format", "json", "test", "--index", path]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(json["source"]["kind"], "index");

    // A failed case: 1, its details beneath it, the summary on stderr.
    project.file(
        "contract-tests/notes.test.toml",
        &project
            .read("contract-tests/notes.test.toml")
            .replace("name = \"a good note is clean\"\nexpect = \"clean\"", "name = \"a good note is clean\"\nexpect = \"diagnostics\"\n[[cases.diagnostics]]\ncode = \"B015\"\nrule = \"bad-title\""),
    );
    let (code, stdout, stderr) = bearout(&["test", path]);
    assert_eq!(code, 1);
    assert_eq!(
        stdout,
        "FAIL a good note is clean (contract-tests/notes.test.toml)\n     expected diagnostics, got clean\n     missing: B015 rule=bad-title\nok   a bad note is reported (contract-tests/notes.test.toml)\nok   no entry module is fatal (contract-tests/notes.test.toml)\n"
    );
    assert_eq!(stderr, "tested 3 case(s): 2 passed, 1 failed\n");
    let (code, stdout, _) = bearout(&["--format", "json", "test", path]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert_eq!(code, 1);
    assert_eq!(json["ok"], false);
    assert_eq!(json["failed"], 1);
    assert_eq!(json["cases"][0]["passed"], false);
    assert_eq!(json["cases"][0]["missing"][0]["code"], "B015");
    assert!(json["fatal"].is_null());
    // The committed suite still passes from the index and HEAD.
    assert_eq!(bearout(&["test", "--index", path]).0, 0);

    // A malformed suite: 2, only the reason, valid JSON with no cases.
    project.file("contract-tests/notes.test.toml", "[[cases]\n");
    let (code, stdout, stderr) = bearout(&["test", path]);
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert!(
        stderr
            .starts_with("bearout: fixture file `contract-tests/notes.test.toml`: not valid TOML"),
        "{stderr}"
    );
    let (code, stdout, _) = bearout(&["--format", "json", "test", path]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json on fatal");
    assert_eq!(code, 2);
    assert_eq!(json["ok"], false);
    assert_eq!(json["total"], 0);
    assert!(json["cases"].as_array().unwrap().is_empty());
    assert!(json["fatal"].as_str().unwrap().contains("not valid TOML"));

    // No grant, no source, and unexpected diagnostics under exact matching.
    project.file("bearout.toml", common::BOOTSTRAP);
    let (code, _, stderr) = bearout(&["test", path]);
    assert_eq!(code, 2);
    assert!(stderr.contains("declares no `[fixtures]`"));
    let (code, _, stderr) = bearout(&["test", "--revision", "nope", path]);
    assert_eq!(code, 2);
    assert!(stderr.contains("`nope` is not a revision"));
    let empty = tempfile::tempdir().expect("dir");
    let (code, stdout, _) = bearout(&[
        "--format",
        "json",
        "test",
        empty.path().to_str().expect("path"),
    ]);
    assert_eq!(code, 2);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    assert!(json["fatal"].as_str().unwrap().contains("bearout.toml"));
}

#[test]
fn test_takes_no_baseline_and_documents_itself() {
    let project = fixture_project();
    let path = project.path().to_str().expect("utf-8 path");
    let (code, _, stderr) = bearout(&["test", "--baseline", "HEAD", path]);
    assert_eq!(code, 2, "an unknown flag is an invocation error");
    assert!(stderr.contains("--baseline"));
    let (code, _, _) = bearout(&["test", "--index", "--revision", "HEAD", path]);
    assert_eq!(code, 2, "conflicting source flags");
    let (code, stdout, _) = bearout(&["test", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("--index"));
    assert!(stdout.contains("--revision <REV>"));
    assert!(stdout.contains("[fixtures]"));
    assert!(stdout.contains("Read-only"));
    assert!(!stdout.contains("--baseline"));
    let (code, stdout, _) = bearout(&["--help"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("test"));
    // The working directory is untouched by a run, whatever the outcome.
    let before = std::fs::read(project.path().join("content/note-a.md")).unwrap();
    assert!(!project.path().join("content/note-b.md").exists());
    bearout(&["test", path]);
    assert_eq!(
        std::fs::read(project.path().join("content/note-a.md")).unwrap(),
        before
    );
    assert!(!project.path().join("content/note-b.md").exists());
    assert!(project.path().join("bearout.star").exists());
    assert_eq!(project.git(&["status", "--porcelain"]), "");
}
