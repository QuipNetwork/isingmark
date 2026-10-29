// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for `isingmark::modes::throughput::run_throughput`.
//!
//! Two independent paths, matching the pattern `tests/registry.rs` and
//! `tests/session_conformance.rs` established: a hermetic run against
//! `isingmark-fake-miner` (via `ThroughputConfig::backend_bin_override`/
//! `backend_args_override`) that gives the whole throughput loop — job
//! drawing, batching, repeated sessions, JSONL/summary writing, and
//! `jobs_per_min` accounting — real coverage on a machine with no miner
//! binaries installed, and a real `quip-cpu-sa` run that FAILS by default
//! when the binary is absent (set `ISINGMARK_ALLOW_MISSING_MINERS=1` to
//! opt out).

use std::collections::HashSet;
use std::path::PathBuf;

use isingmark::backend::builtin_registry;
use isingmark::modes::throughput::{
    run_throughput, ThroughputConfig, DEFAULT_PARAM_CURVE, RECORDS_FILENAME, SUMMARY_FILENAME,
};
use isingmark::modes::TopologyRef;
use isingmark::record::{read_jsonl, Summary};

fn fake_miner_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner"))
}

/// A `ThroughputConfig` pointed at the hermetic fake miner, with every
/// field set to an explicit, distinctive value rather than a type default —
/// callers override only what a given test cares about via `..base()`.
fn base_config(output_dir: PathBuf) -> ThroughputConfig {
    ThroughputConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        hardness: 0.5,
        param_curve: DEFAULT_PARAM_CURVE,
        duration_s: 1.2,
        seed: 42,
        corpus: None,
        instances: None,
        output_dir,
        backend_bin_override: Some(fake_miner_bin()),
        backend_args_override: None,
    }
}

/// Read back and typed-parse the summary a run wrote to `output_dir`.
fn read_summary(output_dir: &std::path::Path) -> Summary {
    let text = std::fs::read_to_string(output_dir.join(SUMMARY_FILENAME))
        .expect("summary.json is readable");
    serde_json::from_str(&text).expect("summary.json parses as Summary")
}

#[test]
fn throughput_cli_fails_only_when_every_job_is_rejected_and_keeps_evidence() {
    use std::os::unix::fs::PermissionsExt;

    for (miner_args, jobs_per_batch, rejected_per_batch, passed_per_batch, success) in [
        ("--disconnect-after 1", 1, 1, 0, false),
        ("--reject-job 2", 8, 1, 7, true),
        ("--energy-lie-milli 1000", 8, 0, 0, true),
    ] {
        let dir = tempfile::tempdir().expect("create temp dir");
        let miner = dir.path().join("quip-cpu-sa");
        std::fs::write(
            &miner,
            format!("#!/bin/sh\nexec \"$ISINGMARK_TEST_MINER\" {miner_args} \"$@\"\n"),
        )
        .expect("write fake miner wrapper");
        std::fs::set_permissions(&miner, std::fs::Permissions::from_mode(0o755))
            .expect("make fake miner wrapper executable");
        let output_dir = dir.path().join("output");
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_isingmark"))
            .args([
                "throughput",
                "--backend",
                "cpu-sa",
                "--topology-preset",
                "smoke",
                "--duration",
                "0.001s",
                "--output-dir",
            ])
            .arg(&output_dir)
            .env("PATH", dir.path())
            .env("ISINGMARK_TEST_MINER", fake_miner_bin())
            .output()
            .expect("run throughput command");

        assert!(
            output_dir.join(SUMMARY_FILENAME).is_file(),
            "throughput must save its summary: {output:?}"
        );
        let summary = read_summary(&output_dir);
        // The duration can admit more than one complete batch.
        let batches = summary.totals.jobs / jobs_per_batch;
        assert!(batches > 0, "{miner_args}");
        assert_eq!(
            summary.totals.jobs,
            jobs_per_batch * batches,
            "{miner_args}"
        );
        assert_eq!(
            summary.totals.rejected,
            rejected_per_batch * batches,
            "{miner_args}"
        );
        assert_eq!(
            summary.totals.passed,
            passed_per_batch * batches,
            "{miner_args}"
        );
        let records = read_jsonl(&output_dir.join(RECORDS_FILENAME)).expect("read records");
        assert_eq!(records.skipped_lines, 0);
        assert_eq!(records.records.len(), summary.totals.jobs);
        assert_eq!(
            output.status.success(),
            success,
            "wrong throughput exit for {miner_args}: {output:?}"
        );
    }
}

#[tokio::test]
async fn hermetic_throughput_run_against_the_fake_miner_produces_jsonl_and_summary() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let report = run_throughput(&cfg).await.expect("throughput run succeeds");

    assert!(report.handshake_ok);
    assert!(
        !report.records.is_empty(),
        "a 1.2s run against a fast fake miner must complete at least one job"
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
        duration_s: 0.001,
        seed: 5,
        ..base_config(dir.path().to_path_buf())
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

    // The measured wall time must also be surfaced next to the requested
    // duration, so a truncated run is distinguishable from a complete one.
    let measured_wall_ms = summary.config["measured_wall_ms"]
        .as_u64()
        .expect("config carries a numeric measured_wall_ms");
    assert_eq!(measured_wall_ms, report.run_wall_ms);
    assert!(
        (summary.config["duration_s"].as_f64().unwrap_or(-1.0) - 0.001).abs() < 1e-9,
        "the requested duration_s must still be reported, unmodified, alongside the measured one"
    );
}

#[tokio::test]
async fn hermetic_throughput_run_draws_distinct_deterministic_jobs_across_batches() {
    // BATCH_SIZE is 8; a batch here measures ~0.3-0.4s, so 1.2s comfortably
    // forces at least a second session/batch (with margin) without adding
    // more wall time to the suite than needed.
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        hardness: 0.2,
        seed: 99,
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_throughput(&cfg).await.expect("throughput run succeeds");

    assert!(
        report.records.len() > 8,
        "a 1.2s run must span more than one 8-job batch, got {} records",
        report.records.len()
    );
    let unique_ids: HashSet<&str> = report.records.iter().map(|r| r.job_id.as_str()).collect();
    assert_eq!(
        unique_ids.len(),
        report.records.len(),
        "every dispatched job must be distinct across batches"
    );
}

/// `--hardness` must reach `ParamCurve::resolve` and the resolved
/// `JobParams` must reach the wire `SetTarget` — pins the exact resolved
/// values via `summary.config`, since a mutation that drops `cfg.hardness`
/// on the way to `resolve` (verified separately to slip past every other
/// test) still leaves `summary.config["hardness"]` reporting the requested
/// value even though every job silently ran at the curve minimum.
#[tokio::test]
async fn hermetic_throughput_summary_config_reports_resolved_params_for_the_requested_hardness() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        hardness: 0.5,
        seed: 21,
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_throughput(&cfg).await.expect("throughput run succeeds");
    assert!(!report.records.is_empty());

    let summary = read_summary(dir.path());
    // hardness 0.5 on the Sweeping DEFAULT_PARAM_CURVE (8..64 reads,
    // 100..1000 sweeps) resolves to exactly these values.
    assert_eq!(summary.config["num_reads"], 36);
    assert_eq!(summary.config["num_sweeps"], 550);
    assert!((summary.config["hardness"].as_f64().unwrap_or(-1.0) - 0.5).abs() < f64::EPSILON);
}

/// Two runs that differ only in `--seed` must draw disjoint job sets.
/// Same-seed determinism alone (already covered by `problem::ProblemSource`'s
/// own tests) cannot catch a bug that swaps the seed and deadline
/// arguments passed to `ProblemSource::new` — that swap is itself
/// deterministic, so it needs a *different-seed* comparison to surface.
#[tokio::test]
async fn hermetic_throughput_different_seeds_draw_disjoint_job_sets() {
    let dir_a = tempfile::tempdir().expect("create temp dir");
    let dir_b = tempfile::tempdir().expect("create temp dir");
    let cfg_a = ThroughputConfig {
        seed: 111,
        ..base_config(dir_a.path().to_path_buf())
    };
    let cfg_b = ThroughputConfig {
        seed: 222,
        ..base_config(dir_b.path().to_path_buf())
    };

    let (report_a, report_b) = tokio::join!(run_throughput(&cfg_a), run_throughput(&cfg_b));
    let report_a = report_a.expect("run a succeeds");
    let report_b = report_b.expect("run b succeeds");

    let ids_a: HashSet<&str> = report_a.records.iter().map(|r| r.job_id.as_str()).collect();
    let ids_b: HashSet<&str> = report_b.records.iter().map(|r| r.job_id.as_str()).collect();
    assert!(!ids_a.is_empty());
    assert!(!ids_b.is_empty());
    assert!(
        ids_a.is_disjoint(&ids_b),
        "different seeds must draw different jobs: {ids_a:?} vs {ids_b:?}"
    );
}

/// A records file left over from an earlier run into the same
/// `--output-dir` must not be silently appended to: two runs with an
/// identical config into one directory would otherwise leave duplicate,
/// byte-identical records on disk while the summary still claims only one
/// run's worth of jobs.
#[tokio::test]
async fn hermetic_throughput_refuses_to_reuse_an_output_dir_with_existing_records() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    run_throughput(&cfg).await.expect("first run succeeds");
    // `backend::session::RunReport` does not implement `Debug`, so
    // `expect_err` (which requires `Ok`'s type to) is not available here;
    // check the `Result` directly instead.
    let second_run = run_throughput(&cfg).await;
    assert!(
        second_run.is_err(),
        "a second run into the same output dir must be refused"
    );
    if let Err(e) = second_run {
        assert!(e.to_string().contains("records"), "message: {e}");
    }

    // The first run's data must survive the refused second attempt intact.
    let jsonl = read_jsonl(&dir.path().join(RECORDS_FILENAME)).expect("records.jsonl is readable");
    assert!(!jsonl.records.is_empty());
}

/// Uses `backend_args_override` to drive `isingmark-fake-miner`'s
/// `--stderr-and-exit`, which exits before ever sending `Hello`. Proves two
/// things at once: `backend_args_override` actually reaches the spawned
/// process (the testability gap flagged in review), and the resulting
/// miner-failure state (`miner_exit_code`, `miner_stderr_tail`) reaches
/// both the `RunReport` and `summary.config` rather than being silently
/// discarded — so a truncated/failed run is distinguishable from a
/// complete one without needing the raw `RunReport`.
#[tokio::test]
async fn hermetic_throughput_surfaces_miner_failure_state_when_it_never_handshakes() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        backend_args_override: Some(vec![
            "--stderr-and-exit".to_string(),
            "isingmark: distinctive throughput failure marker 9d21".to_string(),
        ]),
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_throughput(&cfg)
        .await
        .expect("run_throughput itself does not error on a miner that never handshakes");

    assert!(!report.handshake_ok, "the miner never sent Hello");
    assert!(report.records.is_empty());
    assert_ne!(report.miner_exit_code, 0);
    let tail = report.miner_stderr_tail.as_deref().unwrap_or("");
    assert!(tail.contains("distinctive throughput failure marker 9d21"));

    let summary = read_summary(dir.path());
    assert_eq!(summary.totals.jobs, 0);
    assert_eq!(summary.config["miner_exit_code"], report.miner_exit_code);
    assert_eq!(summary.config["miner_unresponsive"], false);
    let stderr_tail_in_summary = summary.config["miner_stderr_tail"].as_str().unwrap_or("");
    assert!(stderr_tail_in_summary.contains("distinctive throughput failure marker 9d21"));
}

/// Uses `backend_args_override`'s `--reject-job` to force a genuine mix of
/// passed and rejected records in one run, so `Totals.passed`/`rejected`
/// are pinned against real, distinguishable outcomes rather than the
/// all-pass case every other test exercises (which cannot tell
/// `passed: agg.records.len()` apart from a real filter).
#[tokio::test]
async fn hermetic_throughput_summary_totals_distinguish_passed_from_rejected() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        backend_args_override: Some(vec!["--reject-job".to_string(), "2".to_string()]),
        seed: 6,
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_throughput(&cfg).await.expect("throughput run succeeds");
    assert!(
        report.records.len() >= 2,
        "need at least the rejected 2nd job and one more"
    );

    let summary = read_summary(dir.path());
    assert_eq!(summary.totals.jobs, report.records.len());
    assert_eq!(
        summary.totals.passed + summary.totals.rejected,
        summary.totals.jobs,
        "every record here is either passed or rejected, none both/neither"
    );
    assert!(
        summary.totals.rejected >= 1,
        "the --reject-job miner must produce at least one rejection"
    );
    assert!(
        summary.totals.passed >= 1,
        "and at least one job must still pass around it"
    );
}

/// The smoke fixture's 8 nodes and 8 edges cannot distinguish a
/// `num_nodes`/`num_edges` swap in `build_summary`. A topology with
/// distinct counts can.
#[tokio::test]
async fn hermetic_throughput_summary_topology_counts_are_independently_correct() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let topo_path = dir.path().join("asymmetric.spec.json");
    std::fs::write(
        &topo_path,
        r#"{"nodes": [0, 1, 2, 3, 4], "edges": [[0, 1], [1, 2], [2, 3]], "allowed_h_milli": [0], "allowed_j_milli": [0]}"#,
    )
    .expect("write asymmetric topology fixture");

    let cfg = ThroughputConfig {
        topology: TopologyRef::File(topo_path),
        seed: 4,
        ..base_config(dir.path().join("out"))
    };

    let report = run_throughput(&cfg).await.expect("throughput run succeeds");
    assert!(!report.records.is_empty());

    let summary = read_summary(&dir.path().join("out"));
    assert_eq!(summary.topology.num_nodes, 5);
    assert_eq!(summary.topology.num_edges, 3);
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
        duration_s: 5.0,
        seed: 7,
        backend_bin_override: None,
        ..base_config(dir.path().to_path_buf())
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

/// Two nonces from the bundled `chain-h0` corpus. Used with the `smoke`
/// topology through a file path, which skips the bundled-name pairing check
/// so the wiring can be tested without drawing a 4577-node problem per job.
const CORPUS_NONCE_A: &str = "0cf67b0f85e37ddd3dda2ac11685c7ad857d069eda939ae3f2d6d93e3d5b8864";
const CORPUS_NONCE_B: &str = "22681ca9153f4f30a0d5af9da319e2b93848c182a7584a541d52db2214a08d54";

fn write_corpus(dir: &std::path::Path, nonces: &[&str]) -> PathBuf {
    use std::fmt::Write as _;

    let path = dir.join("corpus.jsonl");
    let text = nonces.iter().fold(String::new(), |mut out, n| {
        let _ = writeln!(out, "{{\"nonce\":\"{n}\"}}");
        out
    });
    std::fs::write(&path, text).expect("write corpus");
    path
}

/// A corpus run must dispatch exactly the instances the corpus names, and
/// nothing else. The seed is left at its base value to prove it no longer
/// drives which problems are drawn.
#[tokio::test]
async fn hermetic_throughput_corpus_run_dispatches_exactly_the_corpus_instances() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let corpus = write_corpus(dir.path(), &[CORPUS_NONCE_A, CORPUS_NONCE_B]);
    let cfg = ThroughputConfig {
        // Far longer than the corpus can fill, so the run can only end by
        // exhausting the corpus.
        duration_s: 60.0,
        corpus: Some(corpus.display().to_string()),
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_throughput(&cfg).await.expect("corpus run succeeds");

    assert!(report.handshake_ok);
    let ids: Vec<&str> = report.records.iter().map(|r| r.job_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![CORPUS_NONCE_A, CORPUS_NONCE_B],
        "a corpus run must replay its instances once each, in file order"
    );
    assert!(
        report.run_wall_ms < 60_000,
        "the run must end on corpus exhaustion, not on the 60s duration"
    );
}

/// The summary has to say which problems a run saw. Without this, a corpus
/// run and a seeded run at the same topology are indistinguishable on disk.
#[tokio::test]
async fn hermetic_throughput_corpus_run_records_its_source_in_the_summary() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let corpus = write_corpus(dir.path(), &[CORPUS_NONCE_A, CORPUS_NONCE_B]);
    let cfg = ThroughputConfig {
        duration_s: 60.0,
        corpus: Some(corpus.display().to_string()),
        ..base_config(dir.path().to_path_buf())
    };

    let _ = run_throughput(&cfg).await.expect("corpus run succeeds");

    let summary = read_summary(dir.path());
    assert_eq!(
        summary.config["corpus"].as_str(),
        Some(corpus.display().to_string().as_str())
    );
    assert_eq!(summary.config["corpus_instances"].as_u64(), Some(2));
}

fn smoke_instances_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("corpora")
        .join("structured")
        .join("smoke-ice-glass.jsonl")
}

/// An instance-file run must dispatch exactly the file's instances, once
/// each in file order, and the summary must say where they came from.
#[tokio::test]
async fn hermetic_throughput_instances_run_replays_the_file_and_records_its_source() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = smoke_instances_path();
    let cfg = ThroughputConfig {
        duration_s: 60.0,
        instances: Some(path.clone()),
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_throughput(&cfg).await.expect("instances run succeeds");

    let ids: Vec<&str> = report.records.iter().map(|r| r.job_id.as_str()).collect();
    assert_eq!(ids, vec![format!("{:0>64}", "1"), format!("{:0>64}", "2")]);
    assert!(
        report.run_wall_ms < 60_000,
        "the run must end on exhausting the file, not on the 60s duration"
    );
    let summary = read_summary(dir.path());
    assert!(summary.config["corpus"].is_null());
    assert_eq!(
        summary.config["job_source"].as_str(),
        Some(format!("instances:{}", path.display()).as_str())
    );
    assert_eq!(summary.config["corpus_instances"].as_u64(), Some(2));
}

/// Setting both finite sources through the library API fails before any job
/// runs.
#[tokio::test]
async fn hermetic_throughput_rejects_a_corpus_and_instances_together() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        corpus: Some("chain-h0".to_string()),
        instances: Some(smoke_instances_path()),
        ..base_config(dir.path().to_path_buf())
    };
    let msg = run_throughput(&cfg)
        .await
        .err()
        .map_or_else(String::new, |e| e.to_string());
    assert!(msg.contains("mutually exclusive"), "{msg}");
}

/// An instance file built for another topology fails before any job runs.
#[tokio::test]
async fn hermetic_throughput_rejects_an_instance_file_on_the_wrong_topology() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        topology: TopologyRef::Preset("chain-ternary".to_string()),
        instances: Some(smoke_instances_path()),
        ..base_config(dir.path().to_path_buf())
    };
    let msg = run_throughput(&cfg)
        .await
        .err()
        .map_or_else(String::new, |e| e.to_string());
    assert!(msg.contains("--topology-preset"), "{msg}");
}

/// A seeded run must keep reporting a null corpus, so an older summary and a
/// new one stay comparable.
#[tokio::test]
async fn hermetic_throughput_seeded_run_reports_no_corpus() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    let _ = run_throughput(&cfg).await.expect("seeded run succeeds");

    let summary = read_summary(dir.path());
    assert!(summary.config["corpus"].is_null());
    assert_eq!(summary.config["job_source"].as_str(), Some("seed:42"));
    assert!(summary.config["corpus_instances"].is_null());
}

/// A bundled corpus paired with a topology it was not drawn against must
/// fail before any job runs. Replaying it would produce valid-looking
/// energies for problems the chain never posed.
#[tokio::test]
async fn hermetic_throughput_rejects_a_bundled_corpus_on_the_wrong_topology() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        corpus: Some("chain-h0".to_string()),
        ..base_config(dir.path().to_path_buf())
    };

    let msg = run_throughput(&cfg)
        .await
        .err()
        .map_or_else(String::new, |e| e.to_string());
    assert!(
        !msg.is_empty(),
        "a mismatched corpus pairing must be rejected, but the run succeeded"
    );
    assert!(msg.contains("chain-h0"), "{msg}");
    assert!(msg.contains("--topology-preset"), "{msg}");
}

/// An unknown corpus name must name the bundled ones rather than failing as
/// a missing file.
#[tokio::test]
async fn hermetic_throughput_unknown_corpus_name_lists_the_bundled_corpora() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        corpus: Some("no-such-corpus".to_string()),
        ..base_config(dir.path().to_path_buf())
    };

    let msg = run_throughput(&cfg)
        .await
        .err()
        .map_or_else(String::new, |e| e.to_string());
    assert!(
        !msg.is_empty(),
        "an unknown corpus must be rejected, but the run succeeded"
    );
    assert!(
        msg.contains("chain-h0") && msg.contains("chain-ternary"),
        "{msg}"
    );
}

/// The bundled corpus against the 4577-node topology it is paired with,
/// through the whole dispatch loop. The other corpus tests use the `smoke`
/// topology for speed, so this is the only one that proves a real chain
/// instance draws and dispatches at its real size.
#[tokio::test]
async fn hermetic_throughput_replays_the_bundled_chain_h0_corpus() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = ThroughputConfig {
        topology: TopologyRef::Preset("chain-h0".to_string()),
        corpus: Some("chain-h0".to_string()),
        duration_s: 0.001,
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_throughput(&cfg).await.expect("corpus run succeeds");

    assert!(report.handshake_ok);
    assert!(!report.records.is_empty());
    let summary = read_summary(dir.path());
    assert_eq!(summary.topology.name, "chain-h0");
    assert_eq!(summary.topology.num_nodes, 4577);
    assert_eq!(summary.config["corpus_instances"].as_u64(), Some(4658));

    // Every dispatched job_id must be a real nonce from the corpus file.
    let corpus = std::fs::read_to_string("corpora/chain-h0.jsonl").expect("corpus is readable");
    for record in &report.records {
        assert!(
            corpus.contains(&record.job_id),
            "job_id {} is not a nonce in the bundled corpus",
            record.job_id
        );
    }
}
