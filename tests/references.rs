// SPDX-License-Identifier: Apache-2.0

//! Markdown reference checking across resources and schema-less documents:
//! links, images, fragments, root-relative and percent-encoded targets, the
//! verification boundary, and Phase 1 symbolic-link and gitlink rules.

mod common;

use bearout::{Code, Source};
use common::{Project, assert_clean, assert_line, assert_no_line, codes, lines};

fn bootstrap(documents: &str) -> String {
    format!("{}\n[documents]\n{documents}\n", common::BOOTSTRAP)
}

/// A note project selecting `docs` and `README.md` as documents, with an
/// image and a plain file beneath `docs`.
fn project() -> Project {
    let project = Project::with_note();
    project.file(
        "bearout.toml",
        &bootstrap("roots = [\"docs\"]\nfiles = [\"README.md\"]"),
    );
    project.file("README.md", "# Read me\n");
    project.file(
        "docs/guide.md",
        "# Guide\n\n## Usage\n\n## Usage\n\n## Ĉu vi parolas?\n\n<a id=\"explicit\"></a>\n<a name=\"legacy\"></a>\n",
    );
    project.file("docs/figures/flow.svg", "<svg/>\n");
    project.file("docs/plain.txt", "text\n");
    project
}

#[test]
fn links_resolve_across_documents_and_resources() {
    let project = project();
    project.file(
        "README.md",
        concat!(
            "# Read me\n\n",
            "## Local\n\n",
            "[same doc](#local) [dup](docs/guide.md#usage-1) [unicode](docs/guide.md#%C4%89u-vi-parolas)\n",
            "[explicit](docs/guide.md#explicit) [legacy](docs/guide.md#legacy)\n",
            "[root](/docs/guide.md#usage) [query](docs/guide.md?tab=1#usage) [encoded](docs/figures/flow%2Esvg)\n",
            "[dir](docs) [dir slash](docs/figures/) [plain](docs/plain.txt#any-fragment)\n",
            "[resource](content/note-a.md#a) [self file](README.md) [external](https://example.org/x#y) [mail](mailto:a@b.example)\n",
            "[reference style][ref] and ![Flow](docs/figures/flow.svg)\n\n",
            "[ref]: docs/guide.md#usage\n",
        ),
    );
    // A resource may link into a document too.
    project.file(
        "content/note-a.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-a\"\ntitle = \"A\"\n+++\n\n# A\n\nSee [the guide](../docs/guide.md#explicit) and [the readme](/README.md#local).\n",
    );
    let report = project.check();
    assert_clean(&report);
    assert_eq!(report.documents, 2);
}

#[test]
fn every_broken_reference_is_one_diagnostic_in_stable_order() {
    let project = project();
    project.file(
        "README.md",
        concat!(
            "# Read me\n\n",
            "[missing](docs/nope.md)\n",
            "[bad anchor](docs/guide.md#nope)\n",
            "[bad same](#nope)\n",
            "[escape](../outside.md)\n",
            "[root escape](/../x.md)\n",
            "![no image](docs/figures/none.svg)\n",
            "![dir image](docs/figures)\n",
            "[bad utf8](docs/%ff.md)\n",
            "[resource anchor](content/note-a.md#nope)\n",
        ),
    );
    let report = project.check();
    assert_eq!(
        lines(&report),
        [
            "README.md:3:B011: link `docs/nope.md` points at a missing file",
            "README.md:4:B011: link `docs/guide.md#nope` names anchor `nope`, which `docs/guide.md` does not define",
            "README.md:5:B011: link `#nope` names anchor `nope`, which `README.md` does not define",
            "README.md:6:B011: link `../outside.md`: `../outside.md` leaves the project",
            "README.md:7:B011: link `/../x.md`: `../x.md` leaves the project",
            "README.md:8:B011: image `docs/figures/none.svg` points at a missing file",
            "README.md:9:B011: image `docs/figures` points at a directory, not a file",
            "README.md:10:B011: link `docs/%ff.md`: `docs/%ff.md` does not decode to valid UTF-8",
            "README.md:11:B011: link `content/note-a.md#nope` names anchor `nope`, which `content/note-a.md` does not define",
        ]
    );
    assert!(
        codes(&report)
            .iter()
            .all(|code| *code == Code::UnresolvedLink)
    );
    assert_eq!(lines(&project.check()), lines(&report), "deterministic");
}

#[test]
fn anchors_are_verified_only_in_discovered_markdown() {
    let project = project();
    project.file("notes/outside.md", "# Outside\n\n## Real\n");
    project.file(
        "README.md",
        "# Read me\n\n[file only](notes/outside.md)\n[anchor](notes/outside.md#real)\n",
    );
    let report = project.check();
    assert_eq!(
        lines(&report),
        [
            "README.md:4:B011: link `notes/outside.md#real` names anchor `real` in `notes/outside.md`, which is not a discovered document; select it in `[documents]` to verify its anchors"
        ]
    );
    // Selecting the file makes the anchor verifiable.
    project.file(
        "bearout.toml",
        &bootstrap("roots = [\"docs\", \"notes\"]\nfiles = [\"README.md\"]"),
    );
    assert_clean(&project.check());
}

#[test]
fn consequential_diagnostics_are_suppressed() {
    let project = project();
    // A document that failed to read and a resource that failed its envelope
    // are each reported once; links into them are not reported again.
    project.bytes("docs/broken.md", b"# Caf\xe9\n");
    project.file("content/note-b.md", "# no front matter\n");
    project.file(
        "README.md",
        "# Read me\n\n[into broken](docs/broken.md#anything)\n[into invalid](content/note-b.md#anything)\n",
    );
    let report = project.check();
    assert_line(&report, "docs/broken.md:B022");
    assert_line(&report, "content/note-b.md:B002");
    assert_no_line(&report, "B011");
    assert_eq!(report.errors(), 2);

    // A structurally invalid resource is likewise not a verified target,
    // and its own links are not checked.
    project.remove("docs/broken.md");
    project.file(
        "README.md",
        "# Read me\n\n[into invalid](content/note-b.md#anything)\n",
    );
    project.file(
        "content/note-b.md",
        "+++\nschema = \"example/test/note@1\"\nid = \"note-b\"\ntitle = 3\n+++\n\n# B\n\n[dangling](nope.md)\n",
    );
    let report = project.check();
    assert_line(&report, "content/note-b.md:4:B005");
    assert_no_line(&report, "B011");
}

#[test]
fn links_inside_code_are_not_references() {
    let project = project();
    project.file(
        "README.md",
        "# Read me\n\n```md\n[x](missing.md) ![y](missing.png) <a id=\"fake\"></a>\n```\n\n    [indented](also-missing.md)\n\n`[inline](missing.md)`\n\n[to fake](#fake)\n",
    );
    let report = project.check();
    assert_eq!(
        lines(&report),
        ["README.md:11:B011: link `#fake` names anchor `fake`, which `README.md` does not define"]
    );
}

#[cfg(unix)]
#[test]
fn symbolic_links_follow_phase_one_rules_in_the_working_directory() {
    let outside = tempfile::tempdir().expect("outside");
    std::fs::write(outside.path().join("secret.md"), "# Secret\n").expect("write");
    let project = project();
    std::os::unix::fs::symlink(
        outside.path().join("secret.md"),
        project.path().join("docs/escape.md"),
    )
    .expect("symlink");
    std::os::unix::fs::symlink("guide.md", project.path().join("docs/alias.md")).expect("symlink");
    project.file(
        "README.md",
        "# Read me\n\n[escape](docs/escape.md)\n[alias](docs/alias.md)\n[alias anchor](docs/alias.md#usage)\n",
    );
    let report = project.check();
    // The escaping link is outside the capability: missing. The inside
    // alias exists as a file but is not a discovered document (discovery
    // skips links), so its anchor cannot be verified.
    assert_line(
        &report,
        "README.md:3:B011: link `docs/escape.md` points at a missing file",
    );
    assert_no_line(&report, "README.md:4:");
    assert_line(
        &report,
        "README.md:5:B011: link `docs/alias.md#usage` names anchor `usage` in `docs/alias.md`, which is not a discovered document",
    );
    assert_eq!(report.documents, 2);
}

#[test]
fn symbolic_links_and_gitlinks_follow_phase_one_rules_in_git_trees() {
    let project = project();
    project.file(
        "README.md",
        "# Read me\n\n[alias](docs/alias.md)\n[alias anchor](docs/alias.md#usage)\n[escape](docs/escape.md)\n[vendor](docs/vendor/README.md)\n![vendored](docs/vendor/logo.svg)\n",
    );
    project.git_init();
    project.commit_all("base");
    project.stage_entry("120000", b"guide.md", "docs/alias.md");
    project.stage_entry("120000", b"../../../outside.md", "docs/escape.md");
    project.stage_entry("160000", b"", "docs/vendor");
    let report = project.check_from(Source::Index);
    assert_eq!(
        lines(&report),
        [
            "README.md:4:B011: link `docs/alias.md#usage` names anchor `usage` in `docs/alias.md`, which is not a discovered document; select it in `[documents]` to verify its anchors",
            "README.md:5:B011: link `docs/escape.md` points at a missing file",
            "README.md:6:B011: link `docs/vendor/README.md` points at a missing file",
            "README.md:7:B011: image `docs/vendor/logo.svg` points at a missing file",
        ]
    );
    assert_eq!(report.documents, 2, "the link entry is not a document");
}

#[test]
fn documents_with_a_bom_and_crlf_report_correct_lines() {
    let project = project();
    project.file(
        "docs/bom.md",
        "\u{feff}# With BOM\r\n\r\nSee [usage](guide.md#usage).\r\n[bad](guide.md#nope)\r\n",
    );
    let report = project.check();
    assert_eq!(
        lines(&report),
        [
            "docs/bom.md:4:B011: link `guide.md#nope` names anchor `nope`, which `docs/guide.md` does not define"
        ]
    );
}

#[test]
fn commented_out_anchors_are_not_targets() {
    let project = project();
    project.file(
        "docs/guide.md",
        "# Guide\n\n<!-- <a id=\"ghost\"></a> -->\n\n<a id=\"real\"></a>\n",
    );
    project.file(
        "README.md",
        "# Read me\n\n[ghost](docs/guide.md#ghost)\n[real](docs/guide.md#real)\n",
    );
    assert_eq!(
        lines(&project.check()),
        [
            "README.md:3:B011: link `docs/guide.md#ghost` names anchor `ghost`, which `docs/guide.md` does not define"
        ]
    );
}
