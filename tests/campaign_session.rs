// SPDX-License-Identifier: AGPL-3.0-or-later
//! Trial clocks, complete batches, persistent sessions, and campaign commands.

use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use isingmark::{
    backend::BackendSpec,
    campaign::{
        contracts::TimingStatus,
        instance::{load_instance, Instance},
        protocol::{load_protocol_profile, RuleSet},
        runner::{
            run_campaign_with_backend, CampaignConfig, PolicyManifest, Preparation, TerminalReason,
            Trial, TrialRunner,
        },
        timing::TrialBudget,
        trace::{read_trace, EventKind, TraceEvent},
        CampaignError,
    },
    params::BackendKind,
};
use serde_json::{json, Value};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evaluation/qblock")
        .join(path)
}

fn instance() -> Instance {
    load_instance(&fixture("fixtures/two_spin.instance.json")).expect("fixed instance")
}

fn policy() -> PolicyManifest {
    PolicyManifest {
        policy_id: "test-policy".into(),
        backend: "fake".into(),
        num_reads: 1,
        num_sweeps: 10,
        anneal_time_us: 20,
        host_compute_limit_s: 60.0,
        evaluation_limit: 100_000,
        preparation: Preparation::Cold,
    }
}

fn fake_miner_backend(args: &[&str]) -> BackendSpec {
    BackendSpec {
        name: "fake".into(),
        bin: PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner")),
        args: args.iter().map(ToString::to_string).collect(),
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    }
}

fn rules() -> RuleSet {
    RuleSet::Current(
        load_protocol_profile(&fixture("configs/protocol.json")).expect("pinned profile"),
    )
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

async fn run(instance: &Instance, args: &[&str], wall: Duration) -> (tempfile::TempDir, Trial) {
    let dir = tempfile::tempdir().expect("output directory");
    let mut runner = TrialRunner::new(fake_miner_backend(args), dir.path().to_owned(), rules());
    let budget = TrialBudget::new(Some(wall), Some(30_000_000_000)).expect("trial budget");
    let trial = runner
        .run_trial(instance, &policy(), &budget)
        .await
        .expect("trial");
    runner.finish().await.expect("session shutdown");
    (dir, trial)
}

#[tokio::test]
async fn multiple_batches_accumulate_device_cost() {
    let mut input = instance();
    // The fake miner supplies one fixed read and one search read per job.
    input.objective["min_solutions"] = json!(6);
    let (_dir, trial) = run(
        &input,
        &["--qpu-access-us", "1000000"],
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(trial.terminal, TerminalReason::Accepted);
    assert_eq!(trial.device_total.lower_ns, 3_000_000_000);
    assert_eq!(trial.device_total.upper_ns, Some(3_000_002_997));
    assert_eq!(trial.device_total.status, TimingStatus::Complete);
    assert_eq!(
        events(&trial)
            .iter()
            .filter(|e| e.kind == EventKind::BatchReceipt)
            .count(),
        3
    );
}

#[tokio::test]
async fn accepted_trial_without_qpu_timing_has_unresolved_device_outcome() {
    let (_dir, trial) = run(&instance(), &["--omit-meta"], Duration::from_secs(5)).await;
    assert_eq!(trial.terminal, TerminalReason::Accepted);
    assert!(trial.accepted_wall.is_some());
    assert_eq!(trial.device_total.status, TimingStatus::Missing);
    assert_eq!(trial.device_total.upper_ns, None);
    let trace = events(&trial);
    let receipt = trace
        .iter()
        .find(|event| event.kind == EventKind::BatchReceipt)
        .expect("receipt");
    assert!(receipt.payload["meta"].is_null());
    assert_eq!(
        receipt.device.expect("missing timing").status,
        TimingStatus::Missing
    );
    let termination = trace.last().expect("termination");
    assert_eq!(termination.payload["terminal"], "accepted");
    assert_eq!(termination.payload["device_outcome"], "Unresolved");
}

#[tokio::test]
async fn no_valid_witness_ends_as_timeout() {
    let mut input = instance();
    input.objective["max_energy_milli"] = json!(-1001);
    let (_dir, trial) = run(
        &input,
        &["--result-delay-ms", "100"],
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(trial.terminal, TerminalReason::WallTimeout);
    assert!(trial.accepted_wall.is_none());
    assert!(!events(&trial)
        .iter()
        .any(|e| e.kind == EventKind::ProofAcceptance));
}

#[tokio::test]
async fn delayed_batch_is_timestamped_on_arrival() {
    let (_dir, trial) = run(
        &instance(),
        &["--result-delay-ms", "300"],
        Duration::from_secs(5),
    )
    .await;
    let trace = events(&trial);
    let receipt = trace
        .iter()
        .find(|e| e.kind == EventKind::BatchReceipt)
        .expect("receipt");
    assert!(receipt.wall_ns >= 300_000_000);
    let accepted = trial.accepted_wall.expect("validated acceptance");
    assert!(accepted >= Duration::from_nanos(receipt.wall_ns));
}

#[tokio::test]
async fn crashed_miner_ends_trial_as_failure() {
    let (_dir, trial) = run(&instance(), &["--abort-after", "1"], Duration::from_secs(5)).await;
    assert_eq!(trial.terminal, TerminalReason::MinerCrashed);
    assert!(trial.accepted_wall.is_none());
    assert_eq!(trial.device_total.status, TimingStatus::Missing);
    events(&trial);
}

#[tokio::test]
async fn deadline_cancellation_is_recorded() {
    let (_dir, trial) = run(
        &instance(),
        &["--result-delay-ms", "5000"],
        Duration::from_millis(500),
    )
    .await;
    assert_eq!(trial.terminal, TerminalReason::WallTimeout);
    assert_eq!(trial.device_total.status, TimingStatus::Missing);
    assert!(trial.device_total.upper_ns.is_none());
    let trace = events(&trial);
    let cancel = trace
        .iter()
        .position(|e| e.kind == EventKind::Cancellation)
        .expect("in-flight cancellation");
    assert!(cancel < trace.len() - 1);
    assert!(trace[cancel].job_id.is_some());
    assert_eq!(
        trace.last().expect("termination").payload["terminal"],
        "wall_timeout"
    );
}

fn manifest(dir: &Path, preparation: Preparation) -> PathBuf {
    let mut policy = policy();
    policy.preparation = preparation;
    let path = dir.join("manifest.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "protocol_profile": fixture("configs/protocol.json"),
            "instances": [fixture("fixtures/two_spin.instance.json")],
            "policies": [policy],
            "trials_per_pair": 2,
            "budget": {"wall_limit_s": 5.0, "device_budget_s": 30.0}
        }))
        .expect("manifest JSON"),
    )
    .expect("write manifest");
    path
}

#[tokio::test]
async fn warm_session_is_reused() {
    let dir = tempfile::tempdir().expect("directory");
    let config = CampaignConfig {
        manifest: manifest(dir.path(), Preparation::Warm),
        backend: "fake".into(),
        output: dir.path().join("out"),
        dry_run: false,
    };
    // Restarting the process per trial would make both trials succeed.
    let report = run_campaign_with_backend(&config, fake_miner_backend(&["--abort-after", "2"]))
        .await
        .expect("campaign");
    assert_eq!(report.trials.len(), 2);
    assert_eq!(report.trials[0].terminal, TerminalReason::Accepted);
    assert_eq!(report.trials[1].terminal, TerminalReason::MinerCrashed);
    let first = events(&report.trials[0]);
    let second = events(&report.trials[1]);
    let session_id = &first[0].session_id;
    assert!(first
        .iter()
        .chain(&second)
        .all(|event| &event.session_id == session_id));
}

#[tokio::test]
async fn amortized_session_reuse_ends_at_the_horizon() {
    let dir = tempfile::tempdir().expect("directory");
    let mut runner = TrialRunner::new(
        fake_miner_backend(&["--abort-after", "3"]),
        dir.path().into(),
        rules(),
    );
    let mut policy = policy();
    policy.preparation = Preparation::Amortized { horizon: 2 };
    let budget =
        TrialBudget::new(Some(Duration::from_secs(5)), Some(30_000_000_000)).expect("budget");
    let mut sessions = Vec::new();
    for _ in 0..4 {
        let trial = runner
            .run_trial(&instance(), &policy, &budget)
            .await
            .expect("trial");
        assert_eq!(trial.terminal, TerminalReason::Accepted);
        sessions.push(events(&trial)[0].session_id.clone());
    }
    runner.finish().await.expect("shutdown");
    assert_eq!(sessions[0], sessions[1]);
    assert_ne!(sessions[1], sessions[2]);
    assert_eq!(sessions[2], sessions[3]);
}

#[tokio::test]
async fn event_writer_failure_invalidates_run() {
    let dir = tempfile::tempdir().expect("directory");
    let config = CampaignConfig {
        manifest: manifest(dir.path(), Preparation::Cold),
        backend: "fake".into(),
        output: dir.path().join("read-only"),
        dry_run: false,
    };
    fs::create_dir(&config.output).expect("output directory");
    fs::set_permissions(&config.output, fs::Permissions::from_mode(0o500)).expect("read only");
    let result = run_campaign_with_backend(&config, fake_miner_backend(&[])).await;
    fs::set_permissions(&config.output, fs::Permissions::from_mode(0o700))
        .expect("restore permissions");
    assert!(matches!(result, Err(CampaignError::InvalidRun(_))));
}

#[tokio::test]
async fn wall_only_campaign_writes_null_device_estimate() {
    let dir = tempfile::tempdir().expect("directory");
    let config = CampaignConfig {
        manifest: manifest(dir.path(), Preparation::Warm),
        backend: "fake".into(),
        output: dir.path().join("out"),
        dry_run: false,
    };
    let mut value: Value =
        serde_json::from_slice(&fs::read(&config.manifest).expect("manifest")).expect("JSON");
    value["budget"] = json!({"wall_limit_s": 5.0});
    fs::write(&config.manifest, serde_json::to_vec(&value).expect("JSON")).expect("manifest");
    let report = run_campaign_with_backend(&config, fake_miner_backend(&[]))
        .await
        .expect("campaign");
    assert_eq!(report.trials.len(), 2);
    let saved: Value =
        serde_json::from_slice(&fs::read(config.output.join("campaign.json")).expect("report"))
            .expect("JSON");
    assert_eq!(saved["resources"]["max_device_s"], Value::Null);
    assert_eq!(saved["trials"].as_array().expect("trials").len(), 2);
}

#[tokio::test]
async fn shutdown_failure_preserves_terminated_campaign_trials() {
    for multiple_policies in [false, true] {
        let dir = tempfile::tempdir().expect("directory");
        let config = CampaignConfig {
            manifest: manifest(dir.path(), Preparation::Warm),
            backend: "fake".into(),
            output: dir.path().join("out"),
            dry_run: false,
        };
        if multiple_policies {
            let mut value: Value =
                serde_json::from_slice(&fs::read(&config.manifest).expect("manifest"))
                    .expect("JSON");
            let mut second = value["policies"][0].clone();
            second["policy_id"] = json!("second-policy");
            value["policies"]
                .as_array_mut()
                .expect("policies")
                .push(second);
            fs::write(&config.manifest, serde_json::to_vec(&value).expect("JSON"))
                .expect("manifest");
        }
        let error = run_campaign_with_backend(&config, fake_miner_backend(&["--fail-on-shutdown"]))
            .await
            .expect_err("shutdown fault");
        assert!(
            matches!(error, CampaignError::InvalidRun(ref reason) if reason.contains("shutdown"))
        );
        let saved: Value = serde_json::from_slice(
            &fs::read(config.output.join("campaign.json")).expect("retained report"),
        )
        .expect("JSON");
        let trials: Vec<Trial> = serde_json::from_value(saved["trials"].clone()).expect("trials");
        assert_eq!(trials.len(), 2);
        for trial in trials {
            assert_eq!(trial.terminal, TerminalReason::Accepted);
            events(&trial);
        }
    }
}

#[test]
fn wall_only_dry_run_prints_null_device_estimate() {
    let dir = tempfile::tempdir().expect("directory");
    let path = manifest(dir.path(), Preparation::Cold);
    let mut value: Value =
        serde_json::from_slice(&fs::read(&path).expect("manifest")).expect("JSON");
    value["budget"] = json!({"wall_limit_s": 5.0});
    fs::write(&path, serde_json::to_vec(&value).expect("JSON")).expect("manifest");
    let output = Command::new(env!("CARGO_BIN_EXE_isingmark"))
        .args(["campaign", "--manifest"])
        .arg(path)
        .args(["--backend", "fake", "--output"])
        .arg(dir.path().join("out"))
        .arg("--dry-run")
        .output()
        .expect("dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let estimate: Value = serde_json::from_slice(&output.stdout).expect("estimate");
    assert_eq!(estimate["max_wall_s"], 10.0);
    assert_eq!(estimate["max_device_s"], Value::Null);
}

#[tokio::test]
async fn serve_serializes_submits_and_reuses_prepared_sessions() {
    use isingmark::campaign::runner::serve_campaign;
    for preparation in [Preparation::Warm, Preparation::Amortized { horizon: 2 }] {
        let dir = tempfile::tempdir().expect("directory");
        let config = CampaignConfig {
            manifest: manifest(dir.path(), preparation),
            backend: env!("CARGO_BIN_EXE_isingmark-fake-miner").into(),
            output: dir.path().join("out"),
            dry_run: false,
        };
        let mut value: Value =
            serde_json::from_slice(&fs::read(&config.manifest).expect("manifest")).expect("JSON");
        value["policies"][0]["backend"] = json!(config.backend);
        fs::write(&config.manifest, serde_json::to_vec(&value).expect("JSON")).expect("manifest");
        let mut commands: Vec<Value> = (0..3).map(|id| json!({"cmd": "submit", "instance_id": "two-spin", "policy_id": "test-policy", "trial_id": format!("trial-{id}")})).collect();
        commands.push(json!({"cmd": "finish"}));
        let input = commands
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let mut output = Vec::new();
        serve_campaign(&config, std::io::Cursor::new(input), &mut output)
            .await
            .expect("serve");
        let replies: Vec<Value> = std::str::from_utf8(&output)
            .expect("UTF-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("reply"))
            .collect();
        assert_eq!(replies.len(), 4);
        assert_eq!(replies[3]["errors"], json!([]));
        let mut trials: Vec<Trial> =
            serde_json::from_value(replies[3]["trials"].clone()).expect("trials");
        assert_eq!(trials.len(), 3);
        trials.sort_by_key(|trial| trial.started);
        let sessions: Vec<_> = trials
            .iter()
            .map(|trial| events(trial)[0].session_id.clone())
            .collect();
        assert_eq!(sessions[0], sessions[1]);
        if preparation == Preparation::Warm {
            assert_eq!(sessions[1], sessions[2]);
        } else {
            assert_ne!(sessions[1], sessions[2]);
        }
        for pair in trials.windows(2) {
            assert!(pair[1].started >= pair[0].started + pair[0].accepted_wall.expect("accepted"));
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Keep process-start synchronization and timeout cleanup in one fixture."
)]
async fn serve_skips_queued_trial(eof: bool) {
    use isingmark::campaign::runner::serve_campaign;

    let dir = tempfile::tempdir().expect("directory");
    let started_path = dir.path().join("started");
    assert!(Command::new("mkfifo")
        .arg(&started_path)
        .status()
        .expect("startup notification FIFO")
        .success());
    let mut started = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&started_path)
        .expect("startup notification");
    let mut observer = started.try_clone().expect("startup observer");
    let mut notification = tokio::task::spawn_blocking(move || observer.read_exact(&mut [0]));
    let starts = dir.path().join("starts");
    let backend = dir.path().join("miner");
    fs::write(
        &backend,
        format!(
            "#!/bin/bash\nset -euo pipefail\nprintf 'start\\n' >>'{}'\nprintf s >'{}'\nexec '{}' --result-delay-ms 1000 \"$@\"\n",
            starts.display(), started_path.display(), env!("CARGO_BIN_EXE_isingmark-fake-miner")
        ),
    )
    .expect("miner wrapper");
    fs::set_permissions(&backend, fs::Permissions::from_mode(0o700)).expect("executable miner");
    let config = CampaignConfig {
        manifest: manifest(dir.path(), Preparation::Cold),
        backend: backend.display().to_string(),
        output: dir.path().join("out"),
        dry_run: false,
    };
    let mut value: Value = serde_json::from_slice(&fs::read(&config.manifest).expect("manifest"))
        .expect("manifest JSON");
    value["policies"][0]["backend"] = json!(config.backend);
    fs::write(&config.manifest, serde_json::to_vec(&value).expect("JSON")).expect("manifest");
    let output_dir = config.output.clone();
    let (input, mut commands) = std::io::pipe().expect("command pipe");
    let server = tokio::spawn(async move {
        let mut output = Vec::new();
        let result = serve_campaign(&config, std::io::BufReader::new(input), &mut output).await;
        (result, output)
    });
    writeln!(commands, "{}", json!({"cmd": "submit", "instance_id": "two-spin", "policy_id": "test-policy", "trial_id": "first"})).expect("first submit");
    // Observe a real process start while the first trial holds the runner lock.
    let startup = tokio::time::timeout(Duration::from_secs(5), &mut notification).await;
    if startup.is_err() {
        drop(commands);
        started.write_all(b"x").expect("unblock startup observer");
        notification
            .await
            .expect("observer task")
            .expect("observer");
        let (result, output) = server.await.expect("serve task");
        result.expect("serve startup and EOF cleanup");
        let trace = fs::read_to_string(output_dir.join("first/events.jsonl"))
            .expect("startup failure trace");
        assert!(
            startup.is_ok(),
            "miner did not start: {}\n{trace}",
            String::from_utf8_lossy(&output)
        );
        return;
    }
    startup
        .expect("startup notification")
        .expect("observer task")
        .expect("miner start");
    let first = read_trace(&output_dir.join("first/events.jsonl")).expect("first challenge");
    assert!(first
        .events
        .iter()
        .any(|event| event.kind == EventKind::ChallengeRelease));
    assert!(!first
        .events
        .iter()
        .any(|event| event.kind == EventKind::TrialTermination));
    writeln!(commands, "{}", json!({"cmd": "submit", "instance_id": "two-spin", "policy_id": "test-policy", "trial_id": "second"})).expect("second submit");
    if !eof {
        writeln!(
            commands,
            "{}",
            json!({"cmd": "cancel_trial", "trial_id": "second"})
        )
        .expect("cancel queued trial");
        writeln!(commands, "{}", json!({"cmd": "finish"})).expect("finish");
    }
    drop(commands);
    let (result, output) = tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("serve completes")
        .expect("serve task");
    result.expect("cancellation is not an error");
    let replies: Vec<Value> = std::str::from_utf8(&output)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("reply"))
        .collect();
    assert!(replies.contains(&json!({"event": "cancelled_before_start", "trial_id": "second"})));
    assert!(!replies.iter().any(|reply| reply["event"] == "error"));
    assert!(
        !output_dir.join("second").exists(),
        "no trace or ChallengeRelease"
    );
    assert_eq!(
        fs::read_to_string(starts)
            .expect("miner starts")
            .lines()
            .count(),
        1
    );
    if !eof {
        let finished = replies.last().expect("finish reply");
        assert_eq!(finished["event"], "finished");
        assert_eq!(finished["errors"], json!([]));
        assert_eq!(finished["trials"].as_array().expect("trials").len(), 1);
        assert_eq!(finished["trials"][0]["trial_id"], "first");
    }
}

#[tokio::test]
async fn serve_cancel_trial_skips_queued_cold_trial() {
    serve_skips_queued_trial(false).await;
}

#[tokio::test]
async fn serve_eof_skips_queued_cold_trial() {
    serve_skips_queued_trial(true).await;
}

#[test]
fn dry_run_prints_resources_without_submitting() {
    let dir = tempfile::tempdir().expect("directory");
    let path = manifest(dir.path(), Preparation::Cold);
    let mut value: Value =
        serde_json::from_slice(&fs::read(&path).expect("manifest")).expect("JSON");
    value["policies"][0]["backend"] = json!("/no/such/miner");
    fs::write(&path, serde_json::to_vec(&value).expect("manifest JSON")).expect("write manifest");
    let output = Command::new(env!("CARGO_BIN_EXE_isingmark"))
        .args(["campaign", "--manifest"])
        .arg(path)
        .args(["--backend", "/no/such/miner", "--output"])
        .arg(dir.path().join("out"))
        .arg("--dry-run")
        .output()
        .expect("dry-run command");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let estimate: Value = serde_json::from_slice(&output.stdout).expect("one JSON estimate");
    assert_eq!(estimate["trials"], 2);
    assert_eq!(estimate["max_wall_s"], 10.0);
    assert_eq!(estimate["max_device_s"], 60.0);
    assert!(!dir.path().join("out").exists());
}

#[test]
fn serve_rejects_oversized_line() {
    let dir = tempfile::tempdir().expect("directory");
    let mut child = Command::new(env!("CARGO_BIN_EXE_isingmark"))
        .args(["campaign", "serve", "--manifest"])
        .arg(fixture("fixtures/campaign/fixed.manifest.json"))
        .args(["--backend", "fake", "--output"])
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("serve process");
    let mut stdin = child.stdin.take().expect("stdin pipe");
    stdin
        .write_all(&vec![b'x'; 1024 * 1024 + 1])
        .expect("oversized input");
    stdin
        .write_all(b"\n{\"cmd\":\"unknown\"}\n{\"cmd\":\"finish\"}\n")
        .expect("remaining commands");
    drop(stdin);
    let output = child.wait_with_output().expect("serve output");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("event JSON"))
        .collect();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["event"], "error");
    assert_eq!(replies[1]["event"], "error");
    assert_eq!(replies[2]["event"], "finished");
    assert_eq!(replies[2]["trials"], json!([]));
}

#[tokio::test]
async fn serve_verify_set_uses_pinned_rules_without_starting_a_miner() {
    use isingmark::campaign::runner::serve_campaign;
    let dir = tempfile::tempdir().expect("directory");
    let config = CampaignConfig {
        manifest: fixture("fixtures/campaign/fixed.manifest.json"),
        backend: "fake".into(),
        output: dir.path().join("out"),
        dry_run: false,
    };
    let commands = [
        json!({"cmd": "verify_set", "instance_id": "two-spin", "witnesses": [{
            "instance_id": "two-spin", "spins": [1, -1], "design": null, "route": null, "controls": null}]}),
        json!({"cmd": "verify_set", "instance_id": "two-spin", "witnesses": [{
            "instance_id": "two-spin", "spins": [1, 1], "design": null, "route": null, "controls": null}]}),
        json!({"cmd": "submit", "instance_id": "missing", "policy_id": "fixed-control"}),
        json!({"cmd": "cancel_trial", "trial_id": "missing"}),
        json!({"cmd": "finish"}),
    ];
    let input = commands
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let mut output = Vec::new();
    serve_campaign(&config, std::io::Cursor::new(input), &mut output)
        .await
        .expect("serve");
    let replies: Vec<Value> = std::str::from_utf8(&output)
        .expect("UTF-8")
        .lines()
        .map(|line| serde_json::from_str(line).expect("event"))
        .collect();
    assert_eq!(replies.len(), 5);
    assert!(replies[0]["decision"]["Rejected"].is_object());
    assert!(replies[1]["decision"]["Accepted"].is_object());
    assert_eq!(replies[2]["event"], "error");
    assert_eq!(replies[3]["event"], "error");
    assert_eq!(replies[4]["trials"], json!([]));
    assert!(!config.output.exists());
}

#[test]
fn campaign_run_flags_conflict_with_serve() {
    use clap::Parser;
    use isingmark::cli::Cli;
    assert!(Cli::try_parse_from([
        "isingmark",
        "campaign",
        "--dry-run",
        "serve",
        "--manifest",
        "m.json",
        "--backend",
        "fake",
        "--output",
        "out"
    ])
    .is_err());
    assert!(Cli::try_parse_from(["isingmark", "campaign"]).is_err());
}

#[tokio::test]
async fn late_batch_after_cancellation_keeps_its_full_charge() {
    let (_dir, trial) = run(
        &instance(),
        &["--result-delay-ms", "800", "--qpu-access-us", "1000000"],
        Duration::from_millis(400),
    )
    .await;
    assert_eq!(trial.terminal, TerminalReason::WallTimeout);
    assert!(trial.accepted_wall.expect("late proof") > Duration::from_millis(400));
    assert_eq!(trial.device_total.lower_ns, 1_000_000_000);
    assert_eq!(trial.device_total.status, TimingStatus::Complete);
    let trace = events(&trial);
    let cancel = trace
        .iter()
        .position(|event| event.kind == EventKind::Cancellation)
        .expect("cancel");
    let receipt = trace
        .iter()
        .position(|event| event.kind == EventKind::BatchReceipt)
        .expect("late receipt");
    assert!(cancel < receipt);
    assert!(trace
        .iter()
        .any(|event| event.kind == EventKind::ProofAcceptance));
}
