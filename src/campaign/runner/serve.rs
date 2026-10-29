// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded JSON-lines control. Each input command receives exactly one reply.

use std::io::{BufRead, Write};

use super::{
    invalid, json, load_campaign, mpsc, new_id, oneshot, resolve_backend, BTreeMap, CampaignConfig,
    CampaignError, Deserialize, Instance, JoinHandle, PolicyManifest, RuleSet, Trial, TrialRunner,
    Value, Witness,
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
    task: JoinHandle<Result<Trial, CampaignError>>,
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

async fn collect(running: RunningTrial, cancel: bool) -> Result<Trial, CampaignError> {
    if cancel {
        let _ = running.cancel.send(());
    }
    running.task.await.map_err(invalid)?
}

async fn cancel_outstanding(
    active: BTreeMap<String, RunningTrial>,
    mut result: Result<(), CampaignError>,
) -> Result<(), CampaignError> {
    for (_, running) in active {
        // Preserve termination records even if stdin or stdout fails.
        let cleanup = collect(running, true).await.map(|_| ());
        if result.is_ok() {
            result = cleanup;
        }
    }
    result
}

/// Serve submit, proof verification, cancellation, and finish commands.
///
/// Submit names a manifest instance and policy, with an optional unique trial ID.
/// Verification names an instance and carries a complete `witnesses` array.
/// Finish waits for outstanding trials; EOF cancels them and retains their traces.
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
                    let mut runner = TrialRunner::new(resolve_backend(&config.backend), config.output.clone(), rules.clone());
                    let budget = loaded.budget;
                    let id = trial_id.clone();
                    let (cancel, cancelled) = oneshot::channel();
                    let task = tokio::spawn(async move {
                        let trial = runner.run_cancellable(&instance, &policy, &budget, Some(cancelled), Some(id)).await?;
                        runner.finish().await?;
                        Ok(trial)
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
                            Ok(trial) => {
                                let event = json!({"event": "cancelled", "trial": trial});
                                completed.insert(trial_id, trial);
                                Ok(event)
                            }
                            Err(error) => Err(error),
                        }
                    } else { Err(invalid("unknown active trial_id")) }
                }
                Command::Finish => {
                    let mut failures = Vec::new();
                    for (id, running) in std::mem::take(&mut active) {
                        match collect(running, false).await {
                            Ok(trial) => { completed.insert(id, trial); }
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
    cancel_outstanding(active, result).await
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use super::*;

    #[tokio::test]
    async fn eof_cleanup_awaits_every_trial_and_returns_the_first_error() {
        for (result, expected) in [
            (Ok(()), "invalid campaign input: first trial failed"),
            (
                Err(invalid("input failed")),
                "invalid campaign input: input failed",
            ),
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
            let error = cancel_outstanding(active, result)
                .await
                .expect_err("first failure");
            assert_eq!(completed.load(Ordering::SeqCst), 2);
            assert_eq!(error.to_string(), expected);
        }
    }
}
