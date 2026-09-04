// SPDX-License-Identifier: Apache-2.0

//! Schema-less documents: explicit selection through `[documents]`,
//! classification against resources, limits, symbolic-link rules, and the
//! shared Markdown model.

mod common;

use std::fs;

use bearout::{Code, Source};
use common::{ENTRY, Project, assert_clean, assert_fatal, assert_line, assert_no_line, codes};

/// The default bootstrap plus a `[documents]` table.
fn bootstrap(documents: &str) -> String {
    format!("{}\n[documents]\n{documents}\n", common::BOOTSTRAP)
}

/// A note project that selects `README.md` and everything beneath `docs`.
fn documented_project() -> Project {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &bootstrap("roots = [\"docs\"]\nfiles = [\"README.md\"]"),
    );
    project.file(
        "README.md",
        "# Read me\n\nSee [the guide](docs/guide.md#usage).\n",
    );
    project.file(
        "docs/guide.md",
        "# Guide\n\n## Usage\n\nBack to [the readme](../README.md).\n",
    );
    project.file("docs/deeper/notes.md", "# Notes\n");
    project.file("docs/data.json", "{}\n");
    project.file("docs/plain.txt", "not markdown\n");
    project
}

#[test]
fn an_absent_documents_table_changes_nothing() {
    let project = Project::with_note();
    project.file("README.md", "# Read me\n\n[broken](missing.md)\n");
    let report = project.check();
    assert_clean(&report);
    assert_eq!(report.documents, 0);
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["documents"], 0);
    assert_eq!(json["resources"], 1);
}

#[test]
fn documents_are_selected_explicitly_and_counted() {
    let project = documented_project();
    let report = project.check();
    assert_clean(&report);
    assert_eq!(report.resources, 1);
    assert_eq!(
        report.documents, 3,
        "README.md, docs/guide.md, docs/deeper/notes.md; not JSON or text files"
    );
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["documents"], 3);

    // Selection is explicit: a Markdown file outside every grant is not a document.
    project.file("CHANGELOG.md", "# Changes\n");
    project.file("other/x.md", "# Other\n");
    assert_eq!(project.check().documents, 3);

    // A file listed twice, once by name and once beneath a root, is one document.
    project.file(
        "bearout.toml",
        &bootstrap("roots = [\"docs\"]\nfiles = [\"README.md\", \"docs/guide.md\"]"),
    );
    assert_eq!(project.check().documents, 3);
}

#[test]
fn resources_take_precedence_over_documents() {
    let project = Project::with_note();
    // The resource root is also a document root, and a resource is also
    // listed by name: every such path is processed once, as a resource.
    project.file(
        "bearout.toml",
        &bootstrap("roots = [\"content\"]\nfiles = [\"content/note-a.md\"]"),
    );
    project.file("content/readme.md", "# Not a resource\n");
    let report = project.check();
    assert_eq!(
        report.resources, 2,
        "every Markdown file under a resource root is a resource"
    );
    assert_eq!(report.documents, 0);
    assert_line(
        &report,
        "content/readme.md:B002: resource must begin with TOML front matter",
    );
    assert_no_line(&report, "B022");
}

#[test]
fn malformed_declarations_and_missing_paths_fail_closed() {
    let project = documented_project();
    project.file("bearout.toml", &bootstrap("files = [\"MISSING.md\"]"));
    assert_fatal(
        &project.check(),
        "document `MISSING.md` is not a file inside the project",
    );
    project.file("bearout.toml", &bootstrap("roots = [\"nowhere\"]"));
    assert_fatal(
        &project.check(),
        "document root `nowhere` is not a directory inside the project",
    );
    project.file("bearout.toml", &bootstrap("files = [\"docs\"]"));
    assert_fatal(&project.check(), "must be a `.md` document");
    project.file("bearout.toml", &bootstrap("files = [\"docs/plain.txt\"]"));
    assert_fatal(&project.check(), "must be a `.md` document");
    project.file(
        "bearout.toml",
        &bootstrap("roots = [\"docs\", \"docs/deeper\"]"),
    );
    assert_fatal(&project.check(), "overlap");
    project.file("bearout.toml", &bootstrap("pattern = \"*.md\""));
    assert_fatal(&project.check(), "unknown key `documents.pattern`");
    // A root that is a file is not a directory.
    project.file("bearout.toml", &bootstrap("roots = [\"README.md\"]"));
    assert_fatal(&project.check(), "is not a directory");
}

#[test]
fn document_limits_are_separate_from_resource_limits() {
    let project = documented_project();
    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\ndocuments = 2\n",
            bootstrap("roots = [\"docs\"]\nfiles = [\"README.md\"]")
        ),
    );
    assert_fatal(
        &project.check(),
        "3 documents exceed `limits.documents` = 2",
    );

    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\ndocument_bytes = 20\nresources = 1\n",
            bootstrap("roots = [\"docs\"]\nfiles = [\"README.md\"]")
        ),
    );
    let report = project.check();
    assert!(report.fatal.is_none(), "{:?}", report.fatal);
    assert_line(&report, "README.md:B022: document is ");
    assert_line(&report, " bytes, above `limits.document_bytes` = 20");
    assert_line(&report, "docs/guide.md:B022");
    assert_no_line(&report, "docs/deeper/notes.md:B022");
    assert_eq!(report.documents, 3, "the count is of selected documents");
    assert!(
        codes(&report)
            .iter()
            .all(|code| *code == Code::DocumentUnreadable)
    );
}

#[test]
fn documents_must_be_utf8_and_a_bom_is_tolerated() {
    let project = documented_project();
    project.bytes("docs/latin1.md", b"# Caf\xe9\n");
    let report = project.check();
    assert_line(&report, "docs/latin1.md:B022: document is not valid UTF-8");
    assert_eq!(report.errors(), 1);
    project.remove("docs/latin1.md");
    project.file(
        "docs/bom.md",
        "\u{feff}# With BOM\r\n\r\nSee [usage](guide.md#usage).\r\n",
    );
    let report = project.check();
    assert_clean(&report);
    assert_eq!(report.documents, 4);
}

#[cfg(unix)]
#[test]
fn documents_are_never_reached_through_symbolic_links() {
    let outside = tempfile::tempdir().expect("outside");
    fs::write(outside.path().join("secret.md"), "# Secret\n").expect("write");
    fs::create_dir_all(outside.path().join("tree")).expect("dir");
    fs::write(outside.path().join("tree/leaked.md"), "# Leaked\n").expect("write");

    // Discovery skips a linked file and a linked directory beneath a root.
    let project = documented_project();
    std::os::unix::fs::symlink(
        outside.path().join("secret.md"),
        project.path().join("docs/link.md"),
    )
    .expect("symlink");
    std::os::unix::fs::symlink(
        outside.path().join("tree"),
        project.path().join("docs/linked-dir"),
    )
    .expect("symlink");
    let report = project.check();
    assert_clean(&report);
    assert_eq!(report.documents, 3);

    // A declared file must not be, or pass through, a symbolic link.
    project.file("bearout.toml", &bootstrap("files = [\"docs/link.md\"]"));
    assert_fatal(
        &project.check(),
        "document `docs/link.md` is reached through the symbolic link `docs/link.md`",
    );
    project.file(
        "bearout.toml",
        &bootstrap("files = [\"docs/linked-dir/leaked.md\"]"),
    );
    assert_fatal(&project.check(), "symbolic link `docs/linked-dir`");
    // A root that is itself a link is refused as a directory of the project.
    std::os::unix::fs::symlink(
        outside.path().join("tree"),
        project.path().join("linked-root"),
    )
    .expect("symlink");
    project.file("bearout.toml", &bootstrap("roots = [\"linked-root\"]"));
    let report = project.check();
    assert!(
        report.fatal.is_some() || report.documents == 0,
        "a linked root exposes nothing: {report:?}"
    );
}

#[test]
fn the_shared_markdown_model_reaches_resources_too() {
    let project = Project::with_note();
    project.file(
        ENTRY,
        "def v(r):\n    out = []\n    for l in r[\"links\"]:\n        out.append(warning(\"link \" + l[\"text\"] + \" -> \" + l[\"target\"], line = l[\"line\"]))\n    for i in r[\"images\"]:\n        out.append(warning(\"image \" + i[\"alt\"] + \" -> \" + i[\"target\"], line = i[\"line\"]))\n    for a in r[\"anchors\"]:\n        out.append(warning(\"anchor \" + a[\"id\"], line = a[\"line\"]))\n    return out\nschema(\"example/test/note@1\", shape = \"note.schema.toml\", validate = v)\n",
    );
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n<a id=\"start\"></a>\n\n# A\n\nSee [the *start*](#start) and ![A chart](chart.svg).\n",
    );
    project.file("content/chart.svg", "<svg/>\n");
    let report = project.check();
    assert_clean(&report);
    assert_line(
        &report,
        "content/note-a.md:11:B016: schema `example/test/note@1` validate: link the start -> #start",
    );
    assert_line(
        &report,
        "content/note-a.md:11:B016: schema `example/test/note@1` validate: image A chart -> chart.svg",
    );
    assert_line(
        &report,
        "content/note-a.md:7:B016: schema `example/test/note@1` validate: anchor start",
    );
}

#[test]
fn documents_come_from_the_selected_source() {
    let project = documented_project();
    project.git_init();
    project.commit_all("documents");
    // An unstaged extra document is invisible to the index and the revision.
    project.file("docs/extra.md", "# Extra\n");
    assert_eq!(project.check().documents, 4);
    assert_eq!(project.check_from(Source::Index).documents, 3);
    assert_eq!(
        project
            .check_from(Source::Revision("HEAD".to_owned()))
            .documents,
        3
    );
    // A declared file deleted from the working directory is still a
    // document of the index and the revision.
    project.remove("README.md");
    assert_fatal(&project.check(), "document `README.md` is not a file");
    assert_eq!(project.check_from(Source::Index).documents, 3);
    assert_clean(&project.check_from(Source::Revision("HEAD".to_owned())));
}
