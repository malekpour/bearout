// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::process::ExitCode;

use bearout::{Command, Mode, Options, Report, Source, TestReport};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "bearout", version, about)]
struct Cli {
    /// Report format.
    #[arg(long, global = true, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// Run the formatters bearout.toml declares. They are trusted host
    /// programs chosen by the repository, not confined by Bearout.
    #[arg(long, global = true)]
    allow_formatters: bool,

    #[command(subcommand)]
    command: Subcommands,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    /// One finding per line on standard error and a summary.
    Text,
    /// One JSON report on standard output, for every outcome.
    Json,
}

/// Where to read the project from. Without a selection, the working
/// directory is read. The Git-backed sources are experimental, read-only,
/// and require the `git` executable.
#[derive(Debug, Default, Args)]
struct TreeArgs {
    /// Read the Git index (staged content) instead of the working directory.
    #[arg(long, conflicts_with = "revision")]
    index: bool,
    /// Read one Git revision (a commit, tag, branch, or tree) instead of the
    /// working directory. The name is resolved once, at the start.
    #[arg(long, value_name = "REV")]
    revision: Option<String>,
}

impl TreeArgs {
    fn source(&self) -> Source {
        match (self.index, &self.revision) {
            (true, _) => Source::Index,
            (false, Some(revision)) => Source::Revision(revision.clone()),
            (false, None) => Source::WorkingDirectory,
        }
    }
}

/// The source plus an optional comparison baseline.
#[derive(Debug, Default, Args)]
struct SourceArgs {
    #[command(flatten)]
    tree: TreeArgs,
    /// Compare against one Git revision of the same repository, resolved
    /// once. Nothing is inferred; name the baseline explicitly.
    #[arg(long, value_name = "REV")]
    baseline: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Validate a Bearout project and its resource graph.
    Check {
        /// Project directory containing bearout.toml.
        #[arg(default_value = ".")]
        path: PathBuf,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Rewrite the selected files so they satisfy the configured hygiene
    /// and formatters. Working directory only; runs no repository policy.
    Format {
        /// Project directory containing bearout.toml.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Validate a project, then render its generators' outputs.
    Generate {
        /// Project directory containing bearout.toml.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Verify committed outputs instead of writing them. Required with
        /// --index or --revision, which are read-only.
        #[arg(long)]
        check: bool,
        #[command(flatten)]
        source: SourceArgs,
    },
    /// Run the contract fixture cases bearout.toml declares in [fixtures]
    /// against virtual mutations of the selected source. Read-only: never
    /// formats or delivers; each case decides whether the unmodified
    /// source is its comparison baseline. Experimental.
    Test {
        /// Project directory containing bearout.toml.
        #[arg(default_value = ".")]
        path: PathBuf,
        #[command(flatten)]
        source: TreeArgs,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (path, command, verb, source) = match cli.command {
        Subcommands::Check { path, source } => (path, Command::Check, "checked", source),
        Subcommands::Format { path } => (path, Command::Format, "formatted", SourceArgs::default()),
        Subcommands::Test { path, source } => {
            let options = Options {
                source: source.source(),
                allow_formatters: cli.allow_formatters,
                ..Options::default()
            };
            let report = bearout::test(&path, &options);
            return match cli.format {
                Format::Json => print_json(&report, report.fatal.is_some(), report.ok),
                Format::Text => {
                    print_test_text(&report);
                    exit_code(report.fatal.is_some(), report.ok)
                }
            };
        }
        Subcommands::Generate {
            path,
            check: false,
            source,
        } => (path, Command::Generate(Mode::Write), "generated", source),
        Subcommands::Generate {
            path,
            check: true,
            source,
        } => (path, Command::Generate(Mode::Check), "verified", source),
    };
    let options = Options {
        source: source.tree.source(),
        baseline: source.baseline,
        allow_formatters: cli.allow_formatters,
        ..Options::default()
    };
    let report = bearout::run(&path, command, &options);

    match cli.format {
        Format::Json => print_json(&report, report.fatal.is_some(), report.is_clean()),
        Format::Text => {
            print_text(&report, verb);
            exit_code(report.fatal.is_some(), report.is_clean())
        }
    }
}

/// 0 for a clean outcome, 1 for findings or failed cases, 2 for a fatal
/// outcome.
fn exit_code(fatal: bool, ok: bool) -> ExitCode {
    if fatal {
        ExitCode::from(2)
    } else if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Print one JSON document for any outcome, so JSON stays valid on every
/// path, then exit by the report's own `fatal` and `ok`.
fn print_json(report: &impl serde::Serialize, fatal: bool, ok: bool) -> ExitCode {
    match serde_json::to_string_pretty(report) {
        Ok(text) => println!("{text}"),
        Err(error) => {
            println!(
                "{{\"ok\":false,\"fatal\":{}}}",
                serde_json::Value::String(error.to_string())
            );
            return ExitCode::from(2);
        }
    }
    exit_code(fatal, ok)
}

/// One line per case on standard output, the details of each failed case
/// beneath it, and a summary: on standard output when every case passed,
/// on standard error otherwise. A fatal suite prints only its reason.
fn print_test_text(report: &TestReport) {
    if let Some(fatal) = &report.fatal {
        eprintln!("bearout: {fatal}");
        return;
    }
    for case in &report.cases {
        let status = if case.passed { "ok  " } else { "FAIL" };
        println!("{status} {} ({})", case.name, case.file);
        if case.passed {
            continue;
        }
        if case.expected != case.actual {
            println!("     expected {}, got {}", case.expected, case.actual);
        }
        if let Some(fatal) = &case.fatal {
            println!("     fatal: {fatal}");
        }
        if let (Some(expected), true) =
            (&case.expected_fatal, case.actual == bearout::Outcome::Fatal)
        {
            println!("     expected the fatal message to contain {expected:?}");
        }
        for expectation in &case.missing {
            println!("     missing: {expectation}");
        }
        for diagnostic in &case.unexpected {
            println!("     unexpected: {diagnostic}");
        }
    }
    let summary = format!(
        "tested {} case(s): {} passed, {} failed",
        report.total, report.passed, report.failed
    );
    if report.ok {
        println!("{summary}");
    } else {
        eprintln!("{summary}");
    }
}

fn print_text(report: &Report, verb: &str) {
    if let Some(fatal) = &report.fatal {
        eprintln!("bearout: {fatal}");
        return;
    }
    for diagnostic in &report.diagnostics {
        eprintln!("{diagnostic}");
    }
    if verb == "formatted" {
        for path in &report.formatted {
            println!("formatted {path}");
        }
        if report.is_clean() {
            println!(
                "formatted {} of {} selected file(s)",
                report.formatted.len(),
                report.files
            );
        } else {
            eprintln!(
                "formatted {} of {} selected file(s): {} error(s)",
                report.formatted.len(),
                report.files,
                report.errors()
            );
        }
        return;
    }
    let outputs = if report.outputs.is_empty() {
        String::new()
    } else {
        format!(", {} output(s) {verb}", report.outputs.len())
    };
    let documents = if report.documents == 0 {
        String::new()
    } else {
        format!(" and {} document(s)", report.documents)
    };
    if report.is_clean() {
        println!(
            "checked {} resource(s){documents}: clean{outputs}",
            report.resources
        );
    } else {
        eprintln!(
            "checked {} resource(s){documents}: {} error(s){outputs}",
            report.resources,
            report.errors()
        );
    }
}
