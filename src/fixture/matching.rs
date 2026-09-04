// SPDX-License-Identifier: Apache-2.0

//! Structured matching of expected diagnostics against actual ones.
//!
//! Expectations name fields of a diagnostic, never its rendered text: the
//! code, and optionally the severity, path, line, side, repository rule
//! identifier, and, as a deliberately brittle assertion, the exact
//! message. Matching is a multiset assignment: each expectation consumes
//! at most one diagnostic and each diagnostic satisfies at most one
//! expectation, so a repeated diagnostic cannot satisfy one expectation
//! twice and two identical expectations need two diagnostics. The
//! assignment is a maximum bipartite matching found deterministically from
//! the expectations in declaration order and the diagnostics in report
//! order, so the result never depends on how expectations overlap.

use std::fmt;

use serde::Serialize;

use crate::report::{Code, Diagnostic, Severity, Side};

/// One expected diagnostic: the fields a case asserts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Expectation {
    pub code: Code,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<Severity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Expectation {
    /// `true` when every asserted field equals the diagnostic's.
    #[must_use]
    pub fn matches(&self, diagnostic: &Diagnostic) -> bool {
        self.code == diagnostic.code
            && self
                .severity
                .is_none_or(|severity| severity == diagnostic.severity)
            && self
                .path
                .as_deref()
                .is_none_or(|path| path == diagnostic.path)
            && self.line.is_none_or(|line| Some(line) == diagnostic.line)
            && self.side.is_none_or(|side| side == diagnostic.side)
            && self
                .rule
                .as_deref()
                .is_none_or(|rule| Some(rule) == diagnostic.rule.as_deref())
            && self
                .message
                .as_deref()
                .is_none_or(|message| message == diagnostic.message)
    }
}

impl fmt::Display for Expectation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code)?;
        if let Some(severity) = self.severity {
            let text = match severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            write!(f, " severity={text}")?;
        }
        if let Some(side) = self.side {
            write!(f, " side={side}")?;
        }
        if let Some(path) = &self.path {
            write!(f, " path={path}")?;
        }
        if let Some(line) = self.line {
            write!(f, " line={line}")?;
        }
        if let Some(rule) = &self.rule {
            write!(f, " rule={rule}")?;
        }
        if let Some(message) = &self.message {
            write!(f, " message={message:?}")?;
        }
        Ok(())
    }
}

/// The outcome of matching: the positions of the expectations nothing
/// satisfied and of the diagnostics nothing expected, each in order.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Assignment {
    pub missing: Vec<usize>,
    pub unexpected: Vec<usize>,
}

/// Assign diagnostics to expectations, one each, maximizing the number of
/// satisfied expectations.
#[must_use]
pub fn assign(expectations: &[Expectation], diagnostics: &[Diagnostic]) -> Assignment {
    // Kuhn's augmenting-path matching over the small bipartite graph of
    // (expectation, diagnostic) pairs that agree. `owner[d]` is the
    // expectation currently holding diagnostic `d`.
    let mut owner: Vec<Option<usize>> = vec![None; diagnostics.len()];
    let mut matched_expectation = vec![false; expectations.len()];
    for (index, expectation) in expectations.iter().enumerate() {
        let mut visited = vec![false; diagnostics.len()];
        if augment(
            index,
            expectation,
            expectations,
            diagnostics,
            &mut owner,
            &mut visited,
        ) {
            matched_expectation[index] = true;
        }
    }
    // Re-derive which expectations hold a diagnostic: augmenting paths may
    // have reassigned earlier ones, never released them.
    let mut holds = vec![false; expectations.len()];
    for holder in owner.iter().flatten() {
        holds[*holder] = true;
    }
    Assignment {
        missing: (0..expectations.len())
            .filter(|index| !holds[*index])
            .collect(),
        unexpected: (0..diagnostics.len())
            .filter(|index| owner[*index].is_none())
            .collect(),
    }
}

fn augment(
    index: usize,
    expectation: &Expectation,
    expectations: &[Expectation],
    diagnostics: &[Diagnostic],
    owner: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for (position, diagnostic) in diagnostics.iter().enumerate() {
        if visited[position] || !expectation.matches(diagnostic) {
            continue;
        }
        visited[position] = true;
        let free = match owner[position] {
            None => true,
            Some(holder) => augment(
                holder,
                &expectations[holder],
                expectations,
                diagnostics,
                owner,
                visited,
            ),
        };
        if free {
            owner[position] = Some(index);
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect(code: Code, path: Option<&str>) -> Expectation {
        Expectation {
            code,
            severity: None,
            path: path.map(str::to_owned),
            line: None,
            side: None,
            rule: None,
            message: None,
        }
    }

    fn diagnostic(code: Code, path: &str) -> Diagnostic {
        Diagnostic::new(code, path, "m")
    }

    #[test]
    fn matching_is_a_multiset_assignment() {
        let diagnostics = [
            diagnostic(Code::PolicyError, "a.md"),
            diagnostic(Code::PolicyError, "b.md"),
        ];
        // Two equal expectations need two diagnostics; one diagnostic
        // cannot satisfy both.
        let assignment = assign(
            &[
                expect(Code::PolicyError, None),
                expect(Code::PolicyError, None),
            ],
            &diagnostics[..1],
        );
        assert_eq!(assignment.missing, [1]);
        assert!(assignment.unexpected.is_empty());
        // A general expectation declared first does not steal the only
        // diagnostic a later, more specific one can take.
        let assignment = assign(
            &[
                expect(Code::PolicyError, None),
                expect(Code::PolicyError, Some("a.md")),
            ],
            &diagnostics,
        );
        assert_eq!(assignment, Assignment::default());
        // Unexpected diagnostics keep their report positions.
        let assignment = assign(&[expect(Code::PolicyError, Some("b.md"))], &diagnostics);
        assert!(assignment.missing.is_empty());
        assert_eq!(assignment.unexpected, [0]);
        // Nothing matches nothing.
        assert_eq!(assign(&[], &[]), Assignment::default());
        assert_eq!(
            assign(&[expect(Code::Envelope, None)], &diagnostics),
            Assignment {
                missing: vec![0],
                unexpected: vec![0, 1],
            }
        );
    }

    #[test]
    fn every_asserted_field_must_agree() {
        let actual = Diagnostic::new(Code::PolicyError, "a.md", "text")
            .at_line(Some(3))
            .with_rule(Some("r".to_owned()))
            .on_side(Side::Baseline);
        let full = Expectation {
            code: Code::PolicyError,
            severity: Some(Severity::Error),
            path: Some("a.md".to_owned()),
            line: Some(3),
            side: Some(Side::Baseline),
            rule: Some("r".to_owned()),
            message: Some("text".to_owned()),
        };
        assert!(full.matches(&actual));
        assert_eq!(
            full.to_string(),
            "B015 severity=error side=baseline path=a.md line=3 rule=r message=\"text\""
        );
        let variants = [
            Expectation {
                code: Code::PolicyWarning,
                ..full.clone()
            },
            Expectation {
                severity: Some(Severity::Warning),
                ..full.clone()
            },
            Expectation {
                path: Some("b.md".to_owned()),
                ..full.clone()
            },
            Expectation {
                line: Some(4),
                ..full.clone()
            },
            Expectation {
                side: Some(Side::Candidate),
                ..full.clone()
            },
            Expectation {
                rule: Some("s".to_owned()),
                ..full.clone()
            },
            Expectation {
                message: Some("other".to_owned()),
                ..full.clone()
            },
        ];
        for variant in variants {
            assert!(!variant.matches(&actual), "{variant}");
        }
        // An unasserted field is free.
        assert!(expect(Code::PolicyError, None).matches(&actual));
        let without_line = Expectation {
            line: None,
            ..full.clone()
        };
        assert!(without_line.matches(&actual));
        assert!(!full.matches(&Diagnostic::new(Code::PolicyError, "a.md", "text")));
    }
}
