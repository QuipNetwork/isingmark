// SPDX-License-Identifier: AGPL-3.0-or-later
//! Comparative mode: search hardness in `[0.0, 1.0]` for the resolved
//! sample parameters whose best-energy distribution best matches a
//! reference JSONL file's own best-energy distribution.
//!
//! # Binary search on a monotone comparison, not on `ks_distance` itself
//!
//! `ks_distance` (`stats::ks_distance`) is a *distance*: it falls, then
//! rises, around whichever hardness's distribution happens to match the
//! reference best, so it is not monotone in hardness and a literal binary
//! search over its value has no boundary to converge on — it would
//! converge to an arbitrary point, not the best match. This module instead
//! binary-searches a genuinely monotone comparison: whether this
//! evaluation's own mean `best_energy_milli` is higher (worse) or lower
//! (better) than the reference file's mean. `params::ParamCurve::resolve`
//! resolves `num_reads`/`num_sweeps`/`anneal_time_us` non-decreasingly in
//! hardness by convention (see that module's own doc on `min`/`max`
//! ordering), and a backend whose achieved energy improves — or at worst
//! never worsens — as those parameters increase makes "is our mean <= the
//! reference's" a monotone predicate over hardness: `false` below the
//! crossing point, `true` at and above it. Bisecting *that* boundary is an
//! ordinary, correct binary search. `ks_distance` at the final evaluated
//! hardness is still computed and reported — as the quality of the match,
//! not as what drives the search.
//!
//! The mean, not the median, is the comparison statistic: the bundled
//! topology fixtures quantize `h`/`j` to a handful of milli-values, so
//! `best_energy_milli` only ever takes a handful of distinct values per
//! job, and the *median* of a modest sample collapses onto one of those few
//! values — flat across a wide span of hardness, with no signal to bisect
//! within that span. The *mean* aggregates every sample instead of picking
//! out its middle one, so it moves smoothly as the underlying distribution
//! shifts even though each individual sample is coarse; empirically (see
//! this task's report) it stayed monotone across the bundled "smoke"
//! fixture's whole hardness range at `ComparativeConfig::num_jobs = 40`,
//! where the median did not.
//!
//! **Invariant relied on**: `ParamCurve::resolve`'s bounds are set up with
//! `min <= max` for the active `params::BackendKind` (a curve author's
//! responsibility, not something `resolve` itself enforces — see that
//! function's own doc), and the resolved sampling effort's achieved energy
//! is non-increasing in hardness. For a real backend this is a reasonable
//! but unproven assumption (more reads usually helps, but nothing here
//! verifies it against a live backend). For `isingmark-fake-miner`, this
//! task extended it (see that binary's module doc) so the property
//! provably holds, which is what makes this module's own recovery test
//! meaningful as a test of the search machinery, not of any real
//! annealer's behavior.
//!
//! **What noise does to this — real backends only**: against a real,
//! stochastic backend, each evaluation's mean is a finite-sample statistic
//! over `ComparativeConfig::num_jobs` runs. Near the true crossing point, a
//! single evaluation's own sampling noise can make the comparison point the
//! wrong way for that one iteration, narrowing the bracket in the wrong
//! direction. This module does not detect or correct that — every
//! iteration trusts its own comparison outright. The bracket starts at
//! width `1.0` and only the final iteration or two are ever close enough to
//! the true crossing point for this to matter much, so a wrong late
//! decision costs a bounded fraction of the final bracket width, not the
//! whole search; the returned `ks_distance` (via `summary.json`) is the
//! honest signal for how good the reported match actually is, rather than
//! a claim of exactness.
//!
//! Against `isingmark-fake-miner` specifically, none of the above is
//! sampling noise: that binary's `multi_start_search` is fully
//! deterministic (no RNG, no clock, no job-derived state — see its own
//! module doc), so a given `(seed, num_jobs, hardness)` always evaluates to
//! the exact same mean. The recovery test's residual error on that fixture
//! instead comes from a *different*, deterministic source: `ParamCurve::resolve`
//! is a step function (`round(min + (max - min) * hardness)`), so several
//! consecutive hardness values resolve to the same `num_reads` and can
//! produce an *exactly tied* mean. The `>` (not `>=`) at the comparison
//! below means a tied evaluation is treated as "at least as good as the
//! reference," pushing `hi` down — so the search converges to the *left
//! edge* of whichever plateau contains the reference's own mean, not
//! necessarily to the reference's own hardness. Neither tie-break is
//! "correct" in general (a true plateau has no unique matching hardness),
//! but this is the concrete, reproducible mechanism behind this fixture's
//! recovery error, and it is a property of the comparison's tie-break
//! direction, not of any randomness.
//!
//! # Paired design, one `run_session` per iteration
//!
//! Every iteration draws jobs from a *fresh* `problem::ProblemSource` built
//! from the identical `(cfg.seed, cfg.num_jobs)`, exactly `modes::sweep`'s
//! and `modes::ttt`'s own paired-design mechanism (see `modes::sweep`'s
//! module doc for the full reasoning): every iteration's evaluation is
//! comparable to every other's because they were run against the identical
//! problems, with only the resolved hardness differing. `backend::session`
//! (Task 7, already reviewed) tracks in-flight jobs in a map keyed by
//! `job_id` alone, scoped to one `run_session` call, with no duplicate
//! guard (filed as a known P2 gap, not fixed here) — sharing one
//! `run_session` call across iterations that replay the same `(seed,
//! num_jobs)` problems would let two same-`job_id` jobs collide in that
//! map. This module never does that: each iteration gets its own
//! `run_session` call, awaited to completion before the next iteration
//! starts, the same one-session-per-unit-of-work shape `modes::sweep` uses
//! for its own cells.
//!
//! # Output layout: one directory per iteration, eager pre-pass
//!
//! Each iteration's records land in `<output_dir>/iter_<n>/records.jsonl`
//! (`n` zero-based, generation order), never a shared file. The eager
//! [`ModeError::RecordsFileExists`] conflict check runs across every
//! directory the search *could* touch — `iter_0` through
//! `iter_{max_iterations - 1}` — before any of them are created, mirroring
//! `modes::sweep`'s/`modes::ttt`'s own eager pre-pass, even though the
//! search may (and, once the bracket narrows below
//! [`BRACKET_STOP_WIDTH`], normally does) stop before actually using every
//! one of those directories.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::backend::{builtin_registry, run_session, BackendSpec, RunParams};
use crate::modes::{hex_encode, ModeError, TopologyRef};
use crate::params::ParamCurve;
use crate::problem::ProblemSource;
use crate::record::{
    append_jsonl, read_jsonl, write_summary, HardwareInfo, JobRecord, Summary, TopologyInfo, Totals,
};
use crate::stats::{describe, ks_distance};
use crate::topology::TopologySpec;

/// Basename of one iteration's JSONL record file within its own
/// subdirectory of [`ComparativeConfig::output_dir`] (see
/// [`iter_dir_name`]).
pub const RECORDS_FILENAME: &str = "records.jsonl";
/// Basename of the run's closing summary within
/// [`ComparativeConfig::output_dir`].
pub const SUMMARY_FILENAME: &str = "summary.json";

/// Miner account every iteration derives jobs against. Only affects the
/// derived nonce (via `problem::ProblemSource`), not this run's results, so
/// a fixed value is used across every run — the same reasoning
/// `sweep`/`throughput`/`ttt` give for their own copies of this constant.
const MINER_ACCOUNT: [u8; 32] = [0u8; 32];

/// No wire deadline: this mode measures its own comparison and does not
/// need the miner to expire jobs on its own clock.
const NO_DEADLINE_MS: u64 = 0;

/// Default hardness-to-parameter curve. Every bound is nonzero, per
/// `ParamCurve`'s own documented "0 means unset" contract. `pub`, unlike
/// `throughput::DEFAULT_PARAM_CURVE` (which this otherwise mirrors, per
/// this crate's per-mode-module convention of keeping small constants like
/// this one as separate copies rather than sharing them): a caller building
/// a reference file out-of-band, at a specific known hardness, to later
/// feed back into [`run_comparative`] as `ComparativeConfig::target_file`
/// needs the exact same curve this module resolves hardness against, and
/// duplicating these six numbers a second time (in that caller, or in this
/// module's own tests) would risk the two copies silently drifting apart.
/// Default when neither CLI nor file supplies a curve. Operators override
/// via `--config` `param_curve`; see [`ComparativeConfig::param_curve`].
pub const DEFAULT_PARAM_CURVE: ParamCurve = ParamCurve {
    min_reads: 8,
    max_reads: 64,
    min_sweeps: 100,
    max_sweeps: 1000,
    min_anneal_us: 5,
    max_anneal_us: 200,
};

/// Bracket width, in hardness units, below which the search stops early
/// (in addition to [`ComparativeConfig::max_iterations`]) — the brief's own
/// `0.01` stop condition.
const BRACKET_STOP_WIDTH: f64 = 0.01;

/// Configuration for [`run_comparative`].
#[derive(Debug, Clone)]
pub struct ComparativeConfig {
    /// `backend::BackendSpec::name` to benchmark (e.g. `"cpu-sa"`).
    pub backend: String,
    /// Topology to draw jobs against. Shared by every iteration.
    pub topology: TopologyRef,
    /// Reference benchmark JSONL file (e.g. another run's `records.jsonl`)
    /// whose `best_energy_milli` distribution (over its `passed` records)
    /// this mode searches for a matching hardness against.
    pub target_file: PathBuf,
    /// Jobs drawn per hardness evaluation. Every evaluation draws the
    /// identical `num_jobs`-job sequence (same seed) — see this module's
    /// doc on the paired design this establishes.
    pub num_jobs: u32,
    /// Maximum number of evaluations to run before reporting whatever
    /// hardness the bracket has narrowed to. The search may stop earlier,
    /// once the bracket narrows below [`BRACKET_STOP_WIDTH`]. Must be
    /// nonzero (see [`ModeError::InvalidMaxIterations`]): unlike
    /// `sweep::SweepConfig::num_jobs`/`ttt::TttConfig::num_trials` (whose
    /// `0` CLI-level rejection is this crate's usual convention for a
    /// single numeric flag — see `cli::parse_num_jobs`/`parse_num_trials`),
    /// a `max_iterations: 0` search would return hardness `0.0` — a
    /// plausible-looking, in-range value indistinguishable from a real
    /// finding — rather than an obviously-empty result, so this module
    /// re-checks it itself for callers that construct a
    /// [`ComparativeConfig`] directly, bypassing the CLI parser.
    pub max_iterations: u32,
    /// Seed for deterministic problem generation, shared by every
    /// iteration.
    pub seed: u64,
    /// Directory to write one subdirectory per iteration into, plus a
    /// top-level [`SUMMARY_FILENAME`]. Created if it does not already
    /// exist. Refuses to start (see [`ModeError::RecordsFileExists`]) if
    /// any iteration's [`RECORDS_FILENAME`] already contains data from an
    /// earlier run — checked for every possible iteration (`0` through
    /// `max_iterations - 1`) before any of them run.
    pub output_dir: PathBuf,
    /// Hardness-to-parameter curve for every evaluation. CLI/`--config`
    /// merge supplies this (file value, else [`DEFAULT_PARAM_CURVE`]).
    pub param_curve: ParamCurve,
    /// Explicit backend binary path, bypassing `$PATH` resolution. See
    /// `throughput::ThroughputConfig::backend_bin_override`.
    pub backend_bin_override: Option<PathBuf>,
    /// Explicit backend CLI args, replacing `backend::BackendSpec::args`.
    /// See `throughput::ThroughputConfig::backend_args_override`.
    pub backend_args_override: Option<Vec<String>>,
}

/// Run comparative mode: binary-search `[0.0, 1.0]` (see this module's doc
/// for the monotone comparison the search actually bisects) for the
/// hardness whose evaluation's best-energy distribution best matches
/// `cfg.target_file`'s. Writes each evaluated iteration's records to its
/// own `<output_dir>/iter_<n>/records.jsonl` as it completes, and a closing
/// [`SUMMARY_FILENAME`] (carrying every iteration's own hardness/
/// `ks_distance`, the final chosen hardness, and `cfg.target_file`'s own
/// `record::JsonlRead::skipped_lines`) once the search stops. Returns the
/// final iteration's records and the matched hardness.
///
/// # Errors
///
/// Returns [`ModeError::Topology`] if `cfg.topology` fails to load,
/// [`ModeError::UnknownBackend`] if `cfg.backend` matches no entry in
/// `backend::builtin_registry`, [`ModeError::InvalidMaxIterations`] if
/// `cfg.max_iterations` is `0`, [`ModeError::TargetFileUnreadable`] if
/// `cfg.target_file` cannot be opened or read,
/// [`ModeError::EmptyTargetFile`] if it contains no usable (`passed`)
/// records to compare against, [`ModeError::RecordsFileExists`] if any
/// iteration's records file already contains data,
/// [`ModeError::InvalidComputedHardness`] if the search's own computed
/// hardness is somehow non-finite or outside `[0.0, 1.0]` (see this
/// function's own bracket invariant — unreachable in normal operation, but
/// checked rather than trusted), [`ModeError::Session`] if an iteration's
/// session fails to bind or spawn, and [`ModeError::Io`] if the output
/// directory or any file cannot be written.
pub async fn run_comparative(cfg: &ComparativeConfig) -> Result<(Vec<JobRecord>, f64), ModeError> {
    if cfg.max_iterations == 0 {
        return Err(ModeError::InvalidMaxIterations);
    }
    let spec = cfg.topology.load()?;
    let backend = resolve_backend(cfg)?;

    let jsonl = read_jsonl(&cfg.target_file)
        .map_err(|e| ModeError::TargetFileUnreadable(cfg.target_file.clone(), e))?;
    let target_energies: Vec<f64> = jsonl
        .records
        .iter()
        .filter(|r| r.passed)
        .map(|r| milli_to_f64(r.best_energy_milli))
        .collect();
    let target_mean = describe(&target_energies)
        .map(|s| s.mean)
        .ok_or_else(|| ModeError::EmptyTargetFile(cfg.target_file.clone()))?;

    std::fs::create_dir_all(&cfg.output_dir).map_err(ModeError::Io)?;

    let iter_paths: Vec<(PathBuf, PathBuf, String)> = (0..cfg.max_iterations)
        .map(|i| {
            let dir_name = iter_dir_name(i);
            let dir = cfg.output_dir.join(&dir_name);
            let records_path = dir.join(RECORDS_FILENAME);
            (dir, records_path, dir_name)
        })
        .collect();

    // Eager, whole-range conflict check before anything is created — see
    // this module's doc and `modes::sweep`'s/`modes::ttt`'s identical
    // reasoning. Every possible iteration directory is checked even though
    // the search may stop before using all of them.
    for (_, records_path, _) in &iter_paths {
        if std::fs::metadata(records_path).is_ok_and(|m| m.len() > 0) {
            return Err(ModeError::RecordsFileExists(records_path.clone()));
        }
    }

    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    let mut final_hardness = 0.0_f64;
    let mut final_records: Vec<JobRecord> = Vec::new();
    let mut final_ks: Option<f64> = None;
    let mut iterations_json: Vec<serde_json::Value> = Vec::new();

    for i in 0..cfg.max_iterations {
        if hi - lo < BRACKET_STOP_WIDTH {
            break;
        }
        let mid = lo + (hi - lo) / 2.0;
        let (dir, records_path, dir_name) = &iter_paths[i as usize];
        std::fs::create_dir_all(dir).map_err(ModeError::Io)?;
        std::fs::File::create(records_path).map_err(ModeError::Io)?;

        let params = cfg.param_curve.resolve(backend.kind, mid);
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

        let energies: Vec<f64> = report
            .records
            .iter()
            .filter(|r| r.passed)
            .map(|r| milli_to_f64(r.best_energy_milli))
            .collect();
        // No passed record this iteration is treated as the worst possible
        // outcome (push the search toward more effort) rather than
        // crashing or silently guessing a direction — see this module's
        // doc on what a degenerate evaluation does to the search.
        let our_mean = describe(&energies).map_or(f64::INFINITY, |s| s.mean);
        let ks = ks_distance(&energies, &target_energies);

        iterations_json.push(serde_json::json!({
            "dir": dir_name,
            "hardness": mid,
            "num_reads": params.num_reads,
            "num_sweeps": params.num_sweeps,
            "anneal_time_us": params.anneal_time_us,
            "jobs": report.records.len(),
            "mean_best_energy": finite_or_null(our_mean),
            "ks_distance": ks,
        }));

        final_hardness = mid;
        final_records = report.records;
        final_ks = ks;

        // Strict `>`, not `>=`: an exact tie (common on the bundled
        // fixtures, where `ParamCurve::resolve`'s step function makes
        // several consecutive hardness values resolve to the same
        // `num_reads` and so produce identical means — see this module's
        // doc) is treated as "at least as good as the reference," so the
        // search converges to the left edge of the plateau containing the
        // reference's mean rather than to any particular hardness within
        // it. This is a real, deterministic bias on a fixture with wide
        // plateaus, not sampling noise.
        if our_mean > target_mean {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    validate_hardness(final_hardness)?;

    let summary = build_summary(
        cfg,
        &spec,
        jsonl.skipped_lines,
        final_hardness,
        final_ks,
        &iterations_json,
        &final_records,
    );
    write_summary(&cfg.output_dir.join(SUMMARY_FILENAME), &summary).map_err(ModeError::Io)?;

    Ok((final_records, final_hardness))
}

/// Resolve `cfg.backend` against the built-in registry, applying
/// `backend_bin_override`/`backend_args_override` if set. Mirrors
/// `sweep::resolve_backend`/`ttt::resolve_backend`/`throughput::resolve_backend`
/// exactly; kept as its own copy per this crate's existing
/// per-mode-module convention.
fn resolve_backend(cfg: &ComparativeConfig) -> Result<BackendSpec, ModeError> {
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

/// Draw `count` deterministic jobs from a *fresh* [`ProblemSource`] built
/// from `(spec, seed, count)`, setting `params`'s resolved sample
/// parameters directly on each job's `IsingProblem` fields. Mirrors
/// `sweep::Cell::draw_jobs` exactly (a fresh source per call is this
/// module's own copy of the paired-design mechanism — see this module's
/// doc), specialized to a single already-resolved `params::JobParams`
/// rather than a `sweep::Cell`.
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

/// Validate a hardness value computed by this module's own search before
/// it is returned or written to `summary.json`: reject `NaN`/infinite
/// values and anything outside `[0.0, 1.0]`. `run_comparative`'s own
/// bracket arithmetic (`lo`/`hi` start at `0.0`/`1.0` and only ever narrow
/// toward each other's midpoint) makes an out-of-range result unreachable
/// under normal operation, but Task 8's own ruling for `--hardness`
/// applies here too: a search that somehow diverges must fail loudly with
/// an explicit error, never silently emit a `NaN` or out-of-range hardness
/// into a benchmark summary that looks like every other, valid one.
fn validate_hardness(hardness: f64) -> Result<(), ModeError> {
    if !hardness.is_finite() || !(0.0..=1.0).contains(&hardness) {
        return Err(ModeError::InvalidComputedHardness(hardness));
    }
    Ok(())
}

/// This iteration's subdirectory name under
/// [`ComparativeConfig::output_dir`]: `iter_<index>` — deterministic and
/// disambiguated purely by generation order, since (unlike
/// `sweep::cell_dir_name`) there are no cell parameters to embed and
/// (unlike `ttt::target_dir_name`) no user-supplied value to render.
#[must_use]
fn iter_dir_name(index: u32) -> String {
    format!("iter_{index}")
}

/// Milli-units to natural `f64` units (divide by 1000), for feeding
/// `stats::describe`/`stats::ks_distance`. Mirrors `sweep::milli_to_f64`;
/// kept as its own copy per this crate's per-mode-module convention.
#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark energies fit comfortably within f64's 52-bit exact-integer range"
)]
fn milli_to_f64(milli: i64) -> f64 {
    milli as f64 / 1000.0
}

/// Render `value` as JSON, `null` for a non-finite value (this module's own
/// documented "no passed record this iteration" sentinel, `f64::INFINITY`)
/// rather than a `serde_json` error or a silently-wrong finite substitute.
fn finite_or_null(value: f64) -> serde_json::Value {
    if value.is_finite() {
        serde_json::json!(value)
    } else {
        serde_json::Value::Null
    }
}

/// Build the closing [`Summary`] from every evaluated iteration
/// (`iterations_json`, already rendered by the caller) and the search's
/// final outcome. `totals` aggregates across every iteration's dispatched
/// jobs; `config` carries the requested run parameters, `cfg.target_file`'s
/// own `skipped_lines` (so a reference file with unparseable lines is
/// never silently treated as if it were complete — see this module's doc),
/// every iteration's own record, and the final chosen hardness/
/// `ks_distance`.
fn build_summary(
    cfg: &ComparativeConfig,
    spec: &TopologySpec,
    target_file_skipped_lines: usize,
    final_hardness: f64,
    final_ks_distance: Option<f64>,
    iterations_json: &[serde_json::Value],
    final_records: &[JobRecord],
) -> Summary {
    let totals = Totals {
        jobs: final_records.len(),
        passed: final_records.iter().filter(|r| r.passed).count(),
        rejected: final_records.iter().filter(|r| r.rejected).count(),
        // Every iteration's records file is created empty and exclusively
        // written by this call, same reasoning as `sweep::build_summary`;
        // `target_file_skipped_lines` (a distinct file this run only
        // reads) is carried separately, in `config`, below.
        skipped_lines: 0,
    };

    let config = serde_json::json!({
        "backend": cfg.backend,
        "topology": cfg.topology.label(),
        "seed": cfg.seed,
        "num_jobs": cfg.num_jobs,
        "max_iterations": cfg.max_iterations,
        "target_file": cfg.target_file.display().to_string(),
        "target_file_skipped_lines": target_file_skipped_lines,
        "final_hardness": final_hardness,
        "final_ks_distance": final_ks_distance,
        "iterations": iterations_json,
    });

    Summary {
        mode: "comparative".to_string(),
        backend: cfg.backend.clone(),
        topology: TopologyInfo {
            name: cfg.topology.label(),
            num_nodes: spec.nodes.len(),
            num_edges: spec.edges.len(),
            topology_hash: hex_encode(&spec.topology_hash()),
        },
        hardware: HardwareInfo::detect(),
        config,
        totals,
    }
}

/// A process-wide counter backing [`unique_sock_path`], mirroring
/// `sweep`'s/`ttt`'s/`throughput`'s own (separately-scoped) counters of the
/// same purpose.
static SOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build a coordinator socket path under the system temp directory, unique
/// across every call in this process (including concurrent ones) and
/// disjoint from every other mode's own socket paths (distinct filename
/// prefix).
fn unique_sock_path() -> PathBuf {
    let n = SOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "isingmark-comparative-{}-{n}.sock",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::{finite_or_null, iter_dir_name, milli_to_f64, validate_hardness};
    use crate::modes::ModeError;

    #[test]
    fn iter_dir_name_uses_generation_order() {
        assert_eq!(iter_dir_name(0), "iter_0");
        assert_eq!(iter_dir_name(9), "iter_9");
    }

    #[test]
    fn milli_to_f64_divides_by_one_thousand() {
        assert!((milli_to_f64(1_500) - 1.5).abs() < 1e-12);
        assert!((milli_to_f64(-2_000) - -2.0).abs() < 1e-12);
    }

    #[test]
    fn finite_or_null_passes_through_a_finite_value() {
        assert_eq!(finite_or_null(4.5), serde_json::json!(4.5));
    }

    #[test]
    fn finite_or_null_renders_infinity_as_null() {
        assert_eq!(finite_or_null(f64::INFINITY), serde_json::Value::Null);
        assert_eq!(finite_or_null(f64::NEG_INFINITY), serde_json::Value::Null);
        assert_eq!(finite_or_null(f64::NAN), serde_json::Value::Null);
    }

    /// Pins the exact boundary this function exists to enforce: `0.0`/`1.0`
    /// (the search's own bracket endpoints) are valid, but anything
    /// non-finite or outside `[0.0, 1.0]` is rejected. A mutation that
    /// inverts the comparison, drops the finiteness check, or widens the
    /// range would flip at least one of these assertions.
    #[test]
    fn validate_hardness_accepts_the_closed_unit_interval_and_rejects_everything_else() {
        validate_hardness(0.0).expect("0.0 is a valid hardness");
        validate_hardness(1.0).expect("1.0 is a valid hardness");
        validate_hardness(0.3).expect("0.3 is a valid hardness");

        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0001, 1.0001] {
            let err = validate_hardness(bad).expect_err(&format!("{bad} must be rejected"));
            assert!(
                matches!(err, ModeError::InvalidComputedHardness(_)),
                "got: {err}"
            );
        }
    }
}
