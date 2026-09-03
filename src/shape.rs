// SPDX-License-Identifier: Apache-2.0

//! Declarative shapes: JSON Schema 2020-12 authored in TOML, plus the
//! `x-bearout` vocabulary for typed relations, required sections, and
//! fragment kinds. The vocabulary itself is validated against a kernel
//! meta-schema so that a misspelled or mistyped extension is an error, not
//! a silently ignored keyword.

use std::collections::BTreeMap;

use jsonschema::error::ValidationErrorKind;
use jsonschema::{Draft, Validator};
use serde_json::{Value, json};
use toml_edit::DocumentMut;

use crate::envelope::{RESERVED_KEYS, table_to_json};
use crate::identity;

/// The dialect every shape must declare.
pub const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

const EXTENSION_KEY: &str = "x-bearout";

/// The shape configured for one schema.
pub struct Shape {
    validator: Validator,
    /// Relation field to the node kinds its targets may have. Empty means any.
    pub relations: BTreeMap<String, Vec<String>>,
    /// Titles of sections the body must contain.
    pub sections: Vec<String>,
    /// Fragment kinds this schema accepts, each with its validator.
    pub fragments: BTreeMap<String, Validator>,
}

impl std::fmt::Debug for Shape {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shape")
            .field("relations", &self.relations)
            .field("sections", &self.sections)
            .field("fragments", &self.fragments.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// One shape violation.
pub struct Violation {
    /// Dotted path of the offending value, or empty for the whole object.
    pub location: String,
    pub message: String,
    /// The first unexpected property, for `additionalProperties` violations.
    pub unexpected: Option<String>,
}

/// Parse a TOML-authored shape.
pub fn parse(text: &str) -> Result<Shape, String> {
    let doc: DocumentMut = text
        .parse()
        .map_err(|error: toml_edit::TomlError| format!("not valid TOML: {}", error.message()))?;
    let Value::Object(mut schema) = table_to_json(doc.as_table()) else {
        unreachable!("a TOML document is a table")
    };

    match schema.get("$schema") {
        Some(Value::String(dialect)) if dialect == DIALECT => {}
        Some(other) => return Err(format!("`$schema` must be `{DIALECT}`, found {other}")),
        None => return Err(format!("shape must declare `\"$schema\" = \"{DIALECT}\"`")),
    }

    let extension = schema.remove(EXTENSION_KEY).unwrap_or_else(|| json!({}));
    validate_meta(&top_level_meta(), &extension, EXTENSION_KEY)?;
    let sections = extension["sections"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let mut fragments = BTreeMap::new();
    if let Some(Value::Object(kinds)) = extension.get("fragments") {
        for (kind, definition) in kinds {
            identity::check_kind(kind)
                .map_err(|error| format!("`{EXTENSION_KEY}.fragments`: {error}"))?;
            if definition.get(EXTENSION_KEY).is_some() {
                return Err(format!(
                    "`{EXTENSION_KEY}.fragments.{kind}` must not nest `{EXTENSION_KEY}`"
                ));
            }
            let validator =
                build(definition).map_err(|error| format!("fragment `{kind}`: {error}"))?;
            fragments.insert(kind.clone(), validator);
        }
    }

    let mut relations = BTreeMap::new();
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        for (field, property) in properties.iter_mut() {
            if RESERVED_KEYS.contains(&field.as_str()) {
                return Err(format!(
                    "`{field}` is an envelope key owned by Bearout and cannot be declared as a property"
                ));
            }
            let Value::Object(property) = property else {
                return Err(format!("`properties.{field}` must be a schema object"));
            };
            let Some(extension) = property.remove(EXTENSION_KEY) else {
                continue;
            };
            validate_meta(
                &property_meta(),
                &extension,
                &format!("properties.{field}.{EXTENSION_KEY}"),
            )?;
            let targets = relation_targets(&extension["ref"])
                .map_err(|error| format!("`properties.{field}`: {error}"))?;
            check_relation_type(field, property)?;
            relations.insert(field.clone(), targets);
        }
    }

    let validator = build(&Value::Object(schema))?;
    Ok(Shape {
        validator,
        relations,
        sections,
        fragments,
    })
}

fn relation_targets(value: &Value) -> Result<Vec<String>, String> {
    let targets: Vec<String> = match value {
        Value::String(target) => vec![target.clone()],
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => unreachable!("meta-schema admits only strings and arrays"),
    };
    for target in &targets {
        identity::check_target_kind(target).map_err(|error| format!("relation target: {error}"))?;
    }
    if targets.iter().any(|target| target == "*") {
        return Ok(Vec::new());
    }
    Ok(targets)
}

/// A relation must be declared on a string property or an array of strings.
fn check_relation_type(
    field: &str,
    property: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    match property.get("type").and_then(Value::as_str) {
        Some("string") => Ok(()),
        Some("array") => match property
            .get("items")
            .and_then(|items| items.get("type"))
            .and_then(Value::as_str)
        {
            Some("string") => Ok(()),
            _ => Err(format!(
                "`properties.{field}`: a relation on an array needs `items = {{ type = \"string\" }}`"
            )),
        },
        _ => Err(format!(
            "`properties.{field}`: a relation needs `type = \"string\"` or an array of strings"
        )),
    }
}

fn top_level_meta() -> Value {
    json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "sections": { "type": "array", "uniqueItems": true, "items": { "type": "string", "minLength": 1 } },
            "fragments": { "type": "object", "additionalProperties": { "type": "object" } }
        }
    })
}

fn property_meta() -> Value {
    json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "required": ["ref"],
        "properties": {
            "ref": {
                "oneOf": [
                    { "type": "string", "minLength": 1 },
                    { "type": "array", "minItems": 1, "uniqueItems": true, "items": { "type": "string", "minLength": 1 } }
                ]
            }
        }
    })
}

fn validate_meta(meta: &Value, instance: &Value, label: &str) -> Result<(), String> {
    let validator = build(meta).expect("kernel meta-schema is valid");
    let mut errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|error| {
            let location = error
                .instance_path()
                .as_str()
                .trim_start_matches('/')
                .replace('/', ".");
            if location.is_empty() {
                format!("`{label}`: {error}")
            } else {
                format!("`{label}.{location}`: {error}")
            }
        })
        .collect();
    errors.sort();
    match errors.into_iter().next() {
        None => Ok(()),
        Some(error) => Err(error),
    }
}

fn build(schema: &Value) -> Result<Validator, String> {
    jsonschema::options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .build(schema)
        .map_err(|error| format!("invalid JSON Schema: {error}"))
}

impl Shape {
    /// Violations of the front-matter fields.
    #[must_use]
    pub fn check(&self, fields: &Value) -> Vec<Violation> {
        violations(&self.validator, fields)
    }

    /// Violations of a fragment of `kind`, or `None` when the kind is not declared.
    #[must_use]
    pub fn check_fragment(&self, kind: &str, fields: &Value) -> Option<Vec<Violation>> {
        self.fragments
            .get(kind)
            .map(|validator| violations(validator, fields))
    }
}

fn violations(validator: &Validator, instance: &Value) -> Vec<Violation> {
    let mut found: Vec<Violation> = validator
        .iter_errors(instance)
        .map(|error| Violation {
            location: error
                .instance_path()
                .as_str()
                .trim_start_matches('/')
                .replace('/', "."),
            message: error.to_string(),
            unexpected: match error.kind() {
                ValidationErrorKind::AdditionalProperties { unexpected } => {
                    unexpected.first().cloned()
                }
                _ => None,
            },
        })
        .collect();
    found.sort_by(|a, b| a.location.cmp(&b.location).then(a.message.cmp(&b.message)));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str =
        "\"$schema\" = \"https://json-schema.org/draft/2020-12/schema\"\ntype = \"object\"\n";

    #[test]
    fn requires_the_dialect() {
        assert!(
            parse("type = \"object\"\n")
                .unwrap_err()
                .contains("$schema")
        );
        assert!(
            parse("\"$schema\" = \"http://json-schema.org/draft-07/schema#\"\n")
                .unwrap_err()
                .contains("2020-12")
        );
    }

    #[test]
    fn parses_relations_sections_and_fragments() {
        let text = format!(
            "{HEAD}[properties.next]\ntype = \"string\"\n\"x-bearout\" = {{ ref = \"example/a/b@1\" }}\n[properties.tags]\ntype = \"array\"\nitems = {{ type = \"string\" }}\n\"x-bearout\" = {{ ref = [\"example/a/b@1#tag\", \"example/a/c@1\"] }}\n[\"x-bearout\"]\nsections = [\"Context\"]\n[\"x-bearout\".fragments.tag]\ntype = \"object\"\nrequired = [\"id\"]\n"
        );
        let shape = parse(&text).unwrap();
        assert_eq!(shape.relations["next"], vec!["example/a/b@1".to_owned()]);
        assert_eq!(shape.relations["tags"].len(), 2);
        assert_eq!(shape.sections, vec!["Context".to_owned()]);
        assert!(shape.fragments.contains_key("tag"));
        assert_eq!(shape.check_fragment("tag", &json!({})).unwrap().len(), 1);
        assert!(shape.check_fragment("other", &json!({})).is_none());
    }

    #[test]
    fn rejects_invalid_vocabulary() {
        let cases = [
            (
                "[\"x-bearout\"]\nsection = [\"Context\"]\n",
                "Additional properties",
            ),
            ("[\"x-bearout\"]\nsections = \"Context\"\n", "sections"),
            ("[\"x-bearout\".fragments.Tag]\ntype = \"object\"\n", "kind"),
            (
                "[\"x-bearout\".fragments.tag]\ntype = \"object\"\n\"x-bearout\" = {}\n",
                "must not nest",
            ),
            (
                "[properties.next]\ntype = \"string\"\n\"x-bearout\" = { reference = \"example/a/b@1\" }\n",
                "ref",
            ),
            (
                "[properties.next]\ntype = \"string\"\n\"x-bearout\" = { ref = 3 }\n",
                "ref",
            ),
            (
                "[properties.next]\ntype = \"string\"\n\"x-bearout\" = { ref = \"Bad\" }\n",
                "schema",
            ),
            (
                "[properties.next]\ntype = \"integer\"\n\"x-bearout\" = { ref = \"example/a/b@1\" }\n",
                "relation needs",
            ),
            (
                "[properties.next]\ntype = \"array\"\n\"x-bearout\" = { ref = \"example/a/b@1\" }\n",
                "items",
            ),
            ("[properties.id]\ntype = \"string\"\n", "envelope key"),
        ];
        for (body, expected) in cases {
            let text = format!("{HEAD}{body}");
            let error = parse(&text).unwrap_err();
            assert!(
                error.to_lowercase().contains(&expected.to_lowercase()),
                "{body:?} -> {error}"
            );
        }
    }
}
