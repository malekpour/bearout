// SPDX-License-Identifier: Apache-2.0

//! The sample inventory: every sample checks clean with fresh outputs and
//! follows the naming and README conventions. This is the one place the
//! full sample suite runs.

mod common;

use std::fs;
use std::path::Path;

use bearout::Mode;
use common::{lines, samples_dir};

const README_SECTIONS: [&str; 8] = [
    "## Purpose",
    "## Data classification",
    "## Capabilities demonstrated",
    "## Resource model",
    "## Generated artifacts",
    "## Try breaking it",
    "## Sample omissions",
    "## Engine gaps",
];

const EXPECTED: [&str; 9] = [
    "decision-records",
    "document-assembly",
    "document-references",
    "engineering-evidence",
    "esperanto-reference",
    "formula-language",
    "linked-notes",
    "multilateral-records",
    "project-delivery",
];

fn sample_dirs() -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(samples_dir())
        .expect("samples directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn inventory_matches_the_documented_samples() {
    assert_eq!(sample_dirs(), EXPECTED);
}

#[test]
fn every_sample_is_clean_with_fresh_outputs() {
    for name in sample_dirs() {
        let report = bearout::generate(&samples_dir().join(&name), Mode::Check);
        assert!(report.fatal.is_none(), "{name}: {:?}", report.fatal);
        assert!(
            report.diagnostics.is_empty(),
            "sample {name}:\n{}",
            lines(&report).join("\n")
        );
        assert!(report.resources > 0, "{name} has no resources");
        let has_generators = fs::read_to_string(samples_dir().join(&name).join("bearout.star"))
            .expect("entry")
            .contains("generator(");
        assert_eq!(
            !report.outputs.is_empty(),
            has_generators,
            "{name}: outputs vs generators"
        );
    }
}

#[test]
fn readmes_follow_the_standard_sections() {
    for name in sample_dirs() {
        let readme = fs::read_to_string(samples_dir().join(&name).join("README.md"))
            .unwrap_or_else(|_| panic!("{name} has a README"));
        let mut cursor = 0;
        for section in README_SECTIONS {
            let position = readme[cursor..]
                .find(section)
                .unwrap_or_else(|| panic!("{name}: README lacks `{section}` in order"));
            cursor += position + section.len();
        }
        let classification = readme
            .split("## Data classification")
            .nth(1)
            .and_then(|rest| rest.split("## ").next())
            .expect("classification");
        let named = ["synthetic", "fictional", "sourced snapshot"]
            .iter()
            .filter(|kind| classification.to_lowercase().contains(*kind))
            .count();
        assert_eq!(
            named, 1,
            "{name}: data classification must name exactly one class, found {classification:?}"
        );
        assert!(
            !readme.contains("Not yet expressible"),
            "{name}: README uses the retired phrase"
        );
    }
}

#[test]
fn identifiers_and_files_follow_the_naming_convention() {
    for name in sample_dirs() {
        let root = samples_dir().join(&name);
        let bootstrap = fs::read_to_string(root.join("bearout.toml")).expect("bootstrap");
        let parsed = bearout::bootstrap::parse(&bootstrap).expect("bootstrap parses");
        for resource_root in &parsed.resource_roots {
            walk(&root.join(resource_root.to_native()), &mut |path| {
                let stem = path
                    .file_stem()
                    .expect("stem")
                    .to_string_lossy()
                    .into_owned();
                let text = fs::read_to_string(path).expect("resource");
                let id = text
                    .lines()
                    .find_map(|line| line.strip_prefix("id = "))
                    .map(|v| v.trim_matches('"').to_owned())
                    .unwrap_or_default();
                assert_eq!(
                    id,
                    stem,
                    "{name}: {} must be named after its id",
                    path.display()
                );
                assert!(
                    stem.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                    "{name}: {stem} is not kebab-case"
                );
                let schema = text
                    .lines()
                    .find_map(|line| line.strip_prefix("schema = "))
                    .map(|v| v.trim_matches('"').to_owned())
                    .unwrap_or_default();
                assert!(
                    schema.starts_with(&format!("example/{name}/")),
                    "{name}: {} declares schema {schema}",
                    path.display()
                );
            });
        }
        for entry in fs::read_dir(root.join(parsed.rules_root.to_native())).expect("rules") {
            let entry = entry.expect("entry");
            let file = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type().expect("type").is_file() {
                let is_rule_file = file.strip_suffix(".star").is_some()
                    || file.strip_suffix(".schema.toml").is_some();
                assert!(is_rule_file, "{name}: unexpected rules file {file}");
            }
        }
    }
}

fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .expect("dir")
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, visit);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "toml")
        ) {
            visit(&path);
        }
    }
}

/// Starlark tick usage across the samples stays well under the default
/// limit. The printed figures are how the defaults in `bootstrap.rs` were
/// derived; run with `--nocapture` to see them.
#[test]
fn sample_tick_usage_is_far_below_the_default_limit() {
    let limit = bearout::Limits::default().ticks;
    for name in sample_dirs() {
        let report = bearout::generate(&samples_dir().join(&name), Mode::Check);
        println!(
            "{name}: max ticks per call = {}, max heap bytes = {}, max fuel = {}, max output bytes = {}",
            report.max_ticks, report.max_heap_bytes, report.max_fuel, report.max_output_bytes
        );
        assert!(
            report.max_heap_bytes * 10 < bearout::Limits::default().heap_bytes as u64,
            "{name}: heap"
        );
        assert!(
            report.max_fuel * 10 < bearout::Limits::default().template_fuel,
            "{name}: fuel"
        );
        assert!(
            report.max_ticks * 10 < limit,
            "{name}: {} ticks is within 10x of the {limit} tick limit",
            report.max_ticks
        );
    }
}
