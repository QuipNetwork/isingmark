// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded JSON-lines control with explicit trial cancellation outcomes.

use std::{
    io::{BufRead, Write},
    sync::Arc,
};

use tokio::sync::Mutex;

use super::{
    invalid, json, load_campaign, mpsc, new_id, oneshot, resolve_backend, BTreeMap, CampaignConfig,
    CampaignError, Deserialize, Instance, JoinHandle, PolicyManifest, RuleSet, Trial, TrialBudget,
    TrialRunner, Value, Witness,
};

const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    Submit {
        instance_id: String,
        policy_id: String,
        trial_id: Option<String>,
    },
    VerifySet {
        instance_id: String,
        witnesses: Vec<Witness>,
    },
    CancelTrial {
        trial_id: String,
    },
    Finish,
}

struct RunningTrial {
    cancel: oneshot::Sender<()>,
    task: JoinHandle<Result<Option<Trial>, CampaignError>>,
}

// Drain an oversized line through its newline without retaining the excess.
fn read_command(reader: &mut impl BufRead) -> Result<Option<Command>, CampaignError> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            if line.is_empty() && !oversized {
                return Ok(None);
            }
            break;
        }
        let end = buffer.iter().position(|&byte| byte == b'\n');
        let bytes = end.unwrap_or(buffer.len());
        if !oversized {
            if bytes > MAX_LINE_BYTES.saturating_sub(line.len()) {
                oversized = true;
            } else {
                line.extend_from_slice(&buffer[..bytes]);
            }
        }
        reader.consume(bytes + usize::from(end.is_some()));
        if end.is_some() {
            break;
        }
    }
    if oversized {
        return Err(invalid("command exceeds the 1 MiB line limit"));
    }
    Ok(Some(serde_json::from_slice(&line)?))
}

fn write_event(writer: &mut impl Write, event: &Value) -> Result<(), CampaignError> {
    serde_json::to_writer(&mut *writer, event).map_err(invalid)?;
    writer.write_all(b"\n").map_err(invalid)?;
    writer.flush().map_err(invalid)
}

fn find_instance<'a>(instances: &'a [Instance], id: &str) -> Result<&'a Instance, CampaignError> {
    instances
        .iter()
        .find(|instance| instance.instance_id == id)
        .ok_or_else(|| invalid("unknown instance_id"))
}

fn find_policy<'a>(
    policies: &'a [PolicyManifest],
    id: &str,
) -> Result<&'a PolicyManifest, CampaignError> {
    policies
        .iter()
        .find(|policy| policy.policy_id == id)
        .ok_or_else(|| invalid("unknown policy_id"))
}

async fn collect(running: RunningTrial, cancel: bool) -> Result<Option<Trial>, CampaignError> {
    if cancel {
        let _ = running.cancel.send(());
    }
    running.task.await.map_err(invalid)?
}

async fn cancel_outstanding(
    active: BTreeMap<String, RunningTrial>,
    mut result: Result<(), CampaignError>,
    writer: &mut impl Write,
) -> Result<(), CampaignError> {
    let mut tasks = Vec::new();
    for (id, running) in active {
        let _ = running.cancel.send(());
        tasks.push((id, running.task));
    }
    for (id, task) in tasks {
        // Preserve termination records even if stdin or stdout fails.
        let cleanup = match task.await {
            Ok(Ok(Some(_))) => Ok(()),
            Ok(Ok(None)) => write_event(
                writer,
                &json!({"event": "cancelled_before_start", "trial_id": id}),
            ),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(invalid(error)),
        };
        if result.is_ok() {
            result = cleanup;
        }
    }
    result
}

async fn run_serialized(
    shared: &Mutex<(Option<String>, TrialRunner)>,
    instance: &Instance,
    policy: &PolicyManifest,
    budget: &TrialBudget,
    mut cancelled: oneshot::Receiver<()>,
    id: String,
) -> Result<Option<Trial>, CampaignError> {
    // Hold the lock through termination so no two trials use a live session.
    let mut shared = shared.lock().await;
    match cancelled.try_recv() {
        Ok(()) | Err(oneshot::error::TryRecvError::Closed) => return Ok(None),
        Err(oneshot::error::TryRecvError::Empty) => {}
    }
    let (policy_id, runner) = &mut *shared;
    if policy_id.as_ref().is_some_and(|id| id != &policy.policy_id) {
        runner.finish().await?;
    }
    *policy_id = Some(policy.policy_id.clone());
    let result = runner
        .run_cancellable(instance, policy, budget, Some(cancelled), Some(id))
        .await;
    if result.is_err() {
        runner.session.take();
    }
    result.map(Some)
}

/// Serve submit, proof verification, cancellation, and finish commands.
///
/// Submit names a manifest instance and policy, with an optional unique trial ID.
/// Verification names an instance and carries a complete `witnesses` array.
/// Finish waits for outstanding trials; EOF cancels them and retains their traces.
/// Queued cancellations reply `cancelled_before_start` and produce no trial or trace.
/// Submissions run one at a time on a shared runner for the selected backend.
/// Warm and amortized preparation spans submissions within the same policy.
/// A policy change ends the previous prepared session before the next trial.
///
/// # Errors
/// Returns invalid manifest data, input I/O failures, or output writer failures.
pub async fn serve_campaign<R: BufRead + Send + 'static, W: Write>(
    config: &CampaignConfig,
    mut reader: R,
    mut writer: W,
) -> Result<(), CampaignError> {
    let loaded = load_campaign(config)?;
    let rules = RuleSet::Current(loaded.profile);
    let runner = Arc::new(Mutex::new((
        None::<String>,
        TrialRunner::new(
            resolve_backend(&config.backend),
            config.output.clone(),
            rules.clone(),
        ),
    )));
    let (sender, mut input) = mpsc::channel(1);
    // A dedicated stdin thread can end independently of the async runtime.
    // spawn_blocking would make runtime shutdown wait for an open stdin forever.
    std::thread::spawn(move || loop {
        let command = read_command(&mut reader);
        let end = matches!(
            command,
            Ok(None | Some(Command::Finish)) | Err(CampaignError::Io(_))
        );
        if sender.blocking_send(command).is_err() || end {
            break;
        }
    });
    let mut active: BTreeMap<String, RunningTrial> = BTreeMap::new();
    let mut completed: BTreeMap<String, Trial> = BTreeMap::new();
    let result = async {
        while let Some(command) = input.recv().await {
            let command = match command {
                Ok(Some(command)) => command,
                Ok(None) => break,
                Err(error) => {
                    let io_failed = matches!(error, CampaignError::Io(_));
                    write_event(&mut writer, &json!({"event": "error", "message": error.to_string()}))?;
                    if io_failed { return Err(error); }
                    continue;
                }
            };
            let event: Result<Value, CampaignError> = match command {
                Command::Submit { instance_id, policy_id, trial_id } => (|| {
                    let instance = find_instance(&loaded.instances, &instance_id)?.clone();
                    let policy = find_policy(&loaded.policies, &policy_id)?.clone();
                    let trial_id = trial_id.unwrap_or_else(|| new_id("trial"));
                    if trial_id.is_empty() || !trial_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_') {
                        return Err(invalid("trial_id must contain only ASCII letters, digits, '-' or '_'"));
                    }
                    if active.contains_key(&trial_id) || completed.contains_key(&trial_id) || config.output.join(&trial_id).exists() {
                        return Err(invalid("trial_id already exists"));
                    }
                    let shared_runner = Arc::clone(&runner);
                    let budget = loaded.budget;
                    let id = trial_id.clone();
                    let (cancel, cancelled) = oneshot::channel();
                    let task = tokio::spawn(async move {
                        run_serialized(&shared_runner, &instance, &policy, &budget, cancelled, id).await
                    });
                    active.insert(trial_id.clone(), RunningTrial { cancel, task });
                    Ok(json!({"event": "submitted", "trial_id": trial_id}))
                })(),
                Command::VerifySet { instance_id, witnesses } => (|| {
                    let instance = find_instance(&loaded.instances, &instance_id)?;
                    for witness in &witnesses { witness.validate()?; }
                    let decision = rules.validate_set(instance, &witnesses)?;
                    Ok(json!({"event": "verified", "instance_id": instance_id, "decision": decision}))
                })(),
                Command::CancelTrial { trial_id } => {
                    if let Some(running) = active.remove(&trial_id) {
                        match collect(running, true).await {
                            Ok(Some(trial)) => {
                                let event = json!({"event": "cancelled", "trial": trial});
                                completed.insert(trial_id, trial);
                                Ok(event)
                            }
                            Ok(None) => Ok(json!({"event": "cancelled_before_start", "trial_id": trial_id})),
                            Err(error) => Err(error),
                        }
                    } else { Err(invalid("unknown active trial_id")) }
                }
                Command::Finish => {
                    let mut failures = Vec::new();
                    for (id, running) in std::mem::take(&mut active) {
                        match collect(running, false).await {
                            Ok(Some(trial)) => { completed.insert(id, trial); }
                            Ok(None) => write_event(&mut writer, &json!({"event": "cancelled_before_start", "trial_id": id}))?,
                            Err(error) => failures.push(json!({"trial_id": id, "error": error.to_string()})),
                        }
                    }
                    write_event(&mut writer, &json!({"event": "finished", "trials": completed.values().collect::<Vec<_>>(), "errors": failures}))?;
                    return Ok(());
                }
            };
            match event {
                Ok(event) => write_event(&mut writer, &event)?,
                Err(error) => write_event(&mut writer, &json!({"event": "error", "message": error.to_string()}))?,
            }
        }
        Ok(())
    }.await;
    let result = cancel_outstanding(active, result, &mut writer).await;
    let shutdown = runner.lock().await.1.finish().await;
    result.and(shutdown)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::{future::Future, task::Poll};

    use super::*;

    #[tokio::test]
    async fn queued_cancellation_preserves_the_prepared_session_and_policy() {
        use super::super::{LiveSession, SessionError};

        for close_sender in [false, true] {
            let dir = tempfile::tempdir().expect("directory");
            let config = CampaignConfig {
                manifest: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("evaluation/qblock/fixtures/campaign/fixed.manifest.json"),
                backend: "fake".into(),
                output: dir.path().join("out"),
                dry_run: false,
            };
            let loaded = load_campaign(&config).expect("campaign");
            let (commands, mut command_receiver) = mpsc::channel(1);
            let (_events, receiver) = mpsc::channel(1);
            let mut runner = TrialRunner::new(
                resolve_backend("fake"),
                config.output.clone(),
                RuleSet::Current(loaded.profile),
            );
            runner.session = Some(LiveSession {
                id: "prepared".into(),
                commands,
                events: receiver,
                failed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                task: tokio::spawn(async {
                    Err(SessionError::Spawn(std::io::Error::other(
                        "prepared session must remain open",
                    )))
                }),
                ready: true,
                trials: 0,
            });
            let shared = Mutex::new((Some("previous-policy".to_owned()), runner));
            let guard = shared.lock().await;
            let (cancel, cancelled) = oneshot::channel();
            let mut queued = Box::pin(run_serialized(
                &shared,
                &loaded.instances[0],
                &loaded.policies[0],
                &loaded.budget,
                cancelled,
                "queued".into(),
            ));
            assert!(
                std::future::poll_fn(|cx| Poll::Ready(queued.as_mut().poll(cx)))
                    .await
                    .is_pending()
            );
            if close_sender {
                drop(cancel);
            } else {
                cancel.send(()).expect("queued cancellation");
            }
            drop(guard);
            let result = queued.await;
            assert!(
                result.is_ok(),
                "queued cancellation is not an error: {result:?}"
            );
            assert!(result.expect("queued cancellation").is_none());
            let shared = shared.lock().await;
            assert_eq!(shared.0.as_deref(), Some("previous-policy"));
            assert_eq!(
                shared.1.session.as_ref().map(|session| session.id.as_str()),
                Some("prepared")
            );
            assert!(command_receiver.try_recv().is_err());
            assert!(!config.output.exists(), "no trace or ChallengeRelease");
        }
    }

    #[tokio::test]
    async fn eof_cancels_queued_and_running_trials_before_joining() {
        let (running_cancel, running_cancelled) = oneshot::channel();
        let running = tokio::spawn(async move {
            running_cancelled.await.expect("cancel running trial");
            Err(invalid("running cancelled"))
        });
        let (queued_cancel, queued_cancelled) = oneshot::channel();
        let queued = tokio::spawn(async move {
            queued_cancelled.await.expect("cancel queued trial");
            // The queued trial cannot finish until the current session user releases it.
            running.await.expect("running task")
        });
        let active = BTreeMap::from([
            (
                "a-queued".into(),
                RunningTrial {
                    cancel: queued_cancel,
                    task: queued,
                },
            ),
            (
                "z-running".into(),
                RunningTrial {
                    cancel: running_cancel,
                    task: tokio::spawn(async { Err(invalid("running collected")) }),
                },
            ),
        ]);
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            cancel_outstanding(active, Ok(()), &mut Vec::new()),
        )
        .await;
        assert!(
            result.is_ok(),
            "EOF must cancel the session user before waiting for the queued trial"
        );
    }

    #[tokio::test]
    async fn eof_cleanup_awaits_every_trial_and_returns_the_first_error() {
        for (result, expected) in [
            (Ok(()), "invalid run: first trial failed"),
            (Err(invalid("input failed")), "invalid run: input failed"),
        ] {
            let completed = Arc::new(AtomicUsize::new(0));
            let mut active = BTreeMap::new();
            for id in ["first", "second"] {
                let completed = Arc::clone(&completed);
                let (cancel, cancelled) = oneshot::channel();
                let task = tokio::spawn(async move {
                    cancelled.await.expect("EOF cancellation");
                    completed.fetch_add(1, Ordering::SeqCst);
                    Err(invalid(format!("{id} trial failed")))
                });
                active.insert(id.into(), RunningTrial { cancel, task });
            }
            let error = cancel_outstanding(active, result, &mut Vec::new())
                .await
                .expect_err("first failure");
            assert_eq!(completed.load(Ordering::SeqCst), 2);
            assert_eq!(error.to_string(), expected);
        }
    }
}
