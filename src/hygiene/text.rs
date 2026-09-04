// SPDX-License-Identifier: Apache-2.0

//! Native text hygiene over exact bytes: encoding, line endings, the final
//! newline, and trailing whitespace, each against the effective
//! `EditorConfig` properties. Nothing here knows a file format.
//!
//! Decisions that are Bearout's rather than `EditorConfig`'s:
//!
//! - a file is binary when the bootstrap says so, or, absent a
//!   declaration, when a NUL byte occurs in its first [`SNIFF_BYTES`]
//!   bytes; an empty file is text; binary files are never checked;
//! - a text file must be valid UTF-8 even when `charset` is unset, because
//!   bytes that do not decode cannot be checked line by line; `charset =
//!   utf-8` additionally forbids a byte-order mark and `charset =
//!   utf-8-bom` requires one, except in an empty file;
//! - `insert_final_newline = true` requires a non-empty file to end with
//!   exactly one line terminator, so trailing blank lines are violations;
//!   `false` forbids a final terminator; an empty file satisfies both and
//!   is never changed;
//! - `end_of_line` requires every terminator to be the configured one;
//! - `trim_trailing_whitespace = true` forbids spaces and tabs before a
//!   terminator or the end of the file.
//!
//! Each aspect yields at most one diagnostic per file, naming the first
//! offending line and how many more there are.

use super::editorconfig::{Charset, Effective, LineEnding};
use crate::report::{Code, Diagnostic};

/// Bytes examined for a NUL when deciding whether a file is binary.
pub const SNIFF_BYTES: usize = 8 * 1024;

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// `true` when the bytes are binary by declaration or by content.
#[must_use]
pub fn is_binary(bytes: &[u8], declared: Option<bool>) -> bool {
    match declared {
        Some(binary) => binary,
        None => bytes[..bytes.len().min(SNIFF_BYTES)].contains(&0),
    }
}

/// One line of a text file, without its terminator.
struct Line<'a> {
    content: &'a [u8],
    terminator: Option<LineEnding>,
}

/// Split on `\r\n`, `\n`, and `\r`. The last element is the text after the
/// final terminator, present only when non-empty.
fn lines(text: &[u8]) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut index = 0;
    while index < text.len() {
        let terminator = match text[index] {
            b'\r' if text.get(index + 1) == Some(&b'\n') => Some((LineEnding::CrLf, 2)),
            b'\r' => Some((LineEnding::Cr, 1)),
            b'\n' => Some((LineEnding::Lf, 1)),
            _ => None,
        };
        match terminator {
            Some((kind, width)) => {
                lines.push(Line {
                    content: &text[start..index],
                    terminator: Some(kind),
                });
                index += width;
                start = index;
            }
            None => index += 1,
        }
    }
    if start < text.len() {
        lines.push(Line {
            content: &text[start..],
            terminator: None,
        });
    }
    lines
}

fn has_trailing_whitespace(content: &[u8]) -> bool {
    matches!(content.last(), Some(b' ' | b'\t'))
}

fn more(count: usize) -> String {
    match count {
        0 => String::new(),
        1 => " (and 1 more line)".to_owned(),
        n => format!(" (and {n} more lines)"),
    }
}

/// Check `bytes` of the file at `path`. Encoding failures stop the check;
/// nothing cascades from them.
#[must_use]
pub fn check(
    path: &str,
    bytes: &[u8],
    declared_binary: Option<bool>,
    effective: Effective,
) -> Vec<Diagnostic> {
    if is_binary(bytes, declared_binary) {
        return Vec::new();
    }
    let mut diagnostics = Vec::new();
    let has_bom = bytes.starts_with(BOM);
    let text = if has_bom { &bytes[BOM.len()..] } else { bytes };
    if let Err(error) = std::str::from_utf8(text) {
        let line = text[..error.valid_up_to()]
            .split(|byte| *byte == b'\n')
            .count();
        diagnostics.push(
            Diagnostic::new(
                Code::Encoding,
                path,
                format!("file is not valid UTF-8: {error}"),
            )
            .at_line(u32::try_from(line).ok()),
        );
        return diagnostics;
    }
    match effective.charset {
        Some(Charset::Utf8) if has_bom => diagnostics.push(Diagnostic::new(
            Code::Encoding,
            path,
            "file begins with a byte-order mark; `charset = utf-8` forbids one",
        )),
        Some(Charset::Utf8Bom) if !has_bom && !bytes.is_empty() => {
            diagnostics.push(Diagnostic::new(
                Code::Encoding,
                path,
                "file has no byte-order mark; `charset = utf-8-bom` requires one",
            ));
        }
        _ => {}
    }

    let lines = lines(text);
    if let Some(expected) = effective.end_of_line {
        let offending: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.terminator.is_some_and(|found| found != expected))
            .map(|(index, _)| index + 1)
            .collect();
        if let Some(first) = offending.first() {
            let found = lines[first - 1]
                .terminator
                .expect("offending lines have terminators");
            diagnostics.push(
                Diagnostic::new(
                    Code::LineEnding,
                    path,
                    format!(
                        "line ends with {}; `end_of_line = {}` requires {}{}",
                        found.name(),
                        expected.name(),
                        expected.name(),
                        more(offending.len() - 1)
                    ),
                )
                .at_line(u32::try_from(*first).ok()),
            );
        }
    }
    if let Some(required) = effective.insert_final_newline
        && !text.is_empty()
    {
        let last = lines.last().expect("non-empty text has a line");
        let line_count = lines.len();
        if required {
            if last.terminator.is_none() {
                diagnostics.push(
                    Diagnostic::new(
                        Code::FinalNewline,
                        path,
                        "file does not end with a newline; `insert_final_newline = true` requires exactly one",
                    )
                    .at_line(u32::try_from(line_count).ok()),
                );
            } else {
                let blank = lines
                    .iter()
                    .rev()
                    .take_while(|line| line.content.is_empty() && line.terminator.is_some())
                    .count();
                if blank > 0 {
                    diagnostics.push(
                        Diagnostic::new(
                            Code::FinalNewline,
                            path,
                            format!(
                                "file ends with {blank} blank line(s); `insert_final_newline = true` requires exactly one final newline"
                            ),
                        )
                        .at_line(u32::try_from(line_count - blank).ok()),
                    );
                }
            }
        } else if last.terminator.is_some() {
            diagnostics.push(
                Diagnostic::new(
                    Code::FinalNewline,
                    path,
                    "file ends with a newline; `insert_final_newline = false` forbids one",
                )
                .at_line(u32::try_from(line_count).ok()),
            );
        }
    }
    if effective.trim_trailing_whitespace == Some(true) {
        let offending: Vec<usize> = lines
            .iter()
            .enumerate()
            .filter(|(_, line)| has_trailing_whitespace(line.content))
            .map(|(index, _)| index + 1)
            .collect();
        if let Some(first) = offending.first() {
            diagnostics.push(
                Diagnostic::new(
                    Code::TrailingWhitespace,
                    path,
                    format!(
                        "line ends with whitespace; `trim_trailing_whitespace = true` forbids it{}",
                        more(offending.len() - 1)
                    ),
                )
                .at_line(u32::try_from(*first).ok()),
            );
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> Effective {
        Effective {
            charset: Some(Charset::Utf8),
            end_of_line: Some(LineEnding::Lf),
            insert_final_newline: Some(true),
            trim_trailing_whitespace: Some(true),
        }
    }

    fn codes(bytes: &[u8], effective: Effective) -> Vec<Code> {
        check("f", bytes, None, effective)
            .iter()
            .map(|d| d.code)
            .collect()
    }

    #[test]
    fn binary_sniffing_is_deterministic_and_overridable() {
        assert!(!is_binary(b"", None));
        assert!(!is_binary(b"a", None));
        assert!(is_binary(b"\0", None));
        assert!(is_binary(b"text\0more", None));
        assert!(!is_binary(&[b'x'; SNIFF_BYTES + 1], None));
        let mut late = vec![b'x'; SNIFF_BYTES];
        late.push(0);
        assert!(
            !is_binary(&late, None),
            "a NUL after the sniff window does not count"
        );
        assert!(is_binary(b"text", Some(true)));
        assert!(!is_binary(b"\0", Some(false)));
        assert!(check("f", b"\0\0", None, all()).is_empty());
    }

    #[test]
    fn empty_and_clean_files_pass() {
        assert!(codes(b"", all()).is_empty());
        assert!(codes(b"a\nb\n", all()).is_empty());
        assert!(
            codes(
                b"a\r\nb\r\n",
                Effective {
                    end_of_line: Some(LineEnding::CrLf),
                    ..all()
                }
            )
            .is_empty()
        );
        assert!(
            codes(
                b"a\rb\r",
                Effective {
                    end_of_line: Some(LineEnding::Cr),
                    ..all()
                }
            )
            .is_empty()
        );
        assert!(
            codes(b"no newline", Effective::default()).is_empty(),
            "nothing configured, nothing checked"
        );
    }

    #[test]
    fn each_aspect_reports_once_with_the_first_line() {
        let mixed = check("f", b"a\r\nb  \nc\t\nd\n\n\n", None, all());
        let summary: Vec<(Code, Option<u32>)> = mixed.iter().map(|d| (d.code, d.line)).collect();
        assert_eq!(
            summary,
            [
                (Code::LineEnding, Some(1)),
                (Code::FinalNewline, Some(4)),
                (Code::TrailingWhitespace, Some(2)),
            ]
        );
        assert!(mixed[2].message.contains("(and 1 more line)"));
        assert!(mixed[1].message.contains("2 blank line(s)"));
        assert_eq!(codes(b"a", all()), [Code::FinalNewline]);
        assert_eq!(
            codes(
                b"a\n",
                Effective {
                    insert_final_newline: Some(false),
                    ..Effective::default()
                }
            ),
            [Code::FinalNewline]
        );
        assert!(
            codes(
                b"a",
                Effective {
                    insert_final_newline: Some(false),
                    ..Effective::default()
                }
            )
            .is_empty()
        );
        assert!(
            codes(
                b"a  \n",
                Effective {
                    trim_trailing_whitespace: Some(false),
                    ..all()
                }
            )
            .is_empty(),
            "hard breaks stay when trimming is off"
        );
    }

    #[test]
    fn encoding_problems_stop_the_check() {
        let invalid = check("f", b"ok\n\xff  \n", None, all());
        assert_eq!(invalid.len(), 1);
        assert_eq!(invalid[0].code, Code::Encoding);
        assert_eq!(invalid[0].line, Some(2));
        assert_eq!(codes(b"\xEF\xBB\xBFa\n", all()), [Code::Encoding]);
        let bom = Effective {
            charset: Some(Charset::Utf8Bom),
            ..all()
        };
        assert!(codes(b"\xEF\xBB\xBFa\n", bom).is_empty());
        assert_eq!(codes(b"a\n", bom), [Code::Encoding]);
        assert!(codes(b"", bom).is_empty(), "an empty file needs no mark");
        assert!(
            codes(b"\xEF\xBB\xBFa\n", Effective::default()).is_empty(),
            "unset charset accepts a mark"
        );
    }
}
