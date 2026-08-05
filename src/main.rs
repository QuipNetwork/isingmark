// SPDX-License-Identifier: AGPL-3.0-or-later
//! `IsingMark` CLI entry point.

use std::fmt::Write as _;
use std::process::ExitCode;

use clap::Parser as _;
use isingmark::backend::RunReport;
use isingmark::cli::{Cli, Command};
use isingmark::modes::comparative::{self, ComparativeConfig};
use isingmark::modes::sweep::{self, SweepConfig};
use isingmark::modes::throughput::{self, ThroughputConfig};
use isingmark::modes::ttt::{self, TttConfig};
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
        Command::Ttt(args) => match args.into_config() {
            Ok(cfg) => run_ttt(&cfg).await,
            Err(message) => fail(&message),
        },
        Command::Comparative(args) => match args.into_config() {
            Ok(cfg) => run_comparative(&cfg).await,
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

/// Run time-to-target mode and report the outcome. Same exit-code convention
/// as [`run_throughput`]/[`run_sweep`]: `ExitCode::SUCCESS` even when some
/// target was never reached (a benchmark result, not a tool failure); only a
/// `ModeError` (the mode could not run at all) is a non-zero exit. Does not
/// emit per-trial [`truncated_run_warning`]s the way `run_sweep` does per
/// cell: `ttt::run_ttt` runs one `run_session` call per *trial* (not per
/// target), so a single target's `RunReport`s would be one per trial rather
/// than one per target, and `summary.json`'s own per-target `reached`/
/// `trials_attempted` fields already surface a target that came up short.
async fn run_ttt(cfg: &TttConfig) -> ExitCode {
    match ttt::run_ttt(cfg).await {
        Ok(report) => {
            let mut message = format_summary_table(&report.summary);
            let _ = writeln!(message, "output: {}", cfg.output_dir.display());
            succeed(&message)
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Run comparative mode and report the outcome. Same exit-code convention as
/// the other modes: `ExitCode::SUCCESS` even when the reported `ks_distance`
/// is a poor match (a benchmark result, not a tool failure); only a
/// `ModeError` (the search could not run at all) is a non-zero exit.
///
/// `run_comparative`'s fixed return shape (`(Vec<JobRecord>, f64)`, not a
/// `Summary`) has no room for `ks_distance` — the *only* signal that the
/// reported hardness means anything (a search that silently reported a bad
/// match as good would be exactly the failure this project exists to
/// prevent, see `comparative`'s own module doc). Rather than leave that
/// number out of the human-facing line entirely, [`read_final_ks_distance`]
/// re-reads it from the `summary.json` this call just wrote, the same
/// round trip `record::write_summary`'s own callers already pay for
/// anything not in the typed return value.
async fn run_comparative(cfg: &ComparativeConfig) -> ExitCode {
    match comparative::run_comparative(cfg).await {
        Ok((records, hardness)) => {
            let summary_path = cfg.output_dir.join(comparative::SUMMARY_FILENAME);
            let ks_distance = read_final_ks_distance(&summary_path);
            let mut message = format!("matched hardness: {hardness:.4}\n");
            let _ = writeln!(message, "ks_distance: {}", format_ks_distance(ks_distance));
            let _ = writeln!(message, "records: {}", records.len());
            let _ = writeln!(message, "summary: {}", summary_path.display());
            succeed(&message)
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Read `config.final_ks_distance` back out of a comparative-mode
/// `summary.json` at `summary_path`. Returns `None` if the file cannot be
/// read, does not parse as JSON, or the field is absent/not a number (the
/// last case is not just defensive: `comparative::run_comparative`'s own
/// final iteration can itself have no `ks_distance` — see
/// `stats::ks_distance`'s documented `None` cases — so a present-but-`null`
/// field is an expected, not a malformed, summary).
fn read_final_ks_distance(summary_path: &std::path::Path) -> Option<f64> {
    let text = std::fs::read_to_string(summary_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("config")?.get("final_ks_distance")?.as_f64()
}

/// Render an optional `ks_distance` for the CLI's human-facing line:
/// `"n/a"` for `None` (see [`read_final_ks_distance`]'s doc for the two
/// distinct cases that produce it), four decimal places otherwise —
/// `stats::ks_distance` is already bounded to `[0.0, 1.0]`, so four places
/// is enough precision to distinguish a good match from a poor one without
/// implying false accuracy.
fn format_ks_distance(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |v| format!("{v:.4}"))
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
    use super::{format_ks_distance, read_final_ks_distance, truncated_run_warning};
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

    #[test]
    fn format_ks_distance_renders_four_decimal_places() {
        assert_eq!(format_ks_distance(Some(0.073_437_5)), "0.0734");
        assert_eq!(format_ks_distance(Some(1.0)), "1.0000");
        assert_eq!(format_ks_distance(Some(0.0)), "0.0000");
    }

    #[test]
    fn format_ks_distance_renders_none_as_not_applicable() {
        assert_eq!(format_ks_distance(None), "n/a");
    }

    #[test]
    fn read_final_ks_distance_extracts_the_nested_config_field() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("summary.json");
        std::fs::write(&path, r#"{"config": {"final_ks_distance": 0.42}}"#)
            .expect("write summary.json");
        assert_eq!(read_final_ks_distance(&path), Some(0.42));
    }

    /// A comparative run's final iteration can itself report a `null`
    /// `ks_distance` (see `stats::ks_distance`'s documented `None` cases),
    /// which is a valid, present field — not the same failure mode as a
    /// missing file or malformed JSON, but this function must still return
    /// `None` for it, matching every other case a caller cannot act on.
    #[test]
    fn read_final_ks_distance_treats_a_present_null_as_none() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("summary.json");
        std::fs::write(&path, r#"{"config": {"final_ks_distance": null}}"#)
            .expect("write summary.json");
        assert_eq!(read_final_ks_distance(&path), None);
    }

    #[test]
    fn read_final_ks_distance_returns_none_for_a_missing_file() {
        assert_eq!(
            read_final_ks_distance(std::path::Path::new("/nonexistent/summary.json")),
            None
        );
    }

    #[test]
    fn read_final_ks_distance_returns_none_for_malformed_json() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("summary.json");
        std::fs::write(&path, "{not valid json").expect("write summary.json");
        assert_eq!(read_final_ks_distance(&path), None);
    }
}
