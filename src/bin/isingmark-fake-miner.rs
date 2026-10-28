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
//!
//! # Solve quality depends on `num_reads` when it is set
//!
//! `solve`'s default behavior (one solution, all-`+1` spins) is unchanged
//! when a job leaves `IsingProblem.num_reads` at `0` ("unset" — every path
//! except `isingmark::modes::sweep` and `isingmark::modes::comparative`).
//! When a job *does* set `num_reads` (nonzero), `solve` additionally runs
//! [`multi_start_search`]: a crude, deterministic, non-random multi-restart
//! local search whose energy is provably non-increasing in `num_reads` (see
//! that function's doc). This exists solely so `isingmark::modes::comparative`
//! (Task 12) has something real to search over — a fake miner whose
//! solutions never depended on the requested sampling effort would make
//! every hardness value look identical to that search, and no binary
//! search could recover anything. It is **not** a model of a real
//! annealer's behavior in any quantitative sense, only of the qualitative
//! "more reads finds a same-or-better solution" property.

use std::process::ExitCode as ProcessExitCode;
use std::time::Duration;

use clap::Parser;
use quip_proto::v1::miner_service_client::MinerServiceClient;
use quip_proto::v1::{
    coord_msg, ising_problem, miner_msg, Algorithm, Backend, Capabilities, CoefficientEncoding,
    CoordMsg, Fatal, IsingProblem, Job, JobKind, JobRequest, MinerMsg, Ready, Reject, RejectReason,
    Result as JobResult, SamplerMeta, Solution, Status, Topology,
};
use quip_protocol::scoring::energy_milli;
use quip_protocol::session::{build_hello, check_welcome, ExitCode, SessionError};
use quip_protocol::wire::{decode_i32_le, encode_spins_packed};
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
    /// `Capabilities.protocol_version` to advertise in `Hello`. A value other than the real
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
    /// QPU access time in microseconds to report for each complete batch.
    #[arg(long, default_value_t = 0)]
    qpu_access_us: u64,
    /// Raw sampler metadata entry. Repeat to add keys. The last value for a key wins.
    #[arg(long, value_name = "KEY=VALUE", value_parser = parse_meta_extra)]
    meta_extra: Vec<(String, String)>,
    /// Delay before sending each result, in milliseconds.
    #[arg(long, default_value_t = 0)]
    result_delay_ms: u64,
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
    /// After the handshake, accept `Job` messages without ever answering
    /// any of them (no `Result`, no `Reject`, no further `JobRequest`), and
    /// once this many milliseconds pass with no further inbound message,
    /// print `jobs_received=<count>` to stderr and exit non-zero. Used to
    /// observe exactly how many jobs one `JobRequest` grant actually
    /// dispatched — proving the coordinator's per-request dispatch cap on a
    /// well-behaved (not crashed, not wedged) run, since a miner that never
    /// asks for more only ever receives what that one grant produced.
    #[arg(long)]
    report_jobs_received_after_ms: Option<u64>,
    /// 1-based ordinal: after solving that job, also send a `Result` with a
    /// synthetic unknown `job_id` that the coordinator never dispatched.
    /// Exercises `RunReport::orphan_results` (observability for stray /
    /// duplicate terminal messages) without changing coordinator dispatch.
    #[arg(long)]
    stray_result_after: Option<u32>,
}

fn parse_meta_extra(text: &str) -> Result<(String, String), String> {
    let (key, value) = text
        .split_once('=')
        .ok_or_else(|| "expected KEY=VALUE".to_owned())?;
    Ok((key.to_owned(), value.to_owned()))
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

/// Upper bound on [`multi_start_search`]'s restart count, so a
/// misconfigured or adversarial `num_reads` (this crude model's only input)
/// cannot make one job take an unbounded amount of CPU time. Comfortably
/// above any curve this binary is actually driven at in `isingmark`'s own
/// tests (`isingmark::modes::comparative::DEFAULT_PARAM_CURVE`'s
/// `max_reads: 64`, the largest bound any mode currently resolves to).
const MAX_SEARCH_RESTARTS: u32 = 4_096;

/// `SplitMix64`'s mixing step (Vigna, 2015), used only to turn a small
/// integer `step` into a well-scrambled 64-bit word for [`scan_pattern`].
/// Deterministic and stateless (no external RNG, no seed to persist) —
/// exactly what this binary's "same job, same parameters, same output"
/// contract requires.
fn splitmix64(step: u64) -> u64 {
    let mut z = step.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministically derive spin pattern `step` of a pseudo-random scan
/// over `{-1, +1}^n`: bit `i` (mod 64) of `splitmix64(step)` selects `+1`
/// (`0`) or `-1` (`1`) for spin `i`. Different `step` values scramble to
/// unrelated-looking bit patterns, so scanning `step = 0, 1, 2, ...` behaves
/// like sampling without an obvious short repeating cycle, even though it
/// is fully deterministic given `step`/`n`. For `n > 64` the pattern
/// repeats every 64 spins (only 64 mixed bits exist to draw from) — a
/// known, crude limitation, harmless for this binary's own hermetic tests
/// (the bundled fixtures are far smaller than 64 nodes).
fn scan_pattern(step: u64, n: usize) -> Vec<i8> {
    let mixed = splitmix64(step);
    (0..n)
        .map(|i| {
            if (mixed >> (i % 64)) & 1 == 0 {
                1i8
            } else {
                -1i8
            }
        })
        .collect()
}

/// Deterministic, crude "solve quality" model driving
/// `isingmark`'s comparative-mode hardness search (Task 12): the coordinator
/// (`isingmark::backend::session::build_result_record`, already-reviewed,
/// not modified here) recomputes `best_energy_milli` itself from the
/// returned `spins` — never from a miner's own reported
/// `Solution.energy_milli` — so the only way for this fake miner to make its
/// *actual* solve quality depend on the requested sampling effort is to
/// return spins that really are better, not merely to shift a reported
/// number.
///
/// Evaluates `restarts` independent [`scan_pattern`] draws, seeded `0, 1,
/// ..., restarts - 1`, plus the trivial all-`+1` baseline, and returns
/// whichever reached the lowest true energy — literal multi-start random
/// sampling, deliberately with **no** local refinement (no greedy flip
/// passes, no accepted uphill moves, no temperature schedule): an earlier
/// version of this function did add a greedy single-spin-flip descent pass
/// per restart, but on the bundled fixtures (8-node rings) that reaches a
/// global optimum from nearly any starting point in a single restart,
/// saturating so fast that every hardness above the curve's minimum looked
/// identical — exactly the flat, non-distinguishable response the
/// comparative-mode search cannot work with. Raw, non-refined sampling
/// converges far more slowly (a global optimum among `2^n` configurations
/// generally needs a number of draws on the order of `2^n`, not `n`), which
/// is what actually gives `num_reads` in the tens-to-hundreds range a
/// visibly graduated effect on this crude model's output. This is **not** a
/// real annealer's behavior in any quantitative sense.
///
/// The one guarantee this function does provide, and the one the
/// comparative-mode search actually relies on, is monotonicity: the set of
/// patterns a larger `restarts` count tries is a strict superset of a
/// smaller one's (`0..restarts` extends `0..restarts'` for `restarts >
/// restarts'`), so `multi_start_search(restarts)` can only find an energy
/// less than or equal to `multi_start_search(restarts')`'s whenever
/// `restarts >= restarts'`, for any job.
fn multi_start_search(h: &[f64], j: &[f64], edges: &[(usize, usize)], restarts: u32) -> Vec<i8> {
    let n = h.len();
    let mut best_spins = vec![1i8; n];
    let mut best_energy = energy_milli(&best_spins, h, j, edges);
    for step in 0..u64::from(restarts) {
        let spins = scan_pattern(step, n);
        let energy = energy_milli(&spins, h, j, edges);
        if energy < best_energy {
            best_energy = energy;
            best_spins = spins;
        }
    }
    best_spins
}

/// Solve `job`, reporting `num_solutions` distinct-spin solutions (solution
/// `i` flips the first `i` node spins negative, capped at the node count),
/// each with `energy_lie_milli` added on top of its true recomputed energy.
/// `num_solutions <= 1` reproduces the original trivial all-`+1` behavior.
///
/// `SamplerMeta.reads`/`sweeps` echo the job's own `IsingProblem.num_reads`/
/// `num_sweeps` wire fields when the coordinator set them (nonzero), rather
/// than always reporting `num_solutions`/`0`: `isingmark::modes::sweep`
/// drives per-job parameters through exactly these fields (never a
/// session-level `SetTarget`), and a hermetic test asserting they reached
/// the wire needs a miner that actually reflects what it received instead
/// of a fixed value the harness never gets to see through. Falls back to
/// the prior `num_solutions`/`0` behavior when the job leaves them `0`
/// ("unset" — `problem::ProblemSource::next_job` always does this outside
/// of sweep mode), so every other test driving this binary is unaffected.
///
/// When `ising.num_reads != 0` (the wire's own "unset" sentinel — see
/// `params::ParamCurve`'s doc), one extra solution is appended: the result
/// of [`multi_start_search`] run for `min(ising.num_reads, `
/// [`MAX_SEARCH_RESTARTS`]`)` restarts. This is strictly additive — every
/// solution the old path produced is still produced unchanged — so
/// `n_solutions` grows by exactly one rather than replacing anything, and a
/// job that never sets `num_reads` (every path except
/// `isingmark::modes::sweep` and `isingmark::modes::comparative`) takes
/// exactly the old code path with no extra solution and no behavior change,
/// keeping every prior test's expected energies and solution counts intact.
/// `num_sweeps`/`anneal_time_us` are still echoed into `SamplerMeta` as
/// before but do not otherwise influence solve quality in this crude model
/// — only `num_reads` (via its restart count) does.
fn solve(
    job: &Job,
    topo: Option<&FakeTopo>,
    energy_lie_milli: i64,
    num_solutions: u32,
) -> JobResult {
    let ising = job.ising.clone().unwrap_or_default();
    let h = decode_milli_f64(&ising.h).unwrap_or_default();
    let j = decode_milli_f64(&ising.j).unwrap_or_default();
    let edges = resolve_edges(&ising, topo);
    let n = h.len();
    let count = num_solutions.max(1);

    let mut solutions: Vec<Solution> = (0..count)
        .map(|i| {
            let flips = (i as usize).min(n);
            let mut spins = vec![1i8; n];
            for spin in spins.iter_mut().take(flips) {
                *spin = -1;
            }
            let true_energy = energy_milli(&spins, &h, &j, &edges);
            Solution {
                spins: encode_spins_packed(&spins),
                energy_milli: true_energy + energy_lie_milli,
            }
        })
        .collect();

    if ising.num_reads != 0 {
        let restarts = ising.num_reads.min(MAX_SEARCH_RESTARTS);
        let spins = multi_start_search(&h, &j, &edges, restarts);
        let true_energy = energy_milli(&spins, &h, &j, &edges);
        solutions.push(Solution {
            spins: encode_spins_packed(&spins),
            energy_milli: true_energy + energy_lie_milli,
        });
    }

    let reads = if ising.num_reads == 0 {
        count
    } else {
        ising.num_reads
    };

    JobResult {
        job_id: job.job_id.clone(),
        solutions,
        meta: Some(SamplerMeta {
            reads,
            sweeps: ising.num_sweeps,
            device_access_time_us: 0,
            ..Default::default()
        }),
        ..Default::default()
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

    let hello = build_hello(
        &cli.miner_id,
        Capabilities {
            supported_kinds: vec![JobKind::IsingSample as i32],
            protocol_version: cli.protocol_version,
            encodings: vec![CoefficientEncoding::I32 as i32],
            backend: Backend::Mock as i32,
            algorithm: Algorithm::External as i32,
            ..Capabilities::default()
        },
    )
    .map_err(|e: SessionError| ExitCode::from(e))?;

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
        // `--report-jobs-received-after-ms` shortens this wait so it can
        // detect its own "N ms of quiet" condition rather than waiting out
        // the ordinary 30s idle bound.
        let idle_wait = cli
            .report_jobs_received_after_ms
            .map_or(Duration::from_secs(30), Duration::from_millis);
        let next = tokio::time::timeout(idle_wait, inbound.message()).await;
        let cm: CoordMsg = match next {
            Err(_) => {
                if cli.report_jobs_received_after_ms.is_some() {
                    #[expect(
                        clippy::print_stderr,
                        reason = "test-only flag reporting how many Job messages arrived \
                                  before a quiet period, for isingmark's dispatch-cap test"
                    )]
                    {
                        eprintln!("jobs_received={jobs_seen}");
                    }
                    return Err(ExitCode::EnvIncompatible);
                }
                break;
            }
            Ok(Ok(None)) => break,
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
                if cli.report_jobs_received_after_ms.is_some() {
                    // Accept silently: this mode measures what one
                    // JobRequest grant dispatched, not any per-job
                    // response, and requesting more credits would let the
                    // coordinator dispatch beyond the cap this test targets.
                    continue;
                }
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
                    if cli.stray_result_after == Some(jobs_seen) {
                        // Send the unknown-id Result *before* the real one so
                        // the coordinator still has the real job in-flight and
                        // does not shut down before reading the orphan.
                        let stray = JobResult {
                            job_id: b"isingmark-stray-orphan-job-id".to_vec(),
                            solutions: Vec::new(),
                            meta: None,
                            ..Default::default()
                        };
                        if cli.result_delay_ms > 0 {
                            tokio::time::sleep(Duration::from_millis(cli.result_delay_ms)).await;
                        }
                        tx.send(miner(miner_msg::Msg::Result(stray)))
                            .await
                            .map_err(|_| ExitCode::InternalFatal)?;
                    }
                    let mut result =
                        solve(&job, topo.as_ref(), cli.energy_lie_milli, cli.num_solutions);
                    if let Some(meta) = &mut result.meta {
                        meta.qpu_access_us = cli.qpu_access_us;
                        meta.extra = cli.meta_extra.iter().cloned().collect();
                    }
                    if cli.result_delay_ms > 0 {
                        tokio::time::sleep(Duration::from_millis(cli.result_delay_ms)).await;
                    }
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
                coord_msg::Msg::Cancel(_)
                | coord_msg::Msg::SetTarget(_)
                | coord_msg::Msg::GetCapabilities(_),
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
