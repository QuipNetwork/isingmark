// SPDX-License-Identifier: AGPL-3.0-or-later
//! Conformance tests for `isingmark::backend::session::run_session`.
//!
//! Two independent paths, mirroring the pattern `tests/registry.rs`
//! established: a hermetic fake miner (`isingmark-fake-miner`, a test-only
//! `[[bin]]` target auto-discovered from `src/bin/isingmark-fake-miner.rs`)
//! that speaks the real `MinerService.Session` wire protocol and can
//! misbehave on demand, giving the session loop's
//! handshake/dispatch/result/reject/fatal/disconnect branches real coverage
//! on a machine with no miner binaries installed; and a real `quip-cpu-sa`
//! conformance test that FAILS by default when the binary is absent (set
//! `ISINGMARK_ALLOW_MISSING_MINERS=1` to opt out).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use isingmark::backend::{builtin_registry, run_session, BackendSpec, RunParams, RunReport};
use isingmark::params::BackendKind;
use isingmark::problem::ProblemSource;
use isingmark::record::JobRecord;
use isingmark::topology::{load_fixture, load_topology, TopologySpec};
use quip_proto::v1::{Job, SetTarget};
use quip_protocol::scoring::{energy_milli, set_diversity};
use quip_protocol::wire::decode_i32_le;

/// Draw `count` deterministic jobs against `spec`. `deadline_ms: 0` means
/// "no deadline" on the wire; a small nonzero constant (as
/// `tests/problem_golden.rs` uses for its own unrelated golden-order check)
/// would already be a past-epoch value to a real miner's wall clock and get
/// every job rejected as expired.
fn draw_jobs(spec: &TopologySpec, seed: u64, count: u32) -> Vec<Job> {
    let mut src = ProblemSource::new(spec, [7u8; 32], seed, count, 0);
    let mut jobs = Vec::new();
    while let Some(job) = src.next_job() {
        jobs.push(job);
    }
    jobs
}

/// A hand-built topology whose node ids (`10, 20, 30, 40`) are sparse and,
/// crucially, larger than the node count: a bug that indexes
/// `energy_milli`'s edges by raw node id instead of position would push
/// every edge past `spins.len() == 4`, silently dropping every coupling
/// term instead of computing it. `fixtures/smoke.spec.json`'s `0..7` ids
/// cannot distinguish "position" from "raw id" at all, since they are the
/// same numbers.
fn sparse_ids_spec() -> TopologySpec {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("sparse.spec.json");
    std::fs::write(
        &path,
        r#"{
            "nodes": [10, 20, 30, 40],
            "edges": [[10, 20], [20, 30], [30, 40], [40, 10]],
            "allowed_h_milli": [-1000, 0, 1000],
            "allowed_j_milli": [-1000, 1000]
        }"#,
    )
    .expect("write sparse fixture");
    load_topology(&path).expect("sparse fixture parses")
}

/// A small, fast `SetTarget` used by every test so a real annealing/SA
/// backend does not spend more wall time per job than a conformance test
/// needs. `anneal_time_us` is deliberately non-zero: a stub that hardcodes
/// `JobRecord::anneal_time_us` to `0` would be indistinguishable from a
/// correct read of a `0`-valued target.
fn small_target() -> SetTarget {
    SetTarget {
        max_energy_milli: i64::MAX,
        min_solutions: 0,
        min_diversity_milli: 0,
        num_reads: 8,
        num_sweeps: 50,
        anneal_time_us: 30,
    }
}

fn fake_miner_backend(args: Vec<String>) -> BackendSpec {
    BackendSpec {
        name: "fake".to_string(),
        bin: PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner")),
        args,
        kind: BackendKind::Sweeping,
    }
}

async fn run_with(
    backend: &BackendSpec,
    spec: &TopologySpec,
    jobs: Vec<Job>,
    sock_path: &Path,
) -> RunReport {
    run_session(RunParams {
        backend,
        spec,
        jobs,
        sock_path,
        target: Some(small_target()),
    })
    .await
    .expect("run_session's own setup (bind/spawn) never fails for a valid fixture backend")
}

/// Independently recompute the energy `spins` would score for `job` against
/// `spec`'s edges. Derived from scratch here (its own node-position
/// resolution, not `session.rs`'s private `edge_positions`) so this is a
/// real check of `run_session`'s recompute output, not a tautology that
/// would still pass if that logic silently dropped the `/ 1000.0`
/// milli-unit conversion, mixed up edge endpoints, or indexed by raw node
/// id instead of position.
fn expected_energy_for_spins(spec: &TopologySpec, job: &Job, spins: &[i8]) -> i64 {
    let pos: HashMap<u32, usize> = spec
        .nodes
        .iter()
        .enumerate()
        .map(|(i, &n)| (n, i))
        .collect();
    let edges: Vec<(usize, usize)> = spec
        .edges
        .iter()
        .map(|&(u, v)| (pos[&u], pos[&v]))
        .collect();
    let ising = job
        .ising
        .as_ref()
        .expect("drawn jobs always carry an ising problem");
    let h: Vec<f64> = decode_i32_le(&ising.h_milli_le32)
        .expect("h decodes")
        .iter()
        .map(|&v| f64::from(v) / 1000.0)
        .collect();
    let j: Vec<f64> = decode_i32_le(&ising.j_milli_le32)
        .expect("j decodes")
        .iter()
        .map(|&v| f64::from(v) / 1000.0)
        .collect();
    energy_milli(spins, &h, &j, &edges)
}

/// All-`+1` spins, sized to `job`'s own node count.
fn all_plus_one(job: &Job) -> Vec<i8> {
    let ising = job
        .ising
        .as_ref()
        .expect("drawn jobs always carry an ising problem");
    let n = decode_i32_le(&ising.h_milli_le32).expect("h decodes").len();
    flipped_spins(n, 0)
}

fn expected_all_plus_one_energy(spec: &TopologySpec, job: &Job) -> i64 {
    expected_energy_for_spins(spec, job, &all_plus_one(job))
}

/// `isingmark-fake-miner --num-solutions`'s documented spin-pattern
/// contract: flip the first `flips` node spins negative (capped at `n`),
/// leaving the rest at `+1`. Reimplemented here independently of
/// `src/bin/isingmark-fake-miner.rs` (a separate process this test cannot
/// call into directly) so the test derives its expectation from the
/// documented contract, not from reading the binary's own source.
fn flipped_spins(n: usize, flips: usize) -> Vec<i8> {
    let mut spins = vec![1i8; n];
    for spin in spins.iter_mut().take(flips.min(n)) {
        *spin = -1;
    }
    spins
}

/// Independently compute the milli-scaled diversity `solutions` should
/// score, calling the same upstream `set_diversity` `session.rs` calls (that
/// function's own correctness is upstream's responsibility, already tested
/// at that crate's level) but rounding to milli-units with a rounding
/// expression reimplemented here rather than calling `session.rs`'s private
/// `milli_round`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "set_diversity returns a value in [0.0, 1.0]; scaled by 1000 and rounded it fits u32"
)]
fn expected_diversity_milli(solutions: &[Vec<i8>]) -> u32 {
    (set_diversity(solutions) * 1000.0).round() as u32
}

fn record_by_job_id<'a>(records: &'a [JobRecord], job: &Job) -> &'a JobRecord {
    let job_id = job.job_id.iter().fold(String::new(), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    });
    records
        .iter()
        .find(|r| r.job_id == job_id)
        .expect("a record exists for every job_id this test dispatched")
}

#[tokio::test]
async fn fake_miner_happy_path_solves_every_job_and_recomputes_matching_energy() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 1, 4);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec![]);

    let report = run_with(&backend, &spec, jobs.clone(), &sock_path).await;

    assert!(report.handshake_ok);
    assert_eq!(report.records.len(), 4);
    for job in &jobs {
        let record = record_by_job_id(&report.records, job);
        assert_eq!(record.n_solutions, 1);
        assert!(
            record.passed,
            "record for {} should pass: {record:?}",
            record.job_id
        );
        assert!(!record.rejected);
        assert_eq!(record.reject_reason, None);
        assert_eq!(
            record.best_energy_milli,
            expected_all_plus_one_energy(&spec, job)
        );
        assert_eq!(record.reads, 1, "fake miner reports 1 read via SamplerMeta");
        assert_eq!(record.sweeps, 0);
        assert_eq!(
            record.anneal_time_us, 30,
            "sourced from the requested SetTarget"
        );
    }
    assert_eq!(report.miner_exit_code, 0);
    assert!(
        !sock_path.exists(),
        "coordinator socket must be removed on every exit path"
    );
}

#[tokio::test]
async fn fake_miner_energy_lie_is_caught_by_local_recompute() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 2, 1);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec!["--energy-lie-milli".to_string(), "500".to_string()]);

    let report = run_with(&backend, &spec, jobs.clone(), &sock_path).await;

    assert!(report.handshake_ok);
    assert_eq!(report.records.len(), 1);
    let record = &report.records[0];
    // The recorded energy is the coordinator's own recompute of the true
    // value, never the miner's lied-about report: this is exactly the
    // "never trust a miner-reported energy_milli" contract under test.
    assert_eq!(
        record.best_energy_milli,
        expected_all_plus_one_energy(&spec, &jobs[0])
    );
    assert!(
        !record.passed,
        "a reported/recomputed energy mismatch must clear passed"
    );
    assert!(
        !record.rejected,
        "a mismatch is not the same outcome as an explicit Reject"
    );
    assert_eq!(report.miner_exit_code, 0);
}

#[tokio::test]
async fn fake_miner_best_of_n_selects_the_minimum_energy_and_matches_diversity() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 8, 1);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec!["--num-solutions".to_string(), "3".to_string()]);

    let report = run_with(&backend, &spec, jobs.clone(), &sock_path).await;

    assert!(report.handshake_ok);
    assert_eq!(report.records.len(), 1);
    let record = &report.records[0];
    assert_eq!(record.n_solutions, 3);
    assert_eq!(
        record.reads, 3,
        "fake miner reports one read per solution via SamplerMeta"
    );

    let n = all_plus_one(&jobs[0]).len();
    let expected_solutions: Vec<Vec<i8>> = (0..3usize).map(|i| flipped_spins(n, i)).collect();
    let expected_energies: Vec<i64> = expected_solutions
        .iter()
        .map(|spins| expected_energy_for_spins(&spec, &jobs[0], spins))
        .collect();
    assert!(
        expected_energies.iter().collect::<HashSet<_>>().len() > 1,
        "fixture/seed must produce distinct energies across solutions, or \
         .min()/.max()/.next()/.last() are indistinguishable here: {expected_energies:?}"
    );
    let expected_best = *expected_energies
        .iter()
        .min()
        .expect("3 energies were computed above");
    assert_eq!(
        record.best_energy_milli, expected_best,
        "best-of-N must select the minimum recomputed energy, not the first/last/maximum"
    );

    let expected_diversity = expected_diversity_milli(&expected_solutions);
    assert!(
        expected_diversity > 0,
        "distinct spin patterns must produce non-zero diversity in this independent computation"
    );
    assert_eq!(record.diversity_milli, expected_diversity);
    assert!(record.passed);
}

#[tokio::test]
async fn fake_miner_sparse_node_ids_topology_resolves_edges_by_position() {
    let spec = sparse_ids_spec();
    let jobs = draw_jobs(&spec, 9, 2);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec![]);

    let report = run_with(&backend, &spec, jobs.clone(), &sock_path).await;

    assert!(report.handshake_ok);
    assert_eq!(report.records.len(), 2);
    for job in &jobs {
        let record = record_by_job_id(&report.records, job);
        assert!(
            record.passed,
            "record for {} should pass: {record:?}",
            record.job_id
        );
        assert_eq!(
            record.best_energy_milli,
            expected_all_plus_one_energy(&spec, job),
            "a raw-id-instead-of-position bug would push every edge past \
             spins.len() == 4 and silently skip every coupling term"
        );
    }
}

#[tokio::test]
async fn fake_miner_reject_is_recorded_with_reason_and_does_not_affect_other_jobs() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 3, 3);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    // credits=1 dispatches sequentially, so "reject the 2nd job this miner
    // receives" lands deterministically on jobs[1].
    let backend = fake_miner_backend(vec!["--reject-job".to_string(), "2".to_string()]);

    let report = run_with(&backend, &spec, jobs.clone(), &sock_path).await;

    assert!(report.handshake_ok);
    assert_eq!(report.records.len(), 3);

    let rejected = record_by_job_id(&report.records, &jobs[1]);
    assert!(rejected.rejected);
    assert!(!rejected.passed);
    assert_eq!(rejected.reject_reason.as_deref(), Some("MALFORMED"));
    assert_eq!(rejected.n_solutions, 0);

    for job in [&jobs[0], &jobs[2]] {
        let record = record_by_job_id(&report.records, job);
        assert!(
            record.passed,
            "unaffected job should still pass: {record:?}"
        );
        assert!(!record.rejected);
    }
    assert_eq!(report.miner_exit_code, 0);
}

#[tokio::test]
async fn fake_miner_fatal_mid_flight_fails_every_in_flight_job_and_reports_exit_code() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 4, 4);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    // Request all 4 credits up front so jobs 2-4 are all in flight when the
    // miner sends Fatal instead of handling job #2, exercising "every
    // in-flight job" rather than just the one job Fatal interrupts.
    let backend = fake_miner_backend(vec![
        "--credits".to_string(),
        "4".to_string(),
        "--fatal-after".to_string(),
        "2".to_string(),
    ]);

    let report = run_with(&backend, &spec, jobs.clone(), &sock_path).await;

    assert!(report.handshake_ok);
    assert_eq!(report.records.len(), 4);

    let solved = record_by_job_id(&report.records, &jobs[0]);
    assert!(solved.passed);
    assert!(!solved.rejected);

    for job in &jobs[1..] {
        let record = record_by_job_id(&report.records, job);
        assert!(
            record.rejected,
            "job abandoned by Fatal must be recorded as failing: {record:?}"
        );
        assert!(!record.passed);
        let reason = record.reject_reason.as_deref().unwrap_or("");
        assert!(
            reason.starts_with("fatal:"),
            "reason should be tagged as fatal, got {reason:?}"
        );
    }
    // The fake miner's own `main` maps a synthetic Fatal to exit 70
    // (quip_protocol::session::ExitCode::InternalFatal).
    assert_eq!(report.miner_exit_code, 70);
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn fake_miner_disconnect_mid_flight_fails_every_in_flight_job() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 12, 4);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    // Same batching as the Fatal test, but the miner closes the connection
    // with no message at all for job #2 (simulating a crash/OOM-kill)
    // instead of sending an orderly Fatal report.
    let backend = fake_miner_backend(vec![
        "--credits".to_string(),
        "4".to_string(),
        "--disconnect-after".to_string(),
        "2".to_string(),
    ]);

    let report = run_with(&backend, &spec, jobs.clone(), &sock_path).await;

    assert!(report.handshake_ok);
    assert_eq!(report.records.len(), 4);

    let solved = record_by_job_id(&report.records, &jobs[0]);
    assert!(solved.passed);
    assert!(!solved.rejected);

    for job in &jobs[1..] {
        let record = record_by_job_id(&report.records, job);
        assert!(
            record.rejected,
            "job abandoned by a mid-flight disconnect must be recorded as failing, \
             not silently dropped: {record:?}"
        );
        assert!(!record.passed);
        assert_eq!(
            record.reject_reason.as_deref(),
            Some("session ended with job in flight")
        );
    }
    // A silent disconnect is not a Fatal report, so the fake miner's own
    // `main` still returns success.
    assert_eq!(report.miner_exit_code, 0);
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn fake_miner_bad_protocol_version_fails_handshake_and_dispatches_nothing() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 5, 2);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec!["--protocol-version".to_string(), "2".to_string()]);

    let report = run_with(&backend, &spec, jobs, &sock_path).await;

    assert!(!report.handshake_ok);
    assert!(report.records.is_empty());
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn fake_miner_skip_hello_fails_handshake_on_unexpected_first_message() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 11, 1);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    // Distinct from a closed stream: this sends a real, well-formed message
    // (Ready) that simply is not the expected first one (Hello).
    let backend = fake_miner_backend(vec!["--skip-hello".to_string()]);

    let report = run_with(&backend, &spec, jobs, &sock_path).await;

    assert!(!report.handshake_ok);
    assert!(report.records.is_empty());
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn fake_miner_zero_jobs_shuts_down_immediately_after_handshake() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec![]);

    let report = run_with(&backend, &spec, Vec::new(), &sock_path).await;

    assert!(report.handshake_ok);
    assert!(report.records.is_empty());
    assert_eq!(report.miner_exit_code, 0);
    assert!(!sock_path.exists());
}

/// Environment variable that opts out of the real-binary requirement below,
/// matching `tests/registry.rs`'s `ALLOW_MISSING_MINERS_VAR`.
const ALLOW_MISSING_MINERS_VAR: &str = "ISINGMARK_ALLOW_MISSING_MINERS";

/// Drives the real `quip-cpu-sa` binary against `fixtures/smoke.spec.json`
/// with 4 jobs. FAILS by default when the binary is absent (most
/// development and CI machines today); set `ISINGMARK_ALLOW_MISSING_MINERS=1`
/// to opt out. The hermetic `isingmark-fake-miner`-driven tests above give
/// the session loop real coverage regardless of this variable.
#[tokio::test]
async fn quip_cpu_sa_conformance() {
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

    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 6, 4);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");

    let report = run_with(&backend, &spec, jobs, &sock_path).await;

    assert!(
        report.handshake_ok,
        "handshake with the real quip-cpu-sa binary must succeed"
    );
    assert_eq!(report.records.len(), 4);
    for record in &report.records {
        assert!(
            record.n_solutions > 0,
            "record has no solutions: {record:?}"
        );
        // `passed` is defined as "the coordinator's local recompute of
        // energy_milli for the returned spins matches the miner's own
        // report" (session.rs::build_result_record): RunReport/JobRecord
        // do not expose the winning solution's spins to this test, so this
        // is how the "best_energy_milli equals a locally recomputed
        // energy_milli for the returned spins" requirement is checked here
        // without reaching into session.rs's private state. The
        // fake-miner-driven tests above additionally recompute the expected
        // energy from scratch, independently of this flag, since their
        // solutions are deterministic and known ahead of time.
        assert!(
            record.passed,
            "recomputed energy must match the miner's report: {record:?}"
        );
    }
    assert_eq!(report.miner_exit_code, 0);
    assert!(
        !sock_path.exists(),
        "coordinator socket must be removed on every exit path"
    );
}
