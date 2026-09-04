// SPDX-License-Identifier: Apache-2.0

//! The effective text properties of a selected file, from the
//! `.editorconfig` files of the selected tree and nothing else. Every
//! `.editorconfig` between the project root and the file's directory is
//! parsed with `ec4rs` from the bytes the tree holds, the innermost
//! `root = true` ends the search, and closer files take precedence. The
//! project root is the outer boundary: configuration above the project,
//! or in the live checkout during an index or revision check, never
//! applies. Only the properties Bearout enforces are read; every other
//! property is ignored, and a supported property with a value Bearout
//! cannot enforce is a configuration diagnostic for the file it governs.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::Path;

use ec4rs::property::{Charset as EcCharset, EndOfLine, FinalNewline, TrimTrailingWs};
use ec4rs::rawvalue::RawValue;
use ec4rs::{ConfigParser, ParseError, Properties, PropertyKey, PropertyValue, Section};

use super::Budget;
use crate::paths::ProjectPath;
use crate::report::{Code, Diagnostic};
use crate::tree::ReadTree;

/// The file name `EditorConfig` uses.
pub const FILE_NAME: &str = ".editorconfig";

/// The encoding a file must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charset {
    /// Valid UTF-8 without a byte-order mark.
    Utf8,
    /// Valid UTF-8 beginning with a byte-order mark.
    Utf8Bom,
}

/// The line terminator a file must use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::CrLf => "crlf",
            Self::Cr => "cr",
        }
    }
}

/// The properties Bearout enforces, each `None` when not configured.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Effective {
    pub charset: Option<Charset>,
    pub end_of_line: Option<LineEnding>,
    pub insert_final_newline: Option<bool>,
    pub trim_trailing_whitespace: Option<bool>,
}

/// One parsed `.editorconfig`.
struct Parsed {
    is_root: bool,
    sections: Vec<Section>,
}

/// A directory's configuration: `None` when it has no `.editorconfig`,
/// `Some(Err)` when it has an unusable one.
type Cached = Option<Result<std::rc::Rc<Parsed>, ()>>;

/// Resolves effective properties, parsing each `.editorconfig` once. A
/// configuration file is read like any other hygiene input: never through
/// a symbolic link, within `limits.file_bytes`, and charged to the run's
/// budget.
pub struct Resolver<'a> {
    tree: &'a dyn ReadTree,
    budget: &'a Budget,
    cache: RefCell<BTreeMap<ProjectPath, Cached>>,
    /// Configuration files already reported as unusable.
    reported: RefCell<Vec<Diagnostic>>,
    /// An exhausted budget, which ends the run.
    fatal: RefCell<Option<String>>,
}

impl<'a> Resolver<'a> {
    #[must_use]
    pub fn new(tree: &'a dyn ReadTree, budget: &'a Budget) -> Self {
        Self {
            tree,
            budget,
            cache: RefCell::new(BTreeMap::new()),
            reported: RefCell::new(Vec::new()),
            fatal: RefCell::new(None),
        }
    }

    /// The diagnostics for unusable configuration files, each once.
    pub fn take_diagnostics(&self) -> Vec<Diagnostic> {
        std::mem::take(&mut *self.reported.borrow_mut())
    }

    /// `Err` when reading configuration exhausted the budget.
    pub fn fatal(&self) -> Result<(), String> {
        match self.fatal.borrow().as_ref() {
            Some(message) => Err(message.clone()),
            None => Ok(()),
        }
    }

    /// The effective properties for `path`, or `Err` with the diagnostics
    /// that make them unknowable: an unusable configuration file (reported
    /// once, on that file) or an unsupported value for this file.
    pub fn properties(&self, path: &ProjectPath) -> Result<Effective, Vec<Diagnostic>> {
        let mut directories = vec![ProjectPath::root()];
        directories.extend(path.parent().ancestors());
        let mut applicable: Vec<(ProjectPath, std::rc::Rc<Parsed>)> = Vec::new();
        for directory in directories {
            match self.parsed(&directory) {
                None => {}
                Some(Ok(parsed)) => {
                    if parsed.is_root {
                        applicable.clear();
                    }
                    applicable.push((directory, parsed));
                }
                Some(Err(())) => return Err(Vec::new()),
            }
        }
        let mut properties = Properties::new();
        for (directory, parsed) in &applicable {
            let relative = path
                .strip_prefix(directory)
                .expect("the directory is an ancestor");
            let relative = Path::new(relative.as_str());
            for section in &parsed.sections {
                if section.applies_to(relative) {
                    for (key, value) in section.props() {
                        properties.insert_raw_for_key(key, value.clone());
                    }
                }
            }
        }
        let mut problems = Vec::new();
        let mut unsupported = |key: &str, raw: &RawValue| {
            problems.push(Diagnostic::new(
                Code::HygieneConfig,
                path.as_str(),
                format!(
                    "`{key} = {}` is not a value Bearout can enforce; remove the property, set it to `unset`, or exclude the file from the selection",
                    raw.into_str()
                ),
            ));
        };
        let charset = match typed::<EcCharset>(&properties) {
            Ok(None) => None,
            Ok(Some(EcCharset::Utf8)) => Some(Charset::Utf8),
            Ok(Some(EcCharset::Utf8Bom)) => Some(Charset::Utf8Bom),
            Ok(Some(_)) | Err(()) => {
                unsupported(EcCharset::key(), properties.get_raw::<EcCharset>());
                None
            }
        };
        let end_of_line = match typed::<EndOfLine>(&properties) {
            Ok(None) => None,
            Ok(Some(EndOfLine::Lf)) => Some(LineEnding::Lf),
            Ok(Some(EndOfLine::CrLf)) => Some(LineEnding::CrLf),
            Ok(Some(EndOfLine::Cr)) => Some(LineEnding::Cr),
            Err(()) => {
                unsupported(EndOfLine::key(), properties.get_raw::<EndOfLine>());
                None
            }
        };
        let insert_final_newline = match typed::<FinalNewline>(&properties) {
            Ok(None) => None,
            Ok(Some(FinalNewline::Value(value))) => Some(value),
            Err(()) => {
                unsupported(FinalNewline::key(), properties.get_raw::<FinalNewline>());
                None
            }
        };
        let trim_trailing_whitespace = match typed::<TrimTrailingWs>(&properties) {
            Ok(None) => None,
            Ok(Some(TrimTrailingWs::Value(value))) => Some(value),
            Err(()) => {
                unsupported(
                    TrimTrailingWs::key(),
                    properties.get_raw::<TrimTrailingWs>(),
                );
                None
            }
        };
        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(Effective {
            charset,
            end_of_line,
            insert_final_newline,
            trim_trailing_whitespace,
        })
    }

    fn parsed(&self, directory: &ProjectPath) -> Cached {
        if let Some(cached) = self.cache.borrow().get(directory) {
            return cached.clone();
        }
        let file = directory.join(&ProjectPath::parse(FILE_NAME).expect("constant"));
        let entry = if self.tree.is_file(&file) {
            Some(match self.parse(&file) {
                Ok(parsed) => Ok(std::rc::Rc::new(parsed)),
                Err(diagnostic) => {
                    self.reported.borrow_mut().push(diagnostic);
                    Err(())
                }
            })
        } else {
            None
        };
        self.cache
            .borrow_mut()
            .insert(directory.clone(), entry.clone());
        entry
    }

    fn parse(&self, file: &ProjectPath) -> Result<Parsed, Diagnostic> {
        let report = |message: String, line: Option<u32>| {
            Diagnostic::new(Code::HygieneConfig, file.as_str(), message).at_line(line)
        };
        match self.tree.symlink_component(file) {
            Ok(None) => {}
            Ok(Some(link)) => {
                return Err(report(
                    format!(
                        "`{FILE_NAME}` is reached through the symbolic link `{link}`; configuration is never read through links"
                    ),
                    None,
                ));
            }
            Err(error) => {
                return Err(report(
                    format!("cannot inspect `{FILE_NAME}`: {error}"),
                    None,
                ));
            }
        }
        let limit = self.budget.limits().file_bytes;
        let bytes = match self.tree.read_bounded(file, limit) {
            Ok((_, true)) => {
                return Err(report(
                    super::over_limit(self.tree, file, &format!("`{FILE_NAME}`"), limit),
                    None,
                ));
            }
            Ok((bytes, false)) => bytes,
            Err(error) => {
                return Err(report(format!("cannot read `{FILE_NAME}`: {error}"), None));
            }
        };
        if let Err(message) = self.budget.charge(file.as_str(), bytes.len() as u64) {
            self.fatal.borrow_mut().get_or_insert(message);
            return Err(report("hygiene input budget exhausted".to_owned(), None));
        }
        let mut parser = ConfigParser::new_buffered(Cursor::new(bytes))
            .map_err(|error| report(describe(&error), None))?;
        let is_root = parser.is_root;
        let mut sections = Vec::new();
        loop {
            let line = u32::try_from(parser.line_no()).ok();
            match parser.next() {
                None => break,
                Some(Ok(section)) => sections.push(section),
                Some(Err(error)) => return Err(report(describe(&error), line)),
            }
        }
        Ok(Parsed { is_root, sections })
    }
}

/// A supported property: `Ok(None)` when absent or `unset`, `Ok(Some)`
/// when it parses, `Err` when set to something that does not parse.
fn typed<T: PropertyKey + PropertyValue>(properties: &Properties) -> Result<Option<T>, ()> {
    // `filter_unset` maps the literal value `unset` to the absent value.
    let raw = properties.get_raw::<T>().filter_unset();
    if raw.is_unset() {
        return Ok(None);
    }
    raw.parse::<T>().map(Some).map_err(|_| ())
}

fn describe(error: &ParseError) -> String {
    match error {
        ParseError::Eof => "unexpected end of file".to_owned(),
        ParseError::Io(error) => format!("cannot read `{FILE_NAME}`: {error}"),
        ParseError::InvalidLine => format!(
            "`{FILE_NAME}` has a line that is neither a section header, a property, nor a comment"
        ),
        ParseError::EmptyCharClass => {
            format!("`{FILE_NAME}` has a section header with an empty character class")
        }
    }
}
