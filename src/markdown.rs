// SPDX-License-Identifier: Apache-2.0

//! Structure extracted from a Markdown body with Comrak: headings with
//! GFM-style anchors, fenced code blocks with their info-string attributes,
//! and links, each with source lines.
//!
//! Anchors follow Comrak's GitHub-flavoured algorithm, including the `-1`,
//! `-2` suffixes for duplicate headings. Bearout defines no anchor dialect
//! of its own.

use std::collections::BTreeMap;

use comrak::html::Anchorizer;
use comrak::nodes::{AstNode, NodeValue};
use comrak::{Arena, Options, parse_document};
use serde::Serialize;

/// The structural view of one Markdown body.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Document {
    /// Headings in document order, each with the text beneath it.
    pub sections: Vec<Section>,
    /// Fenced code blocks in document order.
    pub blocks: Vec<Block>,
    /// Links in document order.
    pub links: Vec<Link>,
}

/// One heading and the text it governs.
#[derive(Debug, Clone, Serialize)]
pub struct Section {
    /// Heading level from 1 to 6.
    pub level: u8,
    /// Heading text with inline markup flattened.
    pub title: String,
    /// GFM anchor for the heading.
    pub anchor: String,
    /// One-based line of the heading in the resource file.
    pub line: u32,
    /// Body lines between this heading and the next of equal or higher rank.
    pub text: String,
}

/// One fenced code block.
#[derive(Debug, Clone, Serialize)]
pub struct Block {
    /// First word of the info string.
    pub lang: String,
    /// Remaining `key=value` words of the info string.
    pub attrs: BTreeMap<String, String>,
    /// Block content without the fences.
    pub content: String,
    /// One-based line of the opening fence in the resource file.
    pub line: u32,
    /// Index into `sections` of the enclosing section, if any.
    pub section: Option<usize>,
}

/// One link destination.
#[derive(Debug, Clone, Serialize)]
pub struct Link {
    /// Destination as written, after Markdown unescaping.
    pub target: String,
    /// One-based line in the resource file.
    pub line: u32,
}

struct HeadingDraft {
    level: u8,
    title: String,
    start_line: usize,
    end_line: usize,
}

/// Parse `body`, whose first line is line `first_line` of the resource file.
#[must_use]
pub fn parse(body: &str, first_line: u32) -> Document {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.tasklist = true;
    options.extension.autolink = true;
    let root = parse_document(&arena, body, &options);

    let line_of = |line: usize| {
        first_line
            .saturating_add(u32::try_from(line).unwrap_or(u32::MAX))
            .saturating_sub(1)
    };
    let mut headings: Vec<HeadingDraft> = Vec::new();
    let mut blocks = Vec::new();
    let mut links = Vec::new();

    for node in root.descendants() {
        let data = node.data.borrow();
        let position = data.sourcepos;
        match &data.value {
            NodeValue::Heading(heading) => headings.push(HeadingDraft {
                level: heading.level,
                title: inline_text(node),
                start_line: position.start.line,
                end_line: position.end.line,
            }),
            NodeValue::CodeBlock(code) if code.fenced => {
                let (lang, attrs) = parse_info(&code.info);
                blocks.push(Block {
                    lang,
                    attrs,
                    content: code.literal.clone(),
                    line: line_of(position.start.line),
                    section: headings.len().checked_sub(1),
                });
            }
            NodeValue::Link(link) => links.push(Link {
                target: link.url.clone(),
                line: line_of(position.start.line),
            }),
            _ => {}
        }
    }

    let body_lines: Vec<&str> = body
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect();
    let mut anchorizer = Anchorizer::new();
    let sections = headings
        .iter()
        .enumerate()
        .map(|(index, draft)| {
            let end = headings[index + 1..]
                .iter()
                .find(|next| next.level <= draft.level)
                .map_or(body_lines.len(), |next| next.start_line - 1);
            let last = body_lines.len();
            let text =
                body_lines[draft.end_line.min(last)..end.max(draft.end_line).min(last)].join("\n");
            Section {
                level: draft.level,
                title: draft.title.clone(),
                anchor: anchorizer.anchorize(&draft.title),
                line: line_of(draft.start_line),
                text: text.trim().to_owned(),
            }
        })
        .collect();

    Document {
        sections,
        blocks,
        links,
    }
}

fn inline_text<'a>(node: &'a AstNode<'a>) -> String {
    let mut text = String::new();
    for child in node.descendants().skip(1) {
        match &child.data.borrow().value {
            NodeValue::Text(literal) => text.push_str(literal),
            NodeValue::Code(code) => text.push_str(&code.literal),
            NodeValue::SoftBreak | NodeValue::LineBreak => text.push(' '),
            _ => {}
        }
    }
    text.trim().to_owned()
}

/// Split an info string into its language and `key=value` attributes. A
/// bare word after the language becomes a `true` flag.
fn parse_info(info: &str) -> (String, BTreeMap<String, String>) {
    let mut words = info.split_whitespace();
    let lang = words.next().unwrap_or_default().to_owned();
    let attrs = words
        .map(|word| match word.split_once('=') {
            Some((key, value)) => (key.to_owned(), value.trim_matches('"').to_owned()),
            None => (word.to_owned(), "true".to_owned()),
        })
        .collect();
    (lang, attrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_sections_blocks_and_links() {
        let body = "# Title\n\nIntro [x](a.md#b).\n\n## Part two\n\n```toml bearout=item k=\"v\"\nid = \"i\"\n```\n\n### Sub\n\ntext\n\n## Other\n";
        let doc = parse(body, 5);
        assert_eq!(doc.sections.len(), 4);
        assert_eq!(doc.sections[0].anchor, "title");
        assert_eq!(doc.sections[0].line, 5);
        assert!(doc.sections[0].text.contains("Intro"));
        assert!(doc.sections[0].text.contains("Sub"));
        assert_eq!(doc.sections[1].anchor, "part-two");
        assert!(doc.sections[1].text.contains("text"));
        assert!(!doc.sections[1].text.contains("Other"));
        assert_eq!(doc.blocks.len(), 1);
        assert_eq!(doc.blocks[0].lang, "toml");
        assert_eq!(doc.blocks[0].attrs["bearout"], "item");
        assert_eq!(doc.blocks[0].attrs["k"], "v");
        assert_eq!(doc.blocks[0].content, "id = \"i\"\n");
        assert_eq!(doc.blocks[0].line, 11);
        assert_eq!(doc.blocks[0].section, Some(1));
        assert_eq!(doc.links[0].target, "a.md#b");
        assert_eq!(doc.links[0].line, 7);
    }

    #[test]
    fn anchors_follow_gfm_rules() {
        let doc = parse(
            "# Why it is open?\n\n# Why it is open?\n\n# Ĉu vi parolas Esperanton?\n\n# `code` & stuff\n",
            1,
        );
        let anchors: Vec<&str> = doc
            .sections
            .iter()
            .map(|section| section.anchor.as_str())
            .collect();
        assert_eq!(
            anchors,
            [
                "why-it-is-open",
                "why-it-is-open-1",
                "ĉu-vi-parolas-esperanton",
                "code--stuff"
            ]
        );
    }

    #[test]
    fn crlf_bodies_keep_line_numbers() {
        let doc = parse("# A\r\n\r\ntext\r\n\r\n## B\r\n", 3);
        assert_eq!(doc.sections[1].line, 7);
        assert_eq!(doc.sections[0].text, "text\n\n## B");
        assert_eq!(doc.sections[1].text, "");
    }
}
