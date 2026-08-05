// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reproducibility mode: run two batches of jobs against the identical
//! problem sequence and report whether their `best_energy_milli`
//! distributions are statistically distinguishable.
//!
//! # Why two hardness values, not one
//!
//! [`ReproConfig`] carries **two** hardness values, `hardness_a` and
//! `hardness_b`, both resolved against [`DEFAULT_PARAM_CURVE`] and both
//! applied to jobs drawn from the identical `(seed, num_problems)` sequence
//! (see the next section). The ordinary use of this mode — "does this
//! backend reproduce itself?" — sets both to the same value (the CLI
//! defaults `--hardness-b` to `--hardness`, see `cli::ReproducibilityArgs`),
//! so both runs dispatch at literally identical sampling effort and any
//! reported difference reflects the backend's own run-to-run variance, not
//! a difference this mode introduced.
//!
//! The second value exists because `isingmark-fake-miner` — the only
//! backend this crate's own hermetic tests can drive without a real miner
//! installed — is **fully deterministic**: `multi_start_search` (see that
//! binary's own module doc) uses no RNG, clock, or job-derived state, so two
//! runs at the *same* hardness against it always produce byte-identical
//! energies, unconditionally. A test asserting only `ReproReport::differs ==
//! false` under those conditions would pass for a stub that hardcodes
//! `differs: false`, or one that never dispatches the second batch at all —
//! it cannot tell "correctly detected no difference" apart from "never
//! checked". `hardness_b` lets a test (or an operator investigating a
//! specific concern) deliberately drive the two batches at different
//! sampling effort against the *same* problems, which — via
//! `multi_start_search`'s energy being provably non-increasing in
//! `num_reads` (the same property `modes::comparative`'s own recovery test
//! relies on) — produces two genuinely different, deterministically
//! reproducible distributions. That is what lets this module's own test
//! suite verify the `differs == true` direction at all, not just the
//! (otherwise unfalsifiable) `differs == false` one.
//!
//! # Paired design: both runs replay the identical problem sequence
//!
//! Both batches draw from a *fresh* `problem::ProblemSource` built from the
//! identical `(cfg.seed, cfg.num_problems)`, exactly the paired-design
//! mechanism `modes::sweep`/`modes::ttt`/`modes::comparative` already use
//! (see `modes::sweep`'s module doc for the full reasoning) — this is
//! deliberate, not incidental: comparing two batches that solved *different*
//! problems would attribute any difference to which problems each batch
//! happened to draw, not to whatever the two hardness values (or, for a
//! real, possibly-stochastic backend, its own inherent variance) actually
//! did.
//!
//! # `job_id` collision: two separate directories, two separate sessions
//!
//! `problem::ProblemSource`'s derived `job_id` depends only on `(seed,
//! index)`, not on which of the two runs is dispatching it — replaying the
//! same problems (deliberately, see above) means both runs draw the
//! byte-identical `job_id` sequence. This mode never lets the two runs share
//! one records file or one `run_session` call: each gets its own
//! `<output_dir>/run_a/records.jsonl`/`<output_dir>/run_b/records.jsonl` and
//! its own `run_session` call, sequential, mirroring
//! `modes::scaling`'s/`modes::sweep`'s/`modes::ttt`'s identical reasoning
//! about `backend::session`'s in-flight-job map (keyed on `job_id` alone,
//! with no duplicate guard — a known P2 gap, not fixed here). The eager
//! [`ModeError::RecordsFileExists`] pre-pass runs across both directories
//! before either run is dispatched.
//!
//! # What an empty or non-finite batch does to `p_value`
//!
//! [`ReproReport`] has no `Option` room for `p_value` — it is a plain `f64`
//! per this task's fixed interface. `stats::ks_distance`/
//! `stats::ks_two_sample_p` return `None` if either input batch is empty or
//! contains a non-finite value; in practice only the "empty" case can occur
//! here (`JobRecord::best_energy_milli` is an `i64`, and dividing any `i64`
//! by `1000.0` always yields a finite `f64` — `contains_non_finite` cannot
//! fire), but this module treats both documented `None` causes the same
//! way. Rather than defaulting `p_value` to `1.0` (which would silently
//! report "the two runs are identical" for a run that produced no usable
//! data at all — exactly the failure mode this crate's own conventions
//! exist to prevent) or fabricating `0.0`/some other placeholder,
//! [`run_reproducibility`] returns [`ModeError::EmptyReproBatch`], naming
//! which run(s) had no `passed` records to compare.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::backend::{builtin_registry, run_session, BackendSpec, RunParams, RunReport};
use crate::modes::{ModeError, TopologyRef};
use crate::params::{JobParams, ParamCurve};
use crate::problem::ProblemSource;
use crate::record::{append_jsonl, write_summary, HardwareInfo, Summary, TopologyInfo, Totals};
use crate::stats::{describe, ks_distance, ks_two_sample_p, Stats};
use crate::topology::TopologySpec;

/// Basename of a run's JSONL record file within its own subdirectory of
/// [`ReproConfig::output_dir`] (`run_a`/`run_b`, see [`RUN_A_DIR`]/
/// [`RUN_B_DIR`]).
pub const RECORDS_FILENAME: &str = "records.jsonl";
/// Basename of the run's closing summary within [`ReproConfig::output_dir`].
pub const SUMMARY_FILENAME: &str = "summary.json";

/// Subdirectory name for the first run.
pub const RUN_A_DIR: &str = "run_a";
/// Subdirectory name for the second run.
pub const RUN_B_DIR: &str = "run_b";

/// The conventional two-sided significance threshold for
/// [`ReproReport::differs`]: `p_value < 0.05` means the two batches'
/// distributions are unlikely (under 5% probability) to have arisen from
/// the same underlying distribution by chance alone. This is a standard
/// statistical convention, **not** a value derived from anything specific
/// to this crate's domain (job counts, energy scales, or backend
/// behavior) — a different threshold would be an equally defensible
/// choice; `0.05` is used because it is the customary default for a
/// two-sample distribution comparison.
const DIFFERS_P_THRESHOLD: f64 = 0.05;

/// Miner account both runs derive jobs against. Only affects the derived
/// nonce (via `problem::ProblemSource`), not either run's results — the
/// same reasoning `sweep`/`ttt`/`comparative`/`scaling` give for their own
/// copies of this constant.
const MINER_ACCOUNT: [u8; 32] = [0u8; 32];

/// No wire deadline: this mode measures its own comparison and does not
/// need the miner to expire jobs on its own clock.
const NO_DEADLINE_MS: u64 = 0;

/// Default hardness-to-parameter curve, shared by both runs (each resolved
/// at its own `hardness_a`/`hardness_b`). A separate copy from
/// `comparative::DEFAULT_PARAM_CURVE`/`scaling`'s own copy, per this crate's
/// per-mode-module convention; placeholder until `--config`'s `ParamCurve`
/// section (a later task) lets an operator override it.
const DEFAULT_PARAM_CURVE: ParamCurve = ParamCurve {
    min_reads: 8,
    max_reads: 64,
    min_sweeps: 100,
    max_sweeps: 1000,
    min_anneal_us: 5,
    max_anneal_us: 200,
};

/// Configuration for [`run_reproducibility`].
#[derive(Debug, Clone)]
pub struct ReproConfig {
    /// `backend::BackendSpec::name` to benchmark (e.g. `"cpu-sa"`).
    pub backend: String,
    /// Topology to draw jobs against. Shared by both runs.
    pub topology: TopologyRef,
    /// Jobs drawn per run. Both runs draw the identical
    /// `(cfg.seed, num_problems)` sequence — see this module's doc on the
    /// paired design this establishes.
    pub num_problems: u32,
    /// Difficulty in `[0.0, 1.0]` for the first run.
    pub hardness_a: f64,
    /// Difficulty in `[0.0, 1.0]` for the second run. Equal to
    /// `hardness_a` for the ordinary "is this backend reproducible"
    /// use case; see this module's doc for why a caller might set it
    /// differently.
    pub hardness_b: f64,
    /// Seed for deterministic problem generation, shared by both runs.
    pub seed: u64,
    /// Directory to write `run_a`/`run_b` subdirectories into, plus a
    /// top-level [`SUMMARY_FILENAME`]. Created if it does not already
    /// exist. Refuses to start (see [`ModeError::RecordsFileExists`]) if
    /// either run's [`RECORDS_FILENAME`] already contains data from an
    /// earlier run — checked for both before either runs.
    pub output_dir: PathBuf,
    /// Explicit backend binary path, bypassing `$PATH` resolution. See
    /// `throughput::ThroughputConfig::backend_bin_override`.
    pub backend_bin_override: Option<PathBuf>,
    /// Explicit backend CLI args, replacing `backend::BackendSpec::args`.
    /// See `throughput::ThroughputConfig::backend_args_override`.
    pub backend_args_override: Option<Vec<String>>,
}

/// Outcome of [`run_reproducibility`]'s two-sample comparison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReproReport {
    /// Two-sample Kolmogorov-Smirnov statistic (`stats::ks_distance`)
    /// between the two runs' `passed` `best_energy_milli` distributions.
    /// `0.0` for identical distributions, up to `1.0` for completely
    /// disjoint ones.
    pub ks_statistic: f64,
    /// Asymptotic two-sided KS p-value (`stats::ks_two_sample_p`) for the
    /// same comparison.
    pub p_value: f64,
    /// `p_value < 0.05` (see [`DIFFERS_P_THRESHOLD`]'s own doc for why
    /// `0.05` and not some other value): whether the two runs'
    /// distributions are statistically distinguishable.
    pub differs: bool,
}

/// Run reproducibility mode: dispatch two batches of `cfg.num_problems`
/// jobs each, drawn from the identical `(cfg.seed, cfg.num_problems)`
/// sequence, the first at `cfg.hardness_a` and the second at
/// `cfg.hardness_b`, and compare their `passed` `best_energy_milli`
/// distributions via a two-sample Kolmogorov-Smirnov test. Writes each
/// run's records to its own `<output_dir>/run_a|run_b/records.jsonl` as it
/// completes, and a closing [`SUMMARY_FILENAME`] once both runs are done.
///
/// # Errors
///
/// Returns [`ModeError::Topology`] if `cfg.topology` fails to load,
/// [`ModeError::UnknownBackend`] if `cfg.backend` matches no entry in
/// `backend::builtin_registry`, [`ModeError::RecordsFileExists`] if either
/// run's records file already contains data, [`ModeError::EmptyReproBatch`]
/// if either run produced no `passed` records to compare (see this
/// module's own doc on why this is an error rather than a fabricated
/// `p_value`), [`ModeError::Session`] if either run's session fails to bind
/// or spawn, and [`ModeError::Io`] if the output directory or any file
/// cannot be written.
pub async fn run_reproducibility(cfg: &ReproConfig) -> Result<ReproReport, ModeError> {
    let spec = cfg.topology.load()?;
    let backend = resolve_backend(cfg)?;

    std::fs::create_dir_all(&cfg.output_dir).map_err(ModeError::Io)?;

    let dir_a = cfg.output_dir.join(RUN_A_DIR);
    let dir_b = cfg.output_dir.join(RUN_B_DIR);
    let records_a = dir_a.join(RECORDS_FILENAME);
    let records_b = dir_b.join(RECORDS_FILENAME);

    // Eager, both-directories conflict check before either run is
    // dispatched — see this module's doc and `modes::sweep`'s/
    // `modes::ttt`'s/`modes::scaling`'s identical reasoning.
    for records_path in [&records_a, &records_b] {
        if std::fs::metadata(records_path).is_ok_and(|m| m.len() > 0) {
            return Err(ModeError::RecordsFileExists(records_path.clone()));
        }
    }

    let params_a = DEFAULT_PARAM_CURVE.resolve(backend.kind, cfg.hardness_a);
    let params_b = DEFAULT_PARAM_CURVE.resolve(backend.kind, cfg.hardness_b);

    let report_a = run_one_batch(
        &backend,
        &spec,
        cfg.seed,
        cfg.num_problems,
        &params_a,
        &dir_a,
        &records_a,
    )
    .await?;
    let report_b = run_one_batch(
        &backend,
        &spec,
        cfg.seed,
        cfg.num_problems,
        &params_b,
        &dir_b,
        &records_b,
    )
    .await?;

    let energies_a = passed_energies(&report_a);
    let energies_b = passed_energies(&report_b);

    let (Some(ks_statistic), Some(p_value)) = (
        ks_distance(&energies_a, &energies_b),
        ks_two_sample_p(&energies_a, &energies_b),
    ) else {
        return Err(ModeError::EmptyReproBatch {
            which: which_batch_is_unusable(&energies_a, &energies_b),
        });
    };
    let differs = p_value < DIFFERS_P_THRESHOLD;

    let summary = build_summary(
        cfg,
        &spec,
        &params_a,
        &params_b,
        &report_a,
        &report_b,
        ks_statistic,
        p_value,
        differs,
    );
    write_summary(&cfg.output_dir.join(SUMMARY_FILENAME), &summary).map_err(ModeError::Io)?;

    Ok(ReproReport {
        ks_statistic,
        p_value,
        differs,
    })
}

/// Resolve `cfg.backend` against the built-in registry, applying
/// `backend_bin_override`/`backend_args_override` if set. Mirrors
/// `sweep::resolve_backend`/`ttt::resolve_backend`/
/// `comparative::resolve_backend`/`scaling::resolve_backend` exactly; kept
/// as its own copy per this crate's existing per-mode-module convention.
fn resolve_backend(cfg: &ReproConfig) -> Result<BackendSpec, ModeError> {
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
/// parameters directly on each job's `IsingProblem` fields, dispatch them in
/// one `run_session` call against `backend`, and append every resulting
/// record to `records_path` as it arrives. Mirrors
/// `comparative::draw_jobs`/`scaling::draw_jobs` for the job-drawing half;
/// the dispatch-and-append half mirrors every other mode's own per-unit
/// loop body.
async fn run_one_batch(
    backend: &BackendSpec,
    spec: &TopologySpec,
    seed: u64,
    count: u32,
    params: &JobParams,
    dir: &std::path::Path,
    records_path: &std::path::Path,
) -> Result<RunReport, ModeError> {
    std::fs::create_dir_all(dir).map_err(ModeError::Io)?;
    std::fs::File::create(records_path).map_err(ModeError::Io)?;

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

    let sock_path = unique_sock_path();
    let report = run_session(RunParams {
        backend,
        spec,
        jobs,
        sock_path: &sock_path,
        target: None,
    })
    .await?;

    for record in &report.records {
        append_jsonl(records_path, record).map_err(ModeError::Io)?;
    }

    Ok(report)
}

/// `passed` records' `best_energy_milli`, converted to natural `f64` units,
/// for feeding `stats::ks_distance`/`stats::ks_two_sample_p`. Mirrors
/// `comparative::run_comparative`'s own identical filter-and-convert step.
fn passed_energies(report: &RunReport) -> Vec<f64> {
    report
        .records
        .iter()
        .filter(|r| r.passed)
        .map(|r| milli_to_f64(r.best_energy_milli))
        .collect()
}

/// Name which batch(es) had no usable (`passed`) energies, for
/// [`ModeError::EmptyReproBatch`]'s message. Only called once both
/// `ks_distance`/`ks_two_sample_p` have already reported `None`, so at
/// least one of `a`/`b` is guaranteed to be empty or non-finite at that
/// point — the non-finite case cannot occur in practice (see this module's
/// own doc), so this need only distinguish "empty" from "not empty" to
/// produce an actionable name.
fn which_batch_is_unusable(a: &[f64], b: &[f64]) -> &'static str {
    match (a.is_empty(), b.is_empty()) {
        (true, false) => "run_a",
        (false, true) => "run_b",
        // `(true, true)`: both empty. `(false, false)`: neither is empty,
        // so `ks_distance`/`ks_two_sample_p`'s `None` must have come from a
        // non-finite value instead (see this module's own doc: unreachable
        // in practice for an `i64`-derived energy, but this stays a total
        // match rather than assuming it away). Both cases report the same
        // "can't attribute to one side" name.
        (true, true) | (false, false) => "run_a and run_b",
    }
}

/// Build the closing [`Summary`] from both runs' completed [`RunReport`]s
/// and this comparison's own KS outcome.
#[expect(
    clippy::too_many_arguments,
    reason = "every argument is a distinct, already-computed piece of this comparison's outcome; \
              bundling them into a struct just to pass them once to this call site's only \
              caller would not reduce the information this function needs, only rename it"
)]
fn build_summary(
    cfg: &ReproConfig,
    spec: &TopologySpec,
    params_a: &JobParams,
    params_b: &JobParams,
    report_a: &RunReport,
    report_b: &RunReport,
    ks_statistic: f64,
    p_value: f64,
    differs: bool,
) -> Summary {
    let totals = Totals {
        jobs: report_a.records.len() + report_b.records.len(),
        passed: report_a.records.iter().filter(|r| r.passed).count()
            + report_b.records.iter().filter(|r| r.passed).count(),
        rejected: report_a.records.iter().filter(|r| r.rejected).count()
            + report_b.records.iter().filter(|r| r.rejected).count(),
        // Both runs' records files are created empty and exclusively
        // written by this call, same reasoning as `sweep::build_summary`.
        skipped_lines: 0,
    };

    let run_json = |dir_name: &str, params: &JobParams, report: &RunReport| {
        let energies = passed_energies(report);
        serde_json::json!({
            "dir": dir_name,
            "num_reads": params.num_reads,
            "num_sweeps": params.num_sweeps,
            "anneal_time_us": params.anneal_time_us,
            "jobs": report.records.len(),
            "passed": report.records.iter().filter(|r| r.passed).count(),
            "rejected": report.records.iter().filter(|r| r.rejected).count(),
            "session_wall_ms": report.run_wall_ms,
            "miner_exit_code": report.miner_exit_code,
            "miner_unresponsive": report.miner_unresponsive,
            "miner_stderr_tail": report.miner_stderr_tail,
            "best_energy_milli_stats": stats_json(describe(&energies)),
        })
    };

    let config = serde_json::json!({
        "backend": cfg.backend,
        "topology": cfg.topology.label(),
        "seed": cfg.seed,
        "num_problems": cfg.num_problems,
        "hardness_a": cfg.hardness_a,
        "hardness_b": cfg.hardness_b,
        "differs_p_threshold": DIFFERS_P_THRESHOLD,
        "ks_statistic": ks_statistic,
        "p_value": p_value,
        "differs": differs,
        "run_a": run_json(RUN_A_DIR, params_a, report_a),
        "run_b": run_json(RUN_B_DIR, params_b, report_b),
    });

    Summary {
        mode: "reproducibility".to_string(),
        backend: cfg.backend.clone(),
        topology: TopologyInfo {
            name: cfg.topology.label(),
            num_nodes: spec.nodes.len(),
            num_edges: spec.edges.len(),
            topology_hash: crate::modes::hex_encode(&spec.topology_hash()),
        },
        hardware: HardwareInfo::detect(),
        config,
        totals,
    }
}

/// Render a `stats::describe` result as JSON, `null` for an empty or
/// non-finite sample. Mirrors `sweep::stats_json`'s/`ttt::stats_json`'s/
/// `scaling::stats_json`'s own copy.
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
/// `stats::ks_distance`/`stats::ks_two_sample_p`/`stats::describe`. Mirrors
/// `sweep::milli_to_f64`/`comparative::milli_to_f64`; kept as its own copy
/// per this crate's per-mode-module convention.
#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark energies fit comfortably within f64's 52-bit exact-integer range"
)]
fn milli_to_f64(milli: i64) -> f64 {
    milli as f64 / 1000.0
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
    std::env::temp_dir().join(format!(
        "isingmark-reproducibility-{}-{n}.sock",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::{milli_to_f64, which_batch_is_unusable, DIFFERS_P_THRESHOLD};

    #[test]
    fn milli_to_f64_divides_by_one_thousand() {
        assert!((milli_to_f64(1_500) - 1.5).abs() < 1e-12);
        assert!((milli_to_f64(-2_000) - -2.0).abs() < 1e-12);
    }

    #[test]
    fn differs_p_threshold_is_the_conventional_0_05() {
        assert!((DIFFERS_P_THRESHOLD - 0.05).abs() < f64::EPSILON);
    }

    #[test]
    fn which_batch_is_unusable_names_the_empty_side() {
        assert_eq!(which_batch_is_unusable(&[], &[1.0]), "run_a");
        assert_eq!(which_batch_is_unusable(&[1.0], &[]), "run_b");
        assert_eq!(which_batch_is_unusable(&[], &[]), "run_a and run_b");
    }
}
