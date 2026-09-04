// SPDX-License-Identifier: Apache-2.0

//! The static bootstrap, `bearout.toml`. It is the capability boundary: it
//! names the Starlark entry module and grants the filesystem roots that
//! resources, rules, templates, and outputs may use, plus the read-only
//! selection of schema-less Markdown documents. Repository policy can
//! register schemas, checks, and generators, but cannot widen these grants.

use std::time::Duration;

use toml_edit::{DocumentMut, Item, Table};

use crate::identity;
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
    /// Maximum number of files selected for hygiene and formatting.
    pub files: usize,
    /// Maximum size of one selected file in bytes.
    pub file_bytes: u64,
    /// Maximum total bytes read for hygiene in one run: selected files,
    /// `.editorconfig` files, and formatter support files together.
    pub hygiene_bytes: u64,
    /// Maximum number of fixture cases in one suite. Experimental.
    pub fixture_cases: usize,
    /// Maximum number of mutations across every case of one suite.
    /// Experimental.
    pub fixture_mutations: usize,
    /// Maximum total bytes of fixture files and payloads read for one
    /// suite. Experimental.
    pub fixture_bytes: u64,
    /// Maximum commits one history run inspects. Experimental.
    pub history_commits: usize,
    /// Maximum changed paths across every commit of one history run.
    /// Experimental.
    pub history_changes: usize,
    /// Maximum size of one commit object, headers and message together,
    /// or of a pending message file. Experimental.
    pub history_commit_bytes: u64,
    /// Maximum total bytes read for history facts in one run: commit
    /// objects, listings, and the pending message. Experimental.
    pub history_bytes: u64,
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
            files: 20_000,
            file_bytes: 8 * 1024 * 1024,
            hygiene_bytes: 256 * 1024 * 1024,
            fixture_cases: 200,
            fixture_mutations: 2_000,
            fixture_bytes: 16 * 1024 * 1024,
            history_commits: 10_000,
            history_changes: 100_000,
            history_commit_bytes: 64 * 1024,
            history_bytes: 64 * 1024 * 1024,
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
    /// The files subject to hygiene and formatting, when selected.
    /// Experimental.
    pub hygiene: Option<Hygiene>,
    /// Repository-declared external formatters, in declaration order.
    /// Experimental.
    pub formatters: Vec<Formatter>,
    /// Contract fixture files, sorted: the only files `bearout test`
    /// reads cases from. Empty when the bootstrap declares no
    /// `[fixtures]`. Experimental.
    pub fixture_files: Vec<ProjectPath>,
    /// Resource limits.
    pub limits: Limits,
}

/// How the hygiene selection finds its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every file of the project as Git knows it: the captured index or
    /// revision tree, or for the working directory the tracked plus the
    /// untracked, non-ignored files.
    Repository,
    /// Only the declared roots and files.
    Declared,
}

/// The `[hygiene]` grant: which files native text hygiene and the declared
/// formatters apply to. Every list holds project paths; a directory names
/// everything beneath it.
#[derive(Debug, Clone)]
pub struct Hygiene {
    pub scope: Scope,
    /// Directories walked recursively when the scope is `declared`.
    pub roots: Vec<ProjectPath>,
    /// Files named one by one when the scope is `declared`.
    pub files: Vec<ProjectPath>,
    /// Paths never selected.
    pub exclude: Vec<ProjectPath>,
    /// Selected paths treated as binary regardless of content.
    pub binary: Vec<ProjectPath>,
    /// Selected paths treated as text regardless of content.
    pub text: Vec<ProjectPath>,
}

/// One `[[formatters]]` entry: a trusted host program that canonicalizes
/// the bytes of the selected files it is assigned.
#[derive(Debug, Clone)]
pub struct Formatter {
    /// Lowercase kebab-case name, unique among formatters.
    pub name: String,
    /// Executable followed by its arguments; `{path}` in an argument is
    /// replaced by the project-relative path of the file being formatted.
    pub command: Vec<String>,
    /// Paths (directories or files) the formatter is assigned; empty means
    /// the whole selection.
    pub paths: Vec<ProjectPath>,
    /// File extensions the formatter is assigned, without the dot; empty
    /// means any.
    pub extensions: Vec<String>,
    /// Configuration or support files copied from the selected tree into
    /// the formatter's confined working directory.
    pub support: Vec<ProjectPath>,
    /// Wall-clock bound per file.
    pub timeout: Duration,
}

/// The placeholder replaced by the project-relative path in a formatter
/// argument.
pub const PATH_PLACEHOLDER: &str = "{path}";

/// Default formatter timeout per file.
pub const DEFAULT_FORMATTER_TIMEOUT: Duration = Duration::from_secs(60);

impl Bootstrap {
    /// `true` when the bootstrap declares a `[documents]` table.
    #[must_use]
    pub fn selects_documents(&self) -> bool {
        !self.document_roots.is_empty() || !self.document_files.is_empty()
    }

    /// `true` when the bootstrap declares a `[fixtures]` table.
    #[must_use]
    pub fn declares_fixtures(&self) -> bool {
        !self.fixture_files.is_empty()
    }
}

/// The file extension every fixture file carries.
pub const FIXTURE_EXTENSION: &str = "toml";

/// The file extension of a Markdown document, for resources and
/// schema-less documents alike.
pub const MARKDOWN_EXTENSION: &str = "md";

const TOP_LEVEL: [&str; 11] = [
    "version",
    "entry",
    "resources",
    "rules",
    "templates",
    "outputs",
    "documents",
    "hygiene",
    "formatters",
    "fixtures",
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

    let hygiene = match table(root, "hygiene")? {
        Some(hygiene) => Some(parse_hygiene(hygiene)?),
        None => None,
    };
    let formatters = match root.get("formatters") {
        None => Vec::new(),
        Some(_) if hygiene.is_none() => {
            return Err("`[[formatters]]` needs a `[hygiene]` selection to apply to".to_owned());
        }
        Some(Item::ArrayOfTables(tables)) => {
            let mut formatters = Vec::new();
            for (index, table) in tables.iter().enumerate() {
                let formatter = parse_formatter(table, index)?;
                if formatters
                    .iter()
                    .any(|existing: &Formatter| existing.name == formatter.name)
                {
                    return Err(format!("`[[formatters]]` names `{}` twice", formatter.name));
                }
                formatters.push(formatter);
            }
            formatters
        }
        Some(_) => return Err("`formatters` must be an array of tables".to_owned()),
    };

    let fixture_files = match table(root, "fixtures")? {
        Some(fixtures) => parse_fixtures(fixtures)?,
        None => Vec::new(),
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
        hygiene,
        formatters,
        fixture_files,
        limits,
    };
    check_roots(&bootstrap)?;
    check_fixture_files(&bootstrap)?;
    Ok(bootstrap)
}

/// `[fixtures]`: `files` names every fixture file one by one. Nothing is
/// scanned for; the list is sorted, a repeated entry is an error, and
/// every file carries the TOML extension.
fn parse_fixtures(fixtures: &Table) -> Result<Vec<ProjectPath>, String> {
    reject_unknown(fixtures, "fixtures", &["files"])?;
    let mut files = path_list(
        required(fixtures, "files").map_err(|_| "`fixtures.files` is required".to_owned())?,
        "fixtures.files",
    )?;
    if files.is_empty() {
        return Err("`fixtures.files` must name at least one fixture file".to_owned());
    }
    for file in &files {
        if file.as_str().is_empty() {
            return Err("`fixtures.files` must not include the project root".to_owned());
        }
        if file.extension() != Some(FIXTURE_EXTENSION) {
            return Err(format!(
                "`fixtures.files` `{file}` must be a `.{FIXTURE_EXTENSION}` file"
            ));
        }
        if file.as_str() == MANIFEST_NAME || file.as_str() == STATE_NAME {
            return Err(format!(
                "`fixtures.files` `{file}` is a Bearout manifest, not a fixture file"
            ));
        }
    }
    files.sort();
    Ok(files)
}

/// A fixture file must not lie where discovery or delivery would treat
/// it as something else: beneath a resource root it would be parsed as a
/// resource, beneath an output root it could be delivered over.
fn check_fixture_files(bootstrap: &Bootstrap) -> Result<(), String> {
    for file in &bootstrap.fixture_files {
        for root in &bootstrap.resource_roots {
            if file.is_within(root) {
                return Err(format!(
                    "`fixtures.files` `{file}` lies beneath resource root `{root}`; fixture files must not be discovered as resources"
                ));
            }
        }
        for root in &bootstrap.output_roots {
            if file.is_within(root) {
                return Err(format!(
                    "`fixtures.files` `{file}` lies beneath output root `{root}`; fixture files must not be generated outputs"
                ));
            }
        }
    }
    Ok(())
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

/// `[hygiene]`: `scope` is required; `roots` and `files` select for the
/// `declared` scope and are meaningless for `repository`; `exclude`,
/// `binary`, and `text` refine any selection. No entry may be the project
/// root, and a path may not be both binary and text.
fn parse_hygiene(hygiene: &Table) -> Result<Hygiene, String> {
    reject_unknown(
        hygiene,
        "hygiene",
        &["scope", "roots", "files", "exclude", "binary", "text"],
    )?;
    let scope = match required(hygiene, "scope")
        .map_err(|_| "`hygiene.scope` is required".to_owned())?
        .as_str()
    {
        Some("repository") => Scope::Repository,
        Some("declared") => Scope::Declared,
        _ => {
            return Err("`hygiene.scope` must be `\"repository\"` or `\"declared\"`".to_owned());
        }
    };
    let list = |key: &str| -> Result<Vec<ProjectPath>, String> {
        let mut paths = match hygiene.get(key) {
            Some(item) => path_list(item, &format!("hygiene.{key}"))?,
            None => Vec::new(),
        };
        for path in &paths {
            if path.as_str().is_empty() {
                return Err(format!(
                    "`hygiene.{key}` must not include the project root itself"
                ));
            }
        }
        paths.sort();
        Ok(paths)
    };
    let roots = list("roots")?;
    let files = list("files")?;
    let exclude = list("exclude")?;
    let binary = list("binary")?;
    let text = list("text")?;
    match scope {
        Scope::Declared if roots.is_empty() && files.is_empty() => {
            return Err(
                "`hygiene.scope = \"declared\"` needs `hygiene.roots` or `hygiene.files`"
                    .to_owned(),
            );
        }
        Scope::Repository if !roots.is_empty() || !files.is_empty() => {
            return Err(
                "`hygiene.roots` and `hygiene.files` apply only to `hygiene.scope = \"declared\"`"
                    .to_owned(),
            );
        }
        _ => {}
    }
    for path in &binary {
        if text.iter().any(|other| other == path) {
            return Err(format!(
                "`hygiene.binary` and `hygiene.text` both name `{path}`"
            ));
        }
    }
    Ok(Hygiene {
        scope,
        roots,
        files,
        exclude,
        binary,
        text,
    })
}

/// One `[[formatters]]` table.
fn parse_formatter(table: &Table, index: usize) -> Result<Formatter, String> {
    let label = format!("formatters[{index}]");
    reject_unknown(
        table,
        &label,
        &[
            "name",
            "command",
            "paths",
            "extensions",
            "support",
            "timeout",
        ],
    )?;
    let name = required(table, "name")
        .map_err(|_| format!("`{label}.name` is required"))?
        .as_str()
        .ok_or_else(|| format!("`{label}.name` must be a string"))?;
    identity::check_kind(name).map_err(|error| format!("`{label}.name`: {error}"))?;
    let command = required(table, "command")
        .map_err(|_| format!("`{label}.command` is required"))?
        .as_array()
        .ok_or_else(|| format!("`{label}.command` must be an array of strings"))?;
    let command: Vec<String> = command
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("`{label}.command` must be an array of strings"))
        })
        .collect::<Result<_, _>>()?;
    match command.first() {
        None => return Err(format!("`{label}.command` must name an executable")),
        Some(executable) if executable.is_empty() || executable.contains('\0') => {
            return Err(format!("`{label}.command` must name an executable"));
        }
        Some(executable) if executable.contains(PATH_PLACEHOLDER) => {
            return Err(format!(
                "`{label}.command`: the executable may not contain `{PATH_PLACEHOLDER}`"
            ));
        }
        Some(_) => {}
    }
    if command.iter().any(|argument| argument.contains('\0')) {
        return Err(format!("`{label}.command` must not contain NUL"));
    }
    let list = |key: &str| -> Result<Vec<ProjectPath>, String> {
        let mut paths = match table.get(key) {
            Some(item) => path_list(item, &format!("{label}.{key}"))?,
            None => Vec::new(),
        };
        for path in &paths {
            if path.as_str().is_empty() {
                return Err(format!("`{label}.{key}` must not include the project root"));
            }
        }
        paths.sort();
        Ok(paths)
    };
    let paths = list("paths")?;
    let support = list("support")?;
    let mut extensions: Vec<String> = match table.get("extensions") {
        None => Vec::new(),
        Some(item) => item
            .as_array()
            .ok_or_else(|| format!("`{label}.extensions` must be an array of strings"))?
            .iter()
            .map(|value| {
                let text = value
                    .as_str()
                    .ok_or_else(|| format!("`{label}.extensions` must be an array of strings"))?;
                if text.is_empty() || text.contains('.') || text.contains('/') {
                    return Err(format!(
                        "`{label}.extensions` entries are bare extensions such as `py`, found `{text}`"
                    ));
                }
                Ok(text.to_owned())
            })
            .collect::<Result<_, _>>()?,
    };
    extensions.sort();
    extensions.dedup();
    let timeout = match table.get("timeout") {
        None => DEFAULT_FORMATTER_TIMEOUT,
        Some(item) => {
            let seconds = item
                .as_integer()
                .filter(|seconds| (1..=3600).contains(seconds))
                .ok_or_else(|| {
                    format!("`{label}.timeout` must be a number of seconds from 1 to 3600")
                })?;
            Duration::from_secs(seconds.unsigned_abs())
        }
    };
    Ok(Formatter {
        name: name.to_owned(),
        command,
        paths,
        extensions,
        support,
        timeout,
    })
}

fn parse_limits(limits: &Table) -> Result<Limits, String> {
    const KEYS: [&str; 19] = [
        "ticks",
        "heap_bytes",
        "call_stack",
        "resources",
        "resource_bytes",
        "template_fuel",
        "output_bytes",
        "documents",
        "document_bytes",
        "files",
        "file_bytes",
        "hygiene_bytes",
        "fixture_cases",
        "fixture_mutations",
        "fixture_bytes",
        "history_commits",
        "history_changes",
        "history_commit_bytes",
        "history_bytes",
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
    if let Some(value) = positive("files")? {
        result.files =
            usize::try_from(value).map_err(|_| "`limits.files` is too large".to_owned())?;
    }
    if let Some(value) = positive("file_bytes")? {
        result.file_bytes = value;
    }
    if let Some(value) = positive("hygiene_bytes")? {
        result.hygiene_bytes = value;
    }
    if let Some(value) = positive("fixture_cases")? {
        result.fixture_cases =
            usize::try_from(value).map_err(|_| "`limits.fixture_cases` is too large".to_owned())?;
    }
    if let Some(value) = positive("fixture_mutations")? {
        result.fixture_mutations = usize::try_from(value)
            .map_err(|_| "`limits.fixture_mutations` is too large".to_owned())?;
    }
    if let Some(value) = positive("fixture_bytes")? {
        result.fixture_bytes = value;
    }
    if let Some(value) = positive("history_commits")? {
        result.history_commits = usize::try_from(value)
            .map_err(|_| "`limits.history_commits` is too large".to_owned())?;
    }
    if let Some(value) = positive("history_changes")? {
        result.history_changes = usize::try_from(value)
            .map_err(|_| "`limits.history_changes` is too large".to_owned())?;
    }
    if let Some(value) = positive("history_commit_bytes")? {
        result.history_commit_bytes = value;
    }
    if let Some(value) = positive("history_bytes")? {
        result.history_bytes = value;
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
    fn selects_hygiene_and_formatters_explicitly() {
        let text = format!(
            "{MINIMAL}[hygiene]\nscope = \"repository\"\nexclude = [\"generated\", \"vendor\"]\nbinary = [\"assets\"]\ntext = [\"assets/notes.bin\"]\n\n[[formatters]]\nname = \"py\"\ncommand = [\"ruff\", \"format\", \"--stdin-filename\", \"{{path}}\", \"-\"]\npaths = [\"tools\"]\nextensions = [\"py\", \"pyi\", \"py\"]\nsupport = [\"ruff.toml\"]\ntimeout = 5\n"
        );
        let bootstrap = parse(&text).unwrap();
        let hygiene = bootstrap.hygiene.as_ref().unwrap();
        assert_eq!(hygiene.scope, Scope::Repository);
        assert_eq!(hygiene.exclude.len(), 2);
        assert_eq!(bootstrap.formatters.len(), 1);
        let formatter = &bootstrap.formatters[0];
        assert_eq!(formatter.name, "py");
        assert_eq!(formatter.command[3], "{path}");
        assert_eq!(
            formatter.extensions,
            ["py", "pyi"],
            "sorted and deduplicated"
        );
        assert_eq!(formatter.timeout, Duration::from_secs(5));
        assert!(parse(MINIMAL).unwrap().hygiene.is_none());
        let declared = parse(&format!(
            "{MINIMAL}[hygiene]\nscope = \"declared\"\nroots = [\"docs\"]\nfiles = [\"README.md\"]\n"
        ))
        .unwrap();
        assert_eq!(declared.hygiene.unwrap().scope, Scope::Declared);
        assert_eq!(parse(&text).unwrap().limits.files, 20_000);

        let cases = [
            ("[hygiene]\n", "`hygiene.scope` is required"),
            (
                "[hygiene]\nscope = \"all\"\n",
                "must be `\"repository\"` or `\"declared\"`",
            ),
            (
                "[hygiene]\nscope = \"declared\"\n",
                "needs `hygiene.roots` or `hygiene.files`",
            ),
            (
                "[hygiene]\nscope = \"repository\"\nroots = [\"docs\"]\n",
                "apply only to",
            ),
            (
                "[hygiene]\nscope = \"repository\"\nexclude = [\"\"]\n",
                "project root itself",
            ),
            (
                "[hygiene]\nscope = \"repository\"\nbinary = [\"a\"]\ntext = [\"a\"]\n",
                "both name `a`",
            ),
            (
                "[hygiene]\nscope = \"repository\"\nglob = [\"*.md\"]\n",
                "unknown key `hygiene.glob`",
            ),
            (
                "[[formatters]]\nname = \"x\"\ncommand = [\"x\"]\n",
                "needs a `[hygiene]` selection",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\ncommand = [\"x\"]\n",
                "`formatters[0].name` is required",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\nname = \"Bad Name\"\ncommand = [\"x\"]\n",
                "`formatters[0].name`",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\nname = \"x\"\ncommand = []\n",
                "must name an executable",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\nname = \"x\"\ncommand = \"x -\"\n",
                "array of strings",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\nname = \"x\"\ncommand = [\"{path}\"]\n",
                "may not contain `{path}`",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\nname = \"x\"\ncommand = [\"x\"]\nextensions = [\".py\"]\n",
                "bare extensions",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\nname = \"x\"\ncommand = [\"x\"]\ntimeout = 0\n",
                "1 to 3600",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\nname = \"x\"\ncommand = [\"x\"]\nshell = true\n",
                "unknown key `formatters[0].shell`",
            ),
            (
                "[hygiene]\nscope = \"repository\"\n[[formatters]]\nname = \"x\"\ncommand = [\"x\"]\n[[formatters]]\nname = \"x\"\ncommand = [\"y\"]\n",
                "names `x` twice",
            ),
            ("[limits]\nfiles = 0\n", "limits.files"),
            ("[limits]\nfile_bytes = -5\n", "limits.file_bytes"),
            ("[limits]\nhygiene_bytes = 0\n", "limits.hygiene_bytes"),
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

    #[test]
    fn declares_fixture_files_explicitly() {
        let bootstrap = parse(MINIMAL).unwrap();
        assert!(!bootstrap.declares_fixtures());
        assert_eq!(bootstrap.limits.fixture_cases, 200);
        assert_eq!(bootstrap.limits.fixture_mutations, 2_000);
        assert_eq!(bootstrap.limits.fixture_bytes, 16 * 1024 * 1024);

        let text = format!(
            "{MINIMAL}[fixtures]\nfiles = [\"tests/b.test.toml\", \"tests/a.test.toml\"]\n[limits]\nfixture_cases = 3\nfixture_mutations = 9\nfixture_bytes = 4096\n"
        );
        let bootstrap = parse(&text).unwrap();
        assert!(bootstrap.declares_fixtures());
        let files: Vec<&str> = bootstrap
            .fixture_files
            .iter()
            .map(ProjectPath::as_str)
            .collect();
        assert_eq!(files, ["tests/a.test.toml", "tests/b.test.toml"], "sorted");
        assert_eq!(bootstrap.limits.fixture_cases, 3);
        assert_eq!(bootstrap.limits.fixture_mutations, 9);
        assert_eq!(bootstrap.limits.fixture_bytes, 4096);

        let cases = [
            ("[fixtures]\n", "`fixtures.files` is required"),
            ("[fixtures]\nfiles = []\n", "at least one fixture file"),
            ("[fixtures]\nfiles = \"a.toml\"\n", "array of strings"),
            (
                "[fixtures]\nfiles = [\"a.toml\", \"a.toml\"]\n",
                "lists `a.toml` twice",
            ),
            ("[fixtures]\nfiles = [\"a.md\"]\n", "must be a `.toml` file"),
            ("[fixtures]\nfiles = [\"\"]\n", "project root"),
            ("[fixtures]\nfiles = [\"/a.toml\"]\n", "absolute"),
            ("[fixtures]\nfiles = [\"../a.toml\"]\n", "normalized"),
            (
                "[fixtures]\nfiles = [\"bearout.toml\"]\n",
                "is a Bearout manifest",
            ),
            (
                "[fixtures]\nfiles = [\"bearout-state.toml\"]\n",
                "is a Bearout manifest",
            ),
            (
                "[fixtures]\nfiles = [\"content/a.toml\"]\n",
                "beneath resource root `content`",
            ),
            (
                "[outputs]\nroots = [\"generated\"]\n[fixtures]\nfiles = [\"generated/a.toml\"]\n",
                "beneath output root `generated`",
            ),
            (
                "[fixtures]\nfiles = [\"a.toml\"]\nroots = [\"x\"]\n",
                "unknown key `fixtures.roots`",
            ),
            ("[limits]\nfixture_cases = 0\n", "limits.fixture_cases"),
            (
                "[limits]\nfixture_mutations = -1\n",
                "limits.fixture_mutations",
            ),
            ("[limits]\nfixture_bytes = 0\n", "limits.fixture_bytes"),
            ("[limits]\nhistory_commits = 0\n", "limits.history_commits"),
            ("[limits]\nhistory_changes = -2\n", "limits.history_changes"),
            (
                "[limits]\nhistory_commit_bytes = 0\n",
                "limits.history_commit_bytes",
            ),
            ("[limits]\nhistory_bytes = 0\n", "limits.history_bytes"),
        ];
        for (body, expected) in cases {
            let error = parse(&format!("{MINIMAL}{body}")).unwrap_err();
            assert!(error.contains(expected), "{body:?} -> {error}");
        }
    }
}
