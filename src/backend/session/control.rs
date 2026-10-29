// SPDX-License-Identifier: AGPL-3.0-or-later
//! Opt-in live dispatch using the existing session handshake and capture decoder.

use std::{path::PathBuf, sync::atomic::AtomicBool};

use super::{
    abandon_in_flight, build_batch_capture, build_reject_record, build_result_record,
    build_topology, coord_msg, edge_positions, miner_msg, push_in_flight,
    run_session_inner_with_capture, send, take_front_in_flight, Arc, BatchCapture, CoordMsg,
    Duration, HashMap, Instant, Job, MinerMsg, Ordering, RunParams, RunReport, SessionError,
    SessionProgress, SetTarget, Shutdown, Status, Streaming, TopologySpec, DISPATCH_SEND_TIMEOUT,
    MAX_DISPATCH_CREDITS_PER_REQUEST, MAX_MISSED_PINGS, PING_INTERVAL, SHUTDOWN_GRACE_MS,
};
use tokio::{sync::mpsc, task::AbortHandle};

pub(crate) struct Submission {
    pub job: Job,
    pub spec: TopologySpec,
    pub target: SetTarget,
}

pub(crate) enum SessionCommand {
    Submit(Box<Submission>),
    Finish,
}

pub(crate) enum SessionEvent {
    Ready,
    Dispatched {
        job_id: Vec<u8>,
        at: Instant,
    },
    Batch {
        capture: BatchCapture,
        at: Instant,
    },
    Rejected {
        job_id: Vec<u8>,
        reason: String,
        at: Instant,
    },
    Failed {
        reason: String,
        at: Instant,
    },
}

pub(crate) struct SessionControl {
    pub commands: mpsc::Receiver<SessionCommand>,
    pub events: mpsc::Sender<SessionEvent>,
    pub failed: Arc<AtomicBool>,
}

impl SessionControl {
    fn emit(&self, event: SessionEvent) -> bool {
        if self.events.try_send(event).is_err() {
            self.failed.store(true, Ordering::Release);
            return false;
        }
        true
    }
}

pub(crate) async fn run_controlled(
    params: RunParams<'_>,
    control: SessionControl,
) -> Result<RunReport, SessionError> {
    run_session_inner_with_capture(params, MAX_MISSED_PINGS, None, Some(control)).await
}

// Dropping a timed-out campaign must also stop its server and stderr reader.
pub(super) struct SessionCleanup {
    socket: PathBuf,
    pub tasks: Vec<AbortHandle>,
}

impl SessionCleanup {
    pub fn new(socket: &std::path::Path, server: AbortHandle) -> Self {
        Self {
            socket: socket.to_owned(),
            tasks: vec![server],
        }
    }
}

impl Drop for SessionCleanup {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        let _ = std::fs::remove_file(&self.socket);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "Keep the live transport state transitions in one protocol loop."
)]
pub(super) async fn drive_controlled(
    mut inbound: Streaming<MinerMsg>,
    tx: mpsc::Sender<Result<CoordMsg, Status>>,
    mut control: SessionControl,
    backend: &str,
    progress: &SessionProgress,
) {
    if !control.emit(SessionEvent::Ready) {
        return;
    }
    let mut pending: Option<Box<Submission>> = None;
    let mut active: Option<(TopologySpec, SetTarget)> = None;
    let mut in_flight = HashMap::new();
    let mut credits = 0_u32;
    let mut missed = 0_u32;
    loop {
        if credits > 0 && in_flight.is_empty() {
            if let Some(submission) = pending.take() {
                let Submission { job, spec, target } = *submission;
                let messages = [
                    coord_msg::Msg::Topology(build_topology(&spec)),
                    coord_msg::Msg::SetTarget(target),
                ];
                let mut failed = false;
                for message in messages {
                    if !matches!(
                        tokio::time::timeout(DISPATCH_SEND_TIMEOUT, send(&tx, message)).await,
                        Ok(Ok(()))
                    ) {
                        failed = true;
                        break;
                    }
                }
                if failed {
                    break;
                }
                let at = Instant::now();
                if !control.emit(SessionEvent::Dispatched {
                    job_id: job.job_id.clone(),
                    at,
                }) {
                    break;
                }
                let outbound = coord_msg::Msg::Job(job.clone());
                push_in_flight(&mut in_flight, job.job_id.clone(), at, job);
                active = Some((spec, target));
                credits -= 1;
                if !matches!(
                    tokio::time::timeout(DISPATCH_SEND_TIMEOUT, send(&tx, outbound)).await,
                    Ok(Ok(()))
                ) {
                    break;
                }
            }
        }
        tokio::select! {
            command = control.commands.recv() => match command {
                Some(SessionCommand::Submit(submission)) => {
                    // The campaign runner submits serially. Violations invalidate capture.
                    if pending.is_some() || !in_flight.is_empty() {
                        control.failed.store(true, Ordering::Release);
                        break;
                    }
                    pending = Some(submission);
                }
                Some(SessionCommand::Finish) | None => break,
            },
            message = tokio::time::timeout(PING_INTERVAL, inbound.message()) => {
                let message = match message {
                    Ok(Ok(Some(message))) => { missed = 0; message }
                    Ok(Ok(None) | Err(_)) => break,
                    Err(_) => {
                        missed += 1;
                        if missed > MAX_MISSED_PINGS {
                            progress.set_unresponsive();
                            break;
                        }
                        if send(&tx, coord_msg::Msg::Ping(super::Ping {})).await.is_err() { break; }
                        continue;
                    }
                };
                match message.msg {
                    Some(miner_msg::Msg::JobRequest(request)) => {
                        credits = credits.saturating_add(request.credits).min(MAX_DISPATCH_CREDITS_PER_REQUEST);
                    }
                    Some(miner_msg::Msg::Result(result)) => {
                        if !receive_result(&result, active.as_ref(), &mut in_flight, &control, backend, progress) { break; }
                    }
                    Some(miner_msg::Msg::Reject(reject)) => {
                        if !receive_reject(&reject, active.as_ref(), &mut in_flight, &control, backend, progress) { break; }
                    }
                    Some(miner_msg::Msg::Fatal(fatal)) => {
                        control.emit(SessionEvent::Failed { reason: format!("{fatal:?}"), at: Instant::now() });
                        break;
                    }
                    Some(miner_msg::Msg::Hello(_) | miner_msg::Msg::Ready(_) | miner_msg::Msg::Status(_)
                        | miner_msg::Msg::Capabilities(_) | miner_msg::Msg::LeaseDone(_)) | None => {}
                }
            }
        }
    }
    let _ = send(
        &tx,
        coord_msg::Msg::Shutdown(Shutdown {
            grace_ms: SHUTDOWN_GRACE_MS,
        }),
    )
    .await;
    // Preserve late full batches during the existing shutdown grace. An
    // unanswered launch remains unresolved when that reconciliation ends.
    let _ = tokio::time::timeout(Duration::from_millis(u64::from(SHUTDOWN_GRACE_MS)), async {
        while let Ok(Some(message)) = inbound.message().await {
            let delivered = match message.msg {
                Some(miner_msg::Msg::Result(result)) => receive_result(
                    &result,
                    active.as_ref(),
                    &mut in_flight,
                    &control,
                    backend,
                    progress,
                ),
                Some(miner_msg::Msg::Reject(reject)) => receive_reject(
                    &reject,
                    active.as_ref(),
                    &mut in_flight,
                    &control,
                    backend,
                    progress,
                ),
                Some(miner_msg::Msg::Fatal(fatal)) => control.emit(SessionEvent::Failed {
                    reason: format!("{fatal:?}"),
                    at: Instant::now(),
                }),
                Some(
                    miner_msg::Msg::JobRequest(_)
                    | miner_msg::Msg::Hello(_)
                    | miner_msg::Msg::Ready(_)
                    | miner_msg::Msg::Status(_)
                    | miner_msg::Msg::Capabilities(_)
                    | miner_msg::Msg::LeaseDone(_),
                )
                | None => true,
            };
            if !delivered {
                break;
            }
        }
    })
    .await;
    abandon_in_flight(
        &mut in_flight,
        progress,
        backend,
        active.as_ref().map(|(_, target)| target),
        "controlled session ended with jobs in flight",
    );
}

fn receive_result(
    result: &super::JobResult,
    active: Option<&(TopologySpec, SetTarget)>,
    in_flight: &mut HashMap<Vec<u8>, super::VecDeque<(Instant, Job)>>,
    control: &SessionControl,
    backend: &str,
    progress: &SessionProgress,
) -> bool {
    let at = Instant::now();
    let Some((start, job)) = take_front_in_flight(in_flight, &result.job_id) else {
        progress.note_orphan_result();
        return true;
    };
    let Some((spec, target)) = active else {
        return false;
    };
    let edges = edge_positions(spec);
    let capture = build_batch_capture(&job, result, &edges, at.duration_since(start));
    progress.push_record(build_result_record(
        backend,
        &job,
        result,
        &edges,
        Some(target),
        at.duration_since(start),
    ));
    control.emit(SessionEvent::Batch { capture, at })
}

fn receive_reject(
    reject: &super::Reject,
    active: Option<&(TopologySpec, SetTarget)>,
    in_flight: &mut HashMap<Vec<u8>, super::VecDeque<(Instant, Job)>>,
    control: &SessionControl,
    backend: &str,
    progress: &SessionProgress,
) -> bool {
    let at = Instant::now();
    let Some((start, _)) = take_front_in_flight(in_flight, &reject.job_id) else {
        progress.note_orphan_result();
        return true;
    };
    progress.push_record(build_reject_record(
        backend,
        reject,
        active.map(|(_, target)| target),
        at.duration_since(start),
    ));
    control.emit(SessionEvent::Rejected {
        job_id: reject.job_id.clone(),
        reason: super::reject_reason_text(reject.reason),
        at,
    })
}
