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

impl LineEnding {
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
            Self::Cr => b"\r",
        }
    }
}

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
        // The line of the first invalid byte, counting CRLF, LF, and CR
        // terminators alike.
        let line = lines(&text[..error.valid_up_to()])
            .iter()
            .filter(|line| line.terminator.is_some())
            .count()
            + 1;
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

/// The bytes `bytes` should have under `effective`: `Ok(None)` for a binary
/// file, `Err` when the file cannot be decoded (B025), otherwise the
/// normalized bytes, equal to the input when nothing needs to change. An
/// empty file is never changed. Aspects are applied in a fixed order: the
/// byte-order mark, then every line's terminator, trailing whitespace, and
/// finally the end of the file.
pub fn normalize(
    path: &str,
    bytes: &[u8],
    declared_binary: Option<bool>,
    effective: Effective,
) -> Result<Option<Vec<u8>>, Diagnostic> {
    if is_binary(bytes, declared_binary) {
        return Ok(None);
    }
    let has_bom = bytes.starts_with(BOM);
    let text = if has_bom { &bytes[BOM.len()..] } else { bytes };
    if let Err(error) = std::str::from_utf8(text) {
        return Err(Diagnostic::new(
            Code::Encoding,
            path,
            format!("file is not valid UTF-8 and cannot be formatted: {error}"),
        ));
    }
    if bytes.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut out = Vec::with_capacity(bytes.len() + 4);
    let keep_bom = match effective.charset {
        Some(Charset::Utf8) => false,
        Some(Charset::Utf8Bom) => true,
        None => has_bom,
    };
    if keep_bom {
        out.extend_from_slice(BOM);
    }
    let lines = lines(text);
    let default_terminator = effective
        .end_of_line
        .or_else(|| lines.iter().find_map(|line| line.terminator));
    let final_required = effective.insert_final_newline;
    let trimmed: Vec<(&[u8], Option<LineEnding>)> = lines
        .iter()
        .map(|line| {
            let content = if effective.trim_trailing_whitespace == Some(true) {
                let end = line
                    .content
                    .iter()
                    .rposition(|byte| !matches!(byte, b' ' | b'\t'))
                    .map_or(0, |at| at + 1);
                &line.content[..end]
            } else {
                line.content
            };
            (content, line.terminator)
        })
        .collect();
    // Drop trailing blank lines when the end of the file is governed, so
    // that a file of nothing but blank lines becomes empty.
    let mut last = trimmed.len();
    if final_required.is_some() {
        while last > 0 && trimmed[last - 1].0.is_empty() && trimmed[last - 1].1.is_some() {
            last -= 1;
        }
    }
    for (index, (content, found)) in trimmed[..last].iter().enumerate() {
        out.extend_from_slice(content);
        let line = Line {
            content,
            terminator: *found,
        };
        let is_last = index + 1 == last;
        let terminator = match (line.terminator, effective.end_of_line) {
            (Some(_), Some(configured)) => Some(configured),
            (found, None) => found,
            (None, Some(_)) => None,
        };
        let terminator = match (is_last, final_required) {
            (true, Some(true)) => terminator.or(default_terminator).or(Some(LineEnding::Lf)),
            (true, Some(false)) => None,
            _ => terminator,
        };
        if let Some(terminator) = terminator {
            out.extend_from_slice(terminator.bytes());
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(bytes: &[u8], effective: Effective) -> Vec<u8> {
        normalize("f", bytes, None, effective)
            .unwrap()
            .expect("text")
    }

    #[test]
    fn normalization_fixes_what_the_checks_report_and_is_idempotent() {
        let cases: [(&[u8], &[u8]); 8] = [
            (b"a\r\nb  \nc\t\nd\n\n\n", b"a\nb\nc\nd\n"),
            (b"no newline", b"no newline\n"),
            (b"", b""),
            (b"\n\n\n", b""),
            (b"a\rb", b"a\nb\n"),
            (b"  \n", b""),
            (b"\xEF\xBB\xBFa\n", b"a\n"),
            (b"a\nb\n", b"a\nb\n"),
        ];
        for (input, expected) in cases {
            let once = fix(input, all());
            assert_eq!(once, expected, "{input:?}");
            assert_eq!(fix(&once, all()), once, "idempotent for {input:?}");
            assert!(
                check("f", &once, None, all()).is_empty(),
                "clean after {input:?}"
            );
        }
        let bom = Effective {
            charset: Some(Charset::Utf8Bom),
            ..all()
        };
        assert_eq!(fix(b"a\n", bom), b"\xEF\xBB\xBFa\n");
        assert_eq!(fix(b"", bom), b"", "an empty file stays empty");
        let no_final = Effective {
            insert_final_newline: Some(false),
            ..all()
        };
        assert_eq!(fix(b"a\n\n", no_final), b"a");
        let crlf = Effective {
            end_of_line: Some(LineEnding::CrLf),
            ..all()
        };
        assert_eq!(fix(b"a\nb", crlf), b"a\r\nb\r\n");
        let keep = Effective::default();
        assert_eq!(
            fix(b"a \r\nb", keep),
            b"a \r\nb",
            "nothing configured, nothing changed"
        );
        assert_eq!(
            fix(
                b"a\r\nb",
                Effective {
                    insert_final_newline: Some(true),
                    ..keep
                }
            ),
            b"a\r\nb\r\n",
            "the file's own terminator is reused"
        );
        assert!(normalize("f", b"\xff", None, all()).is_err());
        assert!(normalize("f", b"\0 ", None, all()).unwrap().is_none());
    }

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
        let after_cr = check("f", b"one\rtwo\r\nthree\xff", None, all());
        assert_eq!(after_cr[0].line, Some(3), "CR and CRLF count as lines");
        assert_eq!(check("f", b"\xff", None, all())[0].line, Some(1));
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
