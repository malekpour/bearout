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
