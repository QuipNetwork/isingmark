// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for
//! `isingmark::modes::reproducibility::run_reproducibility`.
//!
//! Mirrors `tests/mode_comparative.rs`'s pattern: a hermetic run against
//! `isingmark-fake-miner` (via `ReproConfig::backend_bin_override`) gives
//! the whole reproducibility loop — the eager output-dir pre-pass, both
//! runs' session dispatch, and the two-sample KS comparison — real coverage
//! on a machine with no miner binaries installed.
//!
//! # Bracketing `ReproReport::differs` from both directions
//!
//! `isingmark-fake-miner` is fully deterministic (see
//! `modes::reproducibility`'s own module doc), so two runs at the *same*
//! hardness against it always produce byte-identical energies — a stub that
//! hardcodes `differs: false` would pass a test that only checks that
//! direction. This file also drives the two runs at deliberately different
//! hardness (`hardness_a: 0.0`, `hardness_b: 1.0`) against the identical
//! problem sequence, which — via `isingmark-fake-miner`'s
//! `multi_start_search` being provably non-increasing in `num_reads` (the
//! same property `tests/mode_comparative.rs` relies on for its own recovery
//! tests) — produces two genuinely different, deterministically reproducible
//! distributions. A stub that hardcodes `differs: true` would fail the
//! identical-hardness test above; a stub that hardcodes `differs: false`
//! would fail this one. Together they bracket the computation from both
//! directions.

use std::path::{Path, PathBuf};

use isingmark::modes::reproducibility::{
    run_reproducibility, ReproConfig, RECORDS_FILENAME, RUN_A_DIR, RUN_B_DIR, SUMMARY_FILENAME,
};
use isingmark::modes::{ModeError, TopologyRef};
use isingmark::record::read_jsonl;

fn fake_miner_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner"))
}

/// A `ReproConfig` pointed at the hermetic fake miner, with every field set
/// to an explicit, distinctive value — callers override only what a given
/// test cares about via `..base()`. `hardness_a == hardness_b` here: the
/// ordinary "is this backend reproducible" use case.
fn base_config(output_dir: PathBuf) -> ReproConfig {
    ReproConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        num_problems: 40,
        hardness_a: 0.5,
        hardness_b: 0.5,
        seed: 42,
        output_dir,
        backend_bin_override: Some(fake_miner_bin()),
        backend_args_override: None,
    }
}

fn read_summary(output_dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(output_dir.join(SUMMARY_FILENAME))
        .expect("summary.json is readable");
    serde_json::from_str(&text).expect("summary.json parses as JSON")
}

/// Step 1 of the brief: two runs at the identical hardness against a fully
/// deterministic backend must report `differs == false`, with `ks_statistic
/// == 0.0` and `p_value == 1.0` (the exact values two byte-identical
/// distributions produce — see `stats::ks_distance`'s own
/// `ks_distance_of_a_sample_against_itself_is_exactly_zero` test).
#[tokio::test]
async fn hermetic_reproducibility_identical_runs_report_differs_false() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let report = run_reproducibility(&cfg).await.expect("run succeeds");

    assert!(!report.differs, "identical hardness runs must not differ");
    assert!(
        (report.ks_statistic - 0.0).abs() < 1e-12,
        "byte-identical distributions must have ks_statistic exactly 0.0, got {}",
        report.ks_statistic
    );
    assert!(
        (report.p_value - 1.0).abs() < 1e-12,
        "byte-identical distributions must have p_value exactly 1.0, got {}",
        report.p_value
    );

    let run_a = read_jsonl(&dir.path().join(RUN_A_DIR).join(RECORDS_FILENAME))
        .expect("run_a records.jsonl is readable");
    let run_b = read_jsonl(&dir.path().join(RUN_B_DIR).join(RECORDS_FILENAME))
        .expect("run_b records.jsonl is readable");
    assert_eq!(run_a.records.len(), cfg.num_problems as usize);
    assert_eq!(run_b.records.len(), cfg.num_problems as usize);

    let energies_a: Vec<i64> = run_a.records.iter().map(|r| r.best_energy_milli).collect();
    let energies_b: Vec<i64> = run_b.records.iter().map(|r| r.best_energy_milli).collect();
    assert_eq!(
        energies_a, energies_b,
        "identical hardness against a deterministic backend must produce byte-identical energies"
    );
}

/// The mirror-image direction (see this file's own module doc): two runs at
/// deliberately different hardness, replaying the identical problem
/// sequence, must report `differs == true` with `p_value < 0.05`. This is
/// the test a `differs`-hardcoded-`false` stub, or one that never runs the
/// second batch at all, cannot pass.
#[tokio::test]
async fn hermetic_reproducibility_genuinely_different_hardness_reports_differs_true() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ReproConfig {
        hardness_a: 0.0,
        hardness_b: 1.0,
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_reproducibility(&cfg).await.expect("run succeeds");

    assert!(
        report.differs,
        "genuinely different sampling effort against the same problems must be detected as \
         differing, got ks_statistic={} p_value={}",
        report.ks_statistic, report.p_value
    );
    assert!(
        report.p_value < 0.05,
        "differs == true must correspond to p_value < 0.05, got {}",
        report.p_value
    );
    assert!(
        report.ks_statistic > 0.0,
        "genuinely different distributions must have a nonzero ks_statistic, got {}",
        report.ks_statistic
    );
}

/// Both runs must replay the identical `(seed, num_problems)` job sequence
/// — the paired design that makes the two runs' distributions comparable at
/// all (see this module's own doc). Checked independently of whether the
/// two hardness values match: even the "genuinely different" case above
/// must still be driven from the same underlying problems, or a difference
/// could be attributed to which problems each run happened to draw instead
/// of the sampling effort.
#[tokio::test]
async fn hermetic_reproducibility_both_runs_replay_the_identical_job_sequence() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ReproConfig {
        hardness_a: 0.0,
        hardness_b: 1.0,
        ..base_config(dir.path().to_path_buf())
    };

    run_reproducibility(&cfg).await.expect("run succeeds");

    let run_a = read_jsonl(&dir.path().join(RUN_A_DIR).join(RECORDS_FILENAME))
        .expect("run_a records.jsonl is readable");
    let run_b = read_jsonl(&dir.path().join(RUN_B_DIR).join(RECORDS_FILENAME))
        .expect("run_b records.jsonl is readable");

    let ids_a: std::collections::HashSet<String> =
        run_a.records.iter().map(|r| r.job_id.clone()).collect();
    let ids_b: std::collections::HashSet<String> =
        run_b.records.iter().map(|r| r.job_id.clone()).collect();
    assert_eq!(
        ids_a.len(),
        cfg.num_problems as usize,
        "no duplicate ids within run_a"
    );
    assert_eq!(
        ids_a, ids_b,
        "both runs must draw the identical job_id set (the paired design)"
    );
}

/// A records file left over from an earlier run must not be silently
/// appended to, for either run — checked for both before either runs, so a
/// conflict on `run_b` cannot leave `run_a`'s real output sitting next to a run
/// that was refused.
#[tokio::test]
async fn hermetic_reproducibility_refuses_to_reuse_an_output_dir_with_existing_records() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    run_reproducibility(&cfg).await.expect("first run succeeds");
    let second_run = run_reproducibility(&cfg).await;
    assert!(
        second_run.is_err(),
        "a second run into the same output dir must be refused"
    );
    if let Err(e) = second_run {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }

    let run_a = read_jsonl(&dir.path().join(RUN_A_DIR).join(RECORDS_FILENAME))
        .expect("first run's run_a records survive");
    assert_eq!(run_a.records.len(), cfg.num_problems as usize);
}

/// The output-dir conflict check must be eager — checked for *both*
/// directories before *either* run is dispatched — not lazy. Seeds a stale
/// records file in only `run_b`'s directory and nothing in `run_a`'s (which
/// does not even exist yet). A lazy implementation would dispatch `run_a`'s
/// real jobs before discovering the conflict on `run_b`, leaving one good run
/// sitting next to a refused comparison with no `summary.json`. The
/// decisive assertion is that `run_a`'s directory was never created at all.
#[tokio::test]
async fn hermetic_reproducibility_output_dir_conflict_check_is_eager_before_either_run() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let conflicting_dir = dir.path().join(RUN_B_DIR);
    std::fs::create_dir_all(&conflicting_dir).expect("create run_b's dir");
    std::fs::write(conflicting_dir.join(RECORDS_FILENAME), "{\"stale\":true}\n")
        .expect("seed a stale records file in run_b only");

    let result = run_reproducibility(&cfg).await;
    assert!(
        result.is_err(),
        "a conflict on either run must refuse the whole comparison"
    );
    if let Err(e) = result {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }

    let run_a_dir = dir.path().join(RUN_A_DIR);
    assert!(
        !run_a_dir.exists(),
        "run_a must never be created when run_b's conflict is detected eagerly"
    );
}

/// `summary.json` must carry the computed `ks_statistic`/`p_value`/`differs`
/// alongside both runs' own directory/parameter detail, so an operator
/// reading the file alone (not just `ReproReport`) gets the full picture.
#[tokio::test]
async fn hermetic_reproducibility_summary_carries_the_comparison_outcome() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ReproConfig {
        hardness_a: 0.0,
        hardness_b: 1.0,
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_reproducibility(&cfg).await.expect("run succeeds");
    let summary = read_summary(dir.path());

    assert!(
        (summary["config"]["ks_statistic"].as_f64().expect("present") - report.ks_statistic).abs()
            < 1e-12
    );
    assert!(
        (summary["config"]["p_value"].as_f64().expect("present") - report.p_value).abs() < 1e-12
    );
    assert_eq!(
        summary["config"]["differs"].as_bool().expect("present"),
        report.differs
    );
    assert_eq!(
        summary["config"]["run_a"]["dir"].as_str().expect("present"),
        RUN_A_DIR
    );
    assert_eq!(
        summary["config"]["run_b"]["dir"].as_str().expect("present"),
        RUN_B_DIR
    );
}
