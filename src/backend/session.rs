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
//! There is no cap on a session's total length — a real annealing run can
//! legitimately run for hours. Liveness is instead checked at the protocol
//! level: after a period of inbound silence the coordinator sends `Ping`,
//! and a miner that answers nothing at all (not `Status`, not a real
//! `Result`, nothing) to several consecutive `Ping`s is declared
//! unresponsive (`RunReport::miner_unresponsive`) rather than left to hang
//! the caller forever.
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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use quip_proto::v1::miner_service_server::{MinerService, MinerServiceServer};
use quip_proto::v1::{
    coord_msg, miner_msg, Configure, CoordMsg, EdgeList, Job, MinerMsg, Ping, Reject, RejectReason,
    Result as JobResult, SetTarget, Shutdown, Topology, Welcome,
};
use quip_protocol::scoring::{energy_milli, set_diversity};
use quip_protocol::session::{check_welcome, PROTOCOL_VERSION};
use quip_protocol::wire::{decode_i32_le, decode_spins};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixListener;
use tokio::process::{ChildStderr, Command};
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

/// How long to wait for *any* inbound message before sending the miner a
/// `Ping`. Deliberately short, not a proxy for expected per-job wall time:
/// a real miner answers `Ping` with `Status` from its control-plane read
/// loop, independent of whatever the sampler thread is doing
/// (`quip-miner-core/src/session.rs` replies to `Ping` unconditionally,
/// never waiting on in-flight sampling), so this interval only needs to
/// tolerate ordinary scheduling jitter in a live process, not the length of
/// a single annealing job.
const PING_INTERVAL: Duration = Duration::from_secs(3);

/// Consecutive silent `PING_INTERVAL` windows (no inbound message of any
/// kind — not just a `Ping` reply) tolerated before the session declares
/// the miner unresponsive and ends. Worst-case detection time is
/// `PING_INTERVAL * (MAX_MISSED_PINGS + 1)` = 30s at the current values.
///
/// This threshold is irrelevant to job duration: the detector only fires if
/// the miner *also* fails to answer the `Ping` that provokes it, and any
/// inbound message of any kind resets the count. It is purely about
/// whether the miner's control plane can respond while it is busy sampling
/// — not about how long the sample itself takes. For all four
/// `quip-miner-core`-based backends (`cpu-sa`, `cpu-gibbs`, `cuda`,
/// `metal`) it provably always can: sampling runs on a dedicated OS thread
/// (`quip-miner-core/src/session.rs` spawns it via `std::thread::spawn`,
/// fed by a channel) while the async control loop answers `Ping`
/// unconditionally, so this constant should never fire against them at
/// all. The margin exists for backends this crate cannot inspect —
/// `quip-dwave-qa` is a separate Python implementation, and this tool
/// exists specifically to benchmark solvers nobody has written yet, some
/// of which may answer `Ping` from the same thread that samples. The
/// asymmetry favors a generous value: a genuinely wedged miner dying a few
/// extra seconds later costs nothing on a run measured in hours, while a
/// working backend wrongly declared unresponsive writes false
/// `rejected: true, passed: false` rows into the benchmark output — an
/// expensive, hard-to-notice error the other direction does not have.
const MAX_MISSED_PINGS: u32 = 9;

/// Trailing stderr lines kept from the spawned miner. Bounded so a miner
/// that logs in a tight loop cannot grow this without limit; generous
/// enough to capture a realistic startup-failure diagnostic (a handful of
/// `tracing::error!` lines), matching the reasoning
/// [`crate::backend::registry`]'s probe stderr capture already applies.
const MAX_STDERR_LINES: usize = 50;

/// Per-`JobRequest` cap on how many jobs are dispatched at once, regardless
/// of the credits the miner asks for. `req.credits` is chosen by the miner
/// — the untrusted, benchmarked component — and a `Job` message carries the
/// full h/J problem payload, not just an id, so an uncapped credit grant
/// could queue enough data to exceed the outbound channel (and any HTTP/2
/// flow-control window backing it) well before `OUTBOUND_CHANNEL_DEPTH`
/// itself is reached. One less than `OUTBOUND_CHANNEL_DEPTH` leaves room for
/// at least one non-`Job` control message (e.g. `Ping`) to still be queued
/// alongside a full batch of dispatches.
#[expect(
    clippy::cast_possible_truncation,
    reason = "OUTBOUND_CHANNEL_DEPTH is a small literal (16) that always fits u32"
)]
const MAX_DISPATCH_CREDITS_PER_REQUEST: u32 = OUTBOUND_CHANNEL_DEPTH as u32 - 1;

/// Bound on a single dispatch `send`. Without this, a miner that requests
/// credits but never polls its own inbound stream while solving (an
/// ordinary naive shape, not malice) can wedge both sides once outbound
/// backpressure fills: the coordinator blocks here, the miner's own
/// `Result` send later blocks on its full outbound, and neither side is
/// left reading. The liveness `Ping` loop does not cover this send, since
/// it only wraps `inbound.message()`. Derived from the same constants as
/// the ping-based liveness window (`PING_INTERVAL * (MAX_MISSED_PINGS + 1)`
/// = 30s at the current values) for the same reason `MAX_MISSED_PINGS`
/// documents: a miner that is merely slow to get back to reading its
/// inbound (not deadlocked) must not be punished for it, and a genuinely
/// wedged one costs nothing extra by dying a little later on a run
/// measured in hours.
const DISPATCH_SEND_TIMEOUT: Duration =
    Duration::from_secs(PING_INTERVAL.as_secs() * (MAX_MISSED_PINGS as u64 + 1));

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
    /// [`quip_protocol::session::PROTOCOL_VERSION`]. `false` means the
    /// handshake itself never completed (no `Hello`, an unexpected first
    /// message, or a version mismatch), so `records` is always empty in
    /// that case. A miner that disconnects, errors, or goes unresponsive
    /// (see `miner_unresponsive`) *after* a successful handshake still
    /// reports `true` here, together with whatever records were completed
    /// before the interruption — this field is never used to (and must
    /// never be read as) discard those records.
    pub handshake_ok: bool,
    /// One record per job that reached a terminal state (`Result`,
    /// `Reject`, or abandoned in-flight on `Fatal`, a mid-flight
    /// disconnect, or a declared-unresponsive miner).
    pub records: Vec<JobRecord>,
    /// The miner subprocess's exit code, or `-1` if it was terminated by a
    /// signal or had to be killed after not exiting within the shutdown
    /// grace period.
    pub miner_exit_code: i32,
    /// Whether the session ended because the miner stopped answering
    /// entirely — no reply of any kind to several consecutive `Ping`s —
    /// rather than because every job reached a terminal state or the miner
    /// reported `Fatal`. This is not a cap on the run's total length: a
    /// long-running backend that keeps answering `Ping` (or sending real
    /// progress) can run indefinitely; this only fires once the miner goes
    /// silent.
    pub miner_unresponsive: bool,
    /// The miner subprocess's captured stderr (up to the last
    /// `MAX_STDERR_LINES` lines), or `None` if it wrote nothing, or its
    /// stderr could not be captured. Populated regardless of exit code. A
    /// miner that never sends `Hello` because `QUIP_SESSION_TOKEN` was
    /// rejected, `--quip-coordinator` was missing, or it failed to open its
    /// backend device reports exactly this kind of failure through
    /// `tracing::error!` to stderr, with no other signal reaching the
    /// coordinator at all.
    pub miner_stderr_tail: Option<String>,
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
/// failures (bad handshake, miner `Fatal`, an unresponsive miner, miner
/// never connecting) are not errors here: they show up as
/// `handshake_ok: false`, `miner_unresponsive: true`, and/or a non-zero
/// `miner_exit_code` in [`RunReport`].
pub async fn run_session(p: RunParams<'_>) -> Result<RunReport, SessionError> {
    run_session_inner(p, MAX_MISSED_PINGS).await
}

/// Identical to [`run_session`], except with the liveness threshold
/// overridable, so `tests/session_conformance.rs` can exercise the
/// unresponsive-detection path without waiting out the production
/// `MAX_MISSED_PINGS` value (`PING_INTERVAL * (MAX_MISSED_PINGS + 1))` =
/// 30s at the production value — deliberately generous per the reasoning on
/// [`MAX_MISSED_PINGS`], but far too slow for a test to wait out on every
/// run). This exists as a separate function, with `run_session`/[`RunParams`]
/// left exactly as they were, rather than a `RunParams` field: the human
/// ruling on this declined to widen that struct for test convenience.
/// `#[doc(hidden)]` and not part of this crate's documented public
/// contract; exported only because an external integration test (a
/// separate compiled crate that cannot see this module's private items,
/// however they might be gated) needs a `pub` item to call.
///
/// # Errors
///
/// Same as [`run_session`].
#[doc(hidden)]
pub async fn run_session_with_max_missed_pings_for_tests(
    p: RunParams<'_>,
    max_missed_pings: u32,
) -> Result<RunReport, SessionError> {
    run_session_inner(p, max_missed_pings).await
}

async fn run_session_inner(
    p: RunParams<'_>,
    max_missed_pings: u32,
) -> Result<RunReport, SessionError> {
    let start = Instant::now();
    let listener = UnixListener::bind(p.sock_path).map_err(SessionError::Bind)?;
    let incoming = UnixListenerStream::new(listener);

    let progress = Arc::new(SessionProgress::default());
    let stderr_tail = Arc::new(StderrTail::default());
    let (finish_tx, mut finish_rx) = oneshot::channel();
    let svc = CoordinatorSvc::new(
        p.backend.name.clone(),
        p.spec.clone(),
        p.target,
        p.jobs,
        finish_tx,
        Arc::clone(&progress),
        max_missed_pings,
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
        .stderr(Stdio::piped())
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

    // Read stderr concurrently, not after the child exits: the OS pipe
    // buffer backing it is finite, so a miner that logs enough before
    // anyone reads it could otherwise block on its own `write()` call.
    // Lines are pushed into `stderr_tail` as they arrive rather than
    // returned once at EOF, so an aborted reader task (see below) still
    // leaves whatever was captured up to that point.
    let stderr_task = child
        .stderr
        .take()
        .map(|stderr| tokio::spawn(capture_stderr_tail(stderr, Arc::clone(&stderr_tail))));

    // `finish_rx` resolves as soon as `drive_session` is done — normally,
    // or via its `FinishSignal` guard firing on panic unwind. Racing against
    // `child.wait()` exists only so a miner that crashes, is rejected, or
    // exits before ever dialing in (a bad binary, a missing
    // `--quip-coordinator`, a rejected session token, ...) cannot leave
    // `finish_rx` waiting forever — `session()` is never called on that
    // path, so `FinishSignal` never fires.
    //
    // But the child's OS-level exit is *not* a reliable proxy for
    // "drive_session has committed its records": if the miner dies mid-job
    // (segfault, SIGKILL/OOM, panic-abort), the socket FD closing and the
    // process exiting are the *same* kernel event, and the wait-side wake
    // (one signal-driver hop) is structurally shorter than the path from
    // socket EOF through the h2 connection task to drive_session's own
    // poll. So the child branch can win before drive_session has read the
    // close, decoded any trailing frames, or run `abandon_in_flight` — and
    // snapshotting `progress` at that instant would silently drop every
    // in-flight job's record. Give drive_session a bounded window to finish
    // regardless of which branch fired: on the normal path drive_session's
    // drain already returned, so this second wait resolves in microseconds;
    // the full grace is only spent when the miner never called `session()`
    // at all, which is already a failed run.
    tokio::select! {
        _ = &mut finish_rx => {}
        _ = child.wait() => {
            let _ = tokio::time::timeout(
                Duration::from_millis(u64::from(SHUTDOWN_GRACE_MS)),
                &mut finish_rx,
            )
            .await;
        }
    }

    let shutdown_wait = Duration::from_millis(u64::from(SHUTDOWN_GRACE_MS) * 3);
    let miner_exit_code = match tokio::time::timeout(shutdown_wait, child.wait()).await {
        Ok(Ok(status)) => status.code().unwrap_or(-1),
        Ok(Err(_)) => -1,
        Err(_) => {
            let _ = child.kill().await;
            -1
        }
    };

    // Bounded: `capture_stderr_tail` only returns at EOF, which requires
    // *every* write end of the pipe to close. `kill_on_drop`/`child.kill()`
    // reach only the direct child — a wrapper script that backgrounds the
    // real miner instead of exec-ing it, or a miner that daemonizes a
    // helper process, can inherit fd 2 and keep the pipe open forever.
    // Whatever was captured before the abort already lives in `stderr_tail`
    // (pushed line-by-line as it arrived), so aborting loses nothing beyond
    // whatever was still in flight.
    if let Some(mut handle) = stderr_task {
        let _ = tokio::time::timeout(shutdown_wait, &mut handle).await;
        handle.abort();
    }
    let miner_stderr_tail = stderr_tail.snapshot();

    server_task.abort();
    let _ = std::fs::remove_file(p.sock_path);

    Ok(RunReport {
        handshake_ok: progress.handshake_ok.load(Ordering::Acquire),
        records: progress.snapshot_records(),
        miner_exit_code,
        miner_unresponsive: progress.miner_unresponsive.load(Ordering::Acquire),
        miner_stderr_tail,
        run_wall_ms: duration_ms(start.elapsed()),
    })
}

/// Drain `stderr` to EOF, pushing each line into `tail` as it arrives.
async fn capture_stderr_tail(stderr: ChildStderr, tail: Arc<StderrTail>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        tail.push_line(line);
    }
}

/// Shared, incrementally-updated tail of the miner's captured stderr,
/// mirroring [`SessionProgress`]'s reasoning: the reader task can be
/// aborted (see `run_session`) if the pipe never reaches EOF, and an abort
/// drops that task's own local state. Writing into this shared buffer as
/// each line arrives means whatever was captured before the abort survives
/// it.
#[derive(Default)]
struct StderrTail(Mutex<VecDeque<String>>);

impl StderrTail {
    /// Push `line`, keeping only the last [`MAX_STDERR_LINES`].
    fn push_line(&self, line: String) {
        let mut lines = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.len() == MAX_STDERR_LINES {
            let _ = lines.pop_front();
        }
        lines.push_back(line);
    }

    /// Join the captured lines with `\n`, or `None` if nothing was captured.
    fn snapshot(&self) -> Option<String> {
        let lines = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.is_empty() {
            None
        } else {
            Some(lines.iter().cloned().collect::<Vec<_>>().join("\n"))
        }
    }
}

/// Shared, incrementally-updated view of a session's progress.
/// `run_session` reads this after the spawned `drive_session` task signals
/// completion (via [`FinishSignal`]), instead of relying on a value
/// `drive_session` returns only once at the very end — so a panic partway
/// through (which aborts that task without ever returning) still leaves
/// whatever records, handshake status, and liveness state were committed
/// *before* the panic visible here, rather than silently discarding them.
#[derive(Default)]
struct SessionProgress {
    handshake_ok: AtomicBool,
    miner_unresponsive: AtomicBool,
    records: Mutex<Vec<JobRecord>>,
}

impl SessionProgress {
    /// Recovers the inner `Vec` on a poisoned lock (`unwrap_or_else`, not
    /// `unwrap` — stays clean under the denied `clippy::unwrap_used`) rather
    /// than silently dropping the record: an unrelated panic elsewhere does
    /// not logically corrupt a list of already-completed records, and
    /// discarding them is exactly the outcome this whole design forbids.
    fn push_record(&self, record: JobRecord) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }

    /// `Release`: pairs with the `Acquire` load in `run_session`, so every
    /// `push_record` call that happens-before this store is visible to
    /// whichever branch of `run_session`'s `select!` reads it afterward —
    /// not just the `finish_rx` path, which already has a happens-before
    /// edge via the oneshot itself.
    fn set_handshake_ok(&self) {
        self.handshake_ok.store(true, Ordering::Release);
    }

    /// See [`Self::set_handshake_ok`]: same cross-task publication reasoning.
    fn set_unresponsive(&self) {
        self.miner_unresponsive.store(true, Ordering::Release);
    }

    fn snapshot_records(&self) -> Vec<JobRecord> {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Sends its held oneshot exactly once, when dropped — whether the guarded
/// scope returned normally or panicked and unwound through it.
/// `run_session` waits on the receiving half as its "the spawned task is
/// done" signal instead of `drive_session`'s return value, so a panic
/// partway through still lets `run_session` proceed (and read whatever
/// [`SessionProgress`] already recorded) instead of hanging forever on a
/// oneshot that would otherwise never fire.
struct FinishSignal(Option<oneshot::Sender<()>>);

impl Drop for FinishSignal {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

/// `MinerService` implementation backing [`run_session`]. Built fresh per
/// call and expected to serve exactly one connection (the backend spawned
/// alongside it); a second connection would find `jobs`/`finish_tx` empty
/// (already taken) and terminate immediately with a shutdown.
struct CoordinatorSvc {
    backend_name: String,
    spec: TopologySpec,
    target: Option<SetTarget>,
    jobs: Mutex<Option<VecDeque<Job>>>,
    finish_tx: Mutex<Option<oneshot::Sender<()>>>,
    progress: Arc<SessionProgress>,
    max_missed_pings: u32,
}

impl CoordinatorSvc {
    fn new(
        backend_name: String,
        spec: TopologySpec,
        target: Option<SetTarget>,
        jobs: Vec<Job>,
        finish_tx: oneshot::Sender<()>,
        progress: Arc<SessionProgress>,
        max_missed_pings: u32,
    ) -> Self {
        Self {
            backend_name,
            spec,
            target,
            jobs: Mutex::new(Some(jobs.into_iter().collect())),
            finish_tx: Mutex::new(Some(finish_tx)),
            progress,
            max_missed_pings,
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

        // Recovering the inner value on a poisoned lock (rather than
        // returning `Status::internal`, as before) removes an error arm
        // that, on the `finish_tx`-poisoned ordering specifically, silently
        // discarded every already-`take()`n job via the `?` that stranded
        // `finish_tx`'s own guard construction. These two mutexes only ever
        // guard a single `Option::take()`, which cannot unwind, so this can
        // only matter if a future change adds a fallible operation inside
        // the guard — exactly the trap `PoisonError::into_inner` closes.
        let jobs = self
            .jobs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .unwrap_or_default();
        let finish_tx = self
            .finish_tx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();

        let (tx, rx) = mpsc::channel(OUTBOUND_CHANNEL_DEPTH);
        let spec = self.spec.clone();
        let target = self.target;
        let backend_name = self.backend_name.clone();
        let progress = Arc::clone(&self.progress);
        let max_missed_pings = self.max_missed_pings;
        tokio::spawn(async move {
            let _finish_guard = finish_tx.map(|tx| FinishSignal(Some(tx)));
            drive_session(
                inbound,
                tx,
                jobs,
                &spec,
                target.as_ref(),
                &backend_name,
                &progress,
                max_missed_pings,
            )
            .await;
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
#[expect(
    clippy::too_many_arguments,
    reason = "one more than the threshold (max_missed_pings, the F8 test-only liveness override) \
              plumbed through as a parameter rather than a RunParams field per an explicit human ruling"
)]
async fn drive_session(
    mut inbound: Streaming<MinerMsg>,
    tx: mpsc::Sender<Result<CoordMsg, Status>>,
    mut remaining: VecDeque<Job>,
    spec: &TopologySpec,
    target: Option<&SetTarget>,
    backend_name: &str,
    progress: &SessionProgress,
    max_missed_pings: u32,
) {
    // 1. Wait for Hello; anything else (wrong first message, closed stream,
    // transport error) is a failed handshake with no jobs ever dispatched.
    let Ok(Some(MinerMsg {
        msg: Some(miner_msg::Msg::Hello(hello)),
    })) = inbound.message().await
    else {
        return;
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
        return;
    }
    progress.set_handshake_ok();

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
        return;
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
        return;
    }

    // Liveness: wait at most PING_INTERVAL for any inbound message. A
    // timeout sends a Ping and counts as one missed reply; any real message
    // (of any kind — Ping only asks "are you there", it does not gate what
    // counts as an answer) resets the count. MAX_MISSED_PINGS consecutive
    // silent windows declares the miner unresponsive.
    let mut missed_pings: u32 = 0;
    let mut unresponsive = false;
    loop {
        match tokio::time::timeout(PING_INTERVAL, inbound.message()).await {
            Ok(Ok(Some(msg))) => {
                missed_pings = 0;
                match msg.msg {
                    // Each `send` below is awaited without reading inbound
                    // in between, so a miner that requests credits and then
                    // solves synchronously without polling its own inbound
                    // stream could wedge both sides once outbound
                    // backpressure fills — an ordinary naive shape, not
                    // malice, and `req.credits` is chosen by the miner
                    // itself. `MAX_DISPATCH_CREDITS_PER_REQUEST` bounds how
                    // much can queue at once; `DISPATCH_SEND_TIMEOUT` bounds
                    // how long any one `send` may block, treating an
                    // elapsed timeout the same as a failed send (dead
                    // miner) rather than hanging with no timeout on any
                    // path — the liveness `Ping` loop does not cover this
                    // send, since it only wraps `inbound.message()`.
                    Some(miner_msg::Msg::JobRequest(req)) => {
                        let credits = req.credits.min(MAX_DISPATCH_CREDITS_PER_REQUEST);
                        for _ in 0..credits {
                            let Some(job) = remaining.pop_front() else {
                                break;
                            };
                            let job_id = job.job_id.clone();
                            // Captured before the send, not after: time
                            // spent blocked on outbound backpressure is
                            // real wall time this job's dispatch cost, and
                            // a job abandoned by the timeout below must not
                            // report `wall_ms: 0`.
                            let dispatch_start = Instant::now();
                            let outbound = coord_msg::Msg::Job(job.clone());
                            in_flight.insert(job_id, (dispatch_start, job));
                            // Two distinct failure causes, not one: the
                            // outbound channel closing (the miner's own
                            // stream ended) and the send timing out (the
                            // miner is still connected but never draining
                            // its inbound) are different diagnoses and get
                            // different reason text.
                            let abandon_reason = match tokio::time::timeout(
                                DISPATCH_SEND_TIMEOUT,
                                send(&tx, outbound),
                            )
                            .await
                            {
                                Ok(Ok(())) => None,
                                Ok(Err(_)) => Some("outbound channel closed while dispatching"),
                                Err(_) => Some(
                                    "dispatch send timed out: miner not reading its inbound stream",
                                ),
                            };
                            if let Some(reason) = abandon_reason {
                                abandon_in_flight(
                                    &mut in_flight,
                                    progress,
                                    backend_name,
                                    target,
                                    reason,
                                );
                                return;
                            }
                        }
                    }
                    Some(miner_msg::Msg::Result(result)) => {
                        if let Some((start, job)) = in_flight.remove(&result.job_id) {
                            progress.push_record(build_result_record(
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
                            progress.push_record(build_reject_record(
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
                        abandon_in_flight(
                            &mut in_flight,
                            progress,
                            backend_name,
                            target,
                            &format!("fatal: {}", fatal.reason),
                        );
                        return;
                    }
                    Some(
                        miner_msg::Msg::Hello(_)
                        | miner_msg::Msg::Ready(_)
                        | miner_msg::Msg::Status(_),
                    )
                    | None => {}
                }
            }
            Ok(Ok(None) | Err(_)) => break,
            Err(_) => {
                missed_pings += 1;
                if missed_pings > max_missed_pings {
                    unresponsive = true;
                    break;
                }
                let _ = send(&tx, coord_msg::Msg::Ping(Ping {})).await;
            }
        }
    }

    if unresponsive {
        progress.set_unresponsive();
        abandon_in_flight(
            &mut in_flight,
            progress,
            backend_name,
            target,
            "miner unresponsive: no reply to repeated Ping messages",
        );
    } else {
        // The loop above otherwise ends two ways: `maybe_shutdown` already
        // drained `in_flight` to empty before breaking (every job reached
        // a terminal state), or the inbound stream closed/errored with
        // jobs still dispatched-but-unanswered (miner crashed, was killed,
        // or hit its own idle timeout and disconnected). The latter must
        // not drop those jobs silently — that would make `records.len()`
        // look like a 100% success rate for a run the miner never
        // finished. Draining here is a no-op in the former case and the
        // safety net in the latter.
        abandon_in_flight(
            &mut in_flight,
            progress,
            backend_name,
            target,
            "miner stream closed with jobs in flight",
        );
        drain(&mut inbound).await;
    }
}

/// Record every job still in `in_flight` as failing with `reason`, and
/// empty `in_flight`. Shared by five call sites — `Fatal`, an unresponsive
/// miner, an outbound channel closing mid-dispatch, a dispatch send timing
/// out, and the end-of-loop cleanup for a closed/errored inbound stream —
/// all "the miner will never answer this job" outcomes, but distinct
/// causes: each passes its own `reason` text so a `JobRecord` in the
/// benchmark output says which one happened, rather than one indistinct
/// string covering causes a reader has no way to tell apart.
fn abandon_in_flight(
    in_flight: &mut HashMap<Vec<u8>, (Instant, Job)>,
    progress: &SessionProgress,
    backend_name: &str,
    target: Option<&SetTarget>,
    reason: &str,
) {
    for (_job_id, (start, job)) in in_flight.drain() {
        progress.push_record(build_abandoned_record(
            backend_name,
            &job,
            reason,
            target,
            start.elapsed(),
        ));
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
///
/// `topology::load_topology` rejects any edge referencing an id outside
/// `nodes` before a `TopologySpec` can exist, so every endpoint is present
/// for any spec that loader produced. But `TopologySpec`'s fields are all
/// `pub` (a caller can hand-construct one with a dangling edge), and
/// `clippy::panic`/`unwrap_used` are denied while `expect_used` is not — an
/// `.expect()` here would compile clean yet still be reachable. Drop any
/// edge whose endpoint is not in `nodes` instead of panicking on it: local
/// re-scoring loses that one edge's contribution, but the session loop
/// keeps running rather than crashing the spawned task (which would
/// otherwise silently degrade the whole run to an empty, "handshake
/// failed" report).
fn edge_positions(spec: &TopologySpec) -> Vec<(usize, usize)> {
    let pos: HashMap<u32, usize> = spec
        .nodes
        .iter()
        .enumerate()
        .map(|(i, &n)| (n, i))
        .collect();
    spec.edges
        .iter()
        .filter_map(|&(u, v)| Some((*pos.get(&u)?, *pos.get(&v)?)))
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

/// Build the [`JobRecord`] for a job abandoned in flight — the miner sent
/// `Fatal`, or the session ended (stream closed/errored) before it answered.
/// Same "no `SamplerMeta`" reasoning as [`build_reject_record`] applies: the
/// job never completed, so requested values stand in.
fn build_abandoned_record(
    backend: &str,
    job: &Job,
    reason: &str,
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
        reject_reason: Some(reason.to_string()),
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
    use super::{duration_ms, edge_positions, hex_encode, milli_round, reject_reason_text};
    use crate::topology::TopologySpec;
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

    /// `TopologySpec`'s fields are all `pub`, so a caller can hand-construct
    /// one with an edge referencing a node id absent from `nodes` — an
    /// invariant only `topology::load_topology` enforces, not the type
    /// itself. `edge_positions` must drop that one edge rather than panic
    /// (`clippy::expect_used` is not in this crate's denied lint set, so an
    /// `.expect()` here would compile clean while still being reachable).
    #[test]
    fn edge_positions_drops_an_edge_with_an_unknown_endpoint_instead_of_panicking() {
        let spec = TopologySpec {
            nodes: vec![0, 1, 2],
            edges: vec![(0, 1), (1, 99)],
            allowed_h_milli: vec![0],
            allowed_j_milli: vec![0],
            allowed_spin_milli: vec![-1000, 1000],
        };
        assert_eq!(edge_positions(&spec), vec![(0, 1)]);
    }
}
