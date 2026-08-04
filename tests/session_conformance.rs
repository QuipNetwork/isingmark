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

use isingmark::backend::{
    builtin_registry, run_session, run_session_with_max_missed_pings_for_tests, BackendSpec,
    RunParams, RunReport,
};
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

/// Like [`run_with`], but with the liveness threshold overridden via
/// `run_session_with_max_missed_pings_for_tests` (`#[doc(hidden)]`, not
/// `RunParams` — the human declined to widen that struct for this).
/// Production's `MAX_MISSED_PINGS` gives a deliberately generous ~30s
/// detection window; a test that actually needs the unresponsive path to
/// trip cannot wait that out on every run.
async fn run_with_max_missed_pings(
    backend: &BackendSpec,
    spec: &TopologySpec,
    jobs: Vec<Job>,
    sock_path: &Path,
    max_missed_pings: u32,
) -> RunReport {
    run_session_with_max_missed_pings_for_tests(
        RunParams {
            backend,
            spec,
            jobs,
            sock_path,
            target: Some(small_target()),
        },
        max_missed_pings,
    )
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
    // Same batching as the Fatal test, but the miner half-closes its
    // outbound stream for job #2 (and stays alive briefly to drain its own
    // inbound) instead of sending an orderly Fatal report. This is a clean
    // process exit, not a crash — see
    // `fake_miner_abort_mid_job_preserves_every_in_flight_record` below for
    // the case where the socket close and the process death are the same
    // kernel event.
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
            Some("miner stream closed with jobs in flight")
        );
    }
    // A silent disconnect is not a Fatal report, so the fake miner's own
    // `main` still returns success.
    assert_eq!(report.miner_exit_code, 0);
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn fake_miner_abort_mid_job_preserves_every_in_flight_record() {
    // `--abort-after` calls `std::process::abort()`, which closes the
    // miner's socket FD and ends the process in the same kernel event —
    // unlike `--disconnect-after`, where the miner stays alive and drains
    // its own inbound first. This is the specific race `run_session`'s
    // `child.wait()` anti-hang backstop must not mistake for "the session
    // finished": without a bounded wait for `drive_session` to commit its
    // records after the child branch fires, `report.records` comes back
    // missing every job that was in flight when the miner died.
    //
    // Whether job 1's `Result` — sent immediately before the abort — was
    // actually flushed out of the miner's own outbound buffer before the
    // process died is itself a real race (an `mpsc::Sender::send` returning
    // only means the message is queued locally, not that it reached the
    // socket), exactly mirroring a real crash's uncertainty about the last
    // in-flight write. This test does not assume an answer either way: the
    // property under test is that every dispatched job is accounted for —
    // as a genuine `Result` or as an abandoned-in-flight record — never
    // silently missing, regardless of which side of that race it landed on.
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 14, 4);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec![
        "--credits".to_string(),
        "4".to_string(),
        "--abort-after".to_string(),
        "2".to_string(),
    ]);

    let report = run_with(&backend, &spec, jobs.clone(), &sock_path).await;

    assert!(report.handshake_ok);
    assert_eq!(
        report.records.len(),
        jobs.len(),
        "every dispatched job must be accounted for, not silently dropped: {:?}",
        report.records
    );
    for job in &jobs {
        let record = record_by_job_id(&report.records, job);
        assert!(
            record.passed || record.rejected,
            "record must be either a genuine pass or an explicit abandonment, \
             not some third, unaccounted-for state: {record:?}"
        );
    }
    // std::process::abort() raises SIGABRT: the process is terminated by a
    // signal, not a normal exit, so `ExitStatus::code()` is `None` and
    // `run_session` reports the documented signal sentinel.
    assert_eq!(report.miner_exit_code, -1);
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn fake_miner_unresponsive_after_handshake_preserves_records_and_sets_the_flag() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let jobs = draw_jobs(&spec, 13, 4);
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    // Request all 4 credits up front, solve job #1 normally, then go
    // completely silent (including ignoring the coordinator's own Ping)
    // for jobs 2-4 — proving the session returns instead of hanging, with
    // whatever records were already completed intact.
    let backend = fake_miner_backend(vec![
        "--credits".to_string(),
        "4".to_string(),
        "--go-silent-after".to_string(),
        "2".to_string(),
    ]);

    // The production MAX_MISSED_PINGS (~30s detection window) is
    // deliberately generous — see its doc comment — and far too slow for a
    // test to wait out on every run. Override to 1 (detection in
    // PING_INTERVAL * 2 = 6s) via the test-only entry point rather than
    // widening RunParams for this.
    let report = run_with_max_missed_pings(&backend, &spec, jobs.clone(), &sock_path, 1).await;

    assert!(
        report.handshake_ok,
        "the handshake itself completed before the miner went silent"
    );
    assert!(
        report.miner_unresponsive,
        "a miner that never answers a Ping must be flagged"
    );
    assert_eq!(
        report.records.len(),
        4,
        "records from before the silence must be preserved"
    );

    let solved = record_by_job_id(&report.records, &jobs[0]);
    assert!(
        solved.passed,
        "job solved before the silence must still be recorded as passed"
    );

    for job in &jobs[1..] {
        let record = record_by_job_id(&report.records, job);
        assert!(
            record.rejected,
            "job abandoned by unresponsiveness must be recorded as failing: {record:?}"
        );
        assert!(!record.passed);
        assert_eq!(
            record.reject_reason.as_deref(),
            Some("miner unresponsive: no reply to repeated Ping messages")
        );
    }
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn fake_miner_stderr_is_captured_when_the_miner_exits_before_connecting() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec![
        "--stderr-and-exit".to_string(),
        "isingmark-fake-miner: distinctive diagnostic marker 8f3c1a".to_string(),
    ]);

    let report = run_with(&backend, &spec, Vec::new(), &sock_path).await;

    assert!(
        !report.handshake_ok,
        "the miner exited before ever sending Hello"
    );
    assert!(report.records.is_empty());
    assert_ne!(report.miner_exit_code, 0);
    let tail = report.miner_stderr_tail.as_deref().unwrap_or("");
    assert!(
        tail.contains("distinctive diagnostic marker 8f3c1a"),
        "stderr tail should carry the miner's diagnostic text, got {tail:?}"
    );
    assert!(!sock_path.exists());
}

#[tokio::test]
async fn fake_miner_bounded_stderr_wait_does_not_hang_on_a_leaked_pipe() {
    // `capture_stderr_tail` only returns at EOF, which needs every write
    // end of the pipe closed. `--leak-stderr-to-grandchild` spawns a
    // detached `sleep 60` inheriting the fake miner's stderr fd, then exits
    // — simulating a wrapper script that backgrounds the real miner instead
    // of exec-ing it. Without a bound on that wait, `run_session` would
    // block for the full 60 seconds (or forever, for a truly persistent
    // helper) with every other part of the report already known. A 20s
    // ceiling on the whole call proves the bound applies without requiring
    // the test to wait out the full leak lifetime.
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let dir = tempfile::tempdir().expect("create temp dir");
    let sock_path = dir.path().join("coordinator.sock");
    let backend = fake_miner_backend(vec![
        "--leak-stderr-to-grandchild".to_string(),
        "isingmark-fake-miner: leaked-stderr marker c4f1a2".to_string(),
    ]);

    let report = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        run_with(&backend, &spec, Vec::new(), &sock_path),
    )
    .await
    .expect("run_session must not hang on a stderr pipe a grandchild keeps open");

    assert!(!report.handshake_ok);
    assert!(report.records.is_empty());
    assert_ne!(report.miner_exit_code, 0);
    let tail = report.miner_stderr_tail.as_deref().unwrap_or("");
    assert!(
        tail.contains("leaked-stderr marker c4f1a2"),
        "the line written before the leak must still be captured: {tail:?}"
    );
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
