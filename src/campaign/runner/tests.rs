// SPDX-License-Identifier: AGPL-3.0-or-later
//! Accounting and event persistence checks that do not bind sockets.

use super::*;
use crate::backend::{CapturedMeta, CapturedSolution};
use crate::campaign::{contracts::TimingStatus, trace::read_trace};

fn inputs() -> (Instance, PolicyManifest, TrialBudget, RuleSet) {
    let config = CampaignConfig {
        manifest: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("research/qblock_eval/fixtures/campaign/fixed.manifest.json"),
        backend: "fake".into(),
        output: PathBuf::new(),
        dry_run: true,
    };
    let loaded = load_campaign(&config).expect("fixture campaign");
    (
        loaded.instances[0].clone(),
        loaded.policies[0].clone(),
        loaded.budget,
        RuleSet::Current(loaded.profile),
    )
}

fn state(
    dir: &Path,
    instance: &Instance,
    policy: &PolicyManifest,
    budget: &TrialBudget,
) -> TrialState {
    let path = dir.join("events.jsonl");
    let writer = TraceWriter::create(&path).expect("writer");
    let mut state = TrialState::new(
        instance,
        policy,
        budget,
        "trial".into(),
        path,
        writer,
        "run".into(),
    );
    state.session_id = "session".into();
    state
        .release(instance, policy, budget)
        .expect("challenge release");
    state
}

fn batch(state: &mut TrialState, id: &str, meta: Option<u64>, spins: &[Vec<i8>]) -> BatchCapture {
    state.ledger.launched_uncharged(id).expect("pending charge");
    state.pending.insert(id.into());
    state
        .emit(
            EventKind::JobDispatch,
            Instant::now(),
            Some(id.into()),
            json!({}),
            None,
        )
        .expect("dispatch");
    let capture = BatchCapture {
        job_id: id.as_bytes().to_vec(),
        received_after_dispatch: Duration::from_millis(300),
        solutions: spins
            .iter()
            .map(|spins| CapturedSolution {
                spins: spins.clone(),
                reported_energy_milli: -9999,
                recomputed_energy_milli: -1000,
            })
            .collect(),
        meta: meta.map(|qpu_access_us| CapturedMeta {
            reads: 2,
            sweeps: 10,
            device_access_time_us: 80_000_000,
            qpu_access_us,
            extra: BTreeMap::from([("raw".into(), "kept".into())]),
        }),
    };
    state
        .receive(&capture, Instant::now())
        .expect("batch receipt");
    capture
}

#[test]
fn three_full_batches_accumulate_and_validate_through_the_pinned_rule() {
    let dir = tempfile::tempdir().expect("directory");
    let (mut instance, policy, budget, rules) = inputs();
    instance.objective["min_solutions"] = json!(6);
    let target = target_from_objective(&instance).expect("target");
    let mut state = state(dir.path(), &instance, &policy, &budget);
    for number in 0..3 {
        let capture = batch(
            &mut state,
            &format!("job-{number}"),
            Some(1_000_000),
            &[vec![1, 1], vec![1, 1]],
        );
        state
            .evaluate(&instance, &policy, &rules, &target, &capture)
            .expect("scoring");
        assert_eq!(state.trial.accepted_wall.is_some(), number == 2);
    }
    state
        .terminate(TerminalReason::Accepted)
        .expect("termination");
    assert_eq!(state.trial.device_total.lower_ns, 3_000_000_000);
    assert_eq!(state.trial.device_total.upper_ns, Some(3_000_002_997));
    let trace = read_trace(&state.trial.events).expect("trace");
    assert_eq!(
        trace
            .events
            .iter()
            .filter(|event| event.kind == EventKind::ProofAcceptance)
            .count(),
        1
    );
    assert!(trace
        .events
        .windows(2)
        .all(|pair| pair[0].wall_ns <= pair[1].wall_ns));
}

#[test]
fn missing_batch_metadata_cannot_certify_device_success() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, _) = inputs();
    let mut state = state(dir.path(), &instance, &policy, &budget);
    batch(&mut state, "missing", None, &[]);
    batch(&mut state, "known", Some(1_000_000), &[]);
    state
        .terminate(TerminalReason::WallTimeout)
        .expect("termination");
    assert_eq!(state.trial.device_total.lower_ns, 1_000_000_000);
    assert_eq!(state.trial.device_total.status, TimingStatus::Missing);
    assert_eq!(state.trial.device_total.upper_ns, None);
    assert_eq!(
        state.ledger.outcome(30_000_000_000),
        BudgetOutcome::Unresolved
    );
}

#[test]
fn zero_distinct_rule_still_needs_a_scored_valid_witness() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, _) = inputs();
    let target = target_from_objective(&instance).expect("target");
    let rules = RuleSet::ProposedUnique { min_distinct: 0 };
    let mut state = state(dir.path(), &instance, &policy, &budget);
    for (id, spins) in [
        ("empty", vec![]),
        ("invalid", vec![vec![0, 0]]),
        ("equal", vec![vec![1, -1]]),
    ] {
        let capture = batch(&mut state, id, Some(0), &spins);
        state
            .evaluate(&instance, &policy, &rules, &target, &capture)
            .expect("evaluate invalid batch");
        assert!(state.trial.accepted_wall.is_none());
    }
    let capture = batch(&mut state, "valid", Some(0), &[vec![1, 1]]);
    state
        .evaluate(&instance, &policy, &rules, &target, &capture)
        .expect("evaluate valid batch");
    assert!(state.trial.accepted_wall.is_some());
}

#[test]
fn useful_first_read_does_not_prorate_the_batch() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, rules) = inputs();
    let target = target_from_objective(&instance).expect("target");
    let mut state = state(dir.path(), &instance, &policy, &budget);
    let capture = batch(
        &mut state,
        "expensive",
        Some(31_000_000),
        &[vec![1, 1], vec![1, -1]],
    );
    state
        .evaluate(&instance, &policy, &rules, &target, &capture)
        .expect("evaluate");
    assert!(state.trial.accepted_wall.is_some());
    assert_eq!(
        state.ledger.outcome(30_000_000_000),
        BudgetOutcome::Exceeded
    );
    state
        .terminate(TerminalReason::DeviceBudgetExhausted)
        .expect("termination");
    let trace = read_trace(&state.trial.events).expect("trace");
    let receipt = trace
        .events
        .iter()
        .find(|event| event.kind == EventKind::BatchReceipt)
        .expect("receipt");
    assert_eq!(
        receipt.device.expect("device time").lower_ns,
        31_000_000_000
    );
    assert_eq!(
        receipt.payload["solutions"]
            .as_array()
            .expect("all reads")
            .len(),
        2
    );
    assert_eq!(receipt.payload["meta"]["device_access_time_us"], 80_000_000);
    assert_eq!(receipt.payload["meta"]["extra"]["raw"], "kept");
}

#[test]
fn every_pending_job_gets_a_cancellation_before_termination() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, _) = inputs();
    let mut state = state(dir.path(), &instance, &policy, &budget);
    batch(&mut state, "known", Some(1_000_000), &[]);
    for id in ["pending-1", "pending-2"] {
        state.ledger.launched_uncharged(id).expect("launch");
        state.pending.insert(id.into());
    }
    state
        .terminate(TerminalReason::WallTimeout)
        .expect("termination");
    assert_eq!(state.trial.device_total.lower_ns, 1_000_000_000);
    assert_eq!(state.trial.device_total.status, TimingStatus::Missing);
    let trace = read_trace(&state.trial.events).expect("trace");
    assert_eq!(
        trace
            .events
            .iter()
            .filter(|event| event.kind == EventKind::Cancellation)
            .count(),
        2
    );
    assert_eq!(
        trace.events.last().expect("termination").kind,
        EventKind::TrialTermination
    );
}

#[test]
fn event_append_failure_is_invalid_run() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, _) = inputs();
    let mut state = state(dir.path(), &instance, &policy, &budget);
    // Force the writer's sequence rejection after a successful persisted release.
    state.seq = 0;
    assert!(matches!(
        state.terminate(TerminalReason::Accepted),
        Err(CampaignError::InvalidRun(_))
    ));
}

#[test]
fn policy_host_compute_limit_prevents_late_acceptance() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, mut policy, budget, rules) = inputs();
    let target = target_from_objective(&instance).expect("target");
    let mut state = state(dir.path(), &instance, &policy, &budget);
    let capture = batch(&mut state, "valid", Some(0), &[vec![1, 1]]);
    state
        .evaluate(&instance, &policy, &rules, &target, &capture)
        .expect("first evaluation");
    state.trial.accepted_wall = None;
    policy.host_compute_limit_s = 0.0;
    state
        .evaluate(&instance, &policy, &rules, &target, &capture)
        .expect("exhausted evaluation");
    assert!(state.trial.accepted_wall.is_none());
}
