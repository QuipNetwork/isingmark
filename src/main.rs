// SPDX-License-Identifier: AGPL-3.0-or-later
//! `IsingMark` CLI entry point.

use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser as _;
use isingmark::backend::RunReport;
use isingmark::cli::{Cli, Command, CompareArgs};
use isingmark::modes::comparative::{self, ComparativeConfig};
use isingmark::modes::compare::{self, format_comparison_table};
use isingmark::modes::reproducibility::{self, ReproConfig, ReproReport};
use isingmark::modes::scaling::{self, ScalingConfig};
use isingmark::modes::sweep::{self, SweepConfig, SweepReport};
use isingmark::modes::throughput::{self, ThroughputConfig};
use isingmark::modes::ttt::{self, TttConfig, TttReport};
use isingmark::report::format_summary_table;
#[cfg(not(feature = "plot"))]
use isingmark::report::plot_feature_required_message;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Throughput(args) => match args.into_config() {
            Ok(cfg) => run_throughput(&cfg).await,
            Err(message) => fail(&message),
        },
        Command::Sweep(args) => {
            let plot = args.plot.clone();
            match args.into_config() {
                Ok(cfg) => run_sweep(&cfg, plot.as_deref()).await,
                Err(message) => fail(&message),
            }
        }
        Command::Ttt(args) => {
            let plot = args.plot.clone();
            match args.into_config() {
                Ok(cfg) => run_ttt(&cfg, plot.as_deref()).await,
                Err(message) => fail(&message),
            }
        }
        Command::Comparative(args) => match args.into_config() {
            Ok(cfg) => run_comparative(&cfg).await,
            Err(message) => fail(&message),
        },
        Command::Scaling(args) => match args.into_config() {
            Ok(cfg) => run_scaling(&cfg).await,
            Err(message) => fail(&message),
        },
        Command::Reproducibility(args) => match args.into_config() {
            Ok(cfg) => run_reproducibility(&cfg).await,
            Err(message) => fail(&message),
        },
        Command::Compare(args) => run_compare_files(&args),
    }
}

/// Run throughput mode and report the outcome. A run with at least one
/// rejected job and no completed jobs exits non-zero after printing its
/// summary and evidence paths. Partial results remain successful, with a
/// warning when the miner stopped early. A `ModeError` also exits non-zero.
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
            if summary.totals.rejected > 0 && summary.totals.rejected == summary.totals.jobs {
                print_line(&message);
                fail("no jobs completed; all dispatched jobs were rejected")
            } else {
                succeed(&message)
            }
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Run sweep mode and report the outcome. Returns `ExitCode::SUCCESS`
/// even when some cell's miner failed partway through (a benchmark result,
/// not a tool failure); each
/// affected cell gets its own warning line via [`truncated_run_warning`],
/// prefixed with that cell's own directory name so a multi-cell warning
/// list is attributable to the cell it came from.
///
/// When `plot` is `Some`, writes a sweep heatmap PNG to that path (or
/// exits non-zero if the binary lacks the `plot` feature, or plotting
/// fails). The flag is always parsed; only the behaviour is feature-gated.
async fn run_sweep(cfg: &SweepConfig, plot: Option<&Path>) -> ExitCode {
    match sweep::run_sweep(cfg).await {
        Ok(report) => {
            if let Some(path) = plot {
                if let Err(message) = write_sweep_plot(&report, path) {
                    return fail(&message);
                }
            }
            let mut message = format_summary_table(&report.summary);
            for cell in &report.cells {
                if let Some(warning) = truncated_run_warning(&cell.report) {
                    let _ = writeln!(message, "[{}] {warning}", cell.dir_name);
                }
            }
            let _ = writeln!(message, "output: {}", cfg.output_dir.display());
            if let Some(path) = plot {
                let _ = writeln!(message, "plot: {}", path.display());
            }
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
///
/// When `plot` is `Some`, writes a TTT CDF PNG to that path (or exits
/// non-zero if the binary lacks the `plot` feature, or plotting fails).
async fn run_ttt(cfg: &TttConfig, plot: Option<&Path>) -> ExitCode {
    match ttt::run_ttt(cfg).await {
        Ok(report) => {
            if let Some(path) = plot {
                if let Err(message) = write_ttt_plot(&report, path) {
                    return fail(&message);
                }
            }
            let mut message = format_summary_table(&report.summary);
            let _ = writeln!(message, "output: {}", cfg.output_dir.display());
            if let Some(path) = plot {
                let _ = writeln!(message, "plot: {}", path.display());
            }
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
        Ok((_final_records, hardness)) => {
            let summary_path = cfg.output_dir.join(comparative::SUMMARY_FILENAME);
            let ks_distance = read_final_ks_distance(&summary_path);
            // `totals.jobs` is the true aggregate across every iteration
            // (matches summary.json and every other multi-session mode),
            // not the final evaluation alone.
            let total_records = read_totals_jobs(&summary_path);
            let mut message = format!("matched hardness: {hardness:.4}\n");
            let _ = writeln!(message, "ks_distance: {}", format_ks_distance(ks_distance));
            let _ = writeln!(
                message,
                "records: {}",
                total_records.map_or_else(|| "?".to_string(), |n| n.to_string())
            );
            let _ = writeln!(message, "summary: {}", summary_path.display());
            succeed(&message)
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Run scaling mode and report the outcome. Mirrors [`run_sweep`]'s
/// exit-code convention: `ExitCode::SUCCESS` even when a size's miner
/// failed partway through (a benchmark result, not a tool failure); each
/// affected size gets its own warning line via [`truncated_run_warning`],
/// prefixed with that size's own directory name.
async fn run_scaling(cfg: &ScalingConfig) -> ExitCode {
    match scaling::run_scaling(cfg).await {
        Ok(report) => {
            let mut message = format_summary_table(&report.summary);
            for size in &report.sizes {
                if let Some(warning) = truncated_run_warning(&size.report) {
                    let _ = writeln!(message, "[{}] {warning}", size.dir_name);
                }
            }
            let _ = writeln!(message, "output: {}", cfg.output_dir.display());
            succeed(&message)
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Run reproducibility mode and report the outcome. Same exit-code
/// convention as the other modes: `ExitCode::SUCCESS` even when the two
/// runs' distributions differ (a benchmark result, not a tool failure);
/// only a `ModeError` (a run could not be dispatched, or produced no usable
/// data to compare at all — see `reproducibility`'s own module doc on
/// `ModeError::EmptyReproBatch`) is a non-zero exit.
async fn run_reproducibility(cfg: &ReproConfig) -> ExitCode {
    match reproducibility::run_reproducibility(cfg).await {
        Ok(report) => {
            let summary_path = cfg.output_dir.join(reproducibility::SUMMARY_FILENAME);
            succeed(&format_reproducibility_message(cfg, &report, &summary_path))
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Run compare mode and report the outcome. Pure post-hoc analysis: reads
/// two JSONL files and prints a table. A `ModeError` (unreadable or empty
/// input — see `compare`'s own module doc) is a non-zero exit; a real
/// comparison that finds a large KS distance still exits `SUCCESS` (that
/// is a benchmark result, not a tool failure).
///
/// When `args.plot` is set, writes an energy-CDF overlay PNG (or exits
/// non-zero if the binary lacks the `plot` feature, or plotting fails).
fn run_compare_files(args: &CompareArgs) -> ExitCode {
    match compare::run_compare(&args.file_a, &args.file_b) {
        Ok(report) => {
            if let Some(path) = &args.plot {
                if let Err(message) = write_compare_plot(&args.file_a, &args.file_b, path) {
                    return fail(&message);
                }
            }
            let mut message = format_comparison_table(&report);
            if let Some(path) = &args.plot {
                let _ = writeln!(message, "plot: {}", path.display());
            }
            succeed(&message)
        }
        Err(e) => fail(&e.to_string()),
    }
}

/// Write a sweep heatmap, or return an error message naming the missing
/// `plot` feature when this binary was built without it.
fn write_sweep_plot(report: &SweepReport, path: &Path) -> Result<(), String> {
    #[cfg(feature = "plot")]
    {
        isingmark::report::plot_sweep_heatmap(report, path).map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "plot"))]
    {
        let _ = (report, path);
        Err(plot_feature_required_message().to_string())
    }
}

/// Write a time-to-target CDF, or return an error message naming the
/// missing `plot` feature when this binary was built without it.
fn write_ttt_plot(report: &TttReport, path: &Path) -> Result<(), String> {
    #[cfg(feature = "plot")]
    {
        isingmark::report::plot_ttt_cdf(report, path).map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "plot"))]
    {
        let _ = (report, path);
        Err(plot_feature_required_message().to_string())
    }
}

/// Write an energy-CDF overlay for two JSONL files, or return an error
/// message naming the missing `plot` feature when this binary was built
/// without it.
fn write_compare_plot(file_a: &Path, file_b: &Path, path: &Path) -> Result<(), String> {
    #[cfg(feature = "plot")]
    {
        let energies_a = compare::load_passed_energies(file_a).map_err(|e| e.to_string())?;
        let energies_b = compare::load_passed_energies(file_b).map_err(|e| e.to_string())?;
        isingmark::report::plot_energy_cdfs(&energies_a, &energies_b, path)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(feature = "plot"))]
    {
        let _ = (file_a, file_b, path);
        Err(plot_feature_required_message().to_string())
    }
}

/// Build [`run_reproducibility`]'s human-facing report. `cfg.hardness_a`/
/// `cfg.hardness_b` are printed **unconditionally**, not only when they
/// differ: an operator who ran the two arms at deliberately different
/// sampling effort (see `reproducibility`'s own module doc on why
/// `hardness_b` exists) and later reads only `differs: true` out of context
/// — a ticket, a log excerpt — would otherwise have no way to tell that
/// result apart from a real reproducibility failure of an identical-effort
/// run. Printing both values always means the ordinary case (`hardness_a ==
/// hardness_b`) *affirmatively confirms* the arms were identical, rather
/// than leaving the reader to assume it.
///
/// `p_value` is rendered in scientific notation (`{:.4e}`), not fixed-point:
/// a fixed 4- or even 6-decimal rendering of a genuinely small p-value (e.g.
/// the `~3e-8` a strongly-differing pair produces) prints as a
/// misleading `0.0000`, indistinguishable from an actually-zero value.
/// `differs` is printed alongside the exact threshold it was computed
/// against (`reproducibility::DIFFERS_P_THRESHOLD`) rather than bare, so a
/// borderline value (e.g. `p = 0.04999`) next to `differs: true` reads as
/// "just under this printed threshold," not as an apparent contradiction of
/// an unstated `0.05` convention.
fn format_reproducibility_message(
    cfg: &ReproConfig,
    report: &ReproReport,
    summary_path: &std::path::Path,
) -> String {
    let mut message = format!("hardness_a: {:.4}\n", cfg.hardness_a);
    let _ = writeln!(message, "hardness_b: {:.4}", cfg.hardness_b);
    let _ = writeln!(message, "ks_statistic: {:.4}", report.ks_statistic);
    let _ = writeln!(message, "p_value: {:.4e}", report.p_value);
    let _ = writeln!(
        message,
        "differs: {} (p < {})",
        report.differs,
        reproducibility::DIFFERS_P_THRESHOLD
    );
    let _ = writeln!(message, "summary: {}", summary_path.display());
    message
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

/// Read `totals.jobs` from a summary.json (aggregate across every
/// iteration for comparative mode). Same round-trip pattern as
/// [`read_final_ks_distance`].
fn read_totals_jobs(summary_path: &std::path::Path) -> Option<u64> {
    let text = std::fs::read_to_string(summary_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("totals")?.get("jobs")?.as_u64()
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
    use super::{
        format_ks_distance, format_reproducibility_message, read_final_ks_distance,
        truncated_run_warning,
    };
    use isingmark::backend::RunReport;
    use isingmark::modes::reproducibility::{ReproConfig, ReproReport, DEFAULT_PARAM_CURVE};
    use isingmark::modes::TopologyRef;

    /// A `RunReport` with every field at its "everything went fine" value,
    /// so a test can override only the one field it cares about.
    fn clean_report() -> RunReport {
        RunReport {
            handshake_ok: true,
            records: Vec::new(),
            miner_exit_code: 0,
            miner_unresponsive: false,
            miner_stderr_tail: None,
            orphan_results: 0,
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

    /// A `ReproConfig` with distinctive, easily-grepped hardness values,
    /// for tests that check the CLI's human-facing message rather than
    /// `run_reproducibility`'s own behavior.
    fn repro_config(hardness_a: f64, hardness_b: f64) -> ReproConfig {
        ReproConfig {
            backend: "cpu-sa".to_string(),
            topology: TopologyRef::Preset("smoke".to_string()),
            num_problems: 20,
            hardness_a,
            hardness_b,
            seed: 0,
            output_dir: std::path::PathBuf::from("./out"),
            param_curve: DEFAULT_PARAM_CURVE,
            backend_bin_override: None,
            backend_args_override: None,
        }
    }

    /// Pins Important 1 from the task-13 fix round: both hardness values
    /// must always appear in the printed message, even when they are equal
    /// (the ordinary case) — not only when they differ. A future refactor
    /// that drops one, or only prints them conditionally, must fail this.
    #[test]
    fn reproducibility_message_always_prints_both_hardness_values() {
        let report = ReproReport {
            ks_statistic: 0.0,
            p_value: 1.0,
            differs: false,
        };
        let message = format_reproducibility_message(
            &repro_config(0.5, 0.5),
            &report,
            std::path::Path::new("/tmp/out/summary.json"),
        );
        assert!(
            message.contains("hardness_a: 0.5"),
            "message must print hardness_a even when it equals hardness_b: {message}"
        );
        assert!(
            message.contains("hardness_b: 0.5"),
            "message must print hardness_b even when it equals hardness_a: {message}"
        );
    }

    /// The same assertion for the deliberately-different-hardness case,
    /// with distinct values, so a mutation that always prints
    /// `cfg.hardness_a` for both fields cannot pass both this test and the
    /// equal-hardness one above.
    #[test]
    fn reproducibility_message_prints_distinct_hardness_values_when_they_differ() {
        let report = ReproReport {
            ks_statistic: 0.65,
            p_value: 3.014_130_229_283_717e-8,
            differs: true,
        };
        let message = format_reproducibility_message(
            &repro_config(0.0, 1.0),
            &report,
            std::path::Path::new("/tmp/out/summary.json"),
        );
        assert!(message.contains("hardness_a: 0.0"), "got: {message}");
        assert!(message.contains("hardness_b: 1.0"), "got: {message}");
    }

    /// A genuinely small p-value must not print as a misleading `0.0000`
    /// (indistinguishable from an exact zero) — the exact defect a
    /// fixed-point `{:.4}` rendering has. Scientific notation keeps the
    /// nonzero magnitude visible regardless of how small the value is.
    #[test]
    fn reproducibility_message_renders_a_tiny_p_value_without_flattening_to_zero() {
        let report = ReproReport {
            ks_statistic: 0.65,
            p_value: 3.014_130_229_283_717e-8,
            differs: true,
        };
        let message = format_reproducibility_message(
            &repro_config(0.0, 1.0),
            &report,
            std::path::Path::new("/tmp/out/summary.json"),
        );
        assert!(
            !message.contains("p_value: 0.0000"),
            "a tiny nonzero p_value must not render as flat 0.0000: {message}"
        );
        assert!(message.to_lowercase().contains("e-8"), "got: {message}");
    }

    /// Ordinary-magnitude p-values must also use `{:.4e}` scientific
    /// notation. The tiny-value test above only pins the lower end of the
    /// constant space; without this, a regression that formats mid-range
    /// values as fixed-point would still pass.
    #[test]
    fn reproducibility_message_renders_an_ordinary_p_value_in_scientific_notation() {
        let report = ReproReport {
            ks_statistic: 0.1,
            p_value: 0.5,
            differs: false,
        };
        let message = format_reproducibility_message(
            &repro_config(0.0, 1.0),
            &report,
            std::path::Path::new("/tmp/out/summary.json"),
        );
        assert!(
            message.contains("p_value: 5.0000e-1"),
            "ordinary p_value must render with {{:.4e}}, got: {message}"
        );
    }

    /// `differs` must always be printed alongside the exact threshold it
    /// was computed against, so a borderline value does not read as
    /// contradicting an unstated convention.
    #[test]
    fn reproducibility_message_prints_differs_with_its_threshold() {
        let report = ReproReport {
            ks_statistic: 0.1,
            p_value: 0.049_99,
            differs: true,
        };
        let message = format_reproducibility_message(
            &repro_config(0.0, 1.0),
            &report,
            std::path::Path::new("/tmp/out/summary.json"),
        );
        assert!(
            message.contains("differs: true (p < 0.05)"),
            "got: {message}"
        );
    }
}
