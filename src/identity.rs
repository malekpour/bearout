// SPDX-License-Identifier: Apache-2.0

//! Identifier grammars. Schema identifiers are repository-owned and never
//! centrally registered; the kernel only requires that they be well formed
//! so that relations, fragments, and reports can name them unambiguously.

/// Validate a schema identifier:
/// `<lowercase namespace segments>/<lowercase kind>@<positive major>`.
pub fn check_schema_id(text: &str) -> Result<(), String> {
    let Some((path, major)) = text.rsplit_once('@') else {
        return Err(format!("schema `{text}` must end with `@<major>`"));
    };
    if major.is_empty() || !major.bytes().all(|b| b.is_ascii_digit()) || major.starts_with('0') {
        return Err(format!(
            "schema `{text}` must have a positive major version after `@`"
        ));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2 {
        return Err(format!(
            "schema `{text}` must have at least one namespace segment before the kind"
        ));
    }
    for segment in segments {
        check_kebab(segment).map_err(|error| format!("schema `{text}`: {error}"))?;
    }
    Ok(())
}

/// Validate a resource or fragment identifier: lowercase, digits, and
/// hyphens, starting with a letter or digit.
pub fn check_id(text: &str) -> Result<(), String> {
    check_kebab(text).map_err(|error| format!("identifier `{text}`: {error}"))
}

/// Validate a fragment kind or rule name.
pub fn check_kind(text: &str) -> Result<(), String> {
    check_kebab(text).map_err(|error| format!("kind `{text}`: {error}"))
}

/// The kind of a fragment node: `<schema>#<fragment kind>`.
#[must_use]
pub fn fragment_kind(schema: &str, kind: &str) -> String {
    format!("{schema}#{kind}")
}

/// Validate a relation target kind: a schema id, `<schema>#<kind>`, or `*`.
pub fn check_target_kind(text: &str) -> Result<(), String> {
    if text == "*" {
        return Ok(());
    }
    match text.split_once('#') {
        Some((schema, kind)) => {
            check_schema_id(schema)?;
            check_kind(kind)
        }
        None => check_schema_id(text),
    }
}

fn check_kebab(segment: &str) -> Result<(), String> {
    if segment.is_empty() {
        return Err("segment must not be empty".to_owned());
    }
    if !segment
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "`{segment}` must use only lowercase letters, digits, and hyphens"
        ));
    }
    if segment.starts_with('-') || segment.ends_with('-') || segment.contains("--") {
        return Err(format!(
            "`{segment}` must not start, end, or double up on hyphens"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_ids() {
        assert!(check_schema_id("example/linked-notes/note@1").is_ok());
        assert!(check_schema_id("example/note@12").is_ok());
        assert!(check_schema_id("note@1").is_err());
        assert!(check_schema_id("Example/note@1").is_err());
        assert!(check_schema_id("example/note@0").is_err());
        assert!(check_schema_id("example/note").is_err());
        assert!(check_schema_id("example//note@1").is_err());
    }

    #[test]
    fn ids_and_kinds() {
        assert!(check_id("decision-0001-ruling-01").is_ok());
        assert!(check_id("Decision").is_err());
        assert!(check_id("a--b").is_err());
        assert!(check_id("-a").is_err());
        assert!(check_target_kind("*").is_ok());
        assert!(check_target_kind("example/x/y@1#term").is_ok());
        assert!(check_target_kind("example/x/y@1#Term").is_err());
    }
}
