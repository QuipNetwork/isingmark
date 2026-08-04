// SPDX-License-Identifier: AGPL-3.0-or-later
//! Coordinator-side `MinerService.Session` loop.
//!
//! `run_session` binds a Unix socket, spawns the backend binary pointed at
//! it (`<bin> <args> --quip-coordinator unix://<sock> --miner-id
//! isingmark-0`, mirroring the arg-before-flag ordering
//! [`crate::backend::registry`] uses for `--capabilities`/`--check`), and
//! serves `MinerService.Session` for that one miner: `Hello` -> `Welcome` /
//! `Configure` / `Topology` / `SetTarget`, then `JobRequest`-driven
//! dispatch, `Result`/`Reject`/`Fatal` handling, and a final `Shutdown` once
//! every job is terminal.
//!
//! Energy is always re-scored locally with
//! [`quip_protocol::scoring::energy_milli`] against the job's own h/J and
//! this run's topology edges — never trusted from the miner's
//! `Solution.energy_milli` — per `quip_protocol::scoring`'s documented
//! contract that the coordinator always re-scores. A miner that lies about
//! its energy is caught here and flagged via [`JobRecord::passed`], though
//! [`JobRecord`] has no separate field for the miner's claimed value, so
//! only the verified, recomputed energy is persisted.

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use quip_proto::v1::miner_service_server::{MinerService, MinerServiceServer};
use quip_proto::v1::{
    coord_msg, miner_msg, Configure, CoordMsg, EdgeList, Fatal, Job, MinerMsg, Reject,
    RejectReason, Result as JobResult, SetTarget, Shutdown, Topology, Welcome,
};
use quip_protocol::scoring::{energy_milli, set_diversity};
use quip_protocol::session::{check_welcome, PROTOCOL_VERSION};
use quip_protocol::wire::{decode_i32_le, decode_spins};
use tokio::net::UnixListener;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::wrappers::{ReceiverStream, UnixListenerStream};
use tonic::{Request, Response, Status, Streaming};

use crate::backend::registry::BackendSpec;
use crate::record::JobRecord;
use crate::topology::TopologySpec;

/// Miner id this harness always advertises to the backend it spawns.
const MINER_ID: &str = "isingmark-0";

/// Placeholder session token. `Hello.session_token` exists on the wire for
/// future auth support; this crate does not implement token issuance or
/// validation, so every run hands the backend the same fixed, non-secret
/// value purely to satisfy `quip_protocol::session::build_hello`'s
/// non-empty-token requirement on the miner side.
const SESSION_TOKEN: &str = "isingmark-benchmark-session";

/// Depth of the outbound `CoordMsg` channel. Sized like the reference
/// miner's own outbound channel (`quip-mock-miner`, `mpsc::channel(16)`):
/// control traffic only, one message per job event, never a backlog.
const OUTBOUND_CHANNEL_DEPTH: usize = 16;

/// Grace period advertised on `Shutdown`, and reused as the bound on the
/// post-shutdown inbound drain and on waiting for the child to exit after
/// the session loop ends.
const SHUTDOWN_GRACE_MS: u32 = 2_000;

/// Upper bound on waiting for the miner to complete the session protocol.
/// Generous: a real annealing backend spends real wall-clock time per job.
/// This only bites when the miner never dials in or the protocol stalls
/// entirely, so a hung/unreachable backend does not hang the caller forever.
const SESSION_TIMEOUT: Duration = Duration::from_mins(2);

/// Inputs to [`run_session`].
pub struct RunParams<'a> {
    /// Backend to spawn and drive.
    pub backend: &'a BackendSpec,
    /// Topology this run's jobs were drawn against; broadcast to the miner
    /// as `Topology` and used to resolve edges for local re-scoring.
    pub spec: &'a TopologySpec,
    /// Jobs to dispatch, in dispatch-priority order. Not every job is
    /// guaranteed a [`JobRecord`]: a job still queued when the miner sends
    /// `Fatal` or the session ends early is simply never attempted.
    pub jobs: Vec<Job>,
    /// Path to bind the coordinator's `UnixListener` at.
    pub sock_path: &'a Path,
    /// Difficulty/sampling target to send as `SetTarget` after `Topology`.
    /// `None` sends an all-zero (fully "unset") `SetTarget`, deferring
    /// every sampling parameter to the miner's own adapt logic.
    pub target: Option<SetTarget>,
}

/// Outcome of one [`run_session`] call.
pub struct RunReport {
    /// Whether the miner's `Hello` arrived and matched
    /// [`quip_protocol::session::PROTOCOL_VERSION`]. `false` means no job
    /// was ever dispatched, so `records` is always empty in that case.
    pub handshake_ok: bool,
    /// One record per job that reached a terminal state (`Result`,
    /// `Reject`, or abandoned in-flight on `Fatal`).
    pub records: Vec<JobRecord>,
    /// The miner subprocess's exit code, or `-1` if it was terminated by a
    /// signal or had to be killed after not exiting within the shutdown
    /// grace period.
    pub miner_exit_code: i32,
    /// Total wall time of the call, in milliseconds.
    pub run_wall_ms: u64,
}

/// Errors from [`run_session`] itself (setup failures outside the protocol
/// loop; protocol-level failures are reported through [`RunReport`]
/// instead, since a miner that misbehaves mid-session is an expected,
/// benchmarkable outcome rather than a harness error).
#[derive(Debug)]
pub enum SessionError {
    /// Failed to bind the coordinator's `UnixListener` at `sock_path`.
    Bind(std::io::Error),
    /// Failed to spawn the backend binary as a subprocess.
    Spawn(std::io::Error),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bind(e) => write!(f, "failed to bind coordinator socket: {e}"),
            Self::Spawn(e) => write!(f, "failed to spawn miner backend: {e}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// Drive one miner session end to end: bind, spawn, serve, and report.
///
/// # Errors
///
/// Returns [`SessionError::Bind`] if `p.sock_path` cannot be bound, or
/// [`SessionError::Spawn`] if `p.backend.bin` cannot be spawned. Protocol
/// failures (bad handshake, miner `Fatal`, miner never connecting) are not
/// errors here: they show up as `handshake_ok: false` and/or a non-zero
/// `miner_exit_code` in [`RunReport`].
pub async fn run_session(p: RunParams<'_>) -> Result<RunReport, SessionError> {
    let start = Instant::now();
    let listener = UnixListener::bind(p.sock_path).map_err(SessionError::Bind)?;
    let incoming = UnixListenerStream::new(listener);

    let (outcome_tx, outcome_rx) = oneshot::channel();
    let svc = CoordinatorSvc::new(
        p.backend.name.clone(),
        p.spec.clone(),
        p.target,
        p.jobs,
        outcome_tx,
    );
    let server_task = tokio::spawn(
        tonic::transport::Server::builder()
            .add_service(MinerServiceServer::new(svc))
            .serve_with_incoming(incoming),
    );

    let spawned = Command::new(&p.backend.bin)
        .args(&p.backend.args)
        .arg("--quip-coordinator")
        .arg(format!("unix://{}", p.sock_path.display()))
        .arg("--miner-id")
        .arg(MINER_ID)
        .env("QUIP_SESSION_TOKEN", SESSION_TOKEN)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn();

    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => {
            server_task.abort();
            let _ = std::fs::remove_file(p.sock_path);
            return Err(SessionError::Spawn(e));
        }
    };

    let outcome = match tokio::time::timeout(SESSION_TIMEOUT, outcome_rx).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(_)) | Err(_) => SessionOutcome::default(),
    };

    let shutdown_wait = Duration::from_millis(u64::from(SHUTDOWN_GRACE_MS) * 3);
    let miner_exit_code = match tokio::time::timeout(shutdown_wait, child.wait()).await {
        Ok(Ok(status)) => status.code().unwrap_or(-1),
        Ok(Err(_)) => -1,
        Err(_) => {
            let _ = child.kill().await;
            -1
        }
    };

    server_task.abort();
    let _ = std::fs::remove_file(p.sock_path);

    Ok(RunReport {
        handshake_ok: outcome.handshake_ok,
        records: outcome.records,
        miner_exit_code,
        run_wall_ms: duration_ms(start.elapsed()),
    })
}

/// Result of the session protocol loop, handed back to [`run_session`]
/// through a oneshot channel once the spawned `session()` task finishes.
#[derive(Default)]
struct SessionOutcome {
    handshake_ok: bool,
    records: Vec<JobRecord>,
}

/// `MinerService` implementation backing [`run_session`]. Built fresh per
/// call and expected to serve exactly one connection (the backend spawned
/// alongside it); a second connection would find `jobs`/`outcome_tx` empty
/// (already taken) and terminate immediately with a shutdown.
struct CoordinatorSvc {
    backend_name: String,
    spec: TopologySpec,
    target: Option<SetTarget>,
    jobs: Mutex<Option<VecDeque<Job>>>,
    outcome_tx: Mutex<Option<oneshot::Sender<SessionOutcome>>>,
}

impl CoordinatorSvc {
    fn new(
        backend_name: String,
        spec: TopologySpec,
        target: Option<SetTarget>,
        jobs: Vec<Job>,
        outcome_tx: oneshot::Sender<SessionOutcome>,
    ) -> Self {
        Self {
            backend_name,
            spec,
            target,
            jobs: Mutex::new(Some(jobs.into_iter().collect())),
            outcome_tx: Mutex::new(Some(outcome_tx)),
        }
    }
}

#[tonic::async_trait]
impl MinerService for CoordinatorSvc {
    type SessionStream = ReceiverStream<Result<CoordMsg, Status>>;

    async fn session(
        &self,
        request: Request<Streaming<MinerMsg>>,
    ) -> Result<Response<Self::SessionStream>, Status> {
        let inbound = request.into_inner();

        let jobs = {
            let mut guard = self
                .jobs
                .lock()
                .map_err(|_| Status::internal("job queue lock poisoned"))?;
            guard.take().unwrap_or_default()
        };
        let outcome_tx = {
            let mut guard = self
                .outcome_tx
                .lock()
                .map_err(|_| Status::internal("outcome channel lock poisoned"))?;
            guard.take()
        };

        let (tx, rx) = mpsc::channel(OUTBOUND_CHANNEL_DEPTH);
        let spec = self.spec.clone();
        let target = self.target;
        let backend_name = self.backend_name.clone();
        tokio::spawn(async move {
            let outcome =
                drive_session(inbound, tx, jobs, &spec, target.as_ref(), &backend_name).await;
            if let Some(sender) = outcome_tx {
                let _ = sender.send(outcome);
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

/// Drive the protocol for one connected miner: handshake, dispatch loop,
/// and shutdown. Runs inside its own spawned task so `session()` can return
/// its outbound stream to tonic immediately.
#[expect(
    clippy::too_many_lines,
    reason = "session loop kept as one linear match over MinerMsg for clarity, mirroring quip-mock-miner's own run_session"
)]
async fn drive_session(
    mut inbound: Streaming<MinerMsg>,
    tx: mpsc::Sender<Result<CoordMsg, Status>>,
    mut remaining: VecDeque<Job>,
    spec: &TopologySpec,
    target: Option<&SetTarget>,
    backend_name: &str,
) -> SessionOutcome {
    let mut records = Vec::new();

    // 1. Wait for Hello; anything else (wrong first message, closed stream,
    // transport error) is a failed handshake with no jobs ever dispatched.
    let Ok(Some(MinerMsg {
        msg: Some(miner_msg::Msg::Hello(hello)),
    })) = inbound.message().await
    else {
        return SessionOutcome {
            handshake_ok: false,
            records,
        };
    };

    // 2. Check the version using check_welcome's own comparison: Hello and
    // Welcome both carry a bare `protocol_version` field checked against
    // the same constant, so wrapping Hello's value in a Welcome reuses that
    // logic instead of reimplementing the `!= PROTOCOL_VERSION` check.
    if check_welcome(&Welcome {
        protocol_version: hello.protocol_version,
    })
    .is_err()
    {
        return SessionOutcome {
            handshake_ok: false,
            records,
        };
    }

    // 3. Reply Welcome, Configure, Topology, SetTarget. Configure is sent
    // all-zero: this crate does not yet drive queue depth / heartbeat /
    // reconnect tuning, so every field defers to the miner's own default
    // (matching quip_protocol::session::SessionConfig::from_configure's
    // documented 0-means-default contract).
    if send(
        &tx,
        coord_msg::Msg::Welcome(Welcome {
            protocol_version: PROTOCOL_VERSION,
        }),
    )
    .await
    .is_err()
    {
        return SessionOutcome {
            handshake_ok: true,
            records,
        };
    }
    let _ = send(&tx, coord_msg::Msg::Configure(Configure::default())).await;
    let _ = send(&tx, coord_msg::Msg::Topology(build_topology(spec))).await;
    let _ = send(
        &tx,
        coord_msg::Msg::SetTarget(target.copied().unwrap_or_default()),
    )
    .await;

    let edges = edge_positions(spec);
    let mut in_flight: HashMap<Vec<u8>, (Instant, Job)> = HashMap::new();

    if maybe_shutdown(&remaining, &in_flight, &tx).await {
        drain(&mut inbound).await;
        return SessionOutcome {
            handshake_ok: true,
            records,
        };
    }

    loop {
        let Ok(Some(msg)) = inbound.message().await else {
            break;
        };
        match msg.msg {
            Some(miner_msg::Msg::JobRequest(req)) => {
                for _ in 0..req.credits {
                    let Some(job) = remaining.pop_front() else {
                        break;
                    };
                    let job_id = job.job_id.clone();
                    if send(&tx, coord_msg::Msg::Job(job.clone())).await.is_err() {
                        return SessionOutcome {
                            handshake_ok: true,
                            records,
                        };
                    }
                    in_flight.insert(job_id, (Instant::now(), job));
                }
            }
            Some(miner_msg::Msg::Result(result)) => {
                if let Some((start, job)) = in_flight.remove(&result.job_id) {
                    records.push(build_result_record(
                        backend_name,
                        &job,
                        &result,
                        &edges,
                        target,
                        start.elapsed(),
                    ));
                }
                if maybe_shutdown(&remaining, &in_flight, &tx).await {
                    break;
                }
            }
            Some(miner_msg::Msg::Reject(reject)) => {
                if let Some((start, _job)) = in_flight.remove(&reject.job_id) {
                    records.push(build_reject_record(
                        backend_name,
                        &reject,
                        target,
                        start.elapsed(),
                    ));
                }
                if maybe_shutdown(&remaining, &in_flight, &tx).await {
                    break;
                }
            }
            Some(miner_msg::Msg::Fatal(fatal)) => {
                for (_job_id, (start, job)) in in_flight.drain() {
                    records.push(build_fatal_record(
                        backend_name,
                        &job,
                        &fatal,
                        target,
                        start.elapsed(),
                    ));
                }
                return SessionOutcome {
                    handshake_ok: true,
                    records,
                };
            }
            Some(
                miner_msg::Msg::Hello(_) | miner_msg::Msg::Ready(_) | miner_msg::Msg::Status(_),
            )
            | None => {}
        }
    }

    drain(&mut inbound).await;
    SessionOutcome {
        handshake_ok: true,
        records,
    }
}

/// Send `msg` on the outbound channel. The `Err` case is silently expected
/// on shutdown (the receiver end closes once the outbound stream itself is
/// dropped); callers that need to react to it (i.e. stop the loop) check
/// the returned `Result` explicitly instead of relying on a panic.
async fn send(
    tx: &mpsc::Sender<Result<CoordMsg, Status>>,
    msg: coord_msg::Msg,
) -> Result<(), mpsc::error::SendError<Result<CoordMsg, Status>>> {
    tx.send(Ok(CoordMsg { msg: Some(msg) })).await
}

/// If every job is terminal (none staged, none in flight), send `Shutdown`
/// and report `true` so the caller ends the loop.
async fn maybe_shutdown(
    remaining: &VecDeque<Job>,
    in_flight: &HashMap<Vec<u8>, (Instant, Job)>,
    tx: &mpsc::Sender<Result<CoordMsg, Status>>,
) -> bool {
    if remaining.is_empty() && in_flight.is_empty() {
        let _ = send(
            tx,
            coord_msg::Msg::Shutdown(Shutdown {
                grace_ms: SHUTDOWN_GRACE_MS,
            }),
        )
        .await;
        true
    } else {
        false
    }
}

/// Drain any trailing inbound traffic (e.g. a final `Status`) for up to
/// `SHUTDOWN_GRACE_MS` after `Shutdown` is sent, so the miner's own writes
/// do not stall against a coordinator that stopped reading. Mirrors the
/// symmetric drain `quip-mock-miner` performs after receiving `Shutdown`.
async fn drain(inbound: &mut Streaming<MinerMsg>) {
    let bound = Duration::from_millis(u64::from(SHUTDOWN_GRACE_MS));
    let _ = tokio::time::timeout(bound, async {
        while matches!(inbound.message().await, Ok(Some(_))) {}
    })
    .await;
}

/// Build the `Topology` message broadcast after `Welcome`/`Configure`.
fn build_topology(spec: &TopologySpec) -> Topology {
    let (u, v) = spec.edges.iter().copied().unzip();
    Topology {
        hash: spec.topology_hash(),
        nodes: spec.nodes.clone(),
        edges: Some(EdgeList { u, v }),
        allowed_h_milli: spec.allowed_h_milli.clone(),
    }
}

/// Resolve `spec.edges` (node-id pairs) to node-position pairs, matching
/// the order `spec.nodes` assigns each id (position = index in that slice).
/// Every endpoint is guaranteed present: `topology::load_topology` rejects
/// any edge referencing an id outside `nodes` before a `TopologySpec` can
/// exist.
fn edge_positions(spec: &TopologySpec) -> Vec<(usize, usize)> {
    let pos: HashMap<u32, usize> = spec
        .nodes
        .iter()
        .enumerate()
        .map(|(i, &n)| (n, i))
        .collect();
    spec.edges
        .iter()
        .map(|&(u, v)| {
            let pu = *pos
                .get(&u)
                .expect("edge endpoint validated at topology load");
            let pv = *pos
                .get(&v)
                .expect("edge endpoint validated at topology load");
            (pu, pv)
        })
        .collect()
}

/// Decode a milli-int-encoded i32-LE field to `f64` natural units (divide
/// by 1000). Miss this and every recomputed energy is off by 1000x — the
/// exact trap `quip-mock-miner::decode_milli_f64` documents and this
/// mirrors. Decode failure (never expected: this crate only ever decodes
/// its own `encode_i32_le` output) degrades to an empty vector rather than
/// panicking.
fn decode_milli_f64(bytes: &[u8]) -> Vec<f64> {
    decode_i32_le(bytes)
        .map(|values| values.iter().map(|&v| f64::from(v) / 1000.0).collect())
        .unwrap_or_default()
}

/// Build the [`JobRecord`] for a completed `Result`. Recomputes energy
/// locally per solution with [`energy_milli`] and compares it against the
/// miner's own reported `Solution.energy_milli`; any mismatch clears
/// `passed` (the miner's reported value is never persisted, only used for
/// this comparison — `JobRecord` has no field for it).
///
/// `reads`/`sweeps`/`device_access_time_us` come from the miner's own
/// `SamplerMeta` (what it actually ran), since those field names map
/// directly onto `JobRecord`'s. `anneal_time_us` instead comes from the
/// requested `SetTarget` (there is no wire field for actual anneal time,
/// only `qpu_access_us`, which this schema does not carry).
fn build_result_record(
    backend: &str,
    job: &Job,
    result: &JobResult,
    edges: &[(usize, usize)],
    target: Option<&SetTarget>,
    wall: Duration,
) -> JobRecord {
    let ising = job.ising.clone().unwrap_or_default();
    let h = decode_milli_f64(&ising.h_milli_le32);
    let j = decode_milli_f64(&ising.j_milli_le32);

    let mut mismatched = false;
    let mut recomputed = Vec::with_capacity(result.solutions.len());
    let mut spins_by_solution = Vec::with_capacity(result.solutions.len());
    for solution in &result.solutions {
        let spins = decode_spins(&solution.spins_bytes).unwrap_or_default();
        let energy = energy_milli(&spins, &h, &j, edges);
        if energy != solution.energy_milli {
            mismatched = true;
        }
        recomputed.push(energy);
        spins_by_solution.push(spins);
    }

    let n_solutions = result.solutions.len();
    let best_energy_milli = recomputed.into_iter().min().unwrap_or(0);
    let diversity_milli = milli_round(set_diversity(&spins_by_solution));
    let (reads, sweeps, device_access_time_us) = result
        .meta
        .as_ref()
        .map_or((0, 0, 0), |m| (m.reads, m.sweeps, m.device_access_time_us));

    JobRecord {
        job_id: hex_encode(&job.job_id),
        backend: backend.to_string(),
        n_solutions,
        best_energy_milli,
        diversity_milli,
        passed: n_solutions > 0 && !mismatched,
        rejected: false,
        reject_reason: None,
        device_access_time_us,
        wall_ms: duration_ms(wall),
        reads,
        sweeps,
        anneal_time_us: target.map_or(0, |t| t.anneal_time_us),
    }
}

/// Build the [`JobRecord`] for an explicit `Reject`. There is no
/// `SamplerMeta` on a reject (the miner never ran the job), so
/// `reads`/`sweeps`/`anneal_time_us` fall back to what was requested via
/// `SetTarget` rather than what was actually used.
fn build_reject_record(
    backend: &str,
    reject: &Reject,
    target: Option<&SetTarget>,
    wall: Duration,
) -> JobRecord {
    JobRecord {
        job_id: hex_encode(&reject.job_id),
        backend: backend.to_string(),
        n_solutions: 0,
        best_energy_milli: 0,
        diversity_milli: 0,
        passed: false,
        rejected: true,
        reject_reason: Some(reject_reason_text(reject.reason)),
        device_access_time_us: 0,
        wall_ms: duration_ms(wall),
        reads: target.map_or(0, |t| t.num_reads),
        sweeps: target.map_or(0, |t| t.num_sweeps),
        anneal_time_us: target.map_or(0, |t| t.anneal_time_us),
    }
}

/// Build the [`JobRecord`] for a job abandoned in flight when the miner
/// sends `Fatal`. Same "no `SamplerMeta`" reasoning as [`build_reject_record`]
/// applies: the job never completed, so requested values stand in.
fn build_fatal_record(
    backend: &str,
    job: &Job,
    fatal: &Fatal,
    target: Option<&SetTarget>,
    wall: Duration,
) -> JobRecord {
    JobRecord {
        job_id: hex_encode(&job.job_id),
        backend: backend.to_string(),
        n_solutions: 0,
        best_energy_milli: 0,
        diversity_milli: 0,
        passed: false,
        rejected: true,
        reject_reason: Some(format!("fatal: {}", fatal.reason)),
        device_access_time_us: 0,
        wall_ms: duration_ms(wall),
        reads: target.map_or(0, |t| t.num_reads),
        sweeps: target.map_or(0, |t| t.num_sweeps),
        anneal_time_us: target.map_or(0, |t| t.anneal_time_us),
    }
}

/// Render a `Reject.reason` as text: the enum's own wire name
/// (`"MALFORMED"`, ...) when it decodes to a known `RejectReason`, else a
/// value that still carries the raw integer for debugging.
fn reject_reason_text(reason: i32) -> String {
    RejectReason::try_from(reason).map_or_else(
        |_| format!("unknown_reject_reason({reason})"),
        |r| r.as_str_name().to_string(),
    )
}

/// Hex-encode `bytes`, lowercase with no separator. Matches the convention
/// `record.rs` documents for `JobRecord::job_id` and the `hex_encode` every
/// test file in this crate already defines locally.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Convert a `Duration` to milliseconds, saturating at `u64::MAX` rather
/// than panicking on an unrealistic overflow.
fn duration_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Scale a `[0.0, 1.0]` diversity fraction to milli-units, rounding to the
/// nearest `u32`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "set_diversity returns a value in [0.0, 1.0]; scaled by 1000 and rounded it fits u32"
)]
fn milli_round(fraction: f64) -> u32 {
    (fraction * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::{duration_ms, hex_encode, milli_round, reject_reason_text};
    use quip_proto::v1::RejectReason;
    use std::time::Duration;

    #[test]
    fn hex_encode_is_lowercase_no_separator() {
        assert_eq!(hex_encode(&[0x0a, 0xff, 0x00]), "0aff00");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn duration_ms_converts_whole_milliseconds() {
        assert_eq!(duration_ms(Duration::from_millis(1_234)), 1_234);
        assert_eq!(duration_ms(Duration::ZERO), 0);
    }

    #[test]
    fn milli_round_scales_and_rounds() {
        assert_eq!(milli_round(0.0), 0);
        assert_eq!(milli_round(1.0), 1000);
        // 1/3 -> 333.33... -> 333.
        assert_eq!(milli_round(1.0 / 3.0), 333);
    }

    #[test]
    fn reject_reason_text_uses_the_wire_enum_name() {
        assert_eq!(
            reject_reason_text(RejectReason::Malformed as i32),
            "MALFORMED"
        );
        assert_eq!(
            reject_reason_text(RejectReason::TopologyMismatch as i32),
            "TOPOLOGY_MISMATCH"
        );
    }

    #[test]
    fn reject_reason_text_reports_unknown_values_without_panicking() {
        assert_eq!(reject_reason_text(999), "unknown_reject_reason(999)");
    }
}
