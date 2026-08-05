// SPDX-License-Identifier: AGPL-3.0-or-later
//! Scaling mode: benchmark a fixed hardness against a randomly generated
//! 3-regular topology at each of several requested node counts, so a result
//! at one size is comparable to another's — only the topology's node count
//! differs, never the sampling effort.
//!
//! # This mode generates its own topology; it does not take one
//!
//! Every other mode takes a [`crate::modes::TopologyRef`] (a bundled fixture
//! or a JSON file). Scaling instead builds a fresh [`TopologySpec`] per
//! requested size via [`generate_topology`]: a random 3-regular graph on
//! that many nodes, so `--sizes "8,16,32"` benchmarks the same *kind* of
//! topology at three different sizes rather than three unrelated fixtures.
//!
//! # Deterministic, genuinely random graph generation
//!
//! [`generate_topology`] must satisfy two things that pull in opposite
//! directions: the graph must be **random** (not some fixed formula that
//! only depends on the node count, e.g. a circulant graph — a benchmark
//! whose "random" topology is secretly the same shape every time would
//! hide any structural effect a real random 3-regular graph has on solve
//! quality), yet **exactly reproducible** given the same `--seed`/`--sizes`
//! (a benchmark whose topology changes run to run is not comparable across
//! runs at all). This module resolves that with its own small, hand-rolled
//! BLAKE3 counter-mode PRNG ([`DeterministicRng`]) rather than a `rand`-family
//! crate (this task adds no new dependencies; `blake3` is already pinned and
//! is exactly what `problem::ProblemSource::salt_for` already uses for its
//! own deterministic derivation): the PRNG's key is derived from `(seed,
//! nodes)` via [`topology_seed_key`], with a domain-separation prefix distinct
//! from `salt_for`'s own, so this module's draws can never collide with — or
//! be confused for — `ProblemSource`'s.
//!
//! The graph itself is built by the standard configuration-model
//! construction: list every node 3 times ("stubs"), shuffle, and pair up
//! consecutive stubs into edges, retrying the whole shuffle-and-pair attempt
//! (see [`generate_3_regular_edges`]) whenever a pairing produces a
//! self-loop or a duplicate edge — both of which a 3-regular *simple* graph
//! (no self-loops, no parallel edges) must not have. [`MAX_GENERATION_ATTEMPTS`]
//! bounds the retry loop so this stays total.
//!
//! # `job_id` collisions across sizes: one directory, one session, per size
//!
//! `problem::ProblemSource`'s derived `job_id` depends only on `(seed,
//! index)` — never on the topology being benchmarked (see that module's own
//! doc) — so the *same* `(seed, index)` draws the identical `job_id` at
//! every requested size. Two sizes must therefore never share one records
//! file or one `run_session` call: sharing a file would let a later size's
//! record silently overwrite an earlier size's line at the same position,
//! and `backend::session`'s in-flight-job map (keyed on `job_id` alone, with
//! no duplicate guard — a known P2 gap, not fixed here) could otherwise
//! collide two same-`job_id` jobs from different sizes within one session.
//! This mode gives every size its own `<output_dir>/size_<n>/records.jsonl`
//! (see [`size_dir_name`]) and its own `run_session` call, run sequentially,
//! exactly the shape `modes::sweep`/`modes::ttt`/`modes::comparative`
//! already establish for their own per-group units of work. The eager
//! [`ModeError::RecordsFileExists`] eager pre-pass runs across every
//! requested size's directory before any of them is dispatched, mirroring
//! those modules' own identical reasoning.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::backend::{builtin_registry, run_session, BackendSpec, RunParams, RunReport};
use crate::modes::{hex_encode, ModeError, ParseError};
use crate::params::ParamCurve;
use crate::problem::ProblemSource;
use crate::record::{append_jsonl, write_summary, HardwareInfo, Summary, Totals};
use crate::stats::{describe, Stats};
use crate::topology::TopologySpec;

/// Basename of one size's JSONL record file within its own subdirectory of
/// [`ScalingConfig::output_dir`] (see [`size_dir_name`]).
pub const RECORDS_FILENAME: &str = "records.jsonl";
/// Basename of the run's closing summary within [`ScalingConfig::output_dir`].
pub const SUMMARY_FILENAME: &str = "summary.json";

/// Smallest node count that can support a 3-regular graph: every node needs
/// 3 distinct neighbours, which is impossible below 4 nodes.
const MIN_REGULAR_SIZE: usize = 4;

/// Miner account every size derives jobs against. Only affects the derived
/// nonce (via `problem::ProblemSource`), not this run's results — the same
/// reasoning `sweep`/`ttt`/`comparative` give for their own copies of this
/// constant.
const MINER_ACCOUNT: [u8; 32] = [0u8; 32];

/// No wire deadline: this mode measures its own completion and does not need
/// the miner to expire jobs on its own clock.
const NO_DEADLINE_MS: u64 = 0;

/// Allowed h-field values for a generated topology, matching the bundled
/// `smoke` fixture's own nonzero value set (`fixtures/smoke.spec.json`) —
/// there is no "natural" choice for a topology this module invents itself,
/// so this reuses the one already exercised by every other hermetic test in
/// this crate.
const GENERATED_ALLOWED_H_MILLI: [i32; 3] = [-1000, 0, 1000];
/// Allowed J-coupling values for a generated topology; see
/// [`GENERATED_ALLOWED_H_MILLI`].
const GENERATED_ALLOWED_J_MILLI: [i32; 2] = [-1000, 1000];
/// Allowed solution-spin values for a generated topology: the same binary
/// set `topology::load_topology` defaults to when a JSON spec omits it.
const GENERATED_ALLOWED_SPIN_MILLI: [i32; 2] = [-1000, 1000];

/// Upper bound on [`generate_3_regular_edges`]'s shuffle-and-pair retry
/// loop. The configuration model's probability of producing a self-loop or
/// duplicate edge on one attempt is small and roughly constant for the node
/// counts this crate's own tests and any realistic `--sizes` value use
/// (a handful to a few hundred nodes), so this attempt budget is reached
/// only in the presence of a bug, not as ordinary bad luck.
const MAX_GENERATION_ATTEMPTS: u32 = 2_000;

/// Default hardness-to-parameter curve, held fixed across every requested
/// size. A separate copy from `comparative::DEFAULT_PARAM_CURVE`/
/// `throughput::DEFAULT_PARAM_CURVE`, per this crate's per-mode-module
/// convention of not sharing small constants like this one across modules;
/// placeholder until `--config`'s `ParamCurve` section (a later task) lets
/// an operator override it.
const DEFAULT_PARAM_CURVE: ParamCurve = ParamCurve {
    min_reads: 8,
    max_reads: 64,
    min_sweeps: 100,
    max_sweeps: 1000,
    min_anneal_us: 5,
    max_anneal_us: 200,
};

/// Configuration for [`run_scaling`].
#[derive(Debug, Clone)]
pub struct ScalingConfig {
    /// `backend::BackendSpec::name` to benchmark (e.g. `"cpu-sa"`).
    pub backend: String,
    /// Node counts to generate a random 3-regular topology for. Every entry
    /// must be even and at least [`MIN_REGULAR_SIZE`] (see
    /// [`ModeError::InvalidSize`]) and the list must not repeat a value
    /// (see [`ModeError::DuplicateSize`]).
    pub sizes: Vec<usize>,
    /// Difficulty in `[0.0, 1.0]`, held fixed across every size — the point
    /// of this mode is to isolate the effect of topology size alone.
    pub hardness: f64,
    /// Jobs drawn per size. Every size draws its own `(cfg.seed,
    /// num_jobs)`-keyed sequence against its own generated topology (sizes
    /// cannot share a paired-design job sequence the way `sweep`'s cells do,
    /// since each size's topology has a different node/edge count).
    pub num_jobs: u32,
    /// Seed for both deterministic problem generation and deterministic
    /// topology generation (see [`generate_topology`]).
    pub seed: u64,
    /// Directory to write one subdirectory per size into, plus a top-level
    /// [`SUMMARY_FILENAME`]. Created if it does not already exist. Refuses
    /// to start (see [`ModeError::RecordsFileExists`]) if any size's
    /// [`RECORDS_FILENAME`] already contains data from an earlier run —
    /// checked for every size before any of them run.
    pub output_dir: PathBuf,
    /// Explicit backend binary path, bypassing `$PATH` resolution. See
    /// `throughput::ThroughputConfig::backend_bin_override`.
    pub backend_bin_override: Option<PathBuf>,
    /// Explicit backend CLI args, replacing `backend::BackendSpec::args`.
    /// See `throughput::ThroughputConfig::backend_args_override`.
    pub backend_args_override: Option<Vec<String>>,
}

/// One dispatched-and-completed size of a scaling run.
///
/// Does not derive `Debug`: `backend::RunReport` does not implement it
/// (same reasoning `sweep::CellOutcome` documents), and this task does not
/// modify already-reviewed code to add that derive.
pub struct SizeOutcome {
    /// Node count this outcome was generated and dispatched for.
    pub nodes: usize,
    /// Edge count of this size's actually generated topology, read from
    /// `TopologySpec::edges.len()` — not recomputed from `3 * nodes / 2`.
    /// A generator bug that produces a non-regular graph (wrong edge count)
    /// must show up here; a formula derived from `nodes` alone could never
    /// disagree with itself and would hide exactly that bug.
    pub num_edges: usize,
    /// This size's subdirectory name under `ScalingConfig::output_dir` (see
    /// [`size_dir_name`]).
    pub dir_name: String,
    /// Hex-encoded `topology::TopologySpec::topology_hash()` of this size's
    /// generated topology.
    pub topology_hash: String,
    /// This size's own session outcome.
    pub report: RunReport,
}

/// Outcome of [`run_scaling`].
pub struct ScalingReport {
    /// One entry per requested size, in `ScalingConfig::sizes` order.
    pub sizes: Vec<SizeOutcome>,
    /// The run summary also written to [`SUMMARY_FILENAME`], returned here
    /// too so a caller (namely `main.rs`) does not have to read that file
    /// back off disk and re-parse it.
    pub summary: Summary,
}

/// Run scaling mode: for every size in `cfg.sizes`, generate a random
/// 3-regular topology (see [`generate_topology`]), draw `cfg.num_jobs`
/// deterministic jobs at `cfg.hardness` against it, and dispatch them in
/// their own session against `cfg.backend`. Writes each size's records to
/// its own `<size dir>/records.jsonl` as it completes, and a closing
/// [`SUMMARY_FILENAME`] once every size is done.
///
/// # Errors
///
/// Returns [`ModeError::EmptySizes`] if `cfg.sizes` is empty,
/// [`ModeError::InvalidSize`] if any entry cannot support a 3-regular graph,
/// [`ModeError::DuplicateSize`] if the list repeats a value,
/// [`ModeError::UnknownBackend`] if `cfg.backend` matches no entry in
/// `backend::builtin_registry`, [`ModeError::GraphGenerationFailed`] if a
/// size's topology generation exhausts its retry budget,
/// [`ModeError::RecordsFileExists`] if any size's records file already
/// contains data, [`ModeError::Session`] if a size's session fails to bind
/// or spawn, and [`ModeError::Io`] if the output directory or any file
/// cannot be written.
pub async fn run_scaling(cfg: &ScalingConfig) -> Result<ScalingReport, ModeError> {
    validate_sizes(&cfg.sizes)?;
    let backend = resolve_backend(cfg)?;

    std::fs::create_dir_all(&cfg.output_dir).map_err(ModeError::Io)?;

    let size_paths: Vec<(usize, PathBuf, PathBuf, String)> = cfg
        .sizes
        .iter()
        .map(|&nodes| {
            let dir_name = size_dir_name(nodes);
            let dir = cfg.output_dir.join(&dir_name);
            let records_path = dir.join(RECORDS_FILENAME);
            (nodes, dir, records_path, dir_name)
        })
        .collect();

    // Eager, whole-list conflict check before anything is created — see this
    // module's doc and `modes::sweep`'s/`modes::ttt`'s identical reasoning.
    for (_, _, records_path, _) in &size_paths {
        if std::fs::metadata(records_path).is_ok_and(|m| m.len() > 0) {
            return Err(ModeError::RecordsFileExists(records_path.clone()));
        }
    }

    let params = DEFAULT_PARAM_CURVE.resolve(backend.kind, cfg.hardness);
    let mut outcomes = Vec::with_capacity(size_paths.len());

    for (nodes, dir, records_path, dir_name) in &size_paths {
        std::fs::create_dir_all(dir).map_err(ModeError::Io)?;
        std::fs::File::create(records_path).map_err(ModeError::Io)?;

        let spec = generate_topology(cfg.seed, *nodes)?;
        let jobs = draw_jobs(&spec, cfg.seed, cfg.num_jobs, &params);

        let sock_path = unique_sock_path();
        let report = run_session(RunParams {
            backend: &backend,
            spec: &spec,
            jobs,
            sock_path: &sock_path,
            target: None,
        })
        .await?;

        for record in &report.records {
            append_jsonl(records_path, record).map_err(ModeError::Io)?;
        }

        outcomes.push(SizeOutcome {
            nodes: *nodes,
            num_edges: spec.edges.len(),
            dir_name: dir_name.clone(),
            topology_hash: hex_encode(&spec.topology_hash()),
            report,
        });
    }

    let summary = build_summary(cfg, &params, &outcomes);
    write_summary(&cfg.output_dir.join(SUMMARY_FILENAME), &summary).map_err(ModeError::Io)?;

    Ok(ScalingReport {
        sizes: outcomes,
        summary,
    })
}

/// Resolve `cfg.backend` against the built-in registry, applying
/// `backend_bin_override`/`backend_args_override` if set. Mirrors
/// `sweep::resolve_backend`/`ttt::resolve_backend`/
/// `comparative::resolve_backend`/`throughput::resolve_backend` exactly;
/// kept as its own copy per this crate's existing per-mode-module
/// convention.
fn resolve_backend(cfg: &ScalingConfig) -> Result<BackendSpec, ModeError> {
    let mut backend = builtin_registry()
        .into_iter()
        .find(|b| b.name == cfg.backend)
        .ok_or_else(|| ModeError::UnknownBackend(cfg.backend.clone()))?;
    if let Some(bin) = &cfg.backend_bin_override {
        backend.bin.clone_from(bin);
    }
    if let Some(args) = &cfg.backend_args_override {
        backend.args.clone_from(args);
    }
    Ok(backend)
}

/// Reject an empty size list, a size that cannot support a 3-regular graph
/// (odd, or below [`MIN_REGULAR_SIZE`]), or a list containing the same size
/// twice.
///
/// # Errors
///
/// Returns [`ModeError::EmptySizes`], [`ModeError::InvalidSize`], or
/// [`ModeError::DuplicateSize`] — see each variant's own doc.
fn validate_sizes(sizes: &[usize]) -> Result<(), ModeError> {
    if sizes.is_empty() {
        return Err(ModeError::EmptySizes);
    }
    let mut seen = HashSet::with_capacity(sizes.len());
    for &n in sizes {
        if n < MIN_REGULAR_SIZE || !n.is_multiple_of(2) {
            return Err(ModeError::InvalidSize(n));
        }
        if !seen.insert(n) {
            return Err(ModeError::DuplicateSize(n));
        }
    }
    Ok(())
}

/// This size's subdirectory name under [`ScalingConfig::output_dir`]:
/// `size_<n>` — deterministic and disambiguated purely by the requested
/// node count, since [`validate_sizes`] already rejects a duplicate size.
#[must_use]
fn size_dir_name(nodes: usize) -> String {
    format!("size_{nodes}")
}

/// Draw `count` deterministic jobs from a *fresh* [`ProblemSource`] built
/// from `(spec, seed, count)`, setting `params`'s resolved sample parameters
/// directly on each job's `IsingProblem` fields. Mirrors
/// `sweep::Cell::draw_jobs`/`comparative::draw_jobs`, specialized to a single
/// already-resolved `params::JobParams`.
fn draw_jobs(
    spec: &TopologySpec,
    seed: u64,
    count: u32,
    params: &crate::params::JobParams,
) -> Vec<quip_proto::v1::Job> {
    let mut source = ProblemSource::new(spec, MINER_ACCOUNT, seed, count, NO_DEADLINE_MS);
    let mut jobs = Vec::with_capacity(count as usize);
    while let Some(mut job) = source.next_job() {
        if let Some(ising) = job.ising.as_mut() {
            ising.num_reads = params.num_reads;
            ising.num_sweeps = params.num_sweeps;
            ising.anneal_time_us = params.anneal_time_us;
        }
        jobs.push(job);
    }
    jobs
}

/// A hand-rolled, BLAKE3-backed counter-mode PRNG: `next_u64` hashes a fixed
/// 32-byte key together with an incrementing counter, so the same key always
/// yields the same sequence of draws, deterministically, with no external
/// randomness source. This is not a cryptographic guarantee of any kind
/// (nothing here needs one) — it exists solely so [`generate_3_regular_edges`]
/// can shuffle without pulling in a `rand`-family crate, which this task's
/// constraints forbid.
struct DeterministicRng {
    key: [u8; 32],
    counter: u64,
}

impl DeterministicRng {
    fn new(key: [u8; 32]) -> Self {
        Self { key, counter: 0 }
    }

    /// Next pseudo-random `u64`, advancing this generator's internal
    /// counter so the next call returns a different value.
    fn next_u64(&mut self) -> u64 {
        let mut hasher = blake3::Hasher::new();
        let _ = hasher.update(&self.key);
        let _ = hasher.update(&self.counter.to_le_bytes());
        self.counter += 1;
        let digest = hasher.finalize();
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&digest.as_bytes()[..8]);
        u64::from_le_bytes(buf)
    }

    /// Uniform integer in `[0, bound)`, via rejection sampling against the
    /// largest multiple of `bound` not exceeding `u64::MAX` — avoids the
    /// small bias a plain `next_u64() % bound` would introduce whenever
    /// `bound` does not evenly divide `2^64`. `bound` must be nonzero; every
    /// call site in this module derives it from a nonempty stub list, so
    /// this is never exercised at `bound == 0`.
    fn below(&mut self, bound: u64) -> u64 {
        debug_assert!(bound > 0, "below(0) has no valid result");
        let limit = u64::MAX - (u64::MAX % bound);
        loop {
            let draw = self.next_u64();
            if draw < limit {
                return draw % bound;
            }
        }
    }
}

/// Fisher-Yates shuffle of `items`, in place, using `rng`.
fn shuffle<T>(items: &mut [T], rng: &mut DeterministicRng) {
    for i in (1..items.len()).rev() {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "i + 1 <= items.len(), and this module's own callers never shuffle a stub \
                      list anywhere near usize::MAX in length"
        )]
        let j = rng.below((i + 1) as u64) as usize;
        items.swap(i, j);
    }
}

/// Derive [`DeterministicRng`]'s key from `(seed, nodes)`: `blake3` of a
/// fixed domain-separation prefix followed by both values, little-endian.
/// The prefix keeps this derivation's output space disjoint from
/// `problem::ProblemSource::salt_for`'s own `blake3(seed || index)`
/// derivation, even though both hash the same `seed` value — a shared
/// prefix would not actually cause any observable bug (the two draws feed
/// entirely different consumers), but keeping them provably distinct costs
/// nothing and rules the possibility out entirely.
fn topology_seed_key(seed: u64, nodes: usize) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    let _ = hasher.update(b"isingmark-scaling-topology");
    let _ = hasher.update(&seed.to_le_bytes());
    // `usize::to_le_bytes` (not a cast to `u64`): no narrowing occurs on any
    // platform, so this needs no truncation lint expectation at all.
    let _ = hasher.update(&nodes.to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// Generate a random 3-regular edge list on `nodes` nodes (`0..nodes as
/// u32`) via the configuration model: list every node 3 times, shuffle, and
/// pair up consecutive stubs into edges. Retries the whole shuffle-and-pair
/// attempt (a fresh shuffle each time, continuing to draw from `rng` rather
/// than restarting it) whenever a pairing produces a self-loop or a
/// duplicate edge, up to [`MAX_GENERATION_ATTEMPTS`] times.
///
/// Returns `None` if no valid pairing was found within that budget — see
/// [`MAX_GENERATION_ATTEMPTS`]'s own doc for why this is not expected to
/// occur for any size this crate's callers use.
fn generate_3_regular_edges(nodes: usize, rng: &mut DeterministicRng) -> Option<Vec<(u32, u32)>> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "node counts in this crate's benchmark domain fit comfortably in u32"
    )]
    let node_ids: Vec<u32> = (0..nodes as u32).collect();

    for _ in 0..MAX_GENERATION_ATTEMPTS {
        let mut stubs: Vec<u32> = node_ids.iter().flat_map(|&n| [n, n, n]).collect();
        shuffle(&mut stubs, rng);

        let mut edges = Vec::with_capacity(stubs.len() / 2);
        let mut seen_edges: HashSet<(u32, u32)> = HashSet::with_capacity(stubs.len() / 2);
        let mut valid = true;
        for pair in stubs.chunks_exact(2) {
            let (a, b) = (pair[0], pair[1]);
            if a == b {
                valid = false;
                break;
            }
            let key = if a < b { (a, b) } else { (b, a) };
            if !seen_edges.insert(key) {
                valid = false;
                break;
            }
            edges.push(key);
        }
        if valid {
            return Some(edges);
        }
    }
    None
}

/// Generate a deterministic, random 3-regular [`TopologySpec`] on `nodes`
/// nodes, keyed by `(seed, nodes)` (see [`topology_seed_key`]). See this
/// module's doc for the "random yet reproducible" requirement this
/// satisfies and the configuration-model construction
/// [`generate_3_regular_edges`] performs.
///
/// # Errors
///
/// Returns [`ModeError::GraphGenerationFailed`] if the underlying generator
/// exhausts its retry budget (see [`MAX_GENERATION_ATTEMPTS`]).
fn generate_topology(seed: u64, nodes: usize) -> Result<TopologySpec, ModeError> {
    let key = topology_seed_key(seed, nodes);
    let mut rng = DeterministicRng::new(key);
    let edges =
        generate_3_regular_edges(nodes, &mut rng).ok_or(ModeError::GraphGenerationFailed(nodes))?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "node counts in this crate's benchmark domain fit comfortably in u32"
    )]
    let node_ids: Vec<u32> = (0..nodes as u32).collect();
    Ok(TopologySpec {
        nodes: node_ids,
        edges,
        allowed_h_milli: GENERATED_ALLOWED_H_MILLI.to_vec(),
        allowed_j_milli: GENERATED_ALLOWED_J_MILLI.to_vec(),
        allowed_spin_milli: GENERATED_ALLOWED_SPIN_MILLI.to_vec(),
    })
}

/// Parse one comma-separated `--sizes` entry into a node count, rejecting
/// anything that cannot support a 3-regular graph at CLI parse time rather
/// than deferring to [`validate_sizes`]'s later, mode-level check — see
/// `ttt::parse_one_energy_target`'s identical "per-token CLI parser, plus a
/// mode-level check for a direct-construction caller" split.
///
/// # Errors
///
/// Returns [`ParseError`] if `entry` (after trimming) is empty, not a valid
/// `usize`, odd, or below [`MIN_REGULAR_SIZE`].
pub fn parse_one_scaling_size(entry: &str) -> Result<usize, ParseError> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return Err(ParseError("--sizes contains an empty entry".to_string()));
    }
    let value: usize = trimmed
        .parse()
        .map_err(|_| ParseError(format!("--sizes entry {trimmed:?} is not a valid size")))?;
    if value < MIN_REGULAR_SIZE || !value.is_multiple_of(2) {
        return Err(ParseError(format!(
            "--sizes entry {value} cannot support a 3-regular graph (every node needs 3 \
             distinct neighbours, so a size must be even and at least {MIN_REGULAR_SIZE})"
        )));
    }
    Ok(value)
}

/// Build the closing [`Summary`] from every requested size's completed
/// [`SizeOutcome`]. `totals` aggregates across every size; `config` carries
/// the requested run parameters alongside each size's own directory, node/
/// edge counts, topology hash, and `stats::describe`-derived energy/wall-time
/// distributions.
///
/// `Summary::topology` has no single applicable value here (unlike every
/// other mode, which benchmarks one shared topology): each size generates
/// its own. It is populated with a descriptive label and zeroed counts,
/// with the real per-size detail carried in `config.sizes` instead — see
/// this function's own `topology_json` construction.
fn build_summary(
    cfg: &ScalingConfig,
    params: &crate::params::JobParams,
    outcomes: &[SizeOutcome],
) -> Summary {
    let totals = Totals {
        jobs: outcomes.iter().map(|o| o.report.records.len()).sum(),
        passed: outcomes
            .iter()
            .flat_map(|o| &o.report.records)
            .filter(|r| r.passed)
            .count(),
        rejected: outcomes
            .iter()
            .flat_map(|o| &o.report.records)
            .filter(|r| r.rejected)
            .count(),
        // Every size's records file is created empty and exclusively
        // written by this call, same reasoning as `sweep::build_summary`.
        skipped_lines: 0,
    };

    let sizes_json: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|o| {
            let energies: Vec<f64> = o
                .report
                .records
                .iter()
                .filter(|r| r.passed)
                .map(|r| milli_to_f64(r.best_energy_milli))
                .collect();
            let wall_mss: Vec<f64> = o
                .report
                .records
                .iter()
                .map(|r| wall_ms_to_f64(r.wall_ms))
                .collect();
            serde_json::json!({
                "dir": o.dir_name,
                "nodes": o.nodes,
                "edges": o.num_edges,
                "topology_hash": o.topology_hash,
                "jobs": o.report.records.len(),
                "passed": o.report.records.iter().filter(|r| r.passed).count(),
                "rejected": o.report.records.iter().filter(|r| r.rejected).count(),
                "session_wall_ms": o.report.run_wall_ms,
                "miner_exit_code": o.report.miner_exit_code,
                "miner_unresponsive": o.report.miner_unresponsive,
                "miner_stderr_tail": o.report.miner_stderr_tail,
                "best_energy_milli_stats": stats_json(describe(&energies)),
                "wall_ms_stats": stats_json(describe(&wall_mss)),
            })
        })
        .collect();

    let config = serde_json::json!({
        "backend": cfg.backend,
        "hardness": cfg.hardness,
        "num_reads": params.num_reads,
        "num_sweeps": params.num_sweeps,
        "anneal_time_us": params.anneal_time_us,
        "seed": cfg.seed,
        "num_jobs_per_size": cfg.num_jobs,
        "sizes": sizes_json,
    });

    Summary {
        mode: "scaling".to_string(),
        backend: cfg.backend.clone(),
        topology: crate::record::TopologyInfo {
            name: "random-3-regular (generated per size; see config.sizes)".to_string(),
            num_nodes: 0,
            num_edges: 0,
            topology_hash: String::new(),
        },
        hardware: HardwareInfo::detect(),
        config,
        totals,
    }
}

/// Render a `stats::describe` result as JSON, `null` for an empty or
/// non-finite sample. Mirrors `sweep::stats_json`'s/`ttt::stats_json`'s own
/// copy (`stats::Stats` does not derive `Serialize`, and this task does not
/// add one).
fn stats_json(stats: Option<Stats>) -> serde_json::Value {
    stats.map_or(serde_json::Value::Null, |s| {
        serde_json::json!({
            "min": s.min,
            "max": s.max,
            "mean": s.mean,
            "std": s.std,
            "p50": s.p50,
            "p90": s.p90,
            "p99": s.p99,
        })
    })
}

/// Milli-units to natural `f64` units (divide by 1000), for feeding
/// `stats::describe`. Mirrors `sweep::milli_to_f64`; kept as its own copy
/// per this crate's per-mode-module convention.
#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark energies fit comfortably within f64's 52-bit exact-integer range"
)]
fn milli_to_f64(milli: i64) -> f64 {
    milli as f64 / 1000.0
}

/// `JobRecord::wall_ms` (a `u64` count of milliseconds) to `f64`, for
/// feeding `stats::describe`. Mirrors `sweep::wall_ms_to_f64`.
#[expect(
    clippy::cast_precision_loss,
    reason = "a single job's wall time in milliseconds fits comfortably within f64's 52-bit \
              exact-integer range"
)]
fn wall_ms_to_f64(wall_ms: u64) -> f64 {
    wall_ms as f64
}

/// A process-wide counter backing [`unique_sock_path`], mirroring every
/// other mode's own (separately-scoped) counter of the same purpose.
static SOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build a coordinator socket path under the system temp directory, unique
/// across every call in this process (including concurrent ones) and
/// disjoint from every other mode's own socket paths (distinct filename
/// prefix).
fn unique_sock_path() -> PathBuf {
    let n = SOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("isingmark-scaling-{}-{n}.sock", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::{
        generate_3_regular_edges, generate_topology, parse_one_scaling_size, size_dir_name,
        topology_seed_key, validate_sizes, DeterministicRng,
    };
    use crate::modes::ModeError;
    use std::collections::{HashMap, HashSet};

    #[test]
    fn validate_sizes_rejects_an_empty_list() {
        let err = validate_sizes(&[]).expect_err("empty list must be rejected");
        assert!(matches!(err, ModeError::EmptySizes));
    }

    #[test]
    fn validate_sizes_rejects_odd_sizes() {
        let err = validate_sizes(&[8, 7]).expect_err("odd size must be rejected");
        assert!(matches!(err, ModeError::InvalidSize(7)), "got: {err}");
    }

    #[test]
    fn validate_sizes_rejects_sizes_below_four() {
        for bad in [0, 1, 2] {
            let err = validate_sizes(&[bad]).expect_err("size below 4 must be rejected");
            assert!(
                matches!(err, ModeError::InvalidSize(n) if n == bad),
                "got: {err}"
            );
        }
    }

    #[test]
    fn validate_sizes_accepts_the_smallest_valid_size() {
        validate_sizes(&[4]).expect("4 is the smallest valid 3-regular size");
    }

    #[test]
    fn validate_sizes_rejects_a_duplicate_value() {
        let err = validate_sizes(&[8, 16, 8]).expect_err("duplicate must be rejected");
        assert!(matches!(err, ModeError::DuplicateSize(8)), "got: {err}");
    }

    #[test]
    fn validate_sizes_accepts_distinct_valid_sizes() {
        validate_sizes(&[8, 16, 32]).expect("distinct valid sizes");
    }

    #[test]
    fn parse_one_scaling_size_rejects_an_empty_entry() {
        assert!(parse_one_scaling_size("").is_err());
        assert!(parse_one_scaling_size("   ").is_err());
    }

    #[test]
    fn parse_one_scaling_size_rejects_non_numeric_entries() {
        assert!(parse_one_scaling_size("not-a-number").is_err());
    }

    /// Pins the exact rejection this trap calls out: an odd size, and a
    /// size below 4, must both be rejected with a message naming the
    /// offending size.
    #[test]
    fn parse_one_scaling_size_rejects_odd_and_too_small_sizes_naming_the_value() {
        let err = parse_one_scaling_size("7").expect_err("odd size must be rejected");
        assert!(
            err.0.contains('7'),
            "message must name the offending size: {err}"
        );

        let err = parse_one_scaling_size("3").expect_err("size below 4 must be rejected");
        assert!(
            err.0.contains('3'),
            "message must name the offending size: {err}"
        );
    }

    #[test]
    fn parse_one_scaling_size_accepts_valid_sizes() {
        assert_eq!(parse_one_scaling_size("8").expect("8 is valid"), 8);
        assert_eq!(parse_one_scaling_size(" 16 ").expect("16 is valid"), 16);
    }

    #[test]
    fn size_dir_name_uses_the_raw_node_count() {
        assert_eq!(size_dir_name(8), "size_8");
        assert_eq!(size_dir_name(32), "size_32");
    }

    /// Degree/edge-count structural check for [`generate_3_regular_edges`]:
    /// every node must appear in exactly 3 edges, no self-loops, no
    /// duplicate edges, and the edge count must be exactly `3n/2` — run for
    /// two distinct sizes so a generator that is merely regular *for one
    /// size by coincidence* cannot pass. A generator that drops one edge
    /// (leaving one pair of nodes at degree 2) fails both the per-node
    /// degree check and the total edge-count check, so this test cannot be
    /// satisfied by a "nearly regular" graph.
    #[test]
    fn generate_3_regular_edges_produces_a_genuinely_3_regular_simple_graph() {
        for nodes in [8usize, 16usize] {
            let key = topology_seed_key(42, nodes);
            let mut rng = DeterministicRng::new(key);
            let edges = generate_3_regular_edges(nodes, &mut rng).expect("generation succeeds");

            assert_eq!(
                edges.len(),
                3 * nodes / 2,
                "edge count must be exactly 3n/2 for n={nodes}"
            );

            let mut degree: HashMap<u32, u32> = HashMap::new();
            let mut seen: HashSet<(u32, u32)> = HashSet::with_capacity(edges.len());
            for &(a, b) in &edges {
                assert_ne!(a, b, "no self-loops allowed, got ({a}, {b})");
                let key = if a < b { (a, b) } else { (b, a) };
                assert!(seen.insert(key), "no duplicate edges allowed, got {key:?}");
                *degree.entry(a).or_insert(0) += 1;
                *degree.entry(b).or_insert(0) += 1;
            }

            assert_eq!(
                degree.len(),
                nodes,
                "every node must appear in at least one edge for n={nodes}"
            );
            for (&node, &d) in &degree {
                assert_eq!(
                    d, 3,
                    "node {node} must have degree exactly 3 for n={nodes}, got {d}"
                );
            }
        }
    }

    /// The same `(seed, nodes)` must always produce the identical edge set —
    /// the determinism guarantee `--sizes`/`--seed` reproducibility depends
    /// on.
    #[test]
    fn generate_topology_is_deterministic_given_seed_and_nodes() {
        let a = generate_topology(42, 16).expect("generation succeeds");
        let b = generate_topology(42, 16).expect("generation succeeds");
        assert_eq!(a.edges, b.edges);
        assert_eq!(a.nodes, b.nodes);
    }

    /// A different seed, or a different node count, must not (in practice)
    /// produce the identical edge set — the "genuinely random" half of this
    /// module's determinism-vs-randomness requirement. Not a mathematical
    /// certainty (two different seeds could theoretically collide), but a
    /// mutation that ignores `seed` entirely (e.g. a generator that is
    /// secretly a fixed formula in `nodes` alone) would fail this
    /// decisively, not by coincidence.
    #[test]
    fn generate_topology_differs_across_seeds_and_across_sizes() {
        let base = generate_topology(1, 16).expect("generation succeeds");
        let different_seed = generate_topology(2, 16).expect("generation succeeds");
        assert_ne!(
            base.edges, different_seed.edges,
            "different seeds must (in practice) produce different graphs"
        );

        let different_size = generate_topology(1, 8).expect("generation succeeds");
        assert_ne!(
            base.nodes, different_size.nodes,
            "different sizes must produce different node counts"
        );
    }

    /// [`generate_topology`]'s output must satisfy every invariant
    /// `topology::load_topology` enforces on a hand-authored spec: nonempty
    /// nodes, nonempty allowed h/J sets, and every edge endpoint present in
    /// `nodes`.
    #[test]
    fn generate_topology_satisfies_load_topologys_own_invariants() {
        let spec = generate_topology(7, 8).expect("generation succeeds");
        assert!(!spec.nodes.is_empty());
        assert!(!spec.allowed_h_milli.is_empty());
        assert!(!spec.allowed_j_milli.is_empty());
        assert!(!spec.allowed_spin_milli.is_empty());
        let node_set: HashSet<u32> = spec.nodes.iter().copied().collect();
        for &(u, v) in &spec.edges {
            assert!(
                node_set.contains(&u),
                "edge endpoint {u} must be a known node"
            );
            assert!(
                node_set.contains(&v),
                "edge endpoint {v} must be a known node"
            );
        }
    }

    #[test]
    fn deterministic_rng_below_never_reaches_or_exceeds_bound() {
        let mut rng = DeterministicRng::new([9u8; 32]);
        for _ in 0..1_000 {
            let draw = rng.below(7);
            assert!(draw < 7, "draw {draw} must be < 7");
        }
    }

    #[test]
    fn deterministic_rng_same_key_yields_the_same_sequence() {
        let mut a = DeterministicRng::new([3u8; 32]);
        let mut b = DeterministicRng::new([3u8; 32]);
        let seq_a: Vec<u64> = (0..10).map(|_| a.next_u64()).collect();
        let seq_b: Vec<u64> = (0..10).map(|_| b.next_u64()).collect();
        assert_eq!(seq_a, seq_b);
    }

    #[test]
    fn deterministic_rng_different_keys_yield_different_sequences() {
        let mut a = DeterministicRng::new([3u8; 32]);
        let mut b = DeterministicRng::new([4u8; 32]);
        assert_ne!(a.next_u64(), b.next_u64());
    }
}
