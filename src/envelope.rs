// SPDX-License-Identifier: Apache-2.0

//! The resource envelope: TOML front matter parsed with `toml_edit` over an
//! exact byte range, a Markdown body kept byte-for-byte, and typed fragments
//! carried in fenced blocks.
//!
//! The kernel owns three envelope keys: `schema`, `id`, and `refs`. Every
//! other top-level key is a repository-owned field validated by the schema's
//! shape. Native TOML dates reach shapes, scripts, and reports as their TOML
//! text (the RFC 3339 profile TOML uses), so `2026-01-02`, `10:00:00`,
//! `2026-01-02T10:00:00`, and `2026-01-02T10:00:00Z` stay distinguishable.

use std::collections::BTreeMap;

use serde_json::{Map, Value};
use toml_edit::{Document as TomlDocument, DocumentMut, Item, Table};

use crate::identity;
use crate::markdown::{self, Document};
use crate::paths::ProjectPath;
use crate::report::{Code, Diagnostic};

const FENCE: &str = "+++";

/// Front-matter keys the kernel owns.
pub const RESERVED_KEYS: [&str; 3] = ["schema", "id", "refs"];

/// The fenced-block attribute that marks a typed fragment.
pub const FRAGMENT_ATTR: &str = "bearout";

/// A typed, identified piece of structured data inside a resource body.
#[derive(Debug, Clone)]
pub struct Fragment {
    /// Kind named by the block's `bearout=` attribute.
    pub kind: String,
    /// Identifier, unique across the project like a resource id.
    pub id: String,
    /// The block's TOML content as JSON, including `id`.
    pub fields: Value,
    /// One-based line of the opening fence.
    pub line: u32,
    /// Index of the enclosing section.
    pub section: Option<usize>,
}

/// One parsed resource.
#[derive(Debug)]
pub struct Resource {
    /// Project-relative path.
    pub path: ProjectPath,
    /// Repository-owned schema identifier.
    pub schema: String,
    /// Stable identifier.
    pub id: String,
    /// Untyped references.
    pub refs: Vec<String>,
    /// Repository-owned front-matter fields as JSON.
    pub fields: Value,
    /// One-based line of each top-level field key, for diagnostics.
    pub field_lines: BTreeMap<String, u32>,
    /// Markdown body, byte-for-byte as written.
    pub body: String,
    /// Number of lines in the file.
    pub line_count: u32,
    /// Structure of the body.
    pub doc: Document,
    /// Typed fragments extracted from fenced blocks.
    pub fragments: Vec<Fragment>,
}

/// Parse one resource. Envelope failures return a diagnostic; fragment
/// failures are appended to `diagnostics` and the resource is still returned
/// so that the graph can see it, but it is not structurally valid.
pub fn parse(
    path: &ProjectPath,
    bytes: &[u8],
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<Resource, Diagnostic> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        Diagnostic::new(
            Code::Envelope,
            path.as_str(),
            format!("resource is not valid UTF-8: {error}"),
        )
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let line_count = u32::try_from(text.split_inclusive('\n').count()).unwrap_or(u32::MAX);

    let (header, header_line, body, body_line) = match path.extension() {
        Some("md") => split_front_matter(text)
            .map_err(|message| Diagnostic::new(Code::Envelope, path.as_str(), message))?,
        Some("toml") => (text, 1, "", line_count + 1),
        other => {
            return Err(Diagnostic::new(
                Code::Envelope,
                path.as_str(),
                format!("unsupported resource extension {other:?}; expected `md` or `toml`"),
            ));
        }
    };

    let doc: TomlDocument<&str> =
        TomlDocument::parse(header).map_err(|error: toml_edit::TomlError| {
            let line = error
                .span()
                .map(|span| header_line + line_offset(header, span.start));
            Diagnostic::new(
                Code::Envelope,
                path.as_str(),
                format!("invalid front matter: {}", error.message()),
            )
            .at_line(line)
        })?;
    let table = doc.as_table();
    let header_diagnostic = |message: String, key: Option<&str>| {
        let line = key
            .and_then(|key| table.get_key_value(key).and_then(|(key, _)| key.span()))
            .map(|span| header_line + line_offset(header, span.start));
        Diagnostic::new(Code::Envelope, path.as_str(), message).at_line(line)
    };

    let schema = table
        .get("schema")
        .and_then(Item::as_str)
        .ok_or_else(|| header_diagnostic("`schema` must be a string".to_owned(), Some("schema")))?
        .to_owned();
    identity::check_schema_id(&schema).map_err(|error| {
        Diagnostic::new(Code::SchemaIdentity, path.as_str(), error).at_line(key_line(
            table,
            header,
            header_line,
            "schema",
        ))
    })?;
    let id = table
        .get("id")
        .and_then(Item::as_str)
        .ok_or_else(|| header_diagnostic("`id` must be a string".to_owned(), Some("id")))?
        .to_owned();
    identity::check_id(&id).map_err(|error| header_diagnostic(error, Some("id")))?;
    let refs = match table.get("refs") {
        None => Vec::new(),
        Some(item) => {
            let array = item.as_array().ok_or_else(|| {
                header_diagnostic(
                    "`refs` must be an array of strings".to_owned(),
                    Some("refs"),
                )
            })?;
            let mut refs = Vec::new();
            for value in array {
                let text = value.as_str().ok_or_else(|| {
                    header_diagnostic(
                        "`refs` must be an array of strings".to_owned(),
                        Some("refs"),
                    )
                })?;
                if refs.contains(&text.to_owned()) {
                    return Err(header_diagnostic(
                        format!("`refs` lists `{text}` twice"),
                        Some("refs"),
                    ));
                }
                refs.push(text.to_owned());
            }
            refs
        }
    };

    let mut fields = Map::new();
    let mut field_lines = BTreeMap::new();
    for (key, item) in table {
        if RESERVED_KEYS.contains(&key) {
            continue;
        }
        fields.insert(key.to_owned(), item_to_json(item));
        if let Some(line) = key_line(table, header, header_line, key) {
            field_lines.insert(key.to_owned(), line);
        }
    }

    let doc = markdown::parse(body, body_line);
    let fragments = extract_fragments(path, &doc, diagnostics);

    Ok(Resource {
        path: path.clone(),
        schema,
        id,
        refs,
        fields: Value::Object(fields),
        field_lines,
        body: body.to_owned(),
        line_count,
        doc,
        fragments,
    })
}

fn key_line(table: &Table, header: &str, header_line: u32, key: &str) -> Option<u32> {
    table
        .get_key_value(key)
        .and_then(|(key, item)| key.span().or_else(|| item.span()))
        .map(|span| header_line + line_offset(header, span.start))
}

/// Number of line breaks before byte `offset`.
fn line_offset(text: &str, offset: usize) -> u32 {
    u32::try_from(text[..offset.min(text.len())].matches('\n').count()).unwrap_or(u32::MAX)
}

/// Split `+++` front matter from the body. Returns the header text, the
/// one-based line on which the header text starts, the body byte-for-byte,
/// and the one-based line on which the body starts.
///
/// A line consisting of `+++` may legitimately occur inside a TOML
/// multi-line string. The closing fence is therefore the first `+++` line
/// at which the accumulated header parses as TOML; if none does, the parse
/// error of the first candidate is reported so that the diagnostic points
/// at the earliest plausible header.
fn split_front_matter(text: &str) -> Result<(&str, u32, &str, u32), String> {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return Err(format!(
            "resource must begin with TOML front matter delimited by `{FENCE}`"
        ));
    };
    if first.trim_end_matches(['\r', '\n']) != FENCE {
        return Err(format!(
            "resource must begin with TOML front matter delimited by `{FENCE}`"
        ));
    }
    let header_start = first.len();
    let mut offset = header_start;
    let mut first_candidate: Option<(&str, &str, u32)> = None;
    for (index, line) in lines.enumerate() {
        if line.trim_end_matches(['\r', '\n']) == FENCE {
            let header = &text[header_start..offset];
            let body = &text[offset + line.len()..];
            let body_line = u32::try_from(index + 3).unwrap_or(u32::MAX);
            if TomlDocument::parse(header).is_ok() {
                return Ok((header, 2, body, body_line));
            }
            first_candidate.get_or_insert((header, body, body_line));
        }
        offset += line.len();
    }
    match first_candidate {
        Some((header, body, body_line)) => Ok((header, 2, body, body_line)),
        None => Err(format!(
            "resource front matter has no closing `{FENCE}` delimiter"
        )),
    }
}

fn extract_fragments(
    path: &ProjectPath,
    doc: &Document,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Fragment> {
    let mut fragments = Vec::new();
    for block in &doc.blocks {
        let Some(kind) = block.attrs.get(FRAGMENT_ATTR) else {
            continue;
        };
        let report = |message: String| {
            Diagnostic::new(Code::FragmentMalformed, path.as_str(), message)
                .at_line(Some(block.line))
        };
        if let Err(error) = identity::check_kind(kind) {
            diagnostics.push(report(format!("fragment kind: {error}")));
            continue;
        }
        if block.lang != "toml" {
            diagnostics.push(report(format!(
                "fragment `{kind}` must be a `toml` block, found `{}`",
                block.lang
            )));
            continue;
        }
        let table: DocumentMut = match block.content.parse() {
            Ok(table) => table,
            Err(error) => {
                let error: toml_edit::TomlError = error;
                diagnostics.push(report(format!(
                    "fragment `{kind}` is not valid TOML: {}",
                    error.message()
                )));
                continue;
            }
        };
        let Some(id) = table.get("id").and_then(Item::as_str) else {
            diagnostics.push(report(format!(
                "fragment `{kind}` must carry a string `id`"
            )));
            continue;
        };
        if let Err(error) = identity::check_id(id) {
            diagnostics.push(report(format!("fragment `{kind}`: {error}")));
            continue;
        }
        fragments.push(Fragment {
            kind: kind.clone(),
            id: id.to_owned(),
            fields: table_to_json(table.as_table()),
            line: block.line,
            section: block.section,
        });
    }
    fragments
}

/// Convert a TOML document table to JSON.
pub fn table_to_json(table: &Table) -> Value {
    Value::Object(
        table
            .iter()
            .map(|(key, item)| (key.to_owned(), item_to_json(item)))
            .collect(),
    )
}

fn item_to_json(item: &Item) -> Value {
    match item {
        Item::None => Value::Null,
        Item::Value(value) => value_to_json(value),
        Item::Table(table) => table_to_json(table),
        Item::ArrayOfTables(tables) => Value::Array(tables.iter().map(table_to_json).collect()),
    }
}

fn value_to_json(value: &toml_edit::Value) -> Value {
    match value {
        toml_edit::Value::String(s) => Value::String(s.value().clone()),
        toml_edit::Value::Integer(i) => Value::from(*i.value()),
        toml_edit::Value::Float(f) => Value::from(*f.value()),
        toml_edit::Value::Boolean(b) => Value::Bool(*b.value()),
        toml_edit::Value::Datetime(d) => Value::String(d.value().to_string()),
        toml_edit::Value::Array(items) => Value::Array(items.iter().map(value_to_json).collect()),
        toml_edit::Value::InlineTable(table) => Value::Object(
            table
                .iter()
                .map(|(key, value)| (key.to_owned(), value_to_json(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(name: &str, text: &str) -> Resource {
        let mut diagnostics = Vec::new();
        let resource = parse(
            &ProjectPath::parse(name).unwrap(),
            text.as_bytes(),
            &mut diagnostics,
        )
        .expect("parses");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        resource
    }

    #[test]
    fn splits_front_matter_and_keeps_the_body_verbatim() {
        let resource = parse_ok(
            "n.md",
            "+++\nschema = \"x/y@1\"\nid = \"a\"\ntitle = \"T\"\n+++\n\n# Body\r\ntext  \n",
        );
        assert_eq!(resource.body, "\n# Body\r\ntext  \n");
        assert_eq!(resource.fields["title"], "T");
        assert_eq!(resource.field_lines["title"], 4);
        assert_eq!(resource.line_count, 8);
    }

    #[test]
    fn fence_lines_inside_toml_strings_do_not_close_the_header() {
        let resource = parse_ok(
            "n.md",
            "+++\nschema = \"x/y@1\"\nid = \"a\"\ntext = \"\"\"\n+++\nstill header\n\"\"\"\n+++\n# Body\n",
        );
        assert_eq!(resource.fields["text"], "+++\nstill header\n");
        assert_eq!(resource.body, "# Body\n");
        let mut diagnostics = Vec::new();
        let error = parse(
            &ProjectPath::parse("n.md").unwrap(),
            b"+++\nschema = \"x/y@1\"\nid = \"a\"\nbroken = [\n+++\nbody\n+++\n",
            &mut diagnostics,
        )
        .unwrap_err();
        assert_eq!(
            error.line,
            Some(4),
            "the first candidate header's error is reported"
        );
    }

    #[test]
    fn accepts_crlf_and_bom() {
        let resource = parse_ok(
            "n.md",
            "\u{feff}+++\r\nschema = \"x/y@1\"\r\nid = \"a\"\r\n+++\r\n# Body\r\n",
        );
        assert_eq!(resource.body, "# Body\r\n");
        assert_eq!(resource.doc.sections[0].title, "Body");
        assert_eq!(resource.doc.sections[0].line, 5);
    }

    #[test]
    fn header_only_toml_resources() {
        let resource = parse_ok(
            "t.toml",
            "schema = \"x/y@1\"\nid = \"tag-a\"\nlabel = \"A\"\n",
        );
        assert_eq!(resource.body, "");
        assert_eq!(resource.fields["label"], "A");
        assert!(resource.doc.sections.is_empty());
    }

    #[test]
    fn dates_have_one_textual_form() {
        let resource = parse_ok(
            "d.toml",
            "schema = \"x/y@1\"\nid = \"a\"\nd = 2026-01-02\nt = 10:00:00\nl = 2026-01-02T10:00:00\nz = 2026-01-02T10:00:00Z\no = 2026-01-02 10:00:00+02:00\n",
        );
        assert_eq!(resource.fields["d"], "2026-01-02");
        assert_eq!(resource.fields["t"], "10:00:00");
        assert_eq!(resource.fields["l"], "2026-01-02T10:00:00");
        assert_eq!(resource.fields["z"], "2026-01-02T10:00:00Z");
        assert_eq!(resource.fields["o"], "2026-01-02T10:00:00+02:00");
    }

    #[test]
    fn rejects_envelope_problems_with_lines() {
        let mut diagnostics = Vec::new();
        let path = ProjectPath::parse("n.md").unwrap();
        let error = parse(&path, b"# no front matter\n", &mut diagnostics).unwrap_err();
        assert_eq!(error.code, Code::Envelope);
        let error = parse(
            &path,
            b"+++\nschema = \"x/y@1\"\nid = 3\n+++\n",
            &mut diagnostics,
        )
        .unwrap_err();
        assert_eq!(error.line, Some(3));
        let error = parse(
            &path,
            b"+++\nschema = \"Bad\"\nid = \"a\"\n+++\n",
            &mut diagnostics,
        )
        .unwrap_err();
        assert_eq!(error.code, Code::SchemaIdentity);
        let error = parse(
            &path,
            b"+++\nschema = \"x/y@1\"\nid = \"a\"\nbroken = [\n+++\n",
            &mut diagnostics,
        )
        .unwrap_err();
        assert_eq!(error.code, Code::Envelope);
        assert!(error.line.is_some());
    }

    #[test]
    fn extracts_fragments() {
        let text = "+++\nschema = \"x/y@1\"\nid = \"a\"\n+++\n\n## a-part-1\n\n```toml bearout=part\nid = \"a-part-1\"\nn = 1\n```\n\n```toml bearout=Part\nid = \"x\"\n```\n";
        let mut diagnostics = Vec::new();
        let resource = parse(
            &ProjectPath::parse("n.md").unwrap(),
            text.as_bytes(),
            &mut diagnostics,
        )
        .unwrap();
        assert_eq!(resource.fragments.len(), 1);
        assert_eq!(resource.fragments[0].id, "a-part-1");
        assert_eq!(resource.fragments[0].fields["n"], 1);
        assert_eq!(resource.fragments[0].section, Some(0));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, Code::FragmentMalformed);
    }
}
