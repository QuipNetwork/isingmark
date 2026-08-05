// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for `isingmark::modes::scaling::run_scaling`.
//!
//! Mirrors `tests/mode_sweep.rs`'s/`tests/mode_ttt.rs`'s pattern: a
//! hermetic run against `isingmark-fake-miner` (via
//! `ScalingConfig::backend_bin_override`) gives the whole scaling loop —
//! size validation, per-size topology generation, per-size session
//! dispatch, and per-size/summary JSONL writing — real coverage on a
//! machine with no miner binaries installed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use isingmark::modes::scaling::{run_scaling, ScalingConfig, RECORDS_FILENAME, SUMMARY_FILENAME};
use isingmark::modes::ModeError;
use isingmark::record::read_jsonl;

fn fake_miner_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner"))
}

/// A `ScalingConfig` pointed at the hermetic fake miner, with every field
/// set to an explicit, distinctive value — callers override only what a
/// given test cares about via `..base()`.
fn base_config(output_dir: PathBuf) -> ScalingConfig {
    ScalingConfig {
        backend: "cpu-sa".to_string(),
        sizes: vec![8, 16],
        hardness: 0.5,
        num_jobs: 3,
        seed: 42,
        output_dir,
        backend_bin_override: Some(fake_miner_bin()),
        backend_args_override: None,
    }
}

fn size_dir(output_dir: &Path, nodes: usize) -> PathBuf {
    output_dir.join(format!("size_{nodes}"))
}

fn read_summary(output_dir: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(output_dir.join(SUMMARY_FILENAME))
        .expect("summary.json is readable");
    serde_json::from_str(&text).expect("summary.json parses as JSON")
}

/// The core hermetic assertion (step 1 of the brief): `--sizes "8,16"`
/// yields two groups, each with its own `records.jsonl` holding `num_jobs`
/// records, and each group's own generated topology actually has the
/// requested node count — the decisive check against a bug that shares one
/// topology across every requested size instead of generating one per size
/// (mutation (d) in the task brief).
#[tokio::test]
async fn hermetic_scaling_produces_one_group_per_size_with_the_right_node_count() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let report = run_scaling(&cfg).await.expect("scaling run succeeds");

    assert_eq!(report.sizes.len(), 2);
    for (outcome, &expected_nodes) in report.sizes.iter().zip(&cfg.sizes) {
        assert_eq!(outcome.nodes, expected_nodes);
        let jsonl = read_jsonl(&size_dir(dir.path(), expected_nodes).join(RECORDS_FILENAME))
            .expect("records.jsonl is readable");
        assert_eq!(jsonl.records.len(), cfg.num_jobs as usize);
    }

    let summary = read_summary(dir.path());
    let sizes_json = summary["config"]["sizes"]
        .as_array()
        .expect("config carries a sizes array");
    assert_eq!(sizes_json.len(), 2);
    let reported_nodes: Vec<u64> = sizes_json
        .iter()
        .map(|s| s["nodes"].as_u64().expect("nodes is present"))
        .collect();
    assert_eq!(reported_nodes, vec![8, 16]);
}

/// Each size's own generated topology must actually have the requested
/// node count and be 3-regular (`edges == 3n/2`), reflected in the summary
/// — not just in `run_scaling`'s in-memory return value, so an operator
/// reading `summary.json` alone can verify this too.
#[tokio::test]
async fn hermetic_scaling_summary_reports_the_right_edge_count_per_size() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ScalingConfig {
        sizes: vec![4, 8],
        ..base_config(dir.path().to_path_buf())
    };

    run_scaling(&cfg).await.expect("scaling run succeeds");

    let summary = read_summary(dir.path());
    let sizes_json = summary["config"]["sizes"].as_array().expect("sizes array");
    for entry in sizes_json {
        let nodes = entry["nodes"].as_u64().expect("nodes present");
        let edges = entry["edges"].as_u64().expect("edges present");
        assert_eq!(edges, 3 * nodes / 2, "3-regular graph must have 3n/2 edges");
    }
}

/// Two different requested sizes must generate two different topologies
/// (different node counts, and — since they are randomly generated —
/// different `topology_hash`es), never the same one reused. Mirrors
/// `tests/mode_ttt.rs`'s equivalent "every target replays the identical job
/// sequence" test in spirit, but for the opposite property: unlike ttt's
/// targets (which *must* share one topology), scaling's sizes must *not*
/// share one.
#[tokio::test]
async fn hermetic_scaling_generates_distinct_topologies_per_size() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let report = run_scaling(&cfg).await.expect("scaling run succeeds");

    let hashes: HashSet<&str> = report
        .sizes
        .iter()
        .map(|o| o.topology_hash.as_str())
        .collect();
    assert_eq!(
        hashes.len(),
        report.sizes.len(),
        "every size must generate its own distinct topology hash"
    );
}

/// A records file left over from an earlier run must not be silently
/// appended to, for any size — checked for every size before any of them
/// run, so a conflict on a later size cannot leave earlier sizes' real
/// output sitting next to a run that was refused. Mirrors
/// `tests/mode_ttt.rs`'s identical test.
#[tokio::test]
async fn hermetic_scaling_refuses_to_reuse_an_output_dir_with_existing_size_records() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    run_scaling(&cfg).await.expect("first run succeeds");
    let second_run = run_scaling(&cfg).await;
    assert!(
        second_run.is_err(),
        "a second run into the same output dir must be refused"
    );
    if let Err(e) = second_run {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }

    let jsonl = read_jsonl(&size_dir(dir.path(), 8).join(RECORDS_FILENAME))
        .expect("first run's records survive");
    assert_eq!(jsonl.records.len(), cfg.num_jobs as usize);
}

/// The output-dir conflict check must be eager — checked for *every* size
/// before *any* size is dispatched — not lazy. Seeds a stale records file
/// in only the *second* (out of two) size's directory and nothing in the
/// first size's directory (which does not even exist yet). A lazy
/// implementation would dispatch the first size's real jobs before
/// discovering the conflict on the second, leaving one good size sitting
/// next to a refused run with no `summary.json`. The decisive assertion is
/// that the first size's directory was never created at all.
#[tokio::test]
async fn hermetic_scaling_output_dir_conflict_check_is_eager_before_any_size_runs() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let conflicting_dir = size_dir(dir.path(), 16);
    std::fs::create_dir_all(&conflicting_dir).expect("create the conflicting size's dir");
    std::fs::write(conflicting_dir.join(RECORDS_FILENAME), "{\"stale\":true}\n")
        .expect("seed a stale records file in the second size only");

    let result = run_scaling(&cfg).await;
    assert!(
        result.is_err(),
        "a conflict on either size must refuse the whole run"
    );
    if let Err(e) = result {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }

    let first_size_dir = size_dir(dir.path(), 8);
    assert!(
        !first_size_dir.exists(),
        "the first size must never be created when the second size's conflict is detected eagerly"
    );
}

/// An empty size list must be rejected rather than silently producing a run
/// with no output.
#[tokio::test]
async fn hermetic_scaling_rejects_an_empty_size_list() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ScalingConfig {
        sizes: vec![],
        ..base_config(dir.path().to_path_buf())
    };

    let result = run_scaling(&cfg).await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, ModeError::EmptySizes), "got: {e}");
    }
}

/// An odd size, or a size below 4, cannot support a 3-regular graph and
/// must be rejected — for a caller that constructs a `ScalingConfig`
/// directly, bypassing the CLI's own `parse_one_scaling_size` rejection.
#[tokio::test]
async fn hermetic_scaling_rejects_a_size_that_cannot_be_3_regular() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ScalingConfig {
        sizes: vec![8, 7],
        ..base_config(dir.path().to_path_buf())
    };

    let result = run_scaling(&cfg).await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, ModeError::InvalidSize(7)), "got: {e}");
    }

    let dir2 = tempfile::tempdir().expect("create temp dir");
    let cfg2 = ScalingConfig {
        sizes: vec![3],
        ..base_config(dir2.path().to_path_buf())
    };
    let result2 = run_scaling(&cfg2).await;
    assert!(result2.is_err());
    if let Err(e) = result2 {
        assert!(matches!(e, ModeError::InvalidSize(3)), "got: {e}");
    }
}

/// Two sizes that repeat the same node count must be rejected: they would
/// collide on the same output subdirectory, and the second size's
/// `File::create` would silently truncate the first size's records.
#[tokio::test]
async fn hermetic_scaling_rejects_a_duplicate_size() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ScalingConfig {
        sizes: vec![8, 16, 8],
        ..base_config(dir.path().to_path_buf())
    };

    let result = run_scaling(&cfg).await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, ModeError::DuplicateSize(8)), "got: {e}");
    }
    assert!(
        std::fs::read_dir(dir.path()).map_or(true, |mut entries| entries.next().is_none()),
        "a rejected size list must not create any size output"
    );
}

/// Rerunning `run_scaling` with the identical `--sizes`/`--seed` against a
/// fresh output directory must reproduce the identical topology — the
/// determinism guarantee this mode's whole "comparable across runs" premise
/// depends on.
#[tokio::test]
async fn hermetic_scaling_is_deterministic_across_separate_runs() {
    let dir_a = tempfile::tempdir().expect("create temp dir");
    let cfg_a = base_config(dir_a.path().to_path_buf());
    let report_a = run_scaling(&cfg_a).await.expect("first run succeeds");

    let dir_b = tempfile::tempdir().expect("create temp dir");
    let cfg_b = base_config(dir_b.path().to_path_buf());
    let report_b = run_scaling(&cfg_b).await.expect("second run succeeds");

    let hashes_a: Vec<&str> = report_a
        .sizes
        .iter()
        .map(|o| o.topology_hash.as_str())
        .collect();
    let hashes_b: Vec<&str> = report_b
        .sizes
        .iter()
        .map(|o| o.topology_hash.as_str())
        .collect();
    assert_eq!(
        hashes_a, hashes_b,
        "the same seed and sizes must reproduce the identical topology hashes"
    );
}

/// Fix-round Minor 4: the top-level `summary.json` `topology` block (which
/// has no single applicable topology — scaling generates one per size) must
/// report a meaningful aggregate, not `nodes: 0, edges: 0, hash: ""`, which
/// reads as a broken run to an operator skimming only that block. Checks
/// the aggregate is both nonzero/non-empty *and* the actual sum over every
/// size's own real node/edge counts (`config.sizes`), not a coincidental
/// nonzero placeholder.
#[tokio::test]
async fn hermetic_scaling_top_level_topology_block_is_a_real_aggregate_not_zeros() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    run_scaling(&cfg).await.expect("scaling run succeeds");
    let summary = read_summary(dir.path());

    let top_nodes = summary["topology"]["num_nodes"].as_u64().expect("present");
    let top_edges = summary["topology"]["num_edges"].as_u64().expect("present");
    let top_hash = summary["topology"]["topology_hash"]
        .as_str()
        .expect("present");

    assert_ne!(
        top_nodes, 0,
        "top-level num_nodes must not be a zero placeholder"
    );
    assert_ne!(
        top_edges, 0,
        "top-level num_edges must not be a zero placeholder"
    );
    assert!(
        !top_hash.is_empty(),
        "top-level topology_hash must not be an empty placeholder"
    );

    let sizes_json = summary["config"]["sizes"].as_array().expect("sizes array");
    let expected_nodes: u64 = sizes_json
        .iter()
        .map(|s| s["nodes"].as_u64().expect("nodes present"))
        .sum();
    let expected_edges: u64 = sizes_json
        .iter()
        .map(|s| s["edges"].as_u64().expect("edges present"))
        .sum();
    assert_eq!(
        top_nodes, expected_nodes,
        "top-level num_nodes must be the real sum over every size's own generated node count"
    );
    assert_eq!(
        top_edges, expected_edges,
        "top-level num_edges must be the real sum over every size's own generated edge count"
    );
}
