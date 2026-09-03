// SPDX-License-Identifier: Apache-2.0

use std::path::PathBuf;
use std::process::ExitCode;

use bearout::{Command, Mode, Options, Report};
use clap::{Parser, Subcommand, ValueEnum};

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

#[derive(Debug, Subcommand)]
enum Subcommands {
    /// Validate a Bearout project and its resource graph.
    Check {
        /// Project directory containing bearout.toml.
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Validate a project, then render its generators' outputs.
    Generate {
        /// Project directory containing bearout.toml.
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Verify committed outputs instead of writing them.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let (path, command, verb) = match cli.command {
        Subcommands::Check { path } => (path, Command::Check, "checked"),
        Subcommands::Generate { path, check: false } => {
            (path, Command::Generate(Mode::Write), "generated")
        }
        Subcommands::Generate { path, check: true } => {
            (path, Command::Generate(Mode::Check), "verified")
        }
    };
    let report = bearout::run(&path, command, &Options::default());

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
    if report.is_clean() {
        println!("checked {} resource(s): clean{outputs}", report.resources);
    } else {
        eprintln!(
            "checked {} resource(s): {} error(s){outputs}",
            report.resources,
            report.errors()
        );
    }
}
