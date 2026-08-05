// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for `isingmark::modes::comparative::run_comparative`.
//!
//! Mirrors `tests/mode_sweep.rs`'s/`tests/mode_ttt.rs`'s pattern: a
//! hermetic run against `isingmark-fake-miner` (via
//! `ComparativeConfig::backend_bin_override`) gives the whole comparative
//! loop — the eager output-dir pre-pass, per-iteration session dispatch,
//! the bracket-narrowing stop condition, and the binary search itself —
//! real coverage on a machine with no miner binaries installed.
//!
//! `isingmark-fake-miner`'s `multi_start_search` (see that binary's module
//! doc) is what makes the recovery test below possible at all: its energy
//! is provably non-increasing in a job's own `num_reads`, so a hardness
//! resolved higher via `comparative::DEFAULT_PARAM_CURVE` reliably produces
//! a same-or-better best-energy distribution. That property was designed
//! for this task, on the bundled "smoke" fixture specifically; it proves
//! this module's search machinery converges against a *model* built to
//! have exactly the property the search needs, not that the search
//! recovers hardness against a real annealer (whose energy-vs-reads
//! response could be noisier, or flatter, in ways this model cannot
//! exhibit).

use std::path::{Path, PathBuf};

use isingmark::backend::{builtin_registry, run_session, RunParams};
use isingmark::modes::comparative::{
    run_comparative, ComparativeConfig, DEFAULT_PARAM_CURVE, RECORDS_FILENAME, SUMMARY_FILENAME,
};
use isingmark::modes::{ModeError, TopologyRef};
use isingmark::problem::ProblemSource;
use isingmark::record::{append_jsonl, read_jsonl, JobRecord};
use isingmark::topology::load_fixture;

fn fake_miner_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner"))
}

/// A `ComparativeConfig` pointed at the hermetic fake miner, with every
/// field set to an explicit, distinctive value — callers override only
/// what a given test cares about via `..base()`.
fn base_config(output_dir: PathBuf, target_file: PathBuf) -> ComparativeConfig {
    ComparativeConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        target_file,
        num_jobs: 40,
        max_iterations: 7,
        seed: 42,
        output_dir,
        backend_bin_override: Some(fake_miner_bin()),
        backend_args_override: None,
    }
}

/// Build a reference JSONL file at a *known* hardness, entirely independent
/// of `run_comparative` (the function under test): resolves
/// `comparative::DEFAULT_PARAM_CURVE` at `hardness` directly (the same
/// public constant `run_comparative` itself resolves against, so this
/// helper's jobs are driven at the exact parameters a matching search
/// iteration would use) and runs one real session against the hermetic
/// fake miner, writing every resulting record to `path`.
async fn generate_reference_at_hardness(hardness: f64, num_jobs: u32, seed: u64, path: &Path) {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let backend = {
        let mut b = builtin_registry()
            .into_iter()
            .find(|b| b.name == "cpu-sa")
            .expect("cpu-sa is a built-in backend");
        b.bin = fake_miner_bin();
        b
    };
    let params = DEFAULT_PARAM_CURVE.resolve(backend.kind, hardness);

    let mut source = ProblemSource::new(&spec, [0u8; 32], seed, num_jobs, 0);
    let mut jobs = Vec::with_capacity(num_jobs as usize);
    while let Some(mut job) = source.next_job() {
        if let Some(ising) = job.ising.as_mut() {
            ising.num_reads = params.num_reads;
            ising.num_sweeps = params.num_sweeps;
            ising.anneal_time_us = params.anneal_time_us;
        }
        jobs.push(job);
    }

    let sock_path = path.with_extension("sock");
    let report = run_session(RunParams {
        backend: &backend,
        spec: &spec,
        jobs,
        sock_path: &sock_path,
        target: None,
    })
    .await
    .expect("reference session succeeds");

    std::fs::File::create(path).expect("create reference file");
    for record in &report.records {
        append_jsonl(path, record).expect("write reference record");
    }
}

fn iter_dir(output_dir: &Path, index: u32) -> PathBuf {
    output_dir.join(format!("iter_{index}"))
}

/// Step 1 of the brief: generate records at hardness 0.3, then confirm
/// `run_comparative` against that file recovers a hardness within 0.15 of
/// 0.3. `0.15` is generous enough to absorb ordinary evaluation-to-evaluation
/// noise (each iteration's mean is a 40-job sample; see
/// `comparative`'s own module doc for why the mean, not the median, is the
/// comparison statistic) without being so loose that a broken search (e.g.
/// one that always returns a fixed `0.5`, `|0.5 - 0.3| = 0.2 > 0.15`) would
/// pass.
#[tokio::test]
async fn hermetic_comparative_recovers_hardness_within_tolerance_of_the_reference() {
    let target_dir = tempfile::tempdir().expect("create temp dir");
    let target_path = target_dir.path().join("reference.jsonl");
    // Same seed/num_jobs as `base_config` below: the reference is built
    // from the identical problem draw a matching search iteration would
    // use, minimizing avoidable evaluation-to-evaluation noise (see this
    // module's own doc on the paired design comparative mode itself
    // relies on between its own iterations).
    generate_reference_at_hardness(0.3, 40, 42, &target_path).await;

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf(), target_path);

    let (records, hardness) = run_comparative(&cfg).await.expect("search succeeds");

    assert!(
        (hardness - 0.3).abs() <= 0.15,
        "expected hardness within 0.15 of 0.3, got {hardness}"
    );
    assert!(!records.is_empty(), "the final iteration must have records");
}

/// A records file left over from an earlier run must not be silently
/// appended to. Mirrors `tests/mode_sweep.rs`'s/`tests/mode_ttt.rs`'s
/// identical test.
#[tokio::test]
async fn hermetic_comparative_refuses_to_reuse_an_output_dir_with_existing_iter_records() {
    let target_dir = tempfile::tempdir().expect("create temp dir");
    let target_path = target_dir.path().join("reference.jsonl");
    generate_reference_at_hardness(0.3, 10, 7, &target_path).await;

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf(), target_path);

    run_comparative(&cfg).await.expect("first run succeeds");
    let second_run = run_comparative(&cfg).await;
    assert!(
        second_run.is_err(),
        "a second run into the same output dir must be refused"
    );
    if let Err(e) = second_run {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }
}

/// The output-dir conflict check must be eager — checked for every
/// possible iteration (`0..max_iterations`) before any of them run, not
/// lazy. Seeds a stale records file in the *last* possible iteration's
/// directory and nothing earlier (which does not even exist yet). A lazy
/// implementation would dispatch earlier iterations' real evaluations
/// before discovering the conflict; the decisive assertion is that
/// `iter_0` was never created at all.
#[tokio::test]
async fn hermetic_comparative_output_dir_conflict_check_is_eager_before_any_iteration_runs() {
    let target_dir = tempfile::tempdir().expect("create temp dir");
    let target_path = target_dir.path().join("reference.jsonl");
    generate_reference_at_hardness(0.3, 10, 7, &target_path).await;

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf(), target_path);

    let conflicting_dir = iter_dir(dir.path(), cfg.max_iterations - 1);
    std::fs::create_dir_all(&conflicting_dir).expect("create the conflicting iteration's dir");
    std::fs::write(conflicting_dir.join(RECORDS_FILENAME), "{\"stale\":true}\n")
        .expect("seed a stale records file in the last possible iteration only");

    let result = run_comparative(&cfg).await;
    assert!(
        result.is_err(),
        "a conflict on any possible iteration must refuse the whole search"
    );
    if let Err(e) = result {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }

    let first_iter_dir = iter_dir(dir.path(), 0);
    assert!(
        !first_iter_dir.exists(),
        "iter_0 must never be created when a later possible iteration's conflict is detected eagerly"
    );
}

/// The bracket-narrowing stop condition must actually stop the search
/// early: with a bracket starting at width `1.0` and a `0.01` stop
/// threshold, no more than `ceil(log2(1.0 / 0.01)) = 7` iterations can ever
/// be needed regardless of `max_iterations`. Setting `max_iterations` far
/// above that bound and counting the `iter_*` directories actually created
/// pins that the stop condition fires — a mutation that always runs
/// `max_iterations` iterations (dropping the bracket-width check) would
/// instead create 30.
#[tokio::test]
async fn hermetic_comparative_stops_once_the_bracket_narrows_below_the_threshold() {
    let target_dir = tempfile::tempdir().expect("create temp dir");
    let target_path = target_dir.path().join("reference.jsonl");
    generate_reference_at_hardness(0.3, 10, 7, &target_path).await;

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ComparativeConfig {
        max_iterations: 30,
        ..base_config(dir.path().to_path_buf(), target_path)
    };

    run_comparative(&cfg).await.expect("search succeeds");

    let created = (0..cfg.max_iterations)
        .filter(|&i| iter_dir(dir.path(), i).exists())
        .count();
    assert!(
        created <= 7,
        "the 0.01 bracket-width stop condition must cap iterations at 7 regardless of \
         max_iterations=30, got {created}"
    );
    assert!(created >= 1, "at least one iteration must always run");
}

/// `--max-iterations 0` (here, `ComparativeConfig::max_iterations`
/// constructed directly, bypassing `cli::parse_max_iterations`) must be
/// rejected rather than silently reporting a plausible-looking hardness of
/// `0.0` having run no evaluation at all.
#[tokio::test]
async fn hermetic_comparative_rejects_zero_max_iterations() {
    let target_dir = tempfile::tempdir().expect("create temp dir");
    let target_path = target_dir.path().join("reference.jsonl");
    generate_reference_at_hardness(0.3, 5, 7, &target_path).await;

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ComparativeConfig {
        max_iterations: 0,
        ..base_config(dir.path().to_path_buf(), target_path)
    };

    let result = run_comparative(&cfg).await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, ModeError::InvalidMaxIterations), "got: {e}");
    }
    assert!(
        std::fs::read_dir(dir.path()).map_or(true, |mut entries| entries.next().is_none()),
        "a rejected max_iterations must not create any iteration output"
    );
}

/// A missing target file must be reported as an actionable, specific error
/// naming the path — not a generic I/O error, and not a panic.
#[tokio::test]
async fn hermetic_comparative_rejects_a_missing_target_file() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(
        dir.path().to_path_buf(),
        dir.path().join("does-not-exist.jsonl"),
    );

    let result = run_comparative(&cfg).await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(
            matches!(e, ModeError::TargetFileUnreadable(_, _)),
            "got: {e}"
        );
        assert!(e.to_string().contains("does-not-exist.jsonl"));
    }
}

/// A target file with no usable (`passed`) records — here, only rejected
/// records — must be rejected rather than searching against an empty
/// distribution (`stats::ks_distance`/`stats::describe` would silently
/// return `None` for every iteration, making the search meaningless).
#[tokio::test]
async fn hermetic_comparative_rejects_a_target_file_with_no_passed_records() {
    let target_dir = tempfile::tempdir().expect("create temp dir");
    let target_path = target_dir.path().join("reference.jsonl");
    let rejected = JobRecord {
        job_id: "ab".to_string(),
        backend: "cpu-sa".to_string(),
        n_solutions: 0,
        best_energy_milli: 0,
        diversity_milli: 0,
        passed: false,
        rejected: true,
        reject_reason: Some("malformed".to_string()),
        device_access_time_us: 0,
        wall_ms: 0,
        reads: 0,
        sweeps: 0,
        anneal_time_us: 0,
    };
    append_jsonl(&target_path, &rejected).expect("write a rejected-only reference file");

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf(), target_path);

    let result = run_comparative(&cfg).await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, ModeError::EmptyTargetFile(_)), "got: {e}");
    }
}

/// `record::read_jsonl`'s `skipped_lines` (malformed lines in the reference
/// file) must be surfaced in `summary.json`'s `config`, not silently
/// dropped — the standing ruling that any task consuming records must carry
/// that count into what it reports.
#[tokio::test]
async fn hermetic_comparative_surfaces_the_target_files_own_skipped_lines() {
    let target_dir = tempfile::tempdir().expect("create temp dir");
    let target_path = target_dir.path().join("reference.jsonl");
    generate_reference_at_hardness(0.3, 5, 7, &target_path).await;
    // Append two malformed lines on top of the valid, passed records
    // `generate_reference_at_hardness` already wrote.
    let mut text = std::fs::read_to_string(&target_path).expect("read reference file");
    text.push_str("{not valid json\n");
    text.push_str("also not valid json\n");
    std::fs::write(&target_path, text).expect("append malformed lines");

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf(), target_path);

    run_comparative(&cfg).await.expect("search succeeds");

    let summary_text = std::fs::read_to_string(dir.path().join(SUMMARY_FILENAME))
        .expect("summary.json is readable");
    let summary: serde_json::Value =
        serde_json::from_str(&summary_text).expect("summary.json parses as JSON");
    assert_eq!(
        summary["config"]["target_file_skipped_lines"].as_u64(),
        Some(2),
        "the two malformed lines appended above must be counted, not silently dropped"
    );
}

/// Every iteration must replay the identical `(seed, num_jobs)` job
/// sequence — the paired design that makes different iterations'
/// evaluations comparable at all.
#[tokio::test]
async fn hermetic_comparative_every_iteration_replays_the_identical_job_sequence() {
    let target_dir = tempfile::tempdir().expect("create temp dir");
    let target_path = target_dir.path().join("reference.jsonl");
    generate_reference_at_hardness(0.3, 10, 7, &target_path).await;

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf(), target_path);

    run_comparative(&cfg).await.expect("search succeeds");

    let mut job_id_sets: Vec<std::collections::HashSet<String>> = Vec::new();
    for i in 0..cfg.max_iterations {
        let path = iter_dir(dir.path(), i).join(RECORDS_FILENAME);
        if !path.exists() {
            break;
        }
        let jsonl = read_jsonl(&path).expect("records.jsonl is readable");
        job_id_sets.push(jsonl.records.iter().map(|r| r.job_id.clone()).collect());
    }
    assert!(
        job_id_sets.len() >= 2,
        "at least two iterations must have run to compare"
    );

    let first = &job_id_sets[0];
    for (i, set) in job_id_sets.iter().enumerate().skip(1) {
        assert_eq!(
            set, first,
            "iteration {i} must draw the identical job_id set as iteration 0 (the paired design)"
        );
    }
}
