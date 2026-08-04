// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hermetic fake miner for `isingmark`'s own coordinator-session tests.
//!
//! Speaks the real `MinerService.Session` wire protocol with `quip-proto`
//! types, dialing the coordinator over the Unix socket given on
//! `--quip-coordinator`, exactly like a real miner would. Modeled on
//! `quip-mock-miner/src/main.rs` (read-only reference; not modified here)
//! but with its behavior driven entirely by CLI flags, so
//! `tests/session_conformance.rs` can exercise the coordinator's handshake,
//! dispatch, result, reject, and fatal-mid-flight paths without any of the
//! five real miner binaries installed.

use std::process::ExitCode as ProcessExitCode;
use std::time::Duration;

use clap::Parser;
use quip_proto::v1::miner_service_client::MinerServiceClient;
use quip_proto::v1::{
    coord_msg, ising_problem, miner_msg, CoordMsg, Fatal, IsingProblem, Job, JobKind, JobRequest,
    MinerMsg, Ready, Reject, RejectReason, Result as JobResult, SamplerMeta, Solution, Topology,
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
    /// Credits requested in the first `JobRequest`, sent right after
    /// `Ready`. A value greater than 1 lets several jobs be in flight at
    /// once, which `--fatal-after` can then abandon mid-session.
    #[arg(long, default_value_t = 1)]
    credits: u32,
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

/// Solve `job` trivially (all-`+1` spins, like `quip-mock-miner`), reporting
/// `energy_lie_milli` added on top of the true recomputed energy.
fn solve(job: &Job, topo: Option<&FakeTopo>, energy_lie_milli: i64) -> JobResult {
    let ising = job.ising.clone().unwrap_or_default();
    let h = decode_milli_f64(&ising.h_milli_le32).unwrap_or_default();
    let j = decode_milli_f64(&ising.j_milli_le32).unwrap_or_default();
    let edges = resolve_edges(&ising, topo);
    let spins = vec![1i8; h.len()];
    let true_energy = energy_milli(&spins, &h, &j, &edges);
    JobResult {
        job_id: job.job_id.clone(),
        solutions: vec![Solution {
            spins_bytes: encode_spins(&spins),
            energy_milli: true_energy + energy_lie_milli,
        }],
        meta: Some(SamplerMeta {
            reads: 1,
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
    tx.send(miner(miner_msg::Msg::Hello(hello)))
        .await
        .map_err(|_| ExitCode::InternalFatal)?;

    let mut inbound = client
        .session(ReceiverStream::new(rx))
        .await
        .map_err(|_| ExitCode::InternalFatal)?
        .into_inner();

    let mut topo: Option<FakeTopo> = None;
    let mut jobs_seen: u32 = 0;
    let mut fatal_sent = false;

    loop {
        let next = tokio::time::timeout(Duration::from_secs(30), inbound.message()).await;
        let cm: CoordMsg = match next {
            Err(_) | Ok(Ok(None)) => break,
            Ok(Ok(Some(cm))) => cm,
            Ok(Err(_)) => return Err(ExitCode::InternalFatal),
        };
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
            Some(coord_msg::Msg::Job(job)) => {
                jobs_seen += 1;
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
                    let result = solve(&job, topo.as_ref(), cli.energy_lie_milli);
                    tx.send(miner(miner_msg::Msg::Result(result)))
                        .await
                        .map_err(|_| ExitCode::InternalFatal)?;
                }
                tx.send(miner(miner_msg::Msg::JobRequest(JobRequest { credits: 1 })))
                    .await
                    .map_err(|_| ExitCode::InternalFatal)?;
            }
            Some(coord_msg::Msg::Shutdown(_)) => break,
            Some(
                coord_msg::Msg::Cancel(_) | coord_msg::Msg::Ping(_) | coord_msg::Msg::SetTarget(_),
            )
            | None => {}
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
