// SPDX-License-Identifier: AGPL-3.0-or-later
//! `IsingMark` CLI entry point.

use std::process::ExitCode;

use clap::Parser as _;
use isingmark::cli::{Cli, Command};
use isingmark::modes::throughput::{self, ThroughputConfig};
use isingmark::report::format_summary_table;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Throughput(args) => match args.into_config() {
            Ok(cfg) => run_throughput(&cfg).await,
            Err(message) => fail(&message),
        },
    }
}

/// Run throughput mode and report the outcome. Returns `ExitCode::SUCCESS`
/// even when some jobs failed/were rejected — that is a benchmark result,
/// not a tool failure; only a `ModeError` (the mode could not run at all)
/// is reported as a non-zero exit.
async fn run_throughput(cfg: &ThroughputConfig) -> ExitCode {
    match throughput::run_throughput(cfg).await {
        Ok(report) => {
            let summary_path = cfg.output_dir.join(throughput::SUMMARY_FILENAME);
            let records_path = cfg.output_dir.join(throughput::RECORDS_FILENAME);
            match std::fs::read_to_string(&summary_path)
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
            {
                Some(summary) => succeed(&format!(
                    "{}\nrecords: {}\nsummary: {}\n",
                    format_summary_table(&summary),
                    records_path.display(),
                    summary_path.display(),
                )),
                None => succeed(&format!(
                    "throughput run complete: {} records\nrecords: {}\nsummary: {}\n",
                    report.records.len(),
                    records_path.display(),
                    summary_path.display(),
                )),
            }
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Print `message` on success and return `ExitCode::SUCCESS`.
fn succeed(message: &str) -> ExitCode {
    print_line(message);
    ExitCode::SUCCESS
}

/// Print `message` as an error and return `ExitCode::FAILURE`.
fn fail(message: &str) -> ExitCode {
    print_line(&format!("error: {message}"));
    ExitCode::FAILURE
}

/// The single documented CLI output boundary: every other function in this
/// binary routes its user-facing text through here rather than adding its
/// own `#[expect(clippy::print_stdout, ...)]`.
fn print_line(message: &str) {
    #[expect(
        clippy::print_stdout,
        reason = "single documented CLI output boundary for this binary's user-facing text"
    )]
    {
        println!("{message}");
    }
}
