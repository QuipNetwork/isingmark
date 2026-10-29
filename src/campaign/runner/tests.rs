// SPDX-License-Identifier: AGPL-3.0-or-later
//! Accounting and event persistence checks that do not bind sockets.

use super::*;
use crate::backend::{CapturedMeta, CapturedSolution};
use crate::campaign::{contracts::TimingStatus, trace::read_trace};

fn inputs() -> (Instance, PolicyManifest, TrialBudget, RuleSet) {
    let config = CampaignConfig {
        manifest: Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("evaluation/qblock/fixtures/campaign/fixed.manifest.json"),
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

fn manifest_config(dir: &Path) -> CampaignConfig {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluation/qblock");
    let (_, policy, _, _) = inputs();
    let manifest = dir.join("manifest.json");
    fs::write(
        &manifest,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "protocol_profile": root.join("configs/protocol.json"),
            "instances": [root.join("fixtures/two_spin.instance.json")],
            "policies": [policy],
            "trials_per_pair": 2,
            "budget": {"wall_limit_s": 5.0}
        }))
        .expect("manifest JSON"),
    )
    .expect("manifest");
    CampaignConfig {
        manifest,
        backend: "fake".into(),
        output: dir.join("out"),
        dry_run: true,
    }
}

#[tokio::test]
async fn absent_resource_limits_round_trip_as_null() {
    let dir = tempfile::tempdir().expect("directory");
    let config = manifest_config(dir.path());
    for (budget, wall, device) in [
        (json!({"wall_limit_s": 5.0}), json!(10.0), Value::Null),
        (json!({"device_budget_s": 30.0}), Value::Null, json!(60.0)),
    ] {
        let mut manifest: Value =
            serde_json::from_reader(File::open(&config.manifest).expect("manifest")).expect("JSON");
        manifest["budget"] = budget;
        fs::write(
            &config.manifest,
            serde_json::to_vec(&manifest).expect("JSON"),
        )
        .expect("manifest");
        let report = run_campaign(&config).await.expect("dry run");
        let encoded = serde_json::to_value(report.resources).expect("resource JSON");
        assert_eq!(encoded["max_wall_s"], wall);
        assert_eq!(encoded["max_device_s"], device);
        let _: ResourceEstimate =
            serde_json::from_value(encoded).expect("finite resource round trip");
    }
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
    let capture = captured_batch(id, meta, spins);
    state
        .receive(&capture, Instant::now())
        .expect("batch receipt");
    capture
}

fn captured_batch(id: &str, meta: Option<u64>, spins: &[Vec<i8>]) -> BatchCapture {
    BatchCapture {
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
    }
}

fn session_channels() -> (
    LiveSession,
    mpsc::Receiver<SessionCommand>,
    mpsc::Sender<SessionEvent>,
) {
    let (commands, receiver) = mpsc::channel(4);
    let (sender, events) = mpsc::channel(32);
    let task = tokio::spawn(async {
        Err(SessionError::Spawn(std::io::Error::other(
            "test transport ended",
        )))
    });
    (
        LiveSession {
            id: "session".into(),
            commands,
            events,
            failed: Arc::new(AtomicBool::new(false)),
            task,
            ready: false,
            trials: 0,
        },
        receiver,
        sender,
    )
}

async fn drive_to_terminal(
    session: &mut LiveSession,
    state: &mut TrialState,
    instance: &Instance,
    policy: &PolicyManifest,
    rules: &RuleSet,
) {
    let encoding = encoding(instance).expect("encoding");
    let spec = topology(&encoding).expect("topology");
    let target = target_from_objective(instance).expect("target");
    let terminal = drive_trial(
        session, state, instance, policy, rules, &spec, &encoding, &target, &mut None,
    )
    .await
    .expect("trial loop");
    state.terminate(terminal).expect("termination");
}

fn assert_terminal(state: &TrialState, expected: &str) {
    assert_eq!(
        serde_json::to_value(state.trial.terminal).expect("terminal JSON"),
        expected
    );
    let trace = read_trace(&state.trial.events).expect("trace");
    let termination = trace.events.last().expect("termination");
    assert_eq!(termination.kind, EventKind::TrialTermination);
    assert_eq!(termination.payload["terminal"], expected);
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
fn accepted_proof_with_missing_device_timing_stays_unresolved() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, rules) = inputs();
    let target = target_from_objective(&instance).expect("target");
    let mut state = state(dir.path(), &instance, &policy, &budget);
    let capture = batch(&mut state, "missing", None, &[vec![1, 1]]);
    state
        .evaluate(&instance, &policy, &rules, &target, &capture)
        .expect("pinned verification");
    assert!(state.trial.accepted_wall.is_some());
    state
        .terminate(TerminalReason::Accepted)
        .expect("termination");
    assert_eq!(state.trial.terminal, TerminalReason::Accepted);
    assert_eq!(state.trial.device_total.status, TimingStatus::Missing);
    assert_eq!(state.trial.device_total.upper_ns, None);
    assert_eq!(
        state.ledger.outcome(30_000_000_000),
        BudgetOutcome::Unresolved
    );
    assert_terminal(&state, "accepted");
    let trace = read_trace(&state.trial.events).expect("trace");
    assert_eq!(
        trace.events.last().expect("termination").payload["device_outcome"],
        "Unresolved"
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

#[tokio::test]
async fn useful_first_read_does_not_prorate_the_batch() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, rules) = inputs();
    let mut state = state(dir.path(), &instance, &policy, &budget);
    let capture = captured_batch("expensive", Some(31_000_000), &[vec![1, 1], vec![1, -1]]);
    let (mut session, _commands, events) = session_channels();
    events
        .send(SessionEvent::Dispatched {
            job_id: capture.job_id.clone(),
            at: Instant::now(),
        })
        .await
        .expect("dispatch");
    events
        .send(SessionEvent::Batch {
            capture,
            at: Instant::now(),
        })
        .await
        .expect("batch");
    drive_to_terminal(&mut session, &mut state, &instance, &policy, &rules).await;
    assert!(state.trial.accepted_wall.is_some());
    assert_eq!(
        state.ledger.outcome(30_000_000_000),
        BudgetOutcome::Exceeded
    );
    assert_terminal(&state, "accepted");
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

#[tokio::test]
async fn drained_proof_after_wall_cap_keeps_device_success() {
    for terminal in [TerminalReason::WallTimeout, TerminalReason::Cancelled] {
        let dir = tempfile::tempdir().expect("directory");
        let (instance, policy, _, rules) = inputs();
        let budget =
            TrialBudget::new(Some(Duration::from_millis(1)), Some(30_000_000_000)).expect("budget");
        let mut state = state(dir.path(), &instance, &policy, &budget);
        state.clock = Instant::now()
            .checked_sub(Duration::from_millis(2))
            .expect("earlier clock");
        state
            .dispatch(b"late".to_vec(), Instant::now())
            .expect("dispatch");
        let (session, _commands, events) = session_channels();
        events
            .send(SessionEvent::Batch {
                capture: captured_batch("late", Some(29_000_000), &[vec![1, 1]]),
                at: Instant::now(),
            })
            .await
            .expect("late batch");
        let mut runner =
            TrialRunner::new(resolve_backend(&policy.backend), dir.path().into(), rules);
        runner.session = Some(session);
        let target = target_from_objective(&instance).expect("target");
        runner
            .stop_trial(&mut state, terminal, &instance, &policy, &target)
            .await
            .expect("drain");
        state.terminate(terminal).expect("termination");
        assert_eq!(state.trial.terminal, terminal);
        assert!(
            state.trial.accepted_wall.expect("late proof") > budget.wall_limit.expect("wall cap")
        );
        assert_eq!(state.ledger.outcome(30_000_000_000), BudgetOutcome::Within);
        let trace = read_trace(&state.trial.events).expect("trace");
        let proof = trace
            .events
            .iter()
            .find(|event| event.kind == EventKind::ProofAcceptance)
            .expect("proof event");
        assert!(proof.wall_ns > 1_000_000);
    }
}

#[tokio::test]
async fn cancellation_grace_scores_a_proof_after_the_wall_timer_fires() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, mut policy, _, rules) = inputs();
    policy.preparation = Preparation::Warm;
    let budget = TrialBudget::new(Some(Duration::from_millis(10)), Some(30_000_000_000))
        .expect("short wall cap");
    let (mut session, mut commands, events) = session_channels();
    session.ready = true;
    session.task = tokio::spawn(async move {
        let submission = match commands.recv().await.expect("submission") {
            SessionCommand::Submit(submission) => Some(submission),
            SessionCommand::Finish => None,
        }
        .expect("submit before shutdown");
        let job_id = submission.job.job_id;
        events
            .send(SessionEvent::Dispatched {
                job_id: job_id.clone(),
                at: Instant::now(),
            })
            .await
            .expect("dispatch");
        assert!(matches!(
            commands.recv().await,
            Some(SessionCommand::Finish)
        ));
        // Arrival is gated by the actual wall-time stop and cancellation grace.
        events
            .send(SessionEvent::Batch {
                capture: captured_batch(
                    std::str::from_utf8(&job_id).expect("job id"),
                    Some(29_000_000),
                    &[vec![1, 1]],
                ),
                at: Instant::now(),
            })
            .await
            .expect("grace batch");
        Err(SessionError::Spawn(std::io::Error::other(
            "test transport ended",
        )))
    });
    let mut runner = TrialRunner::new(resolve_backend(&policy.backend), dir.path().into(), rules);
    runner.session = Some(session);
    let trial = runner
        .run_trial(&instance, &policy, &budget)
        .await
        .expect("trial");
    assert_eq!(trial.terminal, TerminalReason::WallTimeout);
    assert!(trial.accepted_wall.expect("proof") > budget.wall_limit.expect("cap"));
    assert_eq!(
        super::super::timing::budget_outcome(&trial.device_total, 30_000_000_000),
        BudgetOutcome::Within
    );
    let trace = read_trace(&trial.events).expect("trace");
    assert!(trace
        .events
        .iter()
        .any(|event| event.kind == EventKind::ProofAcceptance && event.wall_ns > 10_000_000));
}

#[tokio::test]
async fn drained_batches_respect_scoring_limits() {
    for host_limit in [true, false] {
        let dir = tempfile::tempdir().expect("directory");
        let (instance, mut policy, budget, rules) = inputs();
        if host_limit {
            policy.host_compute_limit_s = 0.0;
        } else {
            policy.evaluation_limit = 0;
        }
        let mut state = state(dir.path(), &instance, &policy, &budget);
        state
            .dispatch(b"late".to_vec(), Instant::now())
            .expect("dispatch");
        let (session, _commands, events) = session_channels();
        events
            .send(SessionEvent::Batch {
                capture: captured_batch("late", Some(29_000_000), &[vec![1, 1]]),
                at: Instant::now(),
            })
            .await
            .expect("late batch");
        let mut runner =
            TrialRunner::new(resolve_backend(&policy.backend), dir.path().into(), rules);
        runner.session = Some(session);
        let target = target_from_objective(&instance).expect("target");
        runner
            .stop_trial(
                &mut state,
                TerminalReason::WallTimeout,
                &instance,
                &policy,
                &target,
            )
            .await
            .expect("drain");
        assert!(state.trial.accepted_wall.is_none());
        assert_eq!(state.trial.resources.get("evaluations"), None);
        assert_eq!(state.ledger.total().lower_ns, 29_000_000_000);
    }
}

#[tokio::test]
async fn miner_reject_is_an_operational_terminal() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, rules) = inputs();
    let mut state = state(dir.path(), &instance, &policy, &budget);
    let (mut session, _commands, events) = session_channels();
    events
        .send(SessionEvent::Dispatched {
            job_id: b"rejected".to_vec(),
            at: Instant::now(),
        })
        .await
        .expect("dispatch");
    events
        .send(SessionEvent::Rejected {
            job_id: b"rejected".to_vec(),
            reason: "unsupported".into(),
            at: Instant::now(),
        })
        .await
        .expect("reject");
    drive_to_terminal(&mut session, &mut state, &instance, &policy, &rules).await;
    assert_terminal(&state, "miner_rejected");
    assert!(state.trial.accepted_wall.is_none());
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

#[tokio::test]
async fn host_limit_stop_ends_host_compute_limit() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, mut policy, budget, rules) = inputs();
    policy.host_compute_limit_s = 0.0;
    let mut state = state(dir.path(), &instance, &policy, &budget);
    let (mut session, _commands, _events) = session_channels();
    drive_to_terminal(&mut session, &mut state, &instance, &policy, &rules).await;
    assert_eq!(state.trial.terminal, TerminalReason::HostComputeLimit);
    assert_terminal(&state, "host_compute_limit");
    assert!(state.trial.accepted_wall.is_none());
}

#[tokio::test]
async fn evaluation_limit_stop_ends_evaluation_limit() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, mut policy, budget, rules) = inputs();
    policy.evaluation_limit = 0;
    let mut state = state(dir.path(), &instance, &policy, &budget);
    let (mut session, _commands, _events) = session_channels();
    drive_to_terminal(&mut session, &mut state, &instance, &policy, &rules).await;
    assert_eq!(state.trial.terminal, TerminalReason::EvaluationLimit);
    assert_terminal(&state, "evaluation_limit");
    assert!(state.trial.accepted_wall.is_none());
}

#[tokio::test]
async fn proof_validated_while_crossing_host_limit_ends_accepted() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, mut policy, budget, rules) = inputs();
    policy.host_compute_limit_s = 0.000_000_001;
    let mut state = state(dir.path(), &instance, &policy, &budget);
    // Previously scored candidates leave only pinned proof validation to time.
    state.witnesses.push(Witness {
        instance_id: instance.instance_id.clone(),
        spins: vec![1, 1],
        design: None,
        route: None,
        controls: None,
    });
    state.energies.push(-1000);
    let (mut session, mut commands, events) = session_channels();
    let transport = async {
        let submission = match commands.recv().await.expect("submission") {
            SessionCommand::Submit(submission) => Some(submission),
            SessionCommand::Finish => None,
        }
        .expect("job submission");
        let job_id = submission.job.job_id;
        events
            .send(SessionEvent::Dispatched {
                job_id: job_id.clone(),
                at: Instant::now(),
            })
            .await
            .expect("dispatch");
        events
            .send(SessionEvent::Batch {
                capture: BatchCapture {
                    job_id,
                    received_after_dispatch: Duration::ZERO,
                    solutions: Vec::new(),
                    meta: None,
                },
                at: Instant::now(),
            })
            .await
            .expect("batch");
    };
    tokio::join!(
        drive_to_terminal(&mut session, &mut state, &instance, &policy, &rules),
        transport
    );
    assert!(state.host_exhausted(&policy).expect("host limit"));
    assert_eq!(state.trial.terminal, TerminalReason::Accepted);
    assert!(state.trial.accepted_wall.is_some());
    assert_terminal(&state, "accepted");
}

#[tokio::test]
async fn startup_failure_writes_no_challenge_release() {
    for preparation in [Preparation::Warm, Preparation::Amortized { horizon: 2 }] {
        let dir = tempfile::tempdir().expect("directory");
        let (instance, mut policy, budget, rules) = inputs();
        policy.preparation = preparation;
        let (session, _commands, events) = session_channels();
        drop(events);
        let mut runner = TrialRunner::new(
            resolve_backend(&policy.backend),
            dir.path().to_owned(),
            rules,
        );
        runner.session = Some(session);
        let trial = runner
            .run_trial(&instance, &policy, &budget)
            .await
            .expect("failed preparation trial");
        assert_eq!(trial.terminal, TerminalReason::MinerCrashed);
        let trace = read_trace(&trial.events).expect("trace");
        assert!(!trace
            .events
            .iter()
            .any(|event| event.kind == EventKind::ChallengeRelease));
        let termination = trace.events.last().expect("termination");
        assert_eq!(termination.kind, EventKind::TrialTermination);
        assert_eq!(termination.payload["terminal"], "miner_crashed");
        assert!(runner.session.is_none());
    }
}

#[tokio::test]
async fn preparation_ready_wait_does_not_spend_the_wall_budget() {
    for preparation in [Preparation::Warm, Preparation::Amortized { horizon: 2 }] {
        let dir = tempfile::tempdir().expect("directory");
        let (instance, mut policy, _, rules) = inputs();
        policy.preparation = preparation;
        let budget = TrialBudget::new(Some(Duration::ZERO), Some(30_000_000_000))
            .expect("zero challenge budget");
        let (session, _commands, events) = session_channels();
        let ready = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            events.send(SessionEvent::Ready).await.is_ok()
        });
        let mut runner = TrialRunner::new(
            resolve_backend(&policy.backend),
            dir.path().to_owned(),
            rules,
        );
        runner.session = Some(session);
        let trial = runner
            .run_trial(&instance, &policy, &budget)
            .await
            .expect("prepared trial");
        let ready_sent = ready.await.expect("readiness task");
        assert_eq!(trial.terminal, TerminalReason::WallTimeout);
        assert!(ready_sent);
        assert!(trial.resources["preparation_ns"] >= 25_000_000);
        let trace = read_trace(&trial.events).expect("trace");
        let releases: Vec<_> = trace
            .events
            .iter()
            .filter(|event| event.kind == EventKind::ChallengeRelease)
            .collect();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].wall_ns, 0);
    }
}

#[tokio::test]
async fn exact_device_lower_bound_stays_unresolved_and_keeps_submitting() {
    let dir = tempfile::tempdir().expect("directory");
    let (instance, policy, budget, rules) = inputs();
    let mut state = state(dir.path(), &instance, &policy, &budget);
    batch(&mut state, "exact-bound", Some(30_000_000), &[]);
    assert_eq!(state.ledger.total().lower_ns, 30_000_000_000);
    assert_eq!(
        state.ledger.outcome(30_000_000_000),
        BudgetOutcome::Unresolved
    );
    let (mut session, mut commands, events) = session_channels();
    events
        .send(SessionEvent::Failed {
            reason: "stop after the next submission".into(),
            at: Instant::now(),
        })
        .await
        .expect("transport ending");
    drive_to_terminal(&mut session, &mut state, &instance, &policy, &rules).await;
    assert_eq!(state.trial.terminal, TerminalReason::MinerCrashed);
    let submission = match commands.try_recv().expect("next submission") {
        SessionCommand::Submit(submission) => Some(submission),
        SessionCommand::Finish => None,
    }
    .expect("job submission");
    assert!(!submission.job.job_id.is_empty());
    assert_eq!(
        state.ledger.outcome(30_000_000_000),
        BudgetOutcome::Unresolved
    );
    assert_terminal(&state, "miner_crashed");
}
