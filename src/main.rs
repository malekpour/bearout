// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::process::ExitCode;

use bearout::{Command, Mode, Options, Report, Source};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(name = "bearout", version, about)]
struct Cli {
    /// Report format.
    #[arg(long, global = true, value_enum, default_value_t = Format::Text)]
    format: Format,

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
#[derive(Debug, Args)]
struct SourceArgs {
    /// Read the Git index (staged content) instead of the working directory.
    #[arg(long, conflicts_with = "revision")]
    index: bool,
    /// Read one Git revision (a commit, tag, branch, or tree) instead of the
    /// working directory. The name is resolved once, at the start.
    #[arg(long, value_name = "REV")]
    revision: Option<String>,
    /// Compare against one Git revision of the same repository, resolved
    /// once. Nothing is inferred; name the baseline explicitly.
    #[arg(long, value_name = "REV")]
    baseline: Option<String>,
}

impl SourceArgs {
    fn source(&self) -> Source {
        match (self.index, &self.revision) {
            (true, _) => Source::Index,
            (false, Some(revision)) => Source::Revision(revision.clone()),
            (false, None) => Source::WorkingDirectory,
        }
    }
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (path, command, verb, source) = match cli.command {
        Subcommands::Check { path, source } => (path, Command::Check, "checked", source),
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
        source: source.source(),
        baseline: source.baseline,
        ..Options::default()
    };
    let report = bearout::run(&path, command, &options);

    match cli.format {
        Format::Json => match serde_json::to_string_pretty(&report) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                println!(
                    "{{\"ok\":false,\"fatal\":{}}}",
                    serde_json::Value::String(error.to_string())
                );
                return ExitCode::from(2);
            }
        },
        Format::Text => print_text(&report, verb),
    }

    if report.fatal.is_some() {
        ExitCode::from(2)
    } else if report.is_clean() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
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
