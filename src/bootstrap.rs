// SPDX-License-Identifier: Apache-2.0

//! The static bootstrap, `bearout.toml`. It is the capability boundary: it
//! names the Starlark entry module and grants the filesystem roots that
//! resources, rules, templates, and outputs may use, plus the read-only
//! selection of schema-less Markdown documents. Repository policy can
//! register schemas, checks, and generators, but cannot widen these grants.

use toml_edit::{DocumentMut, Item, Table};

use crate::paths::ProjectPath;

/// File name of the bootstrap at the project root.
pub const MANIFEST_NAME: &str = "bearout.toml";

/// File name of the generated-state manifest at the project root.
pub const STATE_NAME: &str = "bearout-state.toml";

/// Resource limits applied to every Starlark evaluation and to input size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Maximum Starlark execution ticks per call.
    pub ticks: u64,
    /// Maximum Starlark heap bytes per call.
    pub heap_bytes: usize,
    /// Maximum Starlark call-stack depth per call.
    pub call_stack: usize,
    /// Maximum number of discovered resources.
    pub resources: usize,
    /// Maximum size of one resource file in bytes.
    pub resource_bytes: u64,
    /// Maximum `MiniJinja` fuel per rendered output.
    pub template_fuel: u64,
    /// Maximum size of one rendered output in bytes.
    pub output_bytes: u64,
    /// Maximum number of discovered schema-less documents.
    pub documents: usize,
    /// Maximum size of one schema-less document in bytes.
    pub document_bytes: u64,
}

impl Default for Limits {
    /// Measured (`tests/samples.rs` prints the peaks): the largest sample
    /// uses under 500 ticks and under 40 KiB of call heap per Starlark call,
    /// under 2,000 fuel per render, and under 9 KiB per output. `ticks` and
    /// `template_fuel` allow three orders of magnitude of headroom;
    /// `heap_bytes` allows over a thousand times the measured peak because
    /// a call heap holds only what one call allocates (the frozen views live
    /// outside it). `call_stack`, `resources`, `resource_bytes`,
    /// `output_bytes`, `documents`, and `document_bytes` are conservative
    /// operational bounds, not measured. None of these is a security
    /// boundary.
    fn default() -> Self {
        Self {
            ticks: 1_000_000,
            heap_bytes: 64 * 1024 * 1024,
            call_stack: 64,
            resources: 10_000,
            resource_bytes: 4 * 1024 * 1024,
            template_fuel: 2_000_000,
            output_bytes: 16 * 1024 * 1024,
            documents: 10_000,
            document_bytes: 4 * 1024 * 1024,
        }
    }
}

/// The parsed bootstrap.
#[derive(Debug, Clone)]
pub struct Bootstrap {
    /// Starlark entry module, relative to the project root.
    pub entry: ProjectPath,
    /// Directories discovered for resources.
    pub resource_roots: Vec<ProjectPath>,
    /// Directory beneath which `load()` resolves and shape files live.
    pub rules_root: ProjectPath,
    /// Directory beneath which templates resolve, when generation is enabled.
    pub templates_root: Option<ProjectPath>,
    /// Directories beneath which generated outputs may be delivered.
    pub output_roots: Vec<ProjectPath>,
    /// SPDX license identifier stamped into generated provenance headers.
    pub license: Option<String>,
    /// Directories discovered recursively for schema-less Markdown
    /// documents, sorted. A read-only grant that may overlap any other root.
    pub document_roots: Vec<ProjectPath>,
    /// Individual schema-less Markdown documents, sorted.
    pub document_files: Vec<ProjectPath>,
    /// Resource limits.
    pub limits: Limits,
}

impl Bootstrap {
    /// `true` when the bootstrap declares a `[documents]` table.
    #[must_use]
    pub fn selects_documents(&self) -> bool {
        !self.document_roots.is_empty() || !self.document_files.is_empty()
    }
}

/// The file extension of a Markdown document, for resources and
/// schema-less documents alike.
pub const MARKDOWN_EXTENSION: &str = "md";

const TOP_LEVEL: [&str; 8] = [
    "version",
    "entry",
    "resources",
    "rules",
    "templates",
    "outputs",
    "documents",
    "limits",
];

/// Parse and validate the bootstrap text.
pub fn parse(text: &str) -> Result<Bootstrap, String> {
    let doc: DocumentMut = text.parse().map_err(|error: toml_edit::TomlError| {
        format!("{MANIFEST_NAME} is not valid TOML: {}", error.message())
    })?;
    let root = doc.as_table();
    reject_unknown(root, "", &TOP_LEVEL)?;

    let version = required(root, "version")?
        .as_integer()
        .ok_or_else(|| "`version` must be an integer".to_owned())?;
    if version != 1 {
        return Err(format!(
            "unsupported manifest version {version}; this build supports version 1"
        ));
    }

    let entry = path_field(required(root, "entry")?, "entry")?;

    let resources =
        table(root, "resources")?.ok_or_else(|| "`[resources]` is required".to_owned())?;
    reject_unknown(resources, "resources", &["roots"])?;
    let resource_roots = path_list(
        required(resources, "roots").map_err(|_| "`resources.roots` is required".to_owned())?,
        "resources.roots",
    )?;
    if resource_roots.is_empty() {
        return Err("`resources.roots` must name at least one directory".to_owned());
    }

    let rules = table(root, "rules")?.ok_or_else(|| "`[rules]` is required".to_owned())?;
    reject_unknown(rules, "rules", &["root"])?;
    let rules_root = path_field(
        required(rules, "root").map_err(|_| "`rules.root` is required".to_owned())?,
        "rules.root",
    )?;

    let templates_root = match table(root, "templates")? {
        Some(templates) => {
            reject_unknown(templates, "templates", &["root"])?;
            Some(path_field(
                required(templates, "root")
                    .map_err(|_| "`templates.root` is required".to_owned())?,
                "templates.root",
            )?)
        }
        None => None,
    };

    let (output_roots, license) = match table(root, "outputs")? {
        Some(outputs) => {
            reject_unknown(outputs, "outputs", &["roots", "license"])?;
            let roots = path_list(
                required(outputs, "roots").map_err(|_| "`outputs.roots` is required".to_owned())?,
                "outputs.roots",
            )?;
            let license = match outputs.get("license") {
                None => None,
                Some(item) => {
                    let text = item
                        .as_str()
                        .filter(|text| !text.trim().is_empty())
                        .ok_or_else(|| "`outputs.license` must be a non-empty string".to_owned())?;
                    spdx::Expression::parse(text).map_err(|error| {
                        format!(
                            "`outputs.license` is not a valid SPDX expression: {}",
                            error.reason
                        )
                    })?;
                    Some(text.to_owned())
                }
            };
            (roots, license)
        }
        None => (Vec::new(), None),
    };

    let (document_roots, document_files) = match table(root, "documents")? {
        Some(documents) => parse_documents(documents)?,
        None => (Vec::new(), Vec::new()),
    };

    let limits = match table(root, "limits")? {
        Some(limits) => parse_limits(limits)?,
        None => Limits::default(),
    };

    let bootstrap = Bootstrap {
        entry,
        resource_roots,
        rules_root,
        templates_root,
        output_roots,
        license,
        document_roots,
        document_files,
        limits,
    };
    check_roots(&bootstrap)?;
    Ok(bootstrap)
}

/// `[documents]`: `roots` are walked recursively, `files` are named one by
/// one. Both are optional, at least one must be present, both are sorted,
/// a repeated entry is an error, roots must not nest, and every file must
/// carry the Markdown extension.
fn parse_documents(documents: &Table) -> Result<(Vec<ProjectPath>, Vec<ProjectPath>), String> {
    reject_unknown(documents, "documents", &["roots", "files"])?;
    let mut roots = match documents.get("roots") {
        Some(item) => path_list(item, "documents.roots")?,
        None => Vec::new(),
    };
    let mut files = match documents.get("files") {
        Some(item) => path_list(item, "documents.files")?,
        None => Vec::new(),
    };
    if roots.is_empty() && files.is_empty() {
        return Err("`[documents]` must name at least one root or file".to_owned());
    }
    for root in &roots {
        if root.as_str().is_empty() {
            return Err("`documents.roots` must not include the project root itself".to_owned());
        }
    }
    for (index, root) in roots.iter().enumerate() {
        for other in &roots[index + 1..] {
            if root.is_within(other) || other.is_within(root) {
                return Err(format!(
                    "`documents.roots` `{root}` and `{other}` overlap; roots must be disjoint"
                ));
            }
        }
    }
    for file in &files {
        if file.as_str().is_empty() {
            return Err("`documents.files` must not include the project root".to_owned());
        }
        if file.extension() != Some(MARKDOWN_EXTENSION) {
            return Err(format!(
                "`documents.files` `{file}` must be a `.{MARKDOWN_EXTENSION}` document"
            ));
        }
    }
    roots.sort();
    files.sort();
    Ok((roots, files))
}

fn parse_limits(limits: &Table) -> Result<Limits, String> {
    const KEYS: [&str; 9] = [
        "ticks",
        "heap_bytes",
        "call_stack",
        "resources",
        "resource_bytes",
        "template_fuel",
        "output_bytes",
        "documents",
        "document_bytes",
    ];
    reject_unknown(limits, "limits", &KEYS)?;
    let mut result = Limits::default();
    let positive = |key: &str| -> Result<Option<u64>, String> {
        match limits.get(key) {
            None => Ok(None),
            Some(item) => item
                .as_integer()
                .filter(|value| *value > 0)
                .map(|value| Some(value.unsigned_abs()))
                .ok_or_else(|| format!("`limits.{key}` must be a positive integer")),
        }
    };
    if let Some(value) = positive("ticks")? {
        result.ticks = value;
    }
    if let Some(value) = positive("heap_bytes")? {
        result.heap_bytes =
            usize::try_from(value).map_err(|_| "`limits.heap_bytes` is too large".to_owned())?;
    }
    if let Some(value) = positive("call_stack")? {
        result.call_stack =
            usize::try_from(value).map_err(|_| "`limits.call_stack` is too large".to_owned())?;
    }
    if let Some(value) = positive("resources")? {
        result.resources =
            usize::try_from(value).map_err(|_| "`limits.resources` is too large".to_owned())?;
    }
    if let Some(value) = positive("resource_bytes")? {
        result.resource_bytes = value;
    }
    if let Some(value) = positive("template_fuel")? {
        result.template_fuel = value;
    }
    if let Some(value) = positive("output_bytes")? {
        result.output_bytes = value;
    }
    if let Some(value) = positive("documents")? {
        result.documents =
            usize::try_from(value).map_err(|_| "`limits.documents` is too large".to_owned())?;
    }
    if let Some(value) = positive("document_bytes")? {
        result.document_bytes = value;
    }
    Ok(result)
}

/// Roots must be distinct directories that do not nest, so that a write
/// beneath an output root can never touch a resource, rule, or template,
/// and the bootstrap itself is never beneath any root.
fn check_roots(bootstrap: &Bootstrap) -> Result<(), String> {
    let mut named: Vec<(&str, &ProjectPath)> = Vec::new();
    for root in &bootstrap.resource_roots {
        named.push(("resources.roots", root));
    }
    named.push(("rules.root", &bootstrap.rules_root));
    if let Some(templates) = &bootstrap.templates_root {
        named.push(("templates.root", templates));
    }
    for root in &bootstrap.output_roots {
        named.push(("outputs.roots", root));
    }
    for (label, root) in &named {
        if root.as_str().is_empty() {
            return Err(format!("`{label}` must not be the project root itself"));
        }
    }
    for (index, (label, root)) in named.iter().enumerate() {
        for (other_label, other) in &named[index + 1..] {
            if root.is_within(other) || other.is_within(root) {
                return Err(format!(
                    "`{label}` `{root}` and `{other_label}` `{other}` overlap; roots must be disjoint"
                ));
            }
        }
    }
    for root in &bootstrap.output_roots {
        if bootstrap.entry.is_within(root) {
            return Err(format!(
                "`entry` `{}` must not lie beneath output root `{root}`",
                bootstrap.entry
            ));
        }
    }
    if bootstrap.entry.as_str() == MANIFEST_NAME || bootstrap.entry.as_str() == STATE_NAME {
        return Err("`entry` must be a Starlark module, not the manifest".to_owned());
    }
    if bootstrap.entry.extension() != Some("star") {
        return Err(format!(
            "`entry` `{}` must be a `.star` module",
            bootstrap.entry
        ));
    }
    Ok(())
}

fn reject_unknown(table: &Table, prefix: &str, allowed: &[&str]) -> Result<(), String> {
    for (key, _) in table {
        if !allowed.contains(&key) {
            let full = if prefix.is_empty() {
                key.to_owned()
            } else {
                format!("{prefix}.{key}")
            };
            return Err(format!("unknown key `{full}`; expected one of {allowed:?}"));
        }
    }
    Ok(())
}

fn required<'a>(table: &'a Table, key: &str) -> Result<&'a Item, String> {
    table.get(key).ok_or_else(|| format!("`{key}` is required"))
}

fn table<'a>(root: &'a Table, key: &str) -> Result<Option<&'a Table>, String> {
    match root.get(key) {
        None => Ok(None),
        Some(item) => item
            .as_table()
            .map(Some)
            .ok_or_else(|| format!("`[{key}]` must be a table")),
    }
}

fn path_field(item: &Item, label: &str) -> Result<ProjectPath, String> {
    let text = item
        .as_str()
        .ok_or_else(|| format!("`{label}` must be a string"))?;
    let path = ProjectPath::parse(text).map_err(|error| format!("`{label}`: {error}"))?;
    if path.as_str().is_empty() {
        return Err(format!("`{label}` must not be empty"));
    }
    Ok(path)
}

fn path_list(item: &Item, label: &str) -> Result<Vec<ProjectPath>, String> {
    let array = item
        .as_array()
        .ok_or_else(|| format!("`{label}` must be an array of strings"))?;
    let mut paths = Vec::new();
    for value in array {
        let text = value
            .as_str()
            .ok_or_else(|| format!("`{label}` must be an array of strings"))?;
        let path = ProjectPath::parse(text).map_err(|error| format!("`{label}`: {error}"))?;
        if paths.contains(&path) {
            return Err(format!("`{label}` lists `{path}` twice"));
        }
        paths.push(path);
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "version = 1\nentry = \"bearout.star\"\n[resources]\nroots = [\"content\"]\n[rules]\nroot = \"rules\"\n";

    #[test]
    fn parses_minimal_bootstrap() {
        let bootstrap = parse(MINIMAL).unwrap();
        assert_eq!(bootstrap.entry.as_str(), "bearout.star");
        assert_eq!(bootstrap.rules_root.as_str(), "rules");
        assert!(bootstrap.output_roots.is_empty());
        assert_eq!(bootstrap.limits, Limits::default());
    }

    #[test]
    fn rejects_unknown_keys_and_bad_versions() {
        assert!(
            parse(&format!("extra = 1\n{MINIMAL}"))
                .unwrap_err()
                .contains("unknown key `extra`")
        );
        assert!(
            parse(&format!("{MINIMAL}extra = 1\n"))
                .unwrap_err()
                .contains("unknown key `rules.extra`")
        );
        assert!(
            parse(&MINIMAL.replace("version = 1", "version = 2"))
                .unwrap_err()
                .contains("version 2")
        );
        assert!(
            parse(&format!("{MINIMAL}[limits]\nticks = 0\n"))
                .unwrap_err()
                .contains("limits.ticks")
        );
    }

    #[test]
    fn validates_the_license_expression() {
        let text = format!(
            "{MINIMAL}[outputs]\nroots = [\"generated\"]\nlicense = \"Apache-2.0 OR MIT\"\n"
        );
        assert_eq!(
            parse(&text).unwrap().license.as_deref(),
            Some("Apache-2.0 OR MIT")
        );
        let text = format!("{MINIMAL}[outputs]\nroots = [\"generated\"]\nlicense = \"Apache 2\"\n");
        assert!(parse(&text).unwrap_err().contains("SPDX"));
        let text = format!("{MINIMAL}[limits]\ntemplate_fuel = 5\noutput_bytes = 10\nfuel = 1\n");
        assert!(
            parse(&text)
                .unwrap_err()
                .contains("unknown key `limits.fuel`")
        );
    }

    #[test]
    fn selects_documents_explicitly() {
        let text = format!(
            "{MINIMAL}[documents]\nroots = [\"docs\", \".github\"]\nfiles = [\"README.md\", \"AGENTS.md\"]\n"
        );
        let bootstrap = parse(&text).unwrap();
        assert!(bootstrap.selects_documents());
        let roots: Vec<&str> = bootstrap
            .document_roots
            .iter()
            .map(ProjectPath::as_str)
            .collect();
        assert_eq!(roots, [".github", "docs"], "sorted");
        let files: Vec<&str> = bootstrap
            .document_files
            .iter()
            .map(ProjectPath::as_str)
            .collect();
        assert_eq!(files, ["AGENTS.md", "README.md"], "sorted");
        assert!(!parse(MINIMAL).unwrap().selects_documents());
        assert_eq!(
            parse(&format!("{MINIMAL}[documents]\nfiles = [\"README.md\"]\n"))
                .unwrap()
                .document_files
                .len(),
            1
        );
        // Overlap with every other kind of root is allowed: the grant is
        // read-only.
        assert!(
            parse(&format!(
                "{MINIMAL}[documents]\nroots = [\"content\", \"rules\"]\n"
            ))
            .is_ok()
        );

        let cases = [
            ("[documents]\n", "at least one root or file"),
            ("[documents]\nroots = []\n", "at least one root or file"),
            ("[documents]\nroots = [\"\"]\n", "project root"),
            ("[documents]\nroots = [\"docs\", \"docs/sub\"]\n", "overlap"),
            ("[documents]\nroots = [\"docs\", \"docs\"]\n", "twice"),
            (
                "[documents]\nfiles = [\"README.md\", \"README.md\"]\n",
                "twice",
            ),
            (
                "[documents]\nfiles = [\"notes.txt\"]\n",
                "must be a `.md` document",
            ),
            (
                "[documents]\nfiles = [\"docs\"]\n",
                "must be a `.md` document",
            ),
            ("[documents]\nfiles = [\"../x.md\"]\n", "normalized"),
            ("[documents]\nfiles = [\"a\\\\b.md\"]\n", "backslash"),
            (
                "[documents]\nglob = [\"*.md\"]\n",
                "unknown key `documents.glob`",
            ),
            ("[limits]\ndocuments = 0\n", "limits.documents"),
            ("[limits]\ndocument_bytes = -1\n", "limits.document_bytes"),
        ];
        for (body, expected) in cases {
            let error = parse(&format!("{MINIMAL}{body}")).unwrap_err();
            assert!(error.contains(expected), "{body:?} -> {error}");
        }
    }

    #[test]
    fn rejects_overlapping_roots() {
        let text = format!("{MINIMAL}[outputs]\nroots = [\"content/generated\"]\n");
        assert!(parse(&text).unwrap_err().contains("overlap"));
        let text = format!("{MINIMAL}[outputs]\nroots = [\"rules\"]\n");
        assert!(parse(&text).unwrap_err().contains("overlap"));
        let text = MINIMAL.replace("roots = [\"content\"]", "roots = [\"\"]");
        assert!(parse(&text).unwrap_err().contains("project root"));
        let text = MINIMAL.replace("roots = [\"content\"]", "roots = [\"../x\"]");
        assert!(parse(&text).unwrap_err().contains("normalized"));
    }
}
