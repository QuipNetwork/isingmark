// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for `isingmark::modes::throughput::run_throughput`.
//!
//! Two independent paths, matching the pattern `tests/registry.rs` and
//! `tests/session_conformance.rs` established: a hermetic run against
//! `isingmark-fake-miner` (via `ThroughputConfig::backend_bin_override`)
//! that gives the whole throughput loop — job drawing, batching, repeated
//! sessions, JSONL/summary writing, and `jobs_per_min` accounting — real
//! coverage on a machine with no miner binaries installed, and a real
//! `quip-cpu-sa` run that FAILS by default when the binary is absent (set
//! `ISINGMARK_ALLOW_MISSING_MINERS=1` to opt out).

use std::path::PathBuf;

use isingmark::backend::builtin_registry;
use isingmark::modes::throughput::{
    run_throughput, ThroughputConfig, RECORDS_FILENAME, SUMMARY_FILENAME,
};
use isingmark::modes::TopologyRef;
use isingmark::record::{read_jsonl, Summary};

fn fake_miner_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner"))
}

/// Read back and typed-parse the summary a run wrote to `output_dir`.
fn read_summary(output_dir: &std::path::Path) -> Summary {
    let text = std::fs::read_to_string(output_dir.join(SUMMARY_FILENAME))
        .expect("summary.json is readable");
    serde_json::from_str(&text).expect("summary.json parses as Summary")
}

#[tokio::test]
async fn hermetic_throughput_run_against_the_fake_miner_produces_jsonl_and_summary() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        hardness: 0.5,
        duration_s: 1.5,
        seed: 42,
        output_dir: dir.path().to_path_buf(),
        backend_bin_override: Some(fake_miner_bin()),
    };

    let report = run_throughput(&cfg).await.expect("throughput run succeeds");

    assert!(report.handshake_ok);
    assert!(
        !report.records.is_empty(),
        "a 1.5s run against a fast fake miner must complete at least one job"
    );
    assert!(report.run_wall_ms > 0);
    assert!(report.records.iter().all(|r| r.passed && !r.rejected));

    let jsonl = read_jsonl(&dir.path().join(RECORDS_FILENAME)).expect("records.jsonl is readable");
    assert_eq!(jsonl.skipped_lines, 0);
    assert_eq!(
        jsonl.records.len(),
        report.records.len(),
        "every record in the aggregated report must also be on disk"
    );

    let summary = read_summary(dir.path());
    assert_eq!(summary.mode, "throughput");
    assert_eq!(summary.backend, "cpu-sa");
    assert_eq!(summary.topology.name, "smoke");
    assert_eq!(summary.topology.num_nodes, 8);
    assert_eq!(summary.topology.num_edges, 8);
    assert_eq!(summary.totals.jobs, report.records.len());
    assert_eq!(summary.totals.passed, report.records.len());
    assert_eq!(summary.totals.rejected, 0);

    let jobs_per_min = summary.config["jobs_per_min"]
        .as_f64()
        .expect("config carries a numeric jobs_per_min");
    assert!(
        jobs_per_min > 0.0,
        "jobs_per_min must be positive, got {jobs_per_min}"
    );
}

/// `jobs_per_min` must be derived from measured wall time
/// (`RunReport::run_wall_ms`), never from the requested `duration_s` — the
/// two are only close by coincidence and can diverge sharply, as forced
/// here: a 1ms requested duration is far shorter than any real batch can
/// finish in, so the loop's own "dispatch at least one full batch before
/// checking elapsed time" behavior guarantees the measured wall time comes
/// out far larger than what was requested.
#[expect(
    clippy::cast_precision_loss,
    reason = "job counts and millisecond wall times in a single test run are far below f64's \
              52-bit exact-integer range"
)]
#[tokio::test]
async fn hermetic_throughput_summary_jobs_per_min_uses_measured_wall_time_not_requested_duration() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        hardness: 0.5,
        duration_s: 0.001,
        seed: 5,
        output_dir: dir.path().to_path_buf(),
        backend_bin_override: Some(fake_miner_bin()),
    };

    let report = run_throughput(&cfg).await.expect("throughput run succeeds");
    assert!(!report.records.is_empty());
    assert!(
        report.run_wall_ms > 1,
        "a real batch must take measurably longer than the 1ms requested duration, got {}ms",
        report.run_wall_ms
    );

    let summary = read_summary(dir.path());
    let expected = report.records.len() as f64 / (report.run_wall_ms as f64 / 60_000.0);
    let actual = summary.config["jobs_per_min"]
        .as_f64()
        .expect("config carries a numeric jobs_per_min");
    assert!(
        (actual - expected).abs() < 1e-6,
        "jobs_per_min must be derived from measured wall time ({} ms), got {actual} \
         (expected {expected} from report.run_wall_ms; a mutation that divides by the \
         requested duration instead would produce a very different value)",
        report.run_wall_ms
    );
}

#[tokio::test]
async fn hermetic_throughput_run_draws_distinct_deterministic_jobs_across_batches() {
    // BATCH_SIZE is 8; a longer run than the happy-path test above forces
    // at least a second session/batch, exercising the loop itself (not
    // just a single iteration) and proving job draws do not repeat or
    // reset across batches.
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        hardness: 0.2,
        duration_s: 2.5,
        seed: 99,
        output_dir: dir.path().to_path_buf(),
        backend_bin_override: Some(fake_miner_bin()),
    };

    let report = run_throughput(&cfg).await.expect("throughput run succeeds");

    assert!(
        report.records.len() > 8,
        "a 2.5s run must span more than one 8-job batch, got {} records",
        report.records.len()
    );
    let unique_ids: std::collections::HashSet<&str> =
        report.records.iter().map(|r| r.job_id.as_str()).collect();
    assert_eq!(
        unique_ids.len(),
        report.records.len(),
        "every dispatched job must be distinct across batches"
    );
}

/// Environment variable that opts out of the real-binary requirement below,
/// matching `tests/registry.rs`'s and `tests/session_conformance.rs`'s
/// `ALLOW_MISSING_MINERS_VAR`.
const ALLOW_MISSING_MINERS_VAR: &str = "ISINGMARK_ALLOW_MISSING_MINERS";

/// Drives the real `quip-cpu-sa` binary against `fixtures/smoke.spec.json`
/// for 5 seconds. FAILS by default when the binary is absent (most
/// development and CI machines today); set
/// `ISINGMARK_ALLOW_MISSING_MINERS=1` to opt out. The hermetic tests above
/// give the throughput loop itself real coverage regardless of this
/// variable.
#[tokio::test]
async fn quip_cpu_sa_throughput_conformance() {
    let registry = builtin_registry();
    let backend = registry
        .into_iter()
        .find(|b| b.name == "cpu-sa")
        .expect("builtin_registry always has a cpu-sa entry");

    if !backend.bin.is_file() {
        assert!(
            std::env::var_os(ALLOW_MISSING_MINERS_VAR).is_some(),
            "quip-cpu-sa was not found on $PATH (looked for {}). This test \
             requires the real miner binary by default; set \
             {ALLOW_MISSING_MINERS_VAR}=1 to opt out on a machine without \
             the miner binaries installed.",
            backend.bin.display()
        );
        return;
    }

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        hardness: 0.5,
        duration_s: 5.0,
        seed: 7,
        output_dir: dir.path().to_path_buf(),
        backend_bin_override: None,
    };

    let report = run_throughput(&cfg)
        .await
        .expect("throughput run against the real quip-cpu-sa binary succeeds");

    assert!(report.handshake_ok);
    assert!(!report.records.is_empty());

    let jsonl = read_jsonl(&dir.path().join(RECORDS_FILENAME)).expect("records.jsonl is readable");
    assert_eq!(jsonl.records.len(), report.records.len());

    let summary = read_summary(dir.path());
    assert_eq!(summary.totals.jobs, report.records.len());
    let jobs_per_min = summary.config["jobs_per_min"]
        .as_f64()
        .expect("config carries a numeric jobs_per_min");
    assert!(
        jobs_per_min > 0.0,
        "jobs_per_min must be positive, got {jobs_per_min}"
    );
}
