// SPDX-License-Identifier: Apache-2.0

//! Delivery hardening: atomic replacement, ownership, strict state, the
//! journaled transaction, and rendering limits.

mod common;

use std::fs;

use bearout::{Code, Mode};
use common::{Project, assert_clean, assert_line, assert_no_line, codes, lines};

const TEMPLATE: &str =
    "{% for line in bearout.header %}<!-- {{ line }} -->\n{% endfor %}\n# {{ title }}\n";

fn gen_project(plan: &str) -> Project {
    let project = Project::with_note();
    project.file("bearout.toml", common::BOOTSTRAP_GEN);
    project.file("templates/page.md.j2", TEMPLATE);
    project.file(
        common::ENTRY,
        &format!("def g(p):\n    return [{plan}]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ngenerator(\"pages\", g)\n"),
    );
    project
}

fn one_page() -> Project {
    gen_project("output(\"page.md.j2\", \"generated/a.md\", context = {\"title\": \"A\"})")
}

/// Every file under the project with its bytes, for before/after comparison.
fn snapshot(project: &Project) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .expect("dir")
            .filter_map(Result::ok)
            .collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                let name = path
                    .strip_prefix(base)
                    .expect("relative")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((name, fs::read(&path).expect("read")));
            }
        }
    }
    let mut out = Vec::new();
    walk(project.path(), project.path(), &mut out);
    out
}

fn temp_leftovers(project: &Project, dir: &str) -> Vec<String> {
    fs::read_dir(project.path().join(dir))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with('.') || name.contains("tmp"))
                .collect()
        })
        .unwrap_or_default()
}

// ---- atomic delivery ----------------------------------------------------

#[cfg(unix)]
#[test]
fn predictable_temporary_names_are_never_followed() {
    let victim = tempfile::tempdir().expect("victim dir");
    let victim_file = victim.path().join("victim.txt");
    fs::write(&victim_file, "precious\n").expect("victim");
    let project = one_page();
    fs::create_dir_all(project.path().join("generated")).expect("dir");
    std::os::unix::fs::symlink(
        &victim_file,
        project.path().join("generated/.a.md.bearout-tmp"),
    )
    .expect("symlink");

    let report = project.generate(Mode::Write);
    assert_clean(&report);
    assert_eq!(
        fs::read_to_string(&victim_file).expect("victim"),
        "precious\n"
    );
    let output = project.path().join("generated/a.md");
    assert!(
        fs::symlink_metadata(&output)
            .expect("meta")
            .file_type()
            .is_file(),
        "output is a regular file"
    );
    assert!(project.read("generated/a.md").contains("# A"));
    assert_eq!(
        temp_leftovers(&project, "generated"),
        vec![".a.md.bearout-tmp".to_owned()],
        "the stranger's link is left alone and no temp file remains"
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_at_the_output_path_is_refused() {
    let victim = tempfile::tempdir().expect("victim dir");
    let victim_file = victim.path().join("victim.txt");
    fs::write(&victim_file, "precious\n").expect("victim");
    let project = one_page();
    fs::create_dir_all(project.path().join("generated")).expect("dir");
    std::os::unix::fs::symlink(&victim_file, project.path().join("generated/a.md"))
        .expect("symlink");
    let report = project.generate(Mode::Write);
    assert_line(
        &report,
        "generated/a.md:B021: `generated/a.md` is a symbolic link",
    );
    assert_eq!(
        fs::read_to_string(&victim_file).expect("victim"),
        "precious\n"
    );
    assert!(
        fs::symlink_metadata(project.path().join("generated/a.md"))
            .expect("meta")
            .file_type()
            .is_symlink()
    );
    assert!(!project.exists("bearout-state.toml"));
}

#[test]
fn owned_outputs_are_replaced_atomically_on_every_platform() {
    let project = one_page();
    assert_clean(&project.generate(Mode::Write));
    let first = project.read("generated/a.md");
    project.file(
        "templates/page.md.j2",
        &TEMPLATE.replace("# {{ title }}", "# {{ title }} again"),
    );
    let report = project.generate(Mode::Write);
    assert_clean(&report);
    assert_eq!(report.outputs, ["generated/a.md"]);
    let second = project.read("generated/a.md");
    assert_ne!(first, second);
    assert!(second.contains("# A again"));
    assert!(temp_leftovers(&project, "generated").is_empty());
    assert_clean(&project.generate(Mode::Check));
}

#[cfg(unix)]
#[test]
fn temporary_files_are_removed_after_a_failed_write_and_prior_content_restored() {
    use std::os::unix::fs::PermissionsExt;
    if fs::metadata("/proc/self").is_ok()
        && fs::read_to_string("/proc/self/status").is_ok_and(|s| s.contains("Uid:	0	"))
    {
        eprintln!("skipped: running as root, permission checks do not apply");
        return;
    }
    let project = gen_project(
        "output(\"page.md.j2\", \"generated/a.md\", context = {\"title\": \"A\"}), output(\"page.md.j2\", \"generated/locked/b.md\", context = {\"title\": \"B\"})",
    );
    let locked = project.path().join("generated/locked");
    fs::create_dir_all(&locked).expect("dir");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).expect("chmod");
    let before = snapshot(&project);

    let report = project.generate(Mode::Write);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("chmod back");
    assert_line(&report, "generated/locked/b.md:B021: cannot write output");
    assert_line(
        &report,
        "bearout-state.toml:B021: delivery failed after 1 change(s); prior content was restored",
    );
    assert!(report.outputs.is_empty());
    assert!(
        !project.exists("generated/a.md"),
        "the first write was rolled back"
    );
    assert!(!project.exists("bearout-state.toml"));
    assert!(temp_leftovers(&project, "generated").is_empty());
    assert!(temp_leftovers(&project, "generated/locked").is_empty());
    assert_eq!(snapshot(&project), before);
}

#[test]
fn concurrent_deliveries_never_share_a_temporary_file() {
    let project = one_page();
    assert_clean(&project.generate(Mode::Write));
    project.file(
        "templates/page.md.j2",
        &TEMPLATE.replace("# {{ title }}", "# {{ title }} v2"),
    );
    let path = project.path().to_path_buf();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| bearout::generate(&path, Mode::Write)))
            .collect();
        for handle in handles {
            let report = handle.join().expect("thread");
            for diagnostic in &report.diagnostics {
                assert!(
                    matches!(diagnostic.code, Code::Delivery),
                    "unexpected {diagnostic}"
                );
            }
        }
    });
    assert!(project.read("generated/a.md").contains("# A v2"));
    assert!(temp_leftovers(&project, "generated").is_empty());
    assert_clean(&project.generate(Mode::Check));
}

// ---- ownership ---------------------------------------------------------

#[test]
fn unowned_files_are_never_adopted_even_when_identical() {
    let project = one_page();
    assert_clean(&project.generate(Mode::Write));
    let rendered = project.read("generated/a.md");
    project.remove("bearout-state.toml");
    let before = snapshot(&project);

    let report = project.generate(Mode::Write);
    assert_line(
        &report,
        "generated/a.md:B021: refusing to overwrite a file the state manifest does not own",
    );
    assert!(report.outputs.is_empty());
    assert!(
        !project.exists("bearout-state.toml"),
        "no manifest is written after a refusal"
    );
    assert_eq!(snapshot(&project), before);

    project.file("generated/a.md", "different\n");
    let report = project.generate(Mode::Write);
    assert_line(
        &report,
        "generated/a.md:B021: refusing to overwrite a file the state manifest does not own",
    );
    assert_eq!(project.read("generated/a.md"), "different\n");
    assert!(!project.exists("bearout-state.toml"));

    let report = project.generate(Mode::Check);
    assert_line(
        &report,
        "generated/a.md:B020: output exists but is not owned by the state manifest",
    );
    assert!(report.outputs.is_empty());
    assert_ne!(rendered, "different\n");
}

// ---- strict state ------------------------------------------------------

#[test]
fn state_manifest_has_three_outcomes() {
    let project = one_page();
    let report = project.generate(Mode::Check);
    assert_line(
        &report,
        "bearout-state.toml:B020: state manifest is missing",
    );
    assert_clean(&project.generate(Mode::Write));
    assert_clean(&project.generate(Mode::Check));

    let valid = project.read("bearout-state.toml");
    let cases: [(&str, &str, &str); 12] = [
        ("version = 1", "version = 2", "version 2 is not supported"),
        ("version = 1", "", "must declare `version = 1`"),
        ("bearout = \"", "bearout = \"x", "is not a version string"),
        ("bearout = \"", "engine = \"", "unknown key `engine`"),
        (
            "version = 1",
            "version = 1\nextra = 1",
            "unknown key `extra`",
        ),
        (
            "generator = \"pages\"",
            "generator = \"Pages\"",
            "`generator`",
        ),
        ("generator = \"pages\"", "", "missing `generator`"),
        (
            "template = \"page.md.j2\"",
            "template = \"../page.md.j2\"",
            "`template`",
        ),
        (
            "path = \"generated/a.md\"",
            "path = \"content/a.md\"",
            "not beneath a declared output root",
        ),
        (
            "digest = \"blake3:",
            "digest = \"sha256:",
            "`digest` must be `blake3:` followed by 64 lowercase hexadecimal",
        ),
        (
            "inputs = \"blake3:",
            "inputs = \"blake3:00",
            "`inputs` must be",
        ),
        (
            "[[outputs]]",
            "[[outputs]]\npath = \"generated/a.md\"\ngenerator = \"pages\"\ntemplate = \"page.md.j2\"\ndigest = \"blake3:0000000000000000000000000000000000000000000000000000000000000000\"\ninputs = \"blake3:0000000000000000000000000000000000000000000000000000000000000000\"\n\n[[outputs]]",
            "listed more than once",
        ),
    ];
    for (from, to, expected) in cases {
        assert!(valid.contains(from), "{from:?} present");
        project.file("bearout-state.toml", &valid.replacen(from, to, 1));
        let before = snapshot(&project);
        let report = project.generate(Mode::Write);
        assert_line(&report, "bearout-state.toml:B020: ");
        assert_line(&report, expected);
        assert!(report.outputs.is_empty(), "{expected}: outputs claimed");
        assert_eq!(snapshot(&project), before, "{expected}: tree changed");
        let report = project.generate(Mode::Check);
        assert_line(&report, expected);
        assert!(report.outputs.is_empty());
    }
    project.file("bearout-state.toml", "this = [is not toml\n");
    let report = project.generate(Mode::Write);
    assert_line(
        &report,
        "bearout-state.toml:B020: state manifest is not valid TOML",
    );
    project.file("bearout-state.toml", &valid);
    assert_clean(&project.generate(Mode::Check));
}

#[test]
fn invalid_state_blocks_orphan_removal_and_adoption() {
    let project = one_page();
    assert_clean(&project.generate(Mode::Write));
    project.file(common::ENTRY, "def g(p):\n    return []\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ngenerator(\"pages\", g)\n");
    project.file(
        "bearout-state.toml",
        &project
            .read("bearout-state.toml")
            .replace("version = 1", "version = 3"),
    );
    let before = snapshot(&project);
    let report = project.generate(Mode::Write);
    assert!(codes(&report).contains(&Code::OutputState));
    assert_eq!(snapshot(&project), before);
    assert!(
        project.exists("generated/a.md"),
        "the orphan is not removed under an invalid manifest"
    );
}

// ---- transaction -------------------------------------------------------

#[test]
fn modified_orphans_abort_delivery_and_keep_the_manifest() {
    let project = one_page();
    assert_clean(&project.generate(Mode::Write));
    let state = project.read("bearout-state.toml");
    project.file("generated/a.md", "hand edited\n");
    project.file(common::ENTRY, "def g(p):\n    return [output(\"page.md.j2\", \"generated/b.md\", context = {\"title\": \"B\"})]\nschema(\"example/test/note@1\", shape = \"note.schema.toml\")\ngenerator(\"pages\", g)\n");
    let report = project.generate(Mode::Write);
    assert_line(
        &report,
        "generated/a.md:B020: orphaned output was modified after Bearout wrote it; not removed, and the state manifest is left unchanged",
    );
    assert!(report.outputs.is_empty());
    assert!(
        !project.exists("generated/b.md"),
        "nothing is written when an orphan cannot be handled"
    );
    assert_eq!(
        project.read("bearout-state.toml"),
        state,
        "the orphan stays tracked"
    );
    assert_eq!(project.read("generated/a.md"), "hand edited\n");
}

#[test]
fn report_outputs_are_narrow() {
    let project = one_page();
    assert!(
        project.check().outputs.is_empty(),
        "check runs never list outputs"
    );
    let report = project.generate(Mode::Check);
    assert!(!report.is_clean());
    assert!(report.outputs.is_empty(), "a failed check lists nothing");
    let report = project.generate(Mode::Write);
    assert_eq!(report.outputs, ["generated/a.md"]);
    let report = project.generate(Mode::Check);
    assert_eq!(
        report.outputs,
        ["generated/a.md"],
        "a successful check lists verified outputs"
    );
    let json = serde_json::to_value(&report).expect("json");
    assert_eq!(json["outputs"], serde_json::json!(["generated/a.md"]));
    project.file("templates/page.md.j2", "{{ missing }}\n");
    let report = project.generate(Mode::Write);
    assert!(codes(&report).contains(&Code::PlanInvalid));
    assert!(report.outputs.is_empty(), "a failed render lists nothing");
}

// ---- rendering limits --------------------------------------------------

#[test]
fn template_fuel_and_output_size_are_bounded() {
    let project = one_page();
    project.file(
        "bearout.toml",
        &format!("{}\n[limits]\ntemplate_fuel = 200\n", common::BOOTSTRAP_GEN),
    );
    project.file("templates/page.md.j2", "{% for line in bearout.header %}<!-- {{ line }} -->\n{% endfor %}\n{% for i in range(100000) %}x{% endfor %}\n");
    let before = snapshot(&project);
    let report = project.generate(Mode::Write);
    assert_line(
        &report,
        "bearout.star:B019: generator `pages`: template `page.md.j2`",
    );
    assert!(
        lines(&report).join("\n").to_lowercase().contains("fuel"),
        "{:?}",
        lines(&report)
    );
    assert!(report.outputs.is_empty());
    assert_eq!(snapshot(&project), before);

    project.file(
        "bearout.toml",
        &format!("{}\n[limits]\noutput_bytes = 500\n", common::BOOTSTRAP_GEN),
    );
    let before = snapshot(&project);
    let report = project.generate(Mode::Write);
    assert_line(
        &report,
        "rendered output exceeds `limits.output_bytes` = 500",
    );
    assert!(codes(&report).iter().all(|code| *code == Code::PlanInvalid));
    assert_eq!(snapshot(&project), before);

    project.file(
        "bearout.toml",
        &format!(
            "{}\n[limits]\ntemplate_fuel = 5\noutput_bytes = 10\nfuel = 1\n",
            common::BOOTSTRAP_GEN
        ),
    );
    assert!(
        project
            .generate(Mode::Write)
            .fatal
            .as_deref()
            .is_some_and(|m| m.contains("unknown key `limits.fuel`"))
    );
}

#[test]
fn includes_render_within_limits() {
    let project =
        gen_project("output(\"page.md.j2\", \"generated/a.md\", context = {\"title\": \"A\"})");
    project.file(
        "templates/page.md.j2",
        "{% include \"header.j2\" %}\n# {{ title }}\n{% include \"footer.j2\" %}\n",
    );
    project.file(
        "templates/header.j2",
        "{% for line in bearout.header %}<!-- {{ line }} -->\n{% endfor %}",
    );
    project.file("templates/footer.j2", "\nfooter\n");
    let report = project.generate(Mode::Write);
    assert_clean(&report);
    assert!(project.read("generated/a.md").ends_with("footer\n"));
    assert!(report.max_fuel > 0);
    assert_no_line(&report, "B019");
}

#[test]
fn a_state_manifest_without_outputs_is_the_empty_manifest() {
    // The serializer writes no `outputs` table when Bearout owns nothing, so
    // omission is the canonical empty form; every other omission is an error.
    let project =
        gen_project("output(\"page.md.j2\", \"generated/a.md\", context = {\"title\": \"A\"})");
    project.file("bearout-state.toml", "version = 1\nbearout = \"0.1.0\"\n");
    let report = project.generate(Mode::Check);
    assert_line(&report, "generated/a.md:B020: generated file is missing");
    assert_no_line(&report, "state manifest must");
    project.file("bearout-state.toml", "version = 1\n");
    assert_line(
        &project.generate(Mode::Check),
        "must declare the `bearout` version string",
    );
    project.file("bearout-state.toml", "bearout = \"0.1.0\"\n");
    assert_line(&project.generate(Mode::Check), "must declare `version = 1`");
    project.file(
        "bearout-state.toml",
        "version = 1\nbearout = \"0.1.0\"\noutputs = []\n",
    );
    assert_line(
        &project.generate(Mode::Check),
        "`outputs` must be an array of tables",
    );
}
