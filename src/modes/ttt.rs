// SPDX-License-Identifier: AGPL-3.0-or-later
//! Time-to-target (TTT) mode: for each energy target, dispatch independent
//! trials until one reaches the target or a per-target time budget is
//! exhausted, and report how long each successful trial took.
//!
//! # Units: `--energy-targets` is natural, `TttConfig`/`TttReport` are milli
//!
//! This crate's global convention is `i64` millis throughout, converted to
//! float only at report boundaries. The CLI flag is a report boundary in the
//! other direction (human input, not human output), and — like `--hardness`
//! — takes natural units: `--energy-targets "-5.0,-10.0"` means "-5.0 and
//! -10.0 energy units", not "-5 and -10 milli-units". [`parse_energy_targets`]
//! converts natural to milli (`* 1000.0`, rounded to the nearest `i64`) at
//! parse time, so every internal type
//! (`TttConfig::energy_targets_milli`, `TttReport::per_target`'s keys) is
//! milli from the moment it exists. Getting this backwards — treating
//! `-5.0` as already-milli — is a silent, plausible-looking failure: a
//! target of `-5` milli-units is so close to `0` that ordinary benchmark
//! energies (order 1-20 natural units for the bundled fixtures) satisfy it
//! on the very first trial, producing a full-looking, wrong-by-1000x result
//! set with no error. [`parse_energy_targets`]'s own tests pin the exact
//! `x1000` factor for this reason.
//!
//! # One job, one session, per trial
//!
//! Each trial is exactly one dispatched job in its own `run_session` call —
//! never a "keep sampling until this trial succeeds" inner loop. Two
//! consequences fall out of this:
//!
//! 1. **No hang risk from an unreachable target.** A target nothing can
//!    satisfy still costs exactly `num_trials` sessions (or fewer, if the
//!    time budget below cuts it short), the same as any other target, and
//!    then reports an empty `Vec<f64>` for that target.
//! 2. **The `max_time_s` budget can be checked between trials.** Task 7's
//!    human ruling removed session-level timeouts in favor of `Ping`/`Status`
//!    liveness (`backend::session`'s own, already-reviewed contract this
//!    task must not modify), so there is no way to bound one `run_session`
//!    call's own length from the outside. Bounding *this mode's own loop*
//!    instead — checking accumulated wall time before starting each new
//!    trial's session, mirroring `throughput::dispatch_until_duration`'s own
//!    "check between batches" shape — is what stays outside that contract.
//!    A trial already dispatched is allowed to finish; the budget is honored
//!    within one trial's worth of tolerance, not to the millisecond.
//!
//! # Two different "elapsed time"s, not one
//!
//! Mirrors the split `modes::sweep::build_summary` documents between
//! `session_wall_ms` and `wall_ms_stats`, for the same reason: conflating
//! them silently mixes fixed per-trial overhead into a number that is
//! supposed to measure only the annealer's own work.
//!
//! - The **budget accounting** above uses `RunReport::run_wall_ms`: the
//!   *entire* per-trial session (subprocess spawn, handshake, this one job's
//!   dispatch, and teardown) — the real wall-clock cost this mode's own loop
//!   must not exceed by much.
//! - The **time recorded in `TttReport::per_target`** uses
//!   `JobRecord::wall_ms` instead: dispatch-to-result for that one job only.
//!   A "time to target" number that silently included process-spawn
//!   overhead (which has nothing to do with how hard the target is to reach)
//!   would be misleading in exactly the way `modes::sweep`'s own docs warn
//!   about for its `wall_ms_stats`.
//!
//! # Output layout: one directory per target, all targets over the same jobs
//!
//! Every target's trials replay the *identical* `(seed, num_trials)`-drawn
//! job sequence — a fresh [`ProblemSource`] built per target, exactly
//! `modes::sweep`'s paired-design mechanism (see that module's docs for the
//! full reasoning: `Job.job_id` depends only on `(seed, index)`, never on the
//! target being benchmarked, so a fresh source per target reproduces the
//! same sequence regardless of which target ran first). This is what makes
//! different targets' reach times comparable at all: a difference between
//! two targets' results is attributable to the target, not to which
//! problems each one happened to draw. Each target's records land in its own
//! `<output_dir>/target_<millis>/records.jsonl` (see [`target_dir_name`]),
//! never a shared file, and the [`ModeError::RecordsFileExists`] pre-pass
//! runs across *every* target's directory before *any* target is dispatched
//! — the same eager-check shape `modes::sweep` established, for the same
//! reason: a conflict discovered partway through must not leave earlier
//! targets' real output sitting next to a run that was never finished.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::backend::{builtin_registry, run_session, BackendSpec, RunParams};
use crate::modes::{hex_encode, ModeError, ParseError, TopologyRef};
use crate::problem::ProblemSource;
use crate::record::{
    append_jsonl, write_summary, HardwareInfo, JobRecord, Summary, TopologyInfo, Totals,
};
use crate::stats::{describe, percentile, Stats};
use crate::topology::TopologySpec;

/// Basename of one target's JSONL record file within its own subdirectory of
/// [`TttConfig::output_dir`] (see [`target_dir_name`]).
pub const RECORDS_FILENAME: &str = "records.jsonl";
/// Basename of the run's closing summary within [`TttConfig::output_dir`].
pub const SUMMARY_FILENAME: &str = "summary.json";

/// Miner account every trial derives jobs against. Only affects the derived
/// nonce (via [`ProblemSource`]), not this run's results, so a fixed value
/// is used across every run — the same reasoning `sweep`/`throughput` give
/// for their own copies of this constant.
const MINER_ACCOUNT: [u8; 32] = [0u8; 32];

/// No wire deadline: this mode measures its own timing and does not need the
/// miner to expire jobs on its own clock.
const NO_DEADLINE_MS: u64 = 0;

/// Scale between the natural units `--energy-targets` accepts and the
/// milli-units this module (and `JobRecord::best_energy_milli`) uses
/// internally. See this module's doc for why getting this backwards is a
/// silent, plausible-looking failure rather than an obvious one.
const MILLI_PER_NATURAL: f64 = 1000.0;

/// Configuration for [`run_ttt`].
#[derive(Debug, Clone)]
pub struct TttConfig {
    /// `backend::BackendSpec::name` to benchmark (e.g. `"cpu-sa"`).
    pub backend: String,
    /// Topology to draw jobs against. Shared by every target.
    pub topology: TopologyRef,
    /// Energy targets in **milli-units** (see this module's doc on the
    /// natural-vs-milli boundary; `cli::parse_energy_targets_arg` performs
    /// the conversion before this field is ever populated from the CLI).
    /// Must be non-empty and free of duplicates (a duplicate would collide
    /// on one output subdirectory) — see [`ModeError::EmptyTargets`]/
    /// [`ModeError::DuplicateTarget`].
    pub energy_targets_milli: Vec<i64>,
    /// Trials attempted per target, before any [`TttConfig::max_time_s`]
    /// cutoff. Every target draws the identical `num_trials`-job sequence
    /// (same seed) — see this module's doc on the paired design this
    /// establishes.
    pub num_trials: u32,
    /// Wall-clock budget per target, in seconds, checked between trials
    /// (never mid-trial) — see this module's doc on why a per-trial,
    /// between-sessions check is what this mode can do without adding a
    /// session-level timeout to already-reviewed code.
    pub max_time_s: f64,
    /// Seed for deterministic problem generation, shared by every target.
    pub seed: u64,
    /// Directory to write one subdirectory per target into, plus a
    /// top-level [`SUMMARY_FILENAME`]. Created if it does not already
    /// exist. Refuses to start (see [`ModeError::RecordsFileExists`]) if
    /// *any* target's [`RECORDS_FILENAME`] already contains data from an
    /// earlier run — checked for every target before any of them run.
    pub output_dir: PathBuf,
    /// Explicit backend binary path, bypassing `$PATH` resolution. See
    /// `throughput::ThroughputConfig::backend_bin_override`.
    pub backend_bin_override: Option<PathBuf>,
    /// Explicit backend CLI args, replacing `backend::BackendSpec::args`.
    /// See `throughput::ThroughputConfig::backend_args_override`.
    pub backend_args_override: Option<Vec<String>>,
}

/// Outcome of [`run_ttt`].
pub struct TttReport {
    /// One entry per requested target (milli-units), holding the seconds-to-
    /// reach of every trial that reached it, in trial order (not sorted).
    /// An unreachable target — or one no trial reached before
    /// [`TttConfig::max_time_s`] cut that target's trials short — maps to an
    /// empty `Vec`, not a missing key, `NaN`, or `0.0`.
    pub per_target: BTreeMap<i64, Vec<f64>>,
    /// The run summary also written to [`SUMMARY_FILENAME`], returned here
    /// too so a caller (namely `main.rs`) does not have to read that file
    /// back off disk and re-parse it.
    pub summary: Summary,
}

/// Run time-to-target mode: for every target in `cfg.energy_targets_milli`,
/// draw `cfg.num_trials` deterministic jobs from a fresh, target-scoped
/// `(cfg.seed, cfg.num_trials)` draw, dispatch each in its own one-job
/// session against `cfg.backend`, and record how long each trial that
/// reached the target (`best_energy_milli <= target`) took. Writes each
/// target's records to its own `<target dir>/records.jsonl` as trials
/// complete, and a closing [`SUMMARY_FILENAME`] once every target is done.
///
/// # Errors
///
/// Returns [`ModeError::Topology`] if `cfg.topology` fails to load,
/// [`ModeError::UnknownBackend`] if `cfg.backend` matches no entry in
/// `backend::builtin_registry`, [`ModeError::EmptyTargets`] if
/// `cfg.energy_targets_milli` is empty, [`ModeError::DuplicateTarget`] if it
/// contains the same milli-value target twice, [`ModeError::RecordsFileExists`]
/// if any target's records file already contains data, [`ModeError::Session`]
/// if a trial's session fails to bind or spawn, and [`ModeError::Io`] if the
/// output directory or any file cannot be written.
pub async fn run_ttt(cfg: &TttConfig) -> Result<TttReport, ModeError> {
    let spec = cfg.topology.load()?;
    // The miner opens its device before any job arrives, so the kernel
    // capacity must be on its argv, derived from the topology we are about
    // to run.
    let backend = resolve_backend(cfg)?.with_node_capacity(spec.nodes.len());
    validate_targets(&cfg.energy_targets_milli)?;

    std::fs::create_dir_all(&cfg.output_dir).map_err(ModeError::Io)?;

    let target_paths: Vec<(i64, PathBuf, PathBuf, String)> = cfg
        .energy_targets_milli
        .iter()
        .map(|&target_milli| {
            let dir_name = target_dir_name(target_milli);
            let dir = cfg.output_dir.join(&dir_name);
            let records_path = dir.join(RECORDS_FILENAME);
            (target_milli, dir, records_path, dir_name)
        })
        .collect();

    // Eager, whole-grid conflict check before anything is created — see this
    // module's doc and `modes::sweep`'s identical reasoning.
    for (_, _, records_path, _) in &target_paths {
        if std::fs::metadata(records_path).is_ok_and(|m| m.len() > 0) {
            return Err(ModeError::RecordsFileExists(records_path.clone()));
        }
    }

    let budget = target_wall(cfg.max_time_s);
    let mut outcomes = Vec::with_capacity(target_paths.len());

    for (target_milli, dir, records_path, dir_name) in &target_paths {
        std::fs::create_dir_all(dir).map_err(ModeError::Io)?;
        std::fs::File::create(records_path).map_err(ModeError::Io)?;

        let mut source = ProblemSource::new(
            &spec,
            MINER_ACCOUNT,
            cfg.seed,
            cfg.num_trials,
            NO_DEADLINE_MS,
        );
        let mut records: Vec<JobRecord> = Vec::new();
        let mut trials_attempted: u32 = 0;
        let mut elapsed = Duration::ZERO;

        for _ in 0..cfg.num_trials {
            // Checked before starting a new trial's session, never mid-trial
            // — see this module's doc on why this is the only budget this
            // mode can enforce without reaching into `backend::session`.
            if elapsed >= budget {
                break;
            }
            // `count == cfg.num_trials` above, so this always yields a job
            // for every iteration of this same loop; kept as an explicit
            // break (not a panic/unwrap) to stay total for any future
            // caller that changes that invariant.
            let Some(job) = source.next_job() else {
                break;
            };

            let sock_path = unique_sock_path();
            let report = run_session(RunParams {
                backend: &backend,
                spec: &spec,
                jobs: vec![job],
                sock_path: &sock_path,
                target: None,
            })
            .await?;

            for record in &report.records {
                append_jsonl(records_path, record).map_err(ModeError::Io)?;
            }
            let handshake_ok = report.handshake_ok;
            records.extend(report.records);
            trials_attempted += 1;
            elapsed += Duration::from_millis(report.run_wall_ms);

            // Mirrors `throughput::dispatch_until_duration`: stop early if a
            // session never completes its handshake, since retrying a
            // backend that cannot even start would otherwise burn the rest
            // of this target's trial budget for nothing.
            if !handshake_ok {
                break;
            }
        }

        outcomes.push(TargetOutcome {
            target_milli: *target_milli,
            dir_name: dir_name.clone(),
            trials_attempted,
            records,
        });
    }

    let mut per_target = BTreeMap::new();
    for outcome in &outcomes {
        per_target.insert(outcome.target_milli, reached_times_s(outcome));
    }

    let summary = build_summary(cfg, &spec, &outcomes);
    write_summary(&cfg.output_dir.join(SUMMARY_FILENAME), &summary).map_err(ModeError::Io)?;

    Ok(TttReport {
        per_target,
        summary,
    })
}

/// Resolve `cfg.backend` against the built-in registry, applying
/// `backend_bin_override`/`backend_args_override` if set. Mirrors
/// `sweep::resolve_backend`/`throughput::resolve_backend` exactly; kept as
/// its own copy per this crate's existing per-mode-module convention.
fn resolve_backend(cfg: &TttConfig) -> Result<BackendSpec, ModeError> {
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

/// Reject an empty target list, or one containing the same milli-value
/// target twice.
///
/// # Errors
///
/// Returns [`ModeError::EmptyTargets`] if `targets` is empty, or
/// [`ModeError::DuplicateTarget`] if any value appears more than once — a
/// duplicate would resolve to the same [`target_dir_name`], and the second
/// target's `std::fs::File::create` would silently truncate the first
/// target's just-written records rather than erroring, since `File::create`
/// truncates an existing file rather than refusing to touch it.
fn validate_targets(targets: &[i64]) -> Result<(), ModeError> {
    if targets.is_empty() {
        return Err(ModeError::EmptyTargets);
    }
    let mut seen = HashSet::with_capacity(targets.len());
    for &target in targets {
        if !seen.insert(target) {
            return Err(ModeError::DuplicateTarget(target));
        }
    }
    Ok(())
}

/// One target's subdirectory name under [`TttConfig::output_dir`]:
/// `target_<millis>` (e.g. `target_-5000`) — deterministic and, unlike
/// `modes::sweep::cell_dir_name`, needs no disambiguating index since
/// [`validate_targets`] already rejects duplicate targets.
#[must_use]
fn target_dir_name(target_milli: i64) -> String {
    format!("target_{target_milli}")
}

/// Convert a duration in seconds to a [`Duration`], degrading a negative or
/// non-finite value to zero (an already-exhausted budget: no trial is
/// attempted) rather than panicking — `Duration::from_secs_f64` panics on
/// either. Mirrors `throughput::target_wall` exactly; CLI-level validation
/// (`cli::parse_max_time_s`) is expected to reject those before they ever
/// reach this library function, but this stays total for callers that
/// construct a [`TttConfig`] directly.
fn target_wall(max_time_s: f64) -> Duration {
    if max_time_s.is_finite() {
        Duration::from_secs_f64(max_time_s.max(0.0))
    } else {
        Duration::ZERO
    }
}

/// One target's completed run: every trial's records (across however many
/// trials were actually attempted — fewer than `TttConfig::num_trials` if
/// `max_time_s` cut it short or a session never handshook), plus that
/// target's own directory name.
///
/// Does not derive `Debug`: `record::JobRecord` does (unlike
/// `backend::RunReport`, which is why `sweep::CellOutcome` cannot), but
/// nothing in this module needs it, and this task does not add derives to
/// already-reviewed code elsewhere just to enable one here.
struct TargetOutcome {
    target_milli: i64,
    dir_name: String,
    trials_attempted: u32,
    records: Vec<JobRecord>,
}

/// Seconds-to-reach for every trial in `outcome` that satisfied its own
/// target (`record.passed && best_energy_milli <= target_milli`), in trial
/// order (not sorted — callers needing a distribution, e.g.
/// [`median_and_p99`], sort their own copy first). `record.passed` is
/// checked first; a rejected or abandoned-in-flight record has
/// `best_energy_milli: None` and cannot satisfy the energy bound.
fn reached_times_s(outcome: &TargetOutcome) -> Vec<f64> {
    outcome
        .records
        .iter()
        .filter(|r| {
            r.passed
                && r.best_energy_milli
                    .is_some_and(|e| e <= outcome.target_milli)
        })
        .map(|r| wall_ms_to_seconds(r.wall_ms))
        .collect()
}

/// Build the closing [`Summary`] from every target's completed
/// [`TargetOutcome`]. `totals` aggregates across every target's dispatched
/// trials; `config` carries the requested run parameters alongside each
/// target's own directory, trial counts, reach count, and
/// `stats::describe`/`stats::percentile`-derived timing distribution.
fn build_summary(cfg: &TttConfig, spec: &TopologySpec, outcomes: &[TargetOutcome]) -> Summary {
    let totals = Totals {
        jobs: outcomes.iter().map(|o| o.records.len()).sum(),
        passed: outcomes
            .iter()
            .flat_map(|o| &o.records)
            .filter(|r| r.passed)
            .count(),
        rejected: outcomes
            .iter()
            .flat_map(|o| &o.records)
            .filter(|r| r.rejected)
            .count(),
        // Every target's records file is created empty and exclusively
        // written by this call, same reasoning as `sweep::build_summary`.
        skipped_lines: 0,
    };

    let targets_json: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|outcome| {
            let times = reached_times_s(outcome);
            let (median_reached_at_s, p99_reached_at_s) = median_and_p99(&times);
            serde_json::json!({
                "target_milli": outcome.target_milli,
                "dir": outcome.dir_name,
                "trials_requested": cfg.num_trials,
                "trials_attempted": outcome.trials_attempted,
                "reached": times.len(),
                // Explicit `null` (never a missing key, `NaN`, or `0.0`)
                // when no trial reached this target — see `TttReport::per_target`'s
                // own doc for why that distinction matters.
                "median_reached_at_s": median_reached_at_s,
                "p99_reached_at_s": p99_reached_at_s,
                "times_stats": stats_json(describe(&times)),
            })
        })
        .collect();

    let config = serde_json::json!({
        "backend": cfg.backend,
        "topology": cfg.topology.label(),
        "seed": cfg.seed,
        "num_trials": cfg.num_trials,
        "max_time_s": cfg.max_time_s,
        "targets": targets_json,
    });

    Summary {
        mode: "ttt".to_string(),
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

/// Render a `stats::describe` result as JSON, `null` for an empty or
/// non-finite sample. Mirrors `sweep::stats_json`'s own copy (`stats::Stats`
/// does not derive `Serialize`, and this task does not add one).
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

/// Median and p99 of `times_s`, via `stats::percentile` directly (not
/// `stats::describe`, which would sort its own copy internally and hide any
/// bug in this function's own sort). `stats::percentile` requires a sorted
/// slice as a documented precondition — sorting a local copy here, rather
/// than trusting `times_s`'s own (trial, not value) order, is exactly what
/// this function exists to do. Returns `(None, None)` for an empty input,
/// same as `stats::percentile` itself.
fn median_and_p99(times_s: &[f64]) -> (Option<f64>, Option<f64>) {
    let mut sorted = times_s.to_vec();
    sorted.sort_by(f64::total_cmp);
    (percentile(&sorted, 0.5), percentile(&sorted, 0.99))
}

/// `JobRecord::wall_ms` (a `u64` count of milliseconds) to seconds, for
/// `TttReport::per_target`.
#[expect(
    clippy::cast_precision_loss,
    reason = "a single job's wall time in milliseconds fits comfortably within f64's 52-bit \
              exact-integer range"
)]
fn wall_ms_to_seconds(wall_ms: u64) -> f64 {
    wall_ms as f64 / 1000.0
}

/// Natural energy units to milli-units (`* 1000.0`, rounded to the nearest
/// `i64`) — see this module's doc on why this conversion exists and what
/// dropping it would silently do to a benchmark run.
#[expect(
    clippy::cast_possible_truncation,
    reason = "energy targets in this crate's benchmark domain fit comfortably within i64 after \
              scaling by 1000; float-to-int casts in Rust saturate rather than invoking undefined \
              behavior, so an unrealistic out-of-range input still cannot panic here"
)]
fn natural_to_milli(value: f64) -> i64 {
    (value * MILLI_PER_NATURAL).round() as i64
}

/// Parse a `--energy-targets` value: a comma-separated list of energy
/// targets in **natural units** (e.g. `"-5.0,-10.0,-15.0"`), converted to
/// milli-units via [`natural_to_milli`]. See this module's doc for why the
/// CLI boundary is natural units while every internal type is milli.
///
/// # Errors
///
/// Returns [`ParseError`] if `s` is empty, any comma-separated entry is
/// empty, non-numeric, or non-finite, or if two entries convert to the same
/// milli-value target (see [`validate_targets`] for why a duplicate is
/// rejected rather than silently deduplicated).
pub fn parse_energy_targets(s: &str) -> Result<Vec<i64>, ParseError> {
    let targets: Vec<i64> = s
        .split(',')
        .map(parse_one_energy_target)
        .collect::<Result<_, _>>()?;
    if targets.is_empty() {
        return Err(ParseError("--energy-targets must not be empty".to_string()));
    }
    let mut seen = HashSet::with_capacity(targets.len());
    for &milli in &targets {
        if !seen.insert(milli) {
            return Err(ParseError(format!(
                "--energy-targets {s:?} contains two entries that convert to the same \
                 milli-value target ({milli}); each target must resolve to its own output \
                 subdirectory"
            )));
        }
    }
    Ok(targets)
}

/// Parse one comma-separated `--energy-targets` entry (natural units) into a
/// milli-value target. Split out from [`parse_energy_targets`] so
/// `cli::TttArgs` can drive clap's own `value_delimiter = ','` splitting
/// (required to populate a `Vec<i64>`-typed field one token at a time — see
/// `cli.rs`'s comment on `TttArgs::energy_targets` for why a single
/// whole-string parser returning `Vec<i64>` does not work with clap's
/// derive macro) while still sharing this one conversion with
/// [`parse_energy_targets`] itself.
///
/// # Errors
///
/// Returns [`ParseError`] if `entry` (after trimming) is empty, not a valid
/// number, or non-finite.
pub fn parse_one_energy_target(entry: &str) -> Result<i64, ParseError> {
    let trimmed = entry.trim();
    if trimmed.is_empty() {
        return Err(ParseError(
            "--energy-targets contains an empty entry".to_string(),
        ));
    }
    let value: f64 = trimmed.parse().map_err(|_| {
        ParseError(format!(
            "--energy-targets entry {trimmed:?} is not a valid number"
        ))
    })?;
    if !value.is_finite() {
        return Err(ParseError(format!(
            "--energy-targets entry {trimmed:?} must be finite"
        )));
    }
    Ok(natural_to_milli(value))
}

/// A process-wide counter backing [`unique_sock_path`], mirroring
/// `sweep`'s/`throughput`'s own (separately-scoped) counters of the same
/// purpose.
static SOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build a coordinator socket path under the system temp directory, unique
/// across every call in this process (including concurrent ones) and
/// disjoint from `sweep`'s/`throughput`'s own socket paths (distinct
/// filename prefix).
fn unique_sock_path() -> PathBuf {
    let n = SOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("isingmark-ttt-{}-{n}.sock", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::{
        median_and_p99, natural_to_milli, parse_energy_targets, target_dir_name, validate_targets,
        wall_ms_to_seconds,
    };
    use crate::modes::ModeError;

    #[test]
    fn natural_to_milli_scales_by_one_thousand() {
        assert_eq!(natural_to_milli(-5.0), -5_000);
        assert_eq!(natural_to_milli(10.0), 10_000);
        assert_eq!(natural_to_milli(0.0), 0);
    }

    #[test]
    fn natural_to_milli_rounds_to_the_nearest_integer() {
        assert_eq!(natural_to_milli(1.2345), 1_235);
        assert_eq!(natural_to_milli(-1.2344), -1_234);
    }

    /// Pins the exact `x1000` factor `--energy-targets` relies on: `-5`
    /// (natural) must become `-5_000` (milli), not `-5`. A mutation that
    /// drops the conversion (treats the parsed value as already-milli)
    /// changes this from `-5_000` to `-5`, a difference too large to miss.
    #[test]
    fn parse_energy_targets_converts_natural_units_to_milli() {
        assert_eq!(parse_energy_targets("-5").expect("parses"), vec![-5_000]);
        assert_eq!(
            parse_energy_targets("-5,-10,-15").expect("parses"),
            vec![-5_000, -10_000, -15_000]
        );
    }

    #[test]
    fn parse_energy_targets_accepts_fractional_and_positive_values() {
        assert_eq!(
            parse_energy_targets("0.5,2.25").expect("parses"),
            vec![500, 2_250]
        );
    }

    #[test]
    fn parse_energy_targets_rejects_an_empty_string() {
        assert!(parse_energy_targets("").is_err());
    }

    #[test]
    fn parse_energy_targets_rejects_an_empty_entry_in_a_list() {
        assert!(parse_energy_targets("-5,,--10").is_err());
        assert!(parse_energy_targets("-5,").is_err());
    }

    #[test]
    fn parse_energy_targets_rejects_non_numeric_entries() {
        assert!(parse_energy_targets("not-a-number").is_err());
    }

    #[test]
    fn parse_energy_targets_rejects_non_finite_entries() {
        assert!(parse_energy_targets("nan").is_err());
        assert!(parse_energy_targets("inf").is_err());
        assert!(parse_energy_targets("-5,inf").is_err());
    }

    #[test]
    fn parse_energy_targets_rejects_two_entries_converting_to_the_same_milli_target() {
        let err = parse_energy_targets("-5,-5").expect_err("duplicate must be rejected");
        assert!(err.0.contains("-5000") || err.0.contains("duplicate") || err.0.contains("same"));
    }

    #[test]
    fn validate_targets_rejects_an_empty_list() {
        let err = validate_targets(&[]).expect_err("empty list must be rejected");
        assert!(matches!(err, ModeError::EmptyTargets));
    }

    #[test]
    fn validate_targets_rejects_a_duplicate_value() {
        let err =
            validate_targets(&[-5_000, 10_000, -5_000]).expect_err("duplicate must be rejected");
        assert!(matches!(err, ModeError::DuplicateTarget(-5_000)));
    }

    #[test]
    fn validate_targets_accepts_distinct_values() {
        validate_targets(&[-5_000, -10_000, -15_000]).expect("distinct values are valid");
    }

    #[test]
    fn target_dir_name_uses_the_raw_milli_value_including_the_sign() {
        assert_eq!(target_dir_name(-5_000), "target_-5000");
        assert_eq!(target_dir_name(10_000), "target_10000");
        assert_eq!(target_dir_name(0), "target_0");
    }

    #[test]
    fn wall_ms_to_seconds_divides_by_one_thousand() {
        assert!((wall_ms_to_seconds(1_500) - 1.5).abs() < 1e-12);
        assert!((wall_ms_to_seconds(0) - 0.0).abs() < 1e-12);
    }

    /// `median_and_p99` must sort its own copy before calling
    /// `stats::percentile` (which documents a sorted-input precondition and
    /// only `debug_assert!`s it, rather than erroring, so a dropped sort
    /// would not necessarily fail loudly in a release build). Feeds a
    /// slice in explicitly *descending* order — the opposite of sorted — so
    /// a missing sort would either panic (debug builds, via that
    /// `debug_assert!`) or silently return the wrong values (release
    /// builds); this test's exact expected numbers come from
    /// `stats::aggregate`'s own hand-derived example (`[2.0, 4.0, 4.0, 9.0,
    /// 11.0]`, ascending), fed here in reverse.
    #[test]
    fn median_and_p99_sorts_before_computing_percentiles() {
        let descending = vec![11.0, 9.0, 4.0, 4.0, 2.0];
        let (median, p99) = median_and_p99(&descending);
        assert!((median.expect("non-empty") - 4.0).abs() < 1e-12);
        assert!((p99.expect("non-empty") - 10.92).abs() < 1e-9);
    }

    #[test]
    fn median_and_p99_of_empty_slice_is_none_none() {
        assert_eq!(median_and_p99(&[]), (None, None));
    }
}
