// SPDX-License-Identifier: Apache-2.0

//! Structure extracted from a Markdown body with Comrak: headings with
//! GFM-style anchors, explicit HTML anchors, fenced code blocks with their
//! info-string attributes, links, and images, each with source lines. The
//! same parser serves resource bodies and schema-less documents.
//!
//! Heading anchors follow Comrak's GitHub-flavoured algorithm, including the
//! `-1`, `-2` suffixes for duplicate headings. Bearout defines no anchor
//! dialect of its own. Explicit anchors are the `id` and `name` attributes
//! of `<a>` elements in raw HTML; nothing else in HTML is interpreted, and
//! HTML links and images are not collected.
//!
//! Links and images come from Comrak's inline and reference-style syntax
//! (and autolinks), never from fenced or indented code. Their visible text
//! and alt text are flattened to plain text.

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
    /// Explicit HTML anchors in document order.
    pub anchors: Vec<Anchor>,
    /// Fenced code blocks in document order.
    pub blocks: Vec<Block>,
    /// Links in document order.
    pub links: Vec<Link>,
    /// Images in document order.
    pub images: Vec<Image>,
}

impl Document {
    /// `true` when a `#fragment` names a heading anchor or an explicit
    /// anchor of this document.
    #[must_use]
    pub fn has_anchor(&self, fragment: &str) -> bool {
        self.sections
            .iter()
            .any(|section| section.anchor == fragment)
            || self.anchors.iter().any(|anchor| anchor.id == fragment)
    }
}

/// One explicit anchor: `<a id="...">` or `<a name="...">` in raw HTML.
#[derive(Debug, Clone, Serialize)]
pub struct Anchor {
    /// The attribute value, exactly as written.
    pub id: String,
    /// One-based line of the element in the resource file.
    pub line: u32,
}

/// One image.
#[derive(Debug, Clone, Serialize)]
pub struct Image {
    /// Destination as written, after Markdown unescaping.
    pub target: String,
    /// Alt text with inline markup flattened.
    pub alt: String,
    /// One-based line in the resource file.
    pub line: u32,
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
    /// Visible link text with inline markup flattened.
    pub text: String,
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
    let mut anchors = Vec::new();
    let mut blocks = Vec::new();
    let mut links = Vec::new();
    let mut images = Vec::new();

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
                text: inline_text(node),
                line: line_of(position.start.line),
            }),
            NodeValue::Image(image) => images.push(Image {
                target: image.url.clone(),
                alt: inline_text(node),
                line: line_of(position.start.line),
            }),
            NodeValue::HtmlInline(html) => {
                for (offset, id) in explicit_anchors(html) {
                    anchors.push(Anchor {
                        id,
                        line: line_of(position.start.line + offset),
                    });
                }
            }
            NodeValue::HtmlBlock(html) => {
                for (offset, id) in explicit_anchors(&html.literal) {
                    anchors.push(Anchor {
                        id,
                        line: line_of(position.start.line + offset),
                    });
                }
            }
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
        anchors,
        blocks,
        links,
        images,
    }
}

/// The `id` and `name` attribute values of every `<a>` start tag in a raw
/// HTML literal, each with the number of line breaks before it. Comments,
/// CDATA sections, processing instructions, declarations, and the raw text
/// of `<script>` and `<style>` elements are skipped, so nothing inside them
/// can define an anchor; a `>` inside a quoted attribute value does not end
/// the tag. Attribute names are matched case-insensitively in any order;
/// values may be double-quoted, single-quoted, or unquoted. This is a
/// scanner for one element, not an HTML parser.
fn explicit_anchors(html: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let bytes = html.as_bytes();
    let mut index = 0;
    while let Some(at) = bytes[index..].iter().position(|b| *b == b'<') {
        let start = index + at;
        // `<` is ASCII, so every position found is a character boundary.
        let rest = &html[start..];
        let skip_to = |marker: &str, width: usize| {
            rest.find(marker)
                .map_or(html.len(), |found| start + found + width)
        };
        let lowered = rest
            .get(..rest.len().min(9))
            .unwrap_or("")
            .to_ascii_lowercase();
        if rest.starts_with("<!--") {
            index = skip_to("-->", 3).max(start + 4);
        } else if rest.starts_with("<![CDATA[") {
            index = skip_to("]]>", 3);
        } else if rest.starts_with("<?") {
            index = skip_to("?>", 2);
        } else if rest.starts_with("<!") {
            index = skip_to(">", 1);
        } else if lowered.starts_with("<script") || lowered.starts_with("<style") {
            let closing = if lowered.starts_with("<script") {
                "</script"
            } else {
                "</style"
            };
            index = rest
                .to_ascii_lowercase()
                .find(closing)
                .map_or(html.len(), |found| start + found + closing.len());
        } else if lowered.starts_with("<a")
            && bytes
                .get(start + 2)
                .is_some_and(|b| b.is_ascii_whitespace() || *b == b'>' || *b == b'/')
        {
            let Some(end) = tag_end(&rest[2..]) else {
                break;
            };
            let offset = html[..start].matches('\n').count();
            for (name, value) in attributes(&rest[2..2 + end]) {
                if (name.eq_ignore_ascii_case("id") || name.eq_ignore_ascii_case("name"))
                    && !value.is_empty()
                {
                    found.push((offset, value));
                }
            }
            index = start + 2 + end + 1;
        } else {
            index = start + 1;
        }
    }
    found
}

/// The byte offset of the `>` that ends a start tag's attribute text,
/// ignoring any `>` inside a quoted value.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), c) if c == open => quote = None,
            (None, '>') => return Some(offset),
            _ => {}
        }
    }
    None
}

/// `name=value` pairs of one start tag's attribute text.
fn attributes(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/' || c == '>')
            .unwrap_or(rest.len());
        if name_end == 0 {
            rest = &rest[1..];
            continue;
        }
        let name = &rest[..name_end];
        rest = rest[name_end..].trim_start();
        let value = if let Some(after_equals) = rest.strip_prefix('=') {
            let after_equals = after_equals.trim_start();
            if let Some(quoted) = after_equals.strip_prefix('"') {
                let end = quoted.find('"').unwrap_or(quoted.len());
                rest = quoted.get(end + 1..).unwrap_or("");
                quoted[..end].to_owned()
            } else if let Some(quoted) = after_equals.strip_prefix('\'') {
                let end = quoted.find('\'').unwrap_or(quoted.len());
                rest = quoted.get(end + 1..).unwrap_or("");
                quoted[..end].to_owned()
            } else {
                let end = after_equals
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .unwrap_or(after_equals.len());
                rest = &after_equals[end..];
                after_equals[..end].to_owned()
            }
        } else {
            String::new()
        };
        pairs.push((name.to_owned(), value));
        rest = rest.trim_start();
    }
    pairs
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
        assert_eq!(doc.links[0].text, "x");
        assert_eq!(doc.links[0].line, 7);
    }

    #[test]
    fn collects_link_text_images_and_explicit_anchors() {
        let body = "<a id=\"top\"></a>\n\n# T\n\nSee [the *guide* `now`][g] and ![Flow chart](figures/flow.svg) and\n![](empty.png).\n\n<div>\n<a name='old-name' href=\"#top\">x</a> <A ID=unquoted>y</A>\n</div>\n\n```md\n[not a link](nope.md) ![no](nope.png) <a id=\"code\"></a>\n```\n\n<a href=\"#top\">no anchor here</a> <abbr id=\"x\">not an anchor</abbr>\n\n[g]: guide.md#part\n";
        let doc = parse(body, 1);
        let links: Vec<(&str, &str, u32)> = doc
            .links
            .iter()
            .map(|link| (link.target.as_str(), link.text.as_str(), link.line))
            .collect();
        assert_eq!(links, [("guide.md#part", "the guide now", 5)]);
        let images: Vec<(&str, &str, u32)> = doc
            .images
            .iter()
            .map(|image| (image.target.as_str(), image.alt.as_str(), image.line))
            .collect();
        assert_eq!(
            images,
            [("figures/flow.svg", "Flow chart", 5), ("empty.png", "", 6)]
        );
        let anchors: Vec<(&str, u32)> = doc
            .anchors
            .iter()
            .map(|anchor| (anchor.id.as_str(), anchor.line))
            .collect();
        assert_eq!(anchors, [("top", 1), ("old-name", 9), ("unquoted", 9)]);
        assert!(doc.has_anchor("t"));
        assert!(doc.has_anchor("old-name"));
        assert!(!doc.has_anchor("code"));
        assert!(!doc.has_anchor("x"));
    }

    #[test]
    fn explicit_anchor_scanner_is_tolerant() {
        assert_eq!(
            explicit_anchors("<a\n  name=\"two\"\n  id='three'>"),
            [(0, "two".to_owned()), (0, "three".to_owned())]
        );
        assert_eq!(
            explicit_anchors("<a id=\"\"></a><a></a><abbr id=\"n\">"),
            []
        );
        assert_eq!(
            explicit_anchors("x\n<a href=x id=y>"),
            [(1, "y".to_owned())]
        );
        assert_eq!(explicit_anchors("<a id=\"unterminated"), []);
        assert_eq!(explicit_anchors("<A ID=UP>"), [(0, "UP".to_owned())]);
    }

    #[test]
    fn non_element_contexts_never_define_anchors() {
        assert_eq!(explicit_anchors("<!-- <a id=\"ghost\"></a> -->"), []);
        assert_eq!(
            explicit_anchors("<!-- x -->\n<a id=\"real\"></a>"),
            [(1, "real".to_owned())]
        );
        assert_eq!(explicit_anchors("<!-- unterminated <a id=\"ghost\">"), []);
        assert_eq!(explicit_anchors("<![CDATA[ <a id=\"ghost\"> ]]>"), []);
        assert_eq!(explicit_anchors("<?php echo '<a id=\"ghost\">'; ?>"), []);
        assert_eq!(
            explicit_anchors("<!DOCTYPE html><a id=\"real\">"),
            [(0, "real".to_owned())]
        );
        assert_eq!(
            explicit_anchors("<script>var s = '<a id=\"ghost\">';</script><a id=\"real\">"),
            [(0, "real".to_owned())]
        );
        assert_eq!(
            explicit_anchors("<STYLE>/* <a id=\"ghost\"> */</STYLE>"),
            []
        );
        // A quoted `>` does not end the tag, and the scan resumes after it.
        assert_eq!(
            explicit_anchors("<a id=\"x>y\" title='>'></a> <a id=\"z\">"),
            [(0, "x>y".to_owned()), (0, "z".to_owned())]
        );
        assert_eq!(
            explicit_anchors("<a title=\"a>b\" id=c>"),
            [(0, "c".to_owned())]
        );
    }

    #[test]
    fn commented_anchors_do_not_satisfy_fragments() {
        let doc = parse(
            "<!-- <a id=\"ghost\"></a> -->\n\n<a id=\"real\"></a>\n\n[g](#ghost) [r](#real)\n",
            1,
        );
        assert!(!doc.has_anchor("ghost"));
        assert!(doc.has_anchor("real"));
        assert_eq!(doc.anchors.len(), 1);
        assert_eq!(doc.anchors[0].line, 3);
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
