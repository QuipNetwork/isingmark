// SPDX-License-Identifier: AGPL-3.0-or-later
//! `IsingMark` CLI entry point.

use std::fmt::Write as _;
use std::process::ExitCode;

use clap::Parser as _;
use isingmark::backend::RunReport;
use isingmark::cli::{Cli, Command};
use isingmark::modes::sweep::{self, SweepConfig};
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
        Command::Sweep(args) => match args.into_config() {
            Ok(cfg) => run_sweep(&cfg).await,
            Err(message) => fail(&message),
        },
    }
}

/// Run throughput mode and report the outcome. Returns `ExitCode::SUCCESS`
/// even when some jobs failed/were rejected — that is a benchmark result,
/// not a tool failure; only a `ModeError` (the mode could not run at all)
/// is reported as a non-zero exit. A run whose miner failed partway
/// through still exits `SUCCESS` (its records are real and worth keeping)
/// but prints a warning naming what happened, so it is not mistaken for a
/// complete run.
async fn run_throughput(cfg: &ThroughputConfig) -> ExitCode {
    match throughput::run_throughput_with_summary(cfg).await {
        Ok((report, summary)) => {
            let records_path = cfg.output_dir.join(throughput::RECORDS_FILENAME);
            let summary_path = cfg.output_dir.join(throughput::SUMMARY_FILENAME);
            let mut message = format_summary_table(&summary);
            if let Some(warning) = truncated_run_warning(&report) {
                message.push_str(&warning);
                message.push('\n');
            }
            let _ = write!(
                message,
                "records: {}\nsummary: {}\n",
                records_path.display(),
                summary_path.display(),
            );
            succeed(&message)
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Run sweep mode and report the outcome. Mirrors [`run_throughput`]'s
/// exit-code convention: `ExitCode::SUCCESS` even when some cell's miner
/// failed partway through (a benchmark result, not a tool failure); each
/// affected cell gets its own warning line via [`truncated_run_warning`],
/// prefixed with that cell's own directory name so a multi-cell warning
/// list is attributable to the cell it came from.
async fn run_sweep(cfg: &SweepConfig) -> ExitCode {
    match sweep::run_sweep(cfg).await {
        Ok(report) => {
            let mut message = format_summary_table(&report.summary);
            for cell in &report.cells {
                if let Some(warning) = truncated_run_warning(&cell.report) {
                    let _ = writeln!(message, "[{}] {warning}", cell.dir_name);
                }
            }
            let _ = writeln!(message, "output: {}", cfg.output_dir.display());
            succeed(&message)
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Describe why a run stopped short of a clean, complete finish, or `None`
/// if it did not. Distinguishes three causes an operator cannot otherwise
/// tell apart from `report.records.len()` alone: the miner never completed
/// its handshake (no jobs ran at all), it stopped responding partway
/// through (`miner_unresponsive`), or it exited non-zero without either of
/// those (a crash or a rejected session, mid-run).
fn truncated_run_warning(report: &RunReport) -> Option<String> {
    let mut reason = if !report.handshake_ok {
        Some("the miner never completed its handshake; no jobs were run".to_string())
    } else if report.miner_unresponsive {
        Some(format!(
            "the miner stopped responding partway through the run (exit code {}); \
             results reflect only the jobs completed before that",
            report.miner_exit_code
        ))
    } else if report.miner_exit_code != 0 {
        Some(format!(
            "the miner exited with a non-zero code ({}); results may be incomplete",
            report.miner_exit_code
        ))
    } else {
        None
    }?;
    if let Some(tail) = &report.miner_stderr_tail {
        reason.push_str("\nminer stderr:\n");
        reason.push_str(tail);
    }
    Some(format!("warning: {reason}"))
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

#[cfg(test)]
mod tests {
    use super::truncated_run_warning;
    use isingmark::backend::RunReport;

    /// A `RunReport` with every field at its "everything went fine" value,
    /// so a test can override only the one field it cares about.
    fn clean_report() -> RunReport {
        RunReport {
            handshake_ok: true,
            records: Vec::new(),
            miner_exit_code: 0,
            miner_unresponsive: false,
            miner_stderr_tail: None,
            run_wall_ms: 1_000,
        }
    }

    #[test]
    fn no_warning_for_a_clean_completed_run() {
        assert_eq!(truncated_run_warning(&clean_report()), None);
    }

    #[test]
    fn warns_when_the_handshake_never_completed() {
        let report = RunReport {
            handshake_ok: false,
            ..clean_report()
        };
        let warning = truncated_run_warning(&report).expect("must warn");
        assert!(warning.contains("handshake"));
    }

    #[test]
    fn warns_with_the_exit_code_when_the_miner_goes_unresponsive() {
        let report = RunReport {
            miner_unresponsive: true,
            miner_exit_code: 137,
            ..clean_report()
        };
        let warning = truncated_run_warning(&report).expect("must warn");
        assert!(warning.contains("stopped responding"));
        assert!(warning.contains("137"));
    }

    #[test]
    fn warns_with_the_exit_code_on_a_nonzero_exit_that_is_not_flagged_unresponsive() {
        let report = RunReport {
            miner_exit_code: 70,
            ..clean_report()
        };
        let warning = truncated_run_warning(&report).expect("must warn");
        assert!(warning.contains("non-zero"));
        assert!(warning.contains("70"));
    }

    #[test]
    fn warning_includes_the_stderr_tail_when_present() {
        let report = RunReport {
            handshake_ok: false,
            miner_stderr_tail: Some("distinctive marker 4e21".to_string()),
            ..clean_report()
        };
        let warning = truncated_run_warning(&report).expect("must warn");
        assert!(warning.contains("distinctive marker 4e21"));
    }
}
