// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portfolio catalogs, slot resolution, and shared-budget portfolio trials.

use std::{fs, path::Path};

use isingmark::campaign::runner::portfolio::{
    load_catalog, resolve_panel, Allocation, Applicability, Catalog,
};
use serde_json::{json, Value};

fn solver(id: &str, backend: &str, device: &str, methods: &[&str], capture: &str) -> Value {
    json!({
        "solver_id": id, "backend": backend, "device": device, "methods": methods,
        "applies_to": "any", "state_capture": capture,
        "num_reads": 4, "num_sweeps": 10, "anneal_time_us": 0
    })
}

fn catalog_value(solvers: &[Value]) -> Value {
    json!({
        "schema_version": 1,
        "host_compute_limit_s": 1200.0,
        "evaluation_limit": 100_000,
        "solvers": solvers,
        "panels": [{"panel_id": "toy", "slots": [
            {"slot_id": "cpu-1", "device": "cpu", "method": "generic-annealing"},
            {"slot_id": "cpu-2", "device": "cpu", "method": "loop-tempering"},
            {"slot_id": "cpu-3", "device": "cpu", "method": "toy-flow", "fallback": "generic-annealing"},
            {"slot_id": "cpu-4", "device": "cpu", "schedule": [
                {"slot": "cpu-3", "allocation": {"seconds": 10.0}},
                {"slot": "cpu-2", "allocation": {"seconds": 190.0}},
                {"slot": "cpu-1", "allocation": "remainder"}
            ]},
            {"slot_id": "gpu-1", "device": "gpu", "method": "generic-annealing"},
            {"slot_id": "gpu-2", "device": "gpu", "method": "loop-updates"}
        ]}]
    })
}

fn write_catalog(dir: &Path, value: &Value) -> Catalog {
    let path = dir.join("policies.json");
    fs::write(&path, serde_json::to_vec(value).expect("JSON")).expect("catalog");
    load_catalog(&path).expect("catalog loads")
}

fn full_solvers() -> Vec<Value> {
    vec![
        solver(
            "cpu-sa",
            "cpu-sa",
            "cpu",
            &["generic-annealing"],
            "all_reads",
        ),
        solver(
            "cpu-loops",
            "cpu-loops",
            "cpu",
            &["loop-tempering"],
            "all_reads",
        ),
        solver("cuda", "cuda", "gpu", &["generic-annealing"], "all_reads"),
        solver(
            "cuda-loops",
            "cuda-loops",
            "gpu",
            &["loop-updates"],
            "all_reads",
        ),
    ]
}

#[test]
fn catalog_resolves_native_and_fallback() {
    let dir = tempfile::tempdir().expect("directory");
    let catalog = write_catalog(dir.path(), &catalog_value(&full_solvers()));
    let resolution = resolve_panel(&catalog, "toy", "any/family", &|_| true).expect("resolves");
    assert!(resolution.complete, "{resolution:?}");
    assert_eq!(resolution.solver_starts, 0);
    let cpu3 = &resolution.slots[2];
    let stage = &cpu3.policy.as_ref().expect("cpu-3 policy").stages[0];
    assert_eq!(stage.solver_id, "cpu-sa");
    assert_eq!(stage.method, "generic-annealing");
    assert_eq!(stage.applicability, Applicability::Fallback);
    let cpu1 = &resolution.slots[0]
        .policy
        .as_ref()
        .expect("cpu-1 policy")
        .stages[0];
    assert_eq!(cpu1.applicability, Applicability::Native);
    assert_eq!(cpu1.allocation, Allocation::Remainder);
}

#[test]
fn cpu4_schedule_is_frozen() {
    let dir = tempfile::tempdir().expect("directory");
    let catalog = write_catalog(dir.path(), &catalog_value(&full_solvers()));
    let resolution = resolve_panel(&catalog, "toy", "any/family", &|_| true).expect("resolves");
    let policy = resolution.slots[3].policy.as_ref().expect("cpu-4 policy");
    assert_eq!(policy.policy_id, "toy/cpu-4");
    let plan: Vec<_> = policy
        .stages
        .iter()
        .map(|stage| (stage.stage_id.as_str(), stage.allocation))
        .collect();
    assert_eq!(
        plan,
        [
            ("cpu-3", Allocation::Seconds(10.0)),
            ("cpu-2", Allocation::Seconds(190.0)),
            ("cpu-1", Allocation::Remainder),
        ]
    );
}

#[test]
fn best_only_solver_fails_admission() {
    let dir = tempfile::tempdir().expect("directory");
    let mut solvers = full_solvers();
    solvers[1] = solver(
        "cpu-loops",
        "cpu-loops",
        "cpu",
        &["loop-tempering"],
        "best_only",
    );
    let catalog = write_catalog(dir.path(), &catalog_value(&solvers));
    let resolution = resolve_panel(&catalog, "toy", "any/family", &|_| true).expect("resolves");
    let cpu2 = &resolution.slots[1];
    assert!(cpu2.policy.is_none());
    assert_eq!(cpu2.rejected[0].solver_id, "cpu-loops");
    // The schedule that includes cpu-2 cannot be labeled complete either.
    assert!(resolution.slots[3].policy.is_none());
    assert_eq!(resolution.missing_slots, ["cpu-2", "cpu-4"]);
    assert!(!resolution.complete);
}

#[test]
fn dry_run_reports_missing_slots_and_zero_starts() {
    let dir = tempfile::tempdir().expect("directory");
    let catalog = write_catalog(dir.path(), &catalog_value(&full_solvers()));
    let resolution = resolve_panel(&catalog, "toy", "any/family", &|backend| {
        backend != "cuda-loops"
    })
    .expect("resolves");
    assert_eq!(resolution.missing_slots, ["gpu-2"]);
    assert_eq!(resolution.solver_starts, 0);
    assert!(!resolution.complete);
    let gpu2 = &resolution.slots[5];
    assert!(
        gpu2.missing[0].contains("loop-updates"),
        "{:?}",
        gpu2.missing
    );
    assert!(gpu2.rejected[0].reason.contains("not available"));
}

#[test]
fn invalid_schedules_are_rejected() {
    let dir = tempfile::tempdir().expect("directory");
    let cases = [
        // The remainder must come last.
        json!([{"slot": "cpu-1", "allocation": "remainder"}, {"slot": "cpu-2", "allocation": {"seconds": 10.0}}]),
        // Only one remainder.
        json!([{"slot": "cpu-1", "allocation": "remainder"}, {"slot": "cpu-2", "allocation": "remainder"}]),
        // Positive seconds only.
        json!([{"slot": "cpu-1", "allocation": {"seconds": 0.0}}, {"slot": "cpu-2", "allocation": "remainder"}]),
        // A schedule cannot cross devices.
        json!([{"slot": "gpu-1", "allocation": {"seconds": 5.0}}, {"slot": "cpu-2", "allocation": "remainder"}]),
        json!([]),
    ];
    for schedule in cases {
        let mut value = catalog_value(&full_solvers());
        value["panels"][0]["slots"][3]["schedule"] = schedule.clone();
        let path = dir.path().join("policies.json");
        fs::write(&path, serde_json::to_vec(&value).expect("JSON")).expect("catalog");
        assert!(load_catalog(&path).is_err(), "accepted {schedule}");
    }
}

#[test]
fn committed_catalog_declares_four_cpu_and_two_gpu_slots_per_panel() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evaluation/qblock/configs/suite_fit/policies.json");
    let catalog = load_catalog(&path).expect("committed catalog");
    assert_eq!(catalog.panels.len(), 4);
    for panel in &catalog.panels {
        let ids: Vec<_> = panel
            .slots
            .iter()
            .map(|slot| slot.slot_id.as_str())
            .collect();
        assert_eq!(ids, ["cpu-1", "cpu-2", "cpu-3", "cpu-4", "gpu-1", "gpu-2"]);
        let cpu4 = panel.slots[3].schedule.as_ref().expect("cpu-4 schedule");
        let plan: Vec<_> = cpu4
            .iter()
            .map(|stage| (stage.slot.as_str(), stage.allocation))
            .collect();
        assert_eq!(
            plan,
            [
                ("cpu-3", Allocation::Seconds(10.0)),
                ("cpu-2", Allocation::Seconds(190.0)),
                ("cpu-1", Allocation::Remainder),
            ]
        );
        assert!(
            panel.slots[2].fallback.is_some(),
            "{} cpu-3 needs a fallback",
            panel.panel_id
        );
    }
}

mod execution {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::Duration,
    };

    use isingmark::{
        backend::BackendSpec,
        campaign::{
            families::FamilyRegistry,
            instance::{load_instance, Instance},
            protocol::{load_protocol_profile, RuleSet},
            runner::{
                portfolio::{
                    solver_seed, Allocation, Applicability, PortfolioPolicy, PortfolioRunner,
                    Stage, PORTFOLIO_BACKEND,
                },
                run_portfolio_campaign, CampaignConfig, TerminalReason, Trial,
            },
            timing::TrialBudget,
            trace::{read_trace, EventKind, TraceEvent},
        },
        params::BackendKind,
    };
    use serde_json::{json, Value};
    use tokio::sync::oneshot;

    fn fixture(path: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("evaluation/qblock")
            .join(path)
    }

    fn instance() -> Instance {
        load_instance(&fixture("fixtures/two_spin.instance.json")).expect("fixed instance")
    }

    fn rules() -> RuleSet {
        RuleSet::Current(
            load_protocol_profile(&fixture("configs/protocol.json")).expect("pinned profile"),
        )
    }

    fn fake(name: &str, args: &[&str]) -> BackendSpec {
        BackendSpec {
            name: name.into(),
            bin: PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner")),
            args: args.iter().map(ToString::to_string).collect(),
            kind: BackendKind::Sweeping,
            node_capacity_flag: None,
        }
    }

    fn stage(stage_id: &str, backend: &str, allocation: Allocation) -> Stage {
        Stage {
            stage_id: stage_id.into(),
            solver_id: backend.into(),
            backend: backend.into(),
            method: "generic-annealing".into(),
            applicability: Applicability::Native,
            num_reads: 1,
            num_sweeps: 10,
            anneal_time_us: 0,
            allocation,
        }
    }

    fn portfolio(stages: Vec<Stage>) -> PortfolioPolicy {
        PortfolioPolicy {
            policy_id: "toy/cpu-4".into(),
            host_compute_limit_s: 60.0,
            evaluation_limit: 100_000,
            stages,
        }
    }

    async fn run(
        portfolio: &PortfolioPolicy,
        backends: Vec<BackendSpec>,
        wall: Duration,
        cancellation: Option<oneshot::Receiver<()>>,
    ) -> (tempfile::TempDir, Trial) {
        let dir = tempfile::tempdir().expect("output directory");
        let runner = backends.into_iter().fold(
            PortfolioRunner::new(dir.path().to_owned(), rules()),
            PortfolioRunner::with_backend,
        );
        let budget = TrialBudget::new(Some(wall), Some(30_000_000_000)).expect("budget");
        let trial = runner
            .run_trial(&instance(), portfolio, &budget, 0, cancellation)
            .await
            .expect("trial");
        (dir, trial)
    }

    fn events(trial: &Trial) -> Vec<TraceEvent> {
        let trace = read_trace(&trial.events).expect("complete trace");
        assert!(!trace.truncated_tail);
        assert_eq!(
            trace.events.last().expect("termination").kind,
            EventKind::TrialTermination
        );
        trace.events
    }

    fn stage_ends(events: &[TraceEvent]) -> Vec<(String, Value)> {
        events
            .iter()
            .filter(|event| event.kind == EventKind::StageEnd)
            .map(|event| {
                (
                    event.payload["stage_id"]
                        .as_str()
                        .expect("stage id")
                        .to_owned(),
                    event.payload["outcome"].clone(),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn crashed_stage_stays_in_trace_and_next_stage_runs() {
        let policy = portfolio(vec![
            stage("cpu-3", "crash", Allocation::Seconds(2.0)),
            stage("cpu-1", "good", Allocation::Remainder),
        ]);
        let (_dir, trial) = run(
            &policy,
            vec![
                fake("crash", &["--disconnect-after", "1"]),
                fake("good", &[]),
            ],
            Duration::from_secs(10),
            None,
        )
        .await;
        assert_eq!(trial.terminal, TerminalReason::Accepted);
        assert_eq!(trial.policy_id, "toy/cpu-4");
        let events = events(&trial);
        assert_eq!(
            stage_ends(&events),
            [
                ("cpu-3".to_owned(), json!("miner_crashed")),
                ("cpu-1".to_owned(), json!("accepted"))
            ]
        );
        let sessions: Vec<_> = events
            .iter()
            .filter(|event| event.kind == EventKind::StageStart)
            .map(|event| event.session_id.clone())
            .collect();
        assert_eq!(sessions.len(), 2);
        assert_ne!(sessions[0], sessions[1]);
        // The crashed stage's dispatched job stays cancelled in the shared ledger.
        assert!(events
            .iter()
            .any(|event| event.kind == EventKind::Cancellation && event.session_id == sessions[0]));
        assert_eq!(trial.resources["jobs"], 2);
    }

    #[tokio::test]
    async fn stage_timeout_does_not_reset_parent_budget() {
        let policy = portfolio(vec![
            stage("cpu-3", "slow", Allocation::Seconds(0.5)),
            stage("cpu-1", "slow", Allocation::Remainder),
        ]);
        let (_dir, trial) = run(
            &policy,
            vec![fake("slow", &["--result-delay-ms", "5000"])],
            Duration::from_millis(1500),
            None,
        )
        .await;
        assert_eq!(trial.terminal, TerminalReason::WallTimeout);
        let events = events(&trial);
        assert_eq!(
            stage_ends(&events),
            [
                ("cpu-3".to_owned(), json!("allocation_spent")),
                ("cpu-1".to_owned(), json!("wall_timeout"))
            ]
        );
        let deadlines: Vec<_> = events
            .iter()
            .filter(|event| event.kind == EventKind::StageStart)
            .map(|event| event.payload["deadline_ns"].as_u64().expect("deadline"))
            .collect();
        // The remainder stage ends at the trial deadline, not 1.5 s after its own start.
        assert_eq!(deadlines[1], 1_500_000_000);
        assert!(deadlines[0] >= 500_000_000 && deadlines[0] < 1_500_000_000);
        // The final stage times out at the trial deadline. The graceful drain after it
        // may run longer, as in single-solver trials.
        let second = &events
            .iter()
            .find(|event| event.kind == EventKind::StageStart && event.payload["index"] == 1)
            .expect("second stage")
            .session_id;
        let timed_out = events
            .iter()
            .find(|event| event.kind == EventKind::Cancellation && &event.session_id == second)
            .expect("second stage cancellation")
            .wall_ns;
        assert!(
            (1_500_000_000..2_500_000_000).contains(&timed_out),
            "cancelled at {timed_out} ns"
        );
        assert_eq!(trial.resources["jobs"], 2);
    }

    #[tokio::test]
    async fn cancellation_stops_later_stages() {
        let policy = portfolio(vec![
            stage("cpu-3", "slow", Allocation::Seconds(5.0)),
            stage("cpu-1", "good", Allocation::Remainder),
        ]);
        let (cancel, cancelled) = oneshot::channel();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let _ = cancel.send(());
        });
        let (_dir, trial) = run(
            &policy,
            vec![
                fake("slow", &["--result-delay-ms", "5000"]),
                fake("good", &[]),
            ],
            Duration::from_secs(20),
            Some(cancelled),
        )
        .await;
        assert_eq!(trial.terminal, TerminalReason::Cancelled);
        let events = events(&trial);
        assert_eq!(
            stage_ends(&events),
            [("cpu-3".to_owned(), json!("cancelled"))]
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == EventKind::StageStart)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn all_final_states_reach_trace() {
        let mut first = stage("cpu-1", "many", Allocation::Remainder);
        first.num_reads = 4;
        let policy = portfolio(vec![first]);
        let (_dir, trial) = run(
            &policy,
            vec![fake("many", &["--num-solutions", "4"])],
            Duration::from_secs(10),
            None,
        )
        .await;
        assert_eq!(trial.terminal, TerminalReason::Accepted);
        let events = events(&trial);
        let receipt = events
            .iter()
            .find(|event| event.kind == EventKind::BatchReceipt)
            .expect("receipt");
        let solutions = receipt.payload["solutions"].as_array().expect("solutions");
        // Four fixed reads plus the fake miner's search read.
        assert_eq!(solutions.len(), 5);
        for solution in solutions {
            assert!(solution["spins"].is_array());
            assert!(solution["recomputed_energy_milli"].is_i64());
        }
    }

    fn manifest(dir: &Path, portfolios: &Value, wall: f64) -> CampaignConfig {
        let path = dir.join("manifest.json");
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema_version": 2,
                "acceptance": {"profile": "pinned_current",
                    "protocol_profile": fixture("configs/protocol.json")},
                "instances": [fixture("fixtures/two_spin.instance.json")],
                "policies": [],
                "portfolios": portfolios,
                "trials_per_pair": 2,
                "budget": {"wall_limit_s": wall}
            }))
            .expect("JSON"),
        )
        .expect("manifest");
        CampaignConfig {
            manifest: path,
            backend: PORTFOLIO_BACKEND.into(),
            output: dir.join("out"),
            dry_run: true,
            registry: FamilyRegistry::production(),
        }
    }

    #[tokio::test]
    async fn portfolio_dry_run_starts_no_solver() {
        let dir = tempfile::tempdir().expect("directory");
        let policy = portfolio(vec![
            stage("cpu-3", "absent-a", Allocation::Seconds(1.0)),
            stage("cpu-1", "absent-b", Allocation::Remainder),
        ]);
        let config = manifest(dir.path(), &json!([policy]), 10.0);
        let report = run_portfolio_campaign(&config, Vec::new())
            .await
            .expect("dry run");
        assert!(report.trials.is_empty());
        assert_eq!(report.resources.trials, 2);
        assert_eq!(report.resources.backends, ["absent-a", "absent-b"]);
        assert!(!dir.path().join("out").exists());
    }

    #[tokio::test]
    async fn portfolio_campaign_runs_each_trial_through_its_stages() {
        let dir = tempfile::tempdir().expect("directory");
        let policy = portfolio(vec![
            stage("cpu-3", "crash", Allocation::Seconds(2.0)),
            stage("cpu-1", "good", Allocation::Remainder),
        ]);
        let mut config = manifest(dir.path(), &json!([policy]), 10.0);
        config.dry_run = false;
        let report = run_portfolio_campaign(
            &config,
            vec![
                fake("crash", &["--disconnect-after", "1"]),
                fake("good", &[]),
            ],
        )
        .await
        .expect("campaign");
        assert_eq!(report.trials.len(), 2);
        for trial in &report.trials {
            assert_eq!(trial.terminal, TerminalReason::Accepted);
            assert_eq!(stage_ends(&events(trial)).len(), 2);
        }
        assert!(dir.path().join("out/campaign.json").is_file());
    }

    #[test]
    fn fixed_allocations_must_leave_a_remainder() {
        let dir = tempfile::tempdir().expect("directory");
        let policy = portfolio(vec![
            stage("cpu-3", "a", Allocation::Seconds(10.0)),
            stage("cpu-1", "b", Allocation::Remainder),
        ]);
        let config = manifest(dir.path(), &json!([policy]), 10.0);
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        let error = runtime
            .block_on(run_portfolio_campaign(&config, Vec::new()))
            .expect_err("no remainder time");
        assert!(error.to_string().contains("remainder"), "{error}");
    }

    #[tokio::test]
    async fn best_only_executable_fails_admission_at_runtime() {
        // Without --num-solutions the fake miner returns two states for four reads.
        let mut best = stage("cpu-3", "best", Allocation::Seconds(5.0));
        best.num_reads = 4;
        let policy = portfolio(vec![best, stage("cpu-1", "good", Allocation::Remainder)]);
        let (_dir, trial) = run(
            &policy,
            vec![fake("best", &[]), fake("good", &[])],
            Duration::from_secs(10),
            None,
        )
        .await;
        assert_eq!(trial.terminal, TerminalReason::Accepted);
        let events = events(&trial);
        assert_eq!(
            stage_ends(&events),
            [
                ("cpu-3".to_owned(), json!("incomplete_capture")),
                ("cpu-1".to_owned(), json!("accepted"))
            ]
        );
        let first = &events
            .iter()
            .find(|event| event.kind == EventKind::StageStart)
            .expect("first stage")
            .session_id;
        // The partial batch stays in the trace, but none of its states is scored.
        let receipt = events
            .iter()
            .find(|event| event.kind == EventKind::BatchReceipt && &event.session_id == first)
            .expect("partial receipt");
        assert_eq!(
            receipt.payload["solutions"].as_array().map(Vec::len),
            Some(2)
        );
        assert!(!events
            .iter()
            .any(|event| event.kind == EventKind::WitnessValidation && &event.session_id == first));

        let mut only = stage("cpu-1", "best", Allocation::Remainder);
        only.num_reads = 4;
        let (_dir, trial) = run(
            &portfolio(vec![only]),
            vec![fake("best", &[])],
            Duration::from_secs(10),
            None,
        )
        .await;
        assert_eq!(trial.terminal, TerminalReason::IncompleteCapture);
    }

    #[test]
    fn seed_changes_without_changing_instance() {
        let base = solver_seed("toy/cpu-4", "two-spin", "cpu-3", 0);
        assert_eq!(base, solver_seed("toy/cpu-4", "two-spin", "cpu-3", 0));
        assert_ne!(base, solver_seed("toy/cpu-4", "two-spin", "cpu-3", 1));
        assert_ne!(base, solver_seed("toy/cpu-4", "two-spin", "cpu-2", 0));
        assert_ne!(base, solver_seed("toy/cpu-1", "two-spin", "cpu-3", 0));
        assert_ne!(base, solver_seed("toy/cpu-4", "other", "cpu-3", 0));
        // Length prefixes keep field boundaries apart.
        assert_ne!(
            solver_seed("ab", "c", "d", 0),
            solver_seed("a", "bc", "d", 0)
        );
    }

    #[tokio::test]
    async fn stage_start_records_seed_and_repeat() {
        let policy = portfolio(vec![stage("cpu-1", "good", Allocation::Remainder)]);
        let dir = tempfile::tempdir().expect("output directory");
        let runner =
            PortfolioRunner::new(dir.path().to_owned(), rules()).with_backend(fake("good", &[]));
        let budget = TrialBudget::new(Some(Duration::from_secs(10)), None).expect("budget");
        let trial = runner
            .run_trial(&instance(), &policy, &budget, 3, None)
            .await
            .expect("trial");
        let events = events(&trial);
        let start = events
            .iter()
            .find(|event| event.kind == EventKind::StageStart)
            .expect("stage start");
        assert_eq!(start.payload["repeat"], 3);
        assert_eq!(
            start.payload["solver_seed"],
            json!(format!(
                "{:016x}",
                solver_seed("toy/cpu-4", "two-spin", "cpu-1", 3)
            ))
        );
        assert_eq!(start.payload["seed_delivery"], "unavailable");
        assert_eq!(trial.identity.nonce, instance().nonce);
    }
}
