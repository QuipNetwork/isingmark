// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hermetic fake miner for `isingmark`'s own coordinator-session tests.
//!
//! Speaks the real `MinerService.Session` wire protocol with `quip-proto`
//! types, dialing the coordinator over the Unix socket given on
//! `--quip-coordinator`, exactly like a real miner would. Modeled on
//! `quip-mock-miner/src/main.rs` (read-only reference; not modified here)
//! but with its behavior driven entirely by CLI flags, so
//! `tests/session_conformance.rs` can exercise the coordinator's handshake,
//! dispatch, result, reject, fatal-mid-flight, and abrupt-disconnect paths
//! without any of the five real miner binaries installed.
//!
//! Named `isingmark-fake-miner` (not the shorter `fake-miner`) because
//! Cargo auto-discovers every `src/bin/*.rs` file as a real binary target in
//! every profile, including `cargo install`; the namespaced name keeps this
//! test-only tool from landing in a user's `$PATH` under a generic name.

use std::process::ExitCode as ProcessExitCode;
use std::time::Duration;

use clap::Parser;
use quip_proto::v1::miner_service_client::MinerServiceClient;
use quip_proto::v1::{
    coord_msg, ising_problem, miner_msg, CoordMsg, Fatal, IsingProblem, Job, JobKind, JobRequest,
    MinerMsg, Ready, Reject, RejectReason, Result as JobResult, SamplerMeta, Solution, Status,
    Topology,
};
use quip_protocol::scoring::energy_milli;
use quip_protocol::session::{build_hello, check_welcome, BackendCaps, ExitCode, SessionError};
use quip_protocol::wire::{decode_i32_le, encode_spins};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::transport::{Endpoint, Uri};

#[derive(Parser)]
struct Cli {
    /// `unix://<path>` coordinator address, as passed by
    /// `isingmark::backend::session::run_session`.
    #[arg(long)]
    quip_coordinator: String,
    #[arg(long, default_value = "fake-miner-0")]
    miner_id: String,
    /// `Hello.protocol_version` to advertise. A value other than the real
    /// `quip_protocol::session::PROTOCOL_VERSION` exercises the
    /// coordinator's handshake-rejection path.
    #[arg(long, default_value_t = quip_protocol::session::PROTOCOL_VERSION)]
    protocol_version: u32,
    /// Send `Ready` as the very first message instead of `Hello`, to
    /// exercise the coordinator's "unexpected first message" branch (as
    /// opposed to a closed stream, which never sends anything at all).
    #[arg(long, default_value_t = false)]
    skip_hello: bool,
    /// Credits requested in the first `JobRequest`, sent right after
    /// `Ready`. A value greater than 1 lets several jobs be in flight at
    /// once, which `--fatal-after`/`--disconnect-after` can then abandon
    /// mid-session.
    #[arg(long, default_value_t = 1)]
    credits: u32,
    /// Number of solutions to report per solved job, with distinct spin
    /// patterns (solution `i` flips the first `i` node spins negative,
    /// capped at the node count). `1` (default) matches the trivial
    /// all-`+1` behavior every other test relies on; a higher value gives
    /// the coordinator's best-of-N selection and diversity computation
    /// something non-degenerate to compute over.
    #[arg(long, default_value_t = 1)]
    num_solutions: u32,
    /// Amount (milli-units) added to a solution's reported `energy_milli`
    /// on top of the true value, so the coordinator's local re-score catches
    /// the lie. `0` (default) always reports the true value.
    #[arg(long, default_value_t = 0)]
    energy_lie_milli: i64,
    /// 1-based ordinal (among jobs this process receives) to `Reject`
    /// instead of solving.
    #[arg(long)]
    reject_job: Option<u32>,
    /// 1-based ordinal (among jobs this process receives) at which to send
    /// `Fatal` instead of solving, ending the session mid-flight and
    /// abandoning that job and any others already dispatched but not yet
    /// answered.
    #[arg(long)]
    fatal_after: Option<u32>,
    /// 1-based ordinal (among jobs this process receives) at which to close
    /// the connection with no message at all, instead of solving, rejecting,
    /// or sending `Fatal` — simulating a crash or an OOM-kill rather than an
    /// orderly `Fatal` report. Abandons that job and any others already
    /// dispatched but not yet answered, same as `--fatal-after`.
    #[arg(long)]
    disconnect_after: Option<u32>,
    /// 1-based ordinal (among jobs this process receives) at which to stop
    /// sending anything at all, ever again — including replies to the
    /// coordinator's `Ping` — while keeping the connection open. Distinct
    /// from `--disconnect-after` (which drops the connection): this
    /// exercises the coordinator's ping-based liveness check, which can
    /// only fire against a session that is still technically connected.
    #[arg(long)]
    go_silent_after: Option<u32>,
    /// Print this message to stderr and exit non-zero immediately, before
    /// ever attempting to connect. Mimics a real miner's `tracing::error!`
    /// startup-failure output (missing `--quip-coordinator`, a rejected
    /// session token, a device that failed to open, ...), to test that the
    /// coordinator captures and surfaces it.
    #[arg(long)]
    stderr_and_exit: Option<String>,
    /// 1-based ordinal (among jobs this process receives) at which to call
    /// `std::process::abort()` immediately, with no message and no orderly
    /// shutdown — simulating a segfault, `SIGKILL`/OOM, or a `panic = "abort"`
    /// build. Unlike `--disconnect-after` (which drops the connection but
    /// leaves the process alive to drain its inbound stream first), this
    /// closes the socket FD and terminates the process in the same kernel
    /// event, which is the specific race `run_session`'s `child.wait()`
    /// anti-hang backstop must not mistake for "the session finished".
    #[arg(long)]
    abort_after: Option<u32>,
    /// Print this message to stderr, spawn a detached grandchild that
    /// inherits this process's stderr fd and outlives it (`sleep 60`, never
    /// waited on), then exit non-zero without ever connecting. Simulates a
    /// wrapper script that backgrounds the real miner instead of exec-ing
    /// it, or a miner that daemonizes a device-access helper: the pipe's
    /// write end stays open long after this process is gone, so the
    /// coordinator's stderr reader never sees EOF on its own.
    #[arg(long)]
    leak_stderr_to_grandchild: Option<String>,
}

/// Map a session/CLI failure to a documented exit code (64/69/70/77).
fn coded_exit(code: ExitCode) -> ProcessExitCode {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "ExitCode values are documented 0/64/69/70/77 and fit in u8"
    )]
    {
        ProcessExitCode::from(code.as_i32() as u8)
    }
}

#[tokio::main]
async fn main() -> ProcessExitCode {
    match run().await {
        Ok(()) => ProcessExitCode::SUCCESS,
        Err(code) => coded_exit(code),
    }
}

fn miner(msg: miner_msg::Msg) -> MinerMsg {
    MinerMsg { msg: Some(msg) }
}

/// Milli-int-encoded field -> float vector; mirrors
/// `quip-mock-miner::decode_milli_f64` (the `/ 1000.0` step is the exact
/// units trap documented on the coordinator side too).
fn decode_milli_f64(bytes: &[u8]) -> Result<Vec<f64>, quip_protocol::wire::WireError> {
    Ok(decode_i32_le(bytes)?
        .iter()
        .map(|&v| f64::from(v) / 1000.0)
        .collect())
}

/// Session-cached topology: resolves a `TopologyHash` job's edges to dense
/// positions via the node-id -> position map `Topology.nodes` implies.
/// Mirrors `quip-mock-miner::SessionTopo`.
struct FakeTopo {
    edges: Vec<(u32, u32)>,
    pos: std::collections::HashMap<u32, usize>,
}

impl FakeTopo {
    fn from_proto(t: &Topology) -> Self {
        let mut pos = std::collections::HashMap::with_capacity(t.nodes.len());
        for (i, &node) in t.nodes.iter().enumerate() {
            let _ = pos.insert(node, i);
        }
        let edges = t.edges.as_ref().map_or_else(Vec::new, |e| {
            e.u.iter().zip(&e.v).map(|(&u, &v)| (u, v)).collect()
        });
        Self { edges, pos }
    }

    /// Resolve cached edges to node positions, dropping any edge whose
    /// endpoint is not in this topology (never expected: `isingmark`'s own
    /// `Topology` messages always cover every edge they reference).
    fn edges_as_positions(&self) -> Vec<(usize, usize)> {
        self.edges
            .iter()
            .filter_map(|&(u, v)| Some((*self.pos.get(&u)?, *self.pos.get(&v)?)))
            .collect()
    }
}

fn resolve_edges(ising: &IsingProblem, topo: Option<&FakeTopo>) -> Vec<(usize, usize)> {
    match &ising.graph {
        Some(ising_problem::Graph::Edges(e)) => {
            e.u.iter()
                .zip(&e.v)
                .map(|(&u, &v)| (u as usize, v as usize))
                .collect()
        }
        Some(ising_problem::Graph::TopologyHash(_)) | None => {
            topo.map_or_else(Vec::new, FakeTopo::edges_as_positions)
        }
    }
}

/// Solve `job`, reporting `num_solutions` distinct-spin solutions (solution
/// `i` flips the first `i` node spins negative, capped at the node count),
/// each with `energy_lie_milli` added on top of its true recomputed energy.
/// `num_solutions <= 1` reproduces the original trivial all-`+1` behavior.
fn solve(
    job: &Job,
    topo: Option<&FakeTopo>,
    energy_lie_milli: i64,
    num_solutions: u32,
) -> JobResult {
    let ising = job.ising.clone().unwrap_or_default();
    let h = decode_milli_f64(&ising.h_milli_le32).unwrap_or_default();
    let j = decode_milli_f64(&ising.j_milli_le32).unwrap_or_default();
    let edges = resolve_edges(&ising, topo);
    let n = h.len();
    let count = num_solutions.max(1);

    let solutions = (0..count)
        .map(|i| {
            let flips = (i as usize).min(n);
            let mut spins = vec![1i8; n];
            for spin in spins.iter_mut().take(flips) {
                *spin = -1;
            }
            let true_energy = energy_milli(&spins, &h, &j, &edges);
            Solution {
                spins_bytes: encode_spins(&spins),
                energy_milli: true_energy + energy_lie_milli,
            }
        })
        .collect();

    JobResult {
        job_id: job.job_id.clone(),
        solutions,
        meta: Some(SamplerMeta {
            reads: count,
            sweeps: 0,
            device_access_time_us: 0,
            ..Default::default()
        }),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "session loop kept as one linear match over CoordMsg for clarity, mirroring quip-mock-miner's own run_session"
)]
async fn run() -> Result<(), ExitCode> {
    let cli = Cli::parse();

    if let Some(message) = &cli.stderr_and_exit {
        #[expect(
            clippy::print_stderr,
            reason = "test-only flag that deliberately mimics a real miner's tracing::error! \
                      startup-failure output, for isingmark's stderr-capture test"
        )]
        {
            eprintln!("{message}");
        }
        return Err(ExitCode::EnvIncompatible);
    }

    if let Some(message) = &cli.leak_stderr_to_grandchild {
        #[expect(
            clippy::print_stderr,
            reason = "test-only flag mimicking a real miner's startup-failure diagnostic, \
                      for isingmark's bounded-stderr-wait test"
        )]
        {
            eprintln!("{message}");
        }
        // Inherits stdin/stdout/stderr by default (no `.stdout`/`.stderr`
        // override here), so the grandchild's copy of fd 2 keeps the pipe's
        // write end open long after this process exits. Never waited on:
        // that is the point — a real orphaned helper would not be either.
        let _ = std::process::Command::new("sleep").arg("60").spawn();
        return Err(ExitCode::EnvIncompatible);
    }

    let mut hello = build_hello(
        &cli.miner_id,
        "fake",
        "fake-sa",
        &[JobKind::IsingSample],
        BackendCaps {
            max_nodes: 0,
            max_edges: 0,
        },
    )
    .map_err(|e: SessionError| ExitCode::from(e))?;
    hello.protocol_version = cli.protocol_version;

    let path = cli
        .quip_coordinator
        .strip_prefix("unix://")
        .unwrap_or(&cli.quip_coordinator)
        .to_string();
    let channel = Endpoint::try_from("http://[::]:50051") // dummy authority, unused for UDS
        .map_err(|_| ExitCode::InternalFatal)?
        .connect_with_connector(tower::service_fn(move |_: Uri| {
            let p = path.clone();
            async move {
                let s = tokio::net::UnixStream::connect(p).await?;
                Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(s))
            }
        }))
        .await
        .map_err(|_| ExitCode::InternalFatal)?;
    let mut client = MinerServiceClient::new(channel);

    let (tx, rx) = mpsc::channel::<MinerMsg>(16);
    // `--skip-hello` sends `Ready` first instead of `Hello`, so the
    // coordinator's "unexpected first message" branch gets exercised
    // distinctly from a closed stream (which never sends anything at all).
    let first = if cli.skip_hello {
        miner(miner_msg::Msg::Ready(Ready {}))
    } else {
        miner(miner_msg::Msg::Hello(hello))
    };
    tx.send(first).await.map_err(|_| ExitCode::InternalFatal)?;

    let mut inbound = client
        .session(ReceiverStream::new(rx))
        .await
        .map_err(|_| ExitCode::InternalFatal)?
        .into_inner();

    let mut topo: Option<FakeTopo> = None;
    let mut jobs_seen: u32 = 0;
    let mut fatal_sent = false;
    let mut silent = false;

    loop {
        let next = tokio::time::timeout(Duration::from_secs(30), inbound.message()).await;
        let cm: CoordMsg = match next {
            Err(_) | Ok(Ok(None)) => break,
            Ok(Ok(Some(cm))) => cm,
            Ok(Err(_)) => return Err(ExitCode::InternalFatal),
        };
        // `--go-silent-after` stops all outbound traffic, forever, while
        // leaving the connection open — including ignoring the
        // coordinator's own `Ping`, which is exactly the scenario its
        // ping-based liveness check exists to detect.
        if silent {
            continue;
        }
        match cm.msg {
            Some(coord_msg::Msg::Welcome(w)) => {
                if check_welcome(&w).is_err() {
                    break;
                }
            }
            Some(coord_msg::Msg::Configure(_)) => {
                tx.send(miner(miner_msg::Msg::Ready(Ready {})))
                    .await
                    .map_err(|_| ExitCode::InternalFatal)?;
                tx.send(miner(miner_msg::Msg::JobRequest(JobRequest {
                    credits: cli.credits,
                })))
                .await
                .map_err(|_| ExitCode::InternalFatal)?;
            }
            Some(coord_msg::Msg::Topology(t)) => {
                topo = Some(FakeTopo::from_proto(&t));
            }
            Some(coord_msg::Msg::Ping(_)) => {
                tx.send(miner(miner_msg::Msg::Status(Status {
                    miner_id: cli.miner_id.clone(),
                    utilization: 0.0,
                    jobs_done: u64::from(jobs_seen),
                    abandoned_generation: 0,
                    sampler_stats: std::collections::HashMap::default(),
                })))
                .await
                .map_err(|_| ExitCode::InternalFatal)?;
            }
            Some(coord_msg::Msg::Job(job)) => {
                jobs_seen += 1;
                if cli.abort_after == Some(jobs_seen) {
                    // Deliberately not `std::process::exit`: abort closes
                    // the socket FD and terminates the process in one
                    // kernel event, with no chance for an orderly stream
                    // half-close first. That is the point of this flag.
                    std::process::abort();
                }
                if cli.go_silent_after == Some(jobs_seen) {
                    // Stop answering anything at all, forever, but keep the
                    // connection open (unlike --disconnect-after).
                    silent = true;
                    continue;
                }
                if cli.disconnect_after == Some(jobs_seen) {
                    // Simulate a crash/OOM-kill: close the connection with
                    // no message at all for this job (or any other already
                    // in flight), rather than an orderly Fatal report.
                    break;
                }
                if cli.fatal_after == Some(jobs_seen) {
                    tx.send(miner(miner_msg::Msg::Fatal(Fatal {
                        exit_code: ExitCode::InternalFatal.as_i32().try_into().unwrap_or(70),
                        reason: "synthetic fatal for coordinator conformance testing".into(),
                        restart_required: true,
                    })))
                    .await
                    .map_err(|_| ExitCode::InternalFatal)?;
                    fatal_sent = true;
                    break;
                }
                if cli.reject_job == Some(jobs_seen) {
                    tx.send(miner(miner_msg::Msg::Reject(Reject {
                        job_id: job.job_id,
                        reason: RejectReason::Malformed as i32,
                    })))
                    .await
                    .map_err(|_| ExitCode::InternalFatal)?;
                } else {
                    let result =
                        solve(&job, topo.as_ref(), cli.energy_lie_milli, cli.num_solutions);
                    tx.send(miner(miner_msg::Msg::Result(result)))
                        .await
                        .map_err(|_| ExitCode::InternalFatal)?;
                }
                tx.send(miner(miner_msg::Msg::JobRequest(JobRequest { credits: 1 })))
                    .await
                    .map_err(|_| ExitCode::InternalFatal)?;
            }
            Some(coord_msg::Msg::Shutdown(_)) => break,
            Some(coord_msg::Msg::Cancel(_) | coord_msg::Msg::SetTarget(_)) | None => {}
        }
    }

    drop(tx);
    let drain = async {
        while inbound.message().await?.is_some() {}
        Ok::<(), tonic::Status>(())
    };
    let _ = tokio::time::timeout(Duration::from_secs(5), drain).await;

    if fatal_sent {
        Err(ExitCode::InternalFatal)
    } else {
        Ok(())
    }
}
