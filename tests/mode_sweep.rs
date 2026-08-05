// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for `isingmark::modes::sweep::run_sweep`.
//!
//! Mirrors `tests/mode_throughput.rs`'s pattern: a hermetic run against
//! `isingmark-fake-miner` (via `SweepConfig::backend_bin_override`) gives
//! the whole sweep loop — grid validation, per-cell job drawing, per-job
//! wire parameters, per-cell session dispatch, and per-cell/summary JSONL
//! writing — real coverage on a machine with no miner binaries installed.
//!
//! The `isingmark-fake-miner` binary was changed as part of this task
//! (`src/bin/isingmark-fake-miner.rs::solve`) to echo a job's own
//! `IsingProblem.num_reads`/`num_sweeps` into its `SamplerMeta` response
//! when the coordinator set them (nonzero) — without that, no hermetic
//! test here could tell "the harness put the right value on the wire" apart
//! from "the fake miner made something up regardless of what it received".

use std::collections::HashSet;
use std::path::PathBuf;

use isingmark::modes::sweep::{
    run_sweep, ParamGrid, SweepConfig, RECORDS_FILENAME, SUMMARY_FILENAME,
};
use isingmark::modes::{ModeError, TopologyRef};
use isingmark::record::{read_jsonl, Summary};

fn fake_miner_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner"))
}

/// A 2x2 grid: `num_reads` x `num_sweeps`, every value distinct so a
/// mixed-up dimension or a dropped Cartesian factor is visible in the
/// resulting records rather than masked by a coincidental match.
fn grid_2x2() -> ParamGrid {
    ParamGrid {
        num_reads: vec![8, 16],
        num_sweeps: vec![50, 100],
        anneal_time_us: vec![],
    }
}

/// A `SweepConfig` pointed at the hermetic fake miner, with every field set
/// to an explicit, distinctive value — callers override only what a given
/// test cares about via `..base()`.
fn base_config(output_dir: PathBuf) -> SweepConfig {
    SweepConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        grid: grid_2x2(),
        num_jobs: 3,
        seed: 42,
        output_dir,
        backend_bin_override: Some(fake_miner_bin()),
        backend_args_override: None,
    }
}

fn read_summary(output_dir: &std::path::Path) -> Summary {
    let text = std::fs::read_to_string(output_dir.join(SUMMARY_FILENAME))
        .expect("summary.json is readable");
    serde_json::from_str(&text).expect("summary.json parses as Summary")
}

/// The core hermetic assertion: a 2x2 grid produces 4 cells, each with its
/// own `records.jsonl` holding `num_jobs` records whose `reads`/`sweeps`
/// match *that cell's own* requested values — the assertion that proves
/// per-job parameters actually reached the wire (via the miner's own
/// `SamplerMeta` echo), not merely the harness's own bookkeeping. Also
/// checks the paired design: the same `job_id` set appears in every cell's
/// file, proving every cell drew the identical problem sequence rather than
/// a fresh, cell-specific one.
#[tokio::test]
async fn hermetic_sweep_produces_one_file_per_cell_with_matching_reads_and_sweeps() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let report = run_sweep(&cfg).await.expect("sweep run succeeds");
    assert_eq!(
        report.cells.len(),
        4,
        "a 2x2 grid must produce exactly 4 cells"
    );

    // Expected (num_reads, num_sweeps) pairs in Cartesian generation order:
    // outer loop over num_reads, inner loop over num_sweeps.
    let expected_params = [(8, 50), (8, 100), (16, 50), (16, 100)];
    let mut job_id_sets: Vec<HashSet<String>> = Vec::new();

    for (cell, &(expected_reads, expected_sweeps)) in report.cells.iter().zip(&expected_params) {
        assert_eq!(cell.num_reads, expected_reads);
        assert_eq!(cell.num_sweeps, expected_sweeps);
        assert_eq!(cell.anneal_time_us, 0);
        assert!(cell.report.handshake_ok);

        let cell_dir = dir.path().join(&cell.dir_name);
        let jsonl =
            read_jsonl(&cell_dir.join(RECORDS_FILENAME)).expect("records.jsonl is readable");
        assert_eq!(jsonl.skipped_lines, 0);
        assert_eq!(
            jsonl.records.len(),
            cfg.num_jobs as usize,
            "cell {} must have exactly num_jobs records",
            cell.dir_name
        );

        for record in &jsonl.records {
            assert!(record.passed, "cell {}: {record:?}", cell.dir_name);
            assert_eq!(
                record.reads, expected_reads,
                "cell {} record reads must match its own cell's requested value",
                cell.dir_name
            );
            assert_eq!(
                record.sweeps, expected_sweeps,
                "cell {} record sweeps must match its own cell's requested value",
                cell.dir_name
            );
        }

        job_id_sets.push(jsonl.records.iter().map(|r| r.job_id.clone()).collect());
    }

    let first = &job_id_sets[0];
    assert_eq!(
        first.len(),
        cfg.num_jobs as usize,
        "no duplicate ids within one cell"
    );
    for (i, set) in job_id_sets.iter().enumerate().skip(1) {
        assert_eq!(
            set, first,
            "cell {i} must draw the identical job_id set as cell 0 (the paired design)"
        );
    }
}

/// The top-level summary aggregates every cell's totals and lists each
/// cell's own directory/parameters, so a caller does not have to open every
/// cell's own file to know the shape of the run.
#[tokio::test]
async fn hermetic_sweep_summary_aggregates_totals_and_lists_every_cell() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let report = run_sweep(&cfg).await.expect("sweep run succeeds");
    let summary = read_summary(dir.path());

    assert_eq!(summary.mode, "sweep");
    assert_eq!(summary.backend, "cpu-sa");
    assert_eq!(summary.topology.name, "smoke");

    let expected_jobs = 4 * cfg.num_jobs as usize;
    assert_eq!(summary.totals.jobs, expected_jobs);
    assert_eq!(summary.totals.passed, expected_jobs);
    assert_eq!(summary.totals.rejected, 0);

    let cells = summary.config["cells"]
        .as_array()
        .expect("config carries a cells array");
    assert_eq!(cells.len(), 4);
    for (cell, cell_report) in cells.iter().zip(&report.cells) {
        assert_eq!(cell["dir"].as_str(), Some(cell_report.dir_name.as_str()));
        assert_eq!(
            cell["num_reads"].as_u64(),
            Some(u64::from(cell_report.num_reads))
        );
        assert_eq!(
            cell["num_sweeps"].as_u64(),
            Some(u64::from(cell_report.num_sweeps))
        );
        assert_eq!(cell["jobs"].as_u64(), Some(cfg.num_jobs.into()));
    }
}

/// An empty applicable dimension (here, `num_sweeps: []` for a `Sweeping`
/// backend) must be rejected rather than silently producing a zero-cell
/// sweep that leaves an empty output directory with no explanation.
#[tokio::test]
async fn hermetic_sweep_rejects_an_empty_grid() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = SweepConfig {
        grid: ParamGrid {
            num_reads: vec![8],
            num_sweeps: vec![],
            anneal_time_us: vec![],
        },
        ..base_config(dir.path().to_path_buf())
    };

    // `backend::RunReport` (embedded in `SweepReport::cells`) does not
    // implement `Debug`, so `Result::expect_err` (which requires the `Ok`
    // side to) is unavailable here; check the `Result` directly instead,
    // matching `tests/mode_throughput.rs`'s own workaround for the same
    // constraint.
    let result = run_sweep(&cfg).await;
    assert!(result.is_err(), "an empty dimension must be rejected");
    if let Err(e) = result {
        assert!(matches!(e, ModeError::EmptyGrid), "got: {e}");
    }
    assert!(
        std::fs::read_dir(dir.path()).map_or(true, |mut entries| entries.next().is_none()),
        "a rejected grid must not create any cell output"
    );
}

/// A grid dimension that does not apply to the resolved backend's
/// execution model (`anneal_time_us` for `cpu-sa`, a `Sweeping` backend)
/// must be rejected, not silently dropped.
#[tokio::test]
async fn hermetic_sweep_rejects_an_inapplicable_grid_dimension() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = SweepConfig {
        grid: ParamGrid {
            num_reads: vec![8],
            num_sweeps: vec![],
            anneal_time_us: vec![5],
        },
        ..base_config(dir.path().to_path_buf())
    };

    let result = run_sweep(&cfg).await;
    assert!(
        result.is_err(),
        "anneal_time_us must be rejected for a Sweeping backend"
    );
    if let Err(e) = result {
        assert!(
            matches!(e, ModeError::InapplicableGridDimension(_)),
            "got: {e}"
        );
    }
}

/// A records file left over from an earlier run must not be silently
/// appended to, for any cell — checked for every cell before any of them
/// run, so a conflict on a later cell cannot leave earlier cells'
/// (real, from-this-run) data sitting next to a run that was refused.
#[tokio::test]
async fn hermetic_sweep_refuses_to_reuse_an_output_dir_with_existing_cell_records() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let first = run_sweep(&cfg).await.expect("first run succeeds");
    let second = run_sweep(&cfg).await;
    assert!(
        second.is_err(),
        "a second run into the same output dir must be refused"
    );
    if let Err(e) = second {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }

    // The first run's data must survive the refused second attempt intact.
    for cell in &first.cells {
        let jsonl = read_jsonl(&dir.path().join(&cell.dir_name).join(RECORDS_FILENAME))
            .expect("first run's records survive");
        assert_eq!(jsonl.records.len(), cfg.num_jobs as usize);
    }
}

/// An `Annealing`-kind backend sweeps `num_reads`/`anneal_time_us`, and
/// `reads` (echoed via `SamplerMeta`) is still verifiable per cell.
/// `anneal_time_us` is *not* independently verified here: per this crate's
/// `backend::session::build_result_record`, `JobRecord::anneal_time_us`
/// always reflects the session-level `SetTarget` (this mode passes `None`,
/// so it is always `0`), never a per-job wire echo — see `modes::sweep`'s
/// module docs for why that is a known gap, not a bug in this test.
#[tokio::test]
async fn hermetic_sweep_supports_an_annealing_backend_and_reads_still_match_per_cell() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = SweepConfig {
        backend: "dwave".to_string(),
        grid: ParamGrid {
            num_reads: vec![5, 9],
            num_sweeps: vec![],
            anneal_time_us: vec![30, 60],
        },
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_sweep(&cfg).await.expect("sweep run succeeds");
    assert_eq!(report.cells.len(), 4);

    let expected_params = [(5, 30), (5, 60), (9, 30), (9, 60)];
    for (cell, &(expected_reads, expected_anneal)) in report.cells.iter().zip(&expected_params) {
        assert_eq!(cell.num_reads, expected_reads);
        assert_eq!(cell.num_sweeps, 0);
        assert_eq!(cell.anneal_time_us, expected_anneal);

        let jsonl = read_jsonl(&dir.path().join(&cell.dir_name).join(RECORDS_FILENAME))
            .expect("records.jsonl is readable");
        assert_eq!(jsonl.records.len(), cfg.num_jobs as usize);
        for record in &jsonl.records {
            assert_eq!(record.reads, expected_reads);
            assert_eq!(record.sweeps, 0);
        }
    }
}
