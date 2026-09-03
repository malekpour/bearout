// SPDX-License-Identifier: Apache-2.0

//! CLI smoke tests: exit codes and JSON output for every outcome.

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
