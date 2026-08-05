// SPDX-License-Identifier: AGPL-3.0-or-later
//! Throughput mode: repeatedly dispatch jobs against a backend for a fixed
//! wall-clock duration and report how many it completed.
//!
//! `--duration` means "keep dispatching jobs until the duration elapses,"
//! not "run one session lasting that long." `backend::session::run_session`
//! takes a fixed `Vec<Job>` up front and has no way to feed it more jobs
//! once it is running, so a single session cannot be stretched to an exact
//! wall-clock length without either guessing a job count ahead of time (and
//! risking finishing early or running long) or cancelling the session's
//! future mid-flight, which would drop every record it had not yet returned
//! (`run_session` only reports `RunReport` once, at the very end). Neither
//! is compatible with `duration` meaning "measure real wall time." Instead,
//! this mode runs repeated, modestly sized sessions in a loop, accumulating
//! each session's own measured `run_wall_ms` until the total reaches the
//! requested duration. This costs a spawn/handshake per batch rather than
//! one per run, but keeps the reported duration and `jobs_per_min` honest.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use quip_proto::v1::SetTarget;

use crate::backend::{builtin_registry, run_session, BackendSpec, RunParams, RunReport};
use crate::modes::{hex_encode, ModeError, ParseError, TopologyRef};
use crate::params::{JobParams, ParamCurve};
use crate::problem::ProblemSource;
use crate::record::{
    append_jsonl, write_summary, HardwareInfo, JobRecord, Summary, TopologyInfo, Totals,
};
use crate::topology::TopologySpec;

/// Basename of a throughput run's JSONL record file within
/// [`ThroughputConfig::output_dir`].
pub const RECORDS_FILENAME: &str = "records.jsonl";
/// Basename of a throughput run's summary JSON file within
/// [`ThroughputConfig::output_dir`].
pub const SUMMARY_FILENAME: &str = "summary.json";

/// Jobs drawn and dispatched per session. Small enough that one broken or
/// slow batch cannot burn a large fraction of the requested duration before
/// this mode notices and starts the next one; large enough to amortize each
/// session's own spawn-and-handshake cost across more than a job or two.
const BATCH_SIZE: u32 = 8;

/// Miner account this mode always derives jobs against. Only affects the
/// derived job nonce (via `problem::ProblemSource`), not this run's
/// results, so a fixed value is used across every run rather than
/// generating or accepting one — nothing downstream attaches identity
/// semantics to it.
const MINER_ACCOUNT: [u8; 32] = [0u8; 32];

/// No wire deadline: throughput mode measures wall time itself and does not
/// need the miner to expire jobs on its own clock.
const NO_DEADLINE_MS: u64 = 0;

/// Default hardness-to-parameter curve. Every bound is nonzero, per
/// `ParamCurve`'s own documented "0 means unset" contract. Placeholder
/// Default hardness-to-parameter curve when neither CLI nor file supplies
/// one. Every bound is nonzero, per `ParamCurve`'s "0 means unset" contract.
pub const DEFAULT_PARAM_CURVE: ParamCurve = ParamCurve {
    min_reads: 8,
    max_reads: 64,
    min_sweeps: 100,
    max_sweeps: 1000,
    min_anneal_us: 5,
    max_anneal_us: 200,
};

/// Configuration for [`run_throughput`].
#[derive(Debug, Clone)]
pub struct ThroughputConfig {
    /// `backend::BackendSpec::name` to benchmark (e.g. `"cpu-sa"`).
    pub backend: String,
    /// Topology to draw jobs against.
    pub topology: TopologyRef,
    /// Difficulty in `[0.0, 1.0]`, resolved into concrete sample parameters
    /// via [`Self::param_curve`]. Callers driving this outside the CLI
    /// are responsible for rejecting non-finite/out-of-range values
    /// themselves; `params::ParamCurve::resolve` is total and silently
    /// treats `NaN` as `0.0`.
    pub hardness: f64,
    /// Hardness-to-parameter curve used to resolve [`Self::hardness`].
    /// CLI/`--config` merge supplies this (file value, else
    /// [`DEFAULT_PARAM_CURVE`]).
    pub param_curve: ParamCurve,
    /// How long to keep dispatching jobs, in seconds.
    pub duration_s: f64,
    /// Seed for deterministic problem generation.
    pub seed: u64,
    /// Directory to write [`RECORDS_FILENAME`]/[`SUMMARY_FILENAME`] to.
    /// Created if it does not already exist. Refuses to start (see
    /// [`ModeError::RecordsFileExists`]) if [`RECORDS_FILENAME`] inside it
    /// already contains data from an earlier run.
    pub output_dir: PathBuf,
    /// Explicit backend binary path, bypassing `$PATH` resolution.
    ///
    /// Exists for tests (pointing at the in-repo `isingmark-fake-miner`)
    /// and for operators running a miner binary that is not on `$PATH`.
    /// `None` — the CLI's default — uses whatever `backend::builtin_registry`
    /// resolved for `backend`.
    pub backend_bin_override: Option<PathBuf>,
    /// Explicit backend CLI args, replacing `backend::BackendSpec::args`.
    ///
    /// Exists so tests can drive `isingmark-fake-miner`'s misbehavior flags
    /// (`--reject-job`, `--stderr-and-exit`, ...) through this public
    /// config surface instead of needing a `BackendSpec` escape hatch of
    /// their own. `None` — the CLI's default — leaves the registry's own
    /// (empty, for every built-in backend) args untouched.
    pub backend_args_override: Option<Vec<String>>,
}

/// Run throughput mode and return only the [`RunReport`]. Thin wrapper over
/// [`run_throughput_with_summary`] for callers that do not need the
/// [`Summary`] this call also writes to disk.
///
/// # Errors
///
/// See [`run_throughput_with_summary`].
pub async fn run_throughput(cfg: &ThroughputConfig) -> Result<RunReport, ModeError> {
    let (report, _summary) = run_throughput_with_summary(cfg).await?;
    Ok(report)
}

/// Run throughput mode: draw deterministic jobs from `cfg.topology`/`cfg.seed`
/// and dispatch them against `cfg.backend` in repeated sessions until
/// `cfg.duration_s` of measured wall time has elapsed, writing every
/// completed record to `RECORDS_FILENAME` per batch (as each session
/// completes) and a closing `SUMMARY_FILENAME` once done. Returns both the
/// [`RunReport`] and the [`Summary`] written to `SUMMARY_FILENAME`, so a
/// caller that wants to render the summary (namely `main.rs`) does not have
/// to read that file back off disk and re-parse it — a round trip that can
/// only fail or silently degrade for a file this same call just wrote
/// itself.
///
/// The summary's `config` includes `jobs_per_min` and `measured_wall_ms`
/// (both derived from real measured wall time, never from `cfg.duration_s`)
/// alongside `miner_exit_code`/`miner_unresponsive`/`miner_stderr_tail`, so
/// a run a backend failed partway through is distinguishable from a
/// complete one without needing the raw [`RunReport`].
///
/// # Errors
///
/// Returns [`ModeError::Topology`] if `cfg.topology` fails to load,
/// [`ModeError::UnknownBackend`] if `cfg.backend` matches no entry in
/// `backend::builtin_registry`, [`ModeError::RecordsFileExists`] if
/// `RECORDS_FILENAME` inside `cfg.output_dir` already contains data,
/// [`ModeError::Session`] if the coordinator fails to bind its socket or
/// spawn the backend binary, and [`ModeError::Io`] if the output
/// directory, records file, or summary file cannot be written.
pub async fn run_throughput_with_summary(
    cfg: &ThroughputConfig,
) -> Result<(RunReport, Summary), ModeError> {
    let spec = cfg.topology.load()?;
    let backend = resolve_backend(cfg)?;

    std::fs::create_dir_all(&cfg.output_dir).map_err(ModeError::Io)?;
    let records_path = cfg.output_dir.join(RECORDS_FILENAME);
    let summary_path = cfg.output_dir.join(SUMMARY_FILENAME);
    prepare_records_file(&records_path)?;

    let params = cfg.param_curve.resolve(backend.kind, cfg.hardness);
    let target = SetTarget {
        num_reads: params.num_reads,
        num_sweeps: params.num_sweeps,
        anneal_time_us: params.anneal_time_us,
        ..SetTarget::default()
    };

    let mut source = ProblemSource::new(&spec, MINER_ACCOUNT, cfg.seed, u32::MAX, NO_DEADLINE_MS);
    let agg = dispatch_until_duration(
        &backend,
        &spec,
        &mut source,
        target,
        target_wall(cfg.duration_s),
        &records_path,
    )
    .await?;

    let summary = build_summary(cfg, &spec, params, &agg);
    write_summary(&summary_path, &summary).map_err(ModeError::Io)?;

    let report = RunReport {
        handshake_ok: agg.all_handshake_ok,
        records: agg.records,
        miner_exit_code: agg.last_exit_code,
        miner_unresponsive: agg.any_unresponsive,
        miner_stderr_tail: agg.last_stderr_tail,
        orphan_results: agg.orphan_results,
        run_wall_ms: agg.total_wall_ms,
    };
    Ok((report, summary))
}

/// Resolve `cfg.backend` against the built-in registry, applying
/// [`ThroughputConfig::backend_bin_override`]/[`ThroughputConfig::backend_args_override`]
/// if set.
fn resolve_backend(cfg: &ThroughputConfig) -> Result<BackendSpec, ModeError> {
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

/// Guard against silently appending into a previous run's records: refuse
/// to start if `records_path` already contains data, and otherwise
/// (re)create it empty so a run that completes zero jobs still leaves a
/// real, empty records file behind rather than none at all.
///
/// # Errors
///
/// Returns [`ModeError::RecordsFileExists`] if `records_path` exists and is
/// non-empty, or [`ModeError::Io`] if it cannot be (re)created.
fn prepare_records_file(records_path: &Path) -> Result<(), ModeError> {
    if std::fs::metadata(records_path).is_ok_and(|m| m.len() > 0) {
        return Err(ModeError::RecordsFileExists(records_path.to_path_buf()));
    }
    std::fs::File::create(records_path).map_err(ModeError::Io)?;
    Ok(())
}

/// Convert a duration in seconds to a [`Duration`], degrading a negative or
/// non-finite value to zero (already elapsed) rather than panicking:
/// `Duration::from_secs_f64` panics on either. CLI-level validation (see
/// `cli::parse_hardness`'s sibling `--duration` parser) is expected to
/// reject those before they ever reach this library function, but this
/// stays total for callers that construct a [`ThroughputConfig`] directly.
fn target_wall(duration_s: f64) -> Duration {
    if duration_s.is_finite() {
        Duration::from_secs_f64(duration_s.max(0.0))
    } else {
        Duration::ZERO
    }
}

/// Accumulated outcome of [`dispatch_until_duration`]'s repeated sessions.
struct Aggregate {
    records: Vec<JobRecord>,
    total_wall_ms: u64,
    /// True only when **every** completed batch's handshake succeeded.
    /// A later failed handshake must not be masked by an earlier success
    /// (unlike the prior OR-combine, which reported success if any batch
    /// shook hands). Unresponsive is still OR-ed: any unresponsive batch
    /// surfaces as unresponsive for the run.
    all_handshake_ok: bool,
    any_unresponsive: bool,
    orphan_results: u64,
    last_exit_code: i32,
    last_stderr_tail: Option<String>,
    batches_run: u32,
}

/// Repeatedly draw a [`BATCH_SIZE`]-job batch from `source` and run a
/// session against `backend`, appending every completed record to
/// `records_path` as its batch completes, until the summed measured wall
/// time of every session reaches `target_wall`. Stops early if a session
/// never completes its handshake, since retrying a backend that cannot
/// even start would otherwise busy-loop for the rest of the requested
/// duration.
async fn dispatch_until_duration(
    backend: &BackendSpec,
    spec: &TopologySpec,
    source: &mut ProblemSource,
    target: SetTarget,
    target_wall: Duration,
    records_path: &Path,
) -> Result<Aggregate, ModeError> {
    let mut agg = Aggregate {
        records: Vec::new(),
        total_wall_ms: 0,
        // Vacuous true until the first batch; zero batches leave this false
        // via `batches_run == 0` below.
        all_handshake_ok: true,
        any_unresponsive: false,
        orphan_results: 0,
        last_exit_code: 0,
        last_stderr_tail: None,
        batches_run: 0,
    };
    let mut total_wall = Duration::ZERO;

    while total_wall < target_wall {
        let mut batch = Vec::with_capacity(BATCH_SIZE as usize);
        for _ in 0..BATCH_SIZE {
            let Some(job) = source.next_job() else {
                break;
            };
            batch.push(job);
        }
        if batch.is_empty() {
            // `source` is constructed with `count: u32::MAX`, so this is
            // unreachable in practice; kept as an explicit exit rather than
            // looping forever on an empty batch.
            break;
        }

        let sock_path = unique_sock_path();
        let report = run_session(RunParams {
            backend,
            spec,
            jobs: batch,
            sock_path: &sock_path,
            target: Some(target),
        })
        .await?;

        for record in &report.records {
            append_jsonl(records_path, record).map_err(ModeError::Io)?;
        }

        // AND across batches: a later failed handshake must not be masked.
        agg.all_handshake_ok &= report.handshake_ok;
        agg.any_unresponsive |= report.miner_unresponsive;
        agg.orphan_results = agg.orphan_results.saturating_add(report.orphan_results);
        agg.last_exit_code = report.miner_exit_code;
        if report.miner_stderr_tail.is_some() {
            agg.last_stderr_tail = report.miner_stderr_tail;
        }
        total_wall += Duration::from_millis(report.run_wall_ms);
        let handshake_ok = report.handshake_ok;
        agg.records.extend(report.records);
        agg.batches_run = agg.batches_run.saturating_add(1);

        if !handshake_ok {
            break;
        }
    }

    // No batch ran → handshake did not succeed.
    if agg.batches_run == 0 {
        agg.all_handshake_ok = false;
    }
    agg.total_wall_ms = duration_ms(total_wall);
    Ok(agg)
}

/// Build the closing [`Summary`] from a completed [`Aggregate`].
fn build_summary(
    cfg: &ThroughputConfig,
    spec: &TopologySpec,
    params: JobParams,
    agg: &Aggregate,
) -> Summary {
    let totals = Totals {
        jobs: agg.records.len(),
        passed: agg.records.iter().filter(|r| r.passed).count(),
        rejected: agg.records.iter().filter(|r| r.rejected).count(),
        // `prepare_records_file` refuses to start against a records file
        // that already contains data, and this mode is the exclusive
        // writer of it for the rest of this call — every record it wrote
        // came from a value already held in `agg.records`, so there is no
        // scenario here in which a line could fail to parse.
        skipped_lines: 0,
    };

    let config = serde_json::json!({
        "backend": cfg.backend,
        "topology": cfg.topology.label(),
        "hardness": cfg.hardness,
        "duration_s": cfg.duration_s,
        "measured_wall_ms": agg.total_wall_ms,
        "seed": cfg.seed,
        "num_reads": params.num_reads,
        "num_sweeps": params.num_sweeps,
        "anneal_time_us": params.anneal_time_us,
        "jobs_per_min": jobs_per_min(agg.records.len(), agg.total_wall_ms),
        "miner_exit_code": agg.last_exit_code,
        "miner_unresponsive": agg.any_unresponsive,
        "miner_stderr_tail": agg.last_stderr_tail.clone(),
        "orphan_results": agg.orphan_results,
    });

    Summary {
        mode: "throughput".to_string(),
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

/// Jobs completed per minute of measured wall time, `0.0` if none elapsed
/// (rather than dividing by zero).
#[expect(
    clippy::cast_precision_loss,
    reason = "job counts in a single benchmark run are far below f64's 52-bit exact-integer range"
)]
fn jobs_per_min(jobs: usize, wall_ms: u64) -> f64 {
    if wall_ms == 0 {
        0.0
    } else {
        jobs as f64 / (wall_ms as f64 / 60_000.0)
    }
}

/// Convert a `Duration` to milliseconds, saturating at `u64::MAX` rather
/// than panicking on an unrealistic overflow. Mirrors
/// `backend::session`'s own private helper of the same name/behavior.
fn duration_ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// A process-wide counter backing [`unique_sock_path`], so concurrently
/// running `#[tokio::test]` functions in the same test binary (same PID)
/// never collide on the same coordinator socket path.
static SOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build a coordinator socket path under the system temp directory, unique
/// across every call in this process (including concurrent ones). Sockets
/// live outside `output_dir`: Unix socket paths have a short OS-enforced
/// length limit, and `output_dir` is user-chosen and could easily exceed
/// it.
fn unique_sock_path() -> PathBuf {
    let n = SOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("isingmark-{}-{n}.sock", std::process::id()))
}

/// Parse a duration string like `"30s"`, `"5m"`, or `"2h"` into seconds.
///
/// # Errors
///
/// Returns [`ParseError`] if `s` is empty, does not end in `s`/`m`/`h`, or
/// its numeric part does not parse as a finite, non-negative `f64`.
pub fn parse_duration(s: &str) -> Result<f64, ParseError> {
    let trimmed = s.trim();
    // Split on the last *character*, not the last byte: `str::split_at`
    // panics if the index it is given falls inside a multi-byte character
    // rather than on a char boundary, which `trimmed.len() - 1` can do for
    // any non-ASCII final character (e.g. `"5µ"`, where `µ` is 2 bytes).
    // `char::len_utf8()` gives the exact byte width of the last character,
    // so subtracting it from `trimmed.len()` always lands on a boundary.
    let Some(unit) = trimmed.chars().next_back() else {
        return Err(ParseError("duration must not be empty".to_string()));
    };
    let num_part = &trimmed[..trimmed.len() - unit.len_utf8()];
    let multiplier = match unit {
        's' => 1.0,
        'm' => 60.0,
        'h' => 3600.0,
        _ => {
            return Err(ParseError(format!(
                "duration {trimmed:?} must end in 's', 'm', or 'h'"
            )))
        }
    };
    let value: f64 = num_part.parse().map_err(|_| {
        ParseError(format!(
            "duration {trimmed:?} has an invalid numeric part {num_part:?}"
        ))
    })?;
    if !value.is_finite() || value < 0.0 {
        return Err(ParseError(format!(
            "duration {trimmed:?} must be a finite, non-negative number"
        )));
    }
    Ok(value * multiplier)
}

#[cfg(test)]
mod tests {
    use super::{
        jobs_per_min, parse_duration, prepare_records_file, unique_sock_path, ModeError,
        DEFAULT_PARAM_CURVE, RECORDS_FILENAME,
    };
    use crate::params::BackendKind;

    #[test]
    fn parse_duration_accepts_seconds_minutes_and_hours() {
        assert_eq!(parse_duration("30s"), Ok(30.0));
        assert_eq!(parse_duration("5m"), Ok(300.0));
        assert_eq!(parse_duration("2h"), Ok(7200.0));
    }

    #[test]
    fn parse_duration_accepts_fractional_values() {
        assert_eq!(parse_duration("1.5s"), Ok(1.5));
    }

    #[test]
    fn parse_duration_rejects_an_unknown_unit_suffix() {
        assert!(parse_duration("5x").is_err());
    }

    #[test]
    fn parse_duration_rejects_an_empty_string() {
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn parse_duration_rejects_a_bare_unit_with_no_number() {
        assert!(parse_duration("s").is_err());
    }

    #[test]
    fn parse_duration_rejects_negative_and_non_finite_values() {
        assert!(parse_duration("-1s").is_err());
        assert!(parse_duration("nans").is_err());
        assert!(parse_duration("infs").is_err());
    }

    /// Regression test for a panic: `"5µ"`'s last character (`µ`) is 2
    /// bytes wide, so a byte-index split at `len() - 1` lands inside it
    /// rather than on a char boundary, which `str::split_at` panics on.
    /// This must return an error, not crash the process.
    #[test]
    fn parse_duration_rejects_a_multi_byte_suffix_instead_of_panicking() {
        assert!(parse_duration("5µ").is_err());
        assert!(parse_duration("µ").is_err());
    }

    /// A multi-byte character *inside* the numeric part (not just as the
    /// final byte) must also fail to parse rather than panic, covering a
    /// pasted non-breaking space or similar before an otherwise-valid unit.
    #[test]
    fn parse_duration_rejects_a_multi_byte_character_in_the_numeric_part() {
        assert!(parse_duration("30\u{a0}s").is_err());
    }

    #[test]
    fn jobs_per_min_is_zero_when_no_wall_time_elapsed() {
        assert!((jobs_per_min(5, 0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn jobs_per_min_scales_jobs_over_measured_minutes() {
        // 30 jobs in 30_000 ms (0.5 min) = 60 jobs/min.
        assert!((jobs_per_min(30, 30_000) - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn unique_sock_path_never_repeats_across_calls() {
        let a = unique_sock_path();
        let b = unique_sock_path();
        assert_ne!(a, b);
    }

    /// Pins the exact resolved values at a known hardness, so a change to
    /// `DEFAULT_PARAM_CURVE`'s bounds (or a bug that stops `--hardness`
    /// from reaching `resolve` at all) is caught here independently of the
    /// slower integration-level check in `tests/mode_throughput.rs`.
    #[test]
    fn default_param_curve_resolves_known_values_at_half_hardness() {
        let params = DEFAULT_PARAM_CURVE.resolve(BackendKind::Sweeping, 0.5);
        assert_eq!(params.num_reads, 36);
        assert_eq!(params.num_sweeps, 550);
    }

    #[test]
    fn prepare_records_file_errors_when_the_file_already_has_data() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join(RECORDS_FILENAME);
        std::fs::write(&path, "{\"stale\":true}\n").expect("seed a stale records file");

        let err = prepare_records_file(&path).expect_err("must refuse to reuse a non-empty file");
        assert!(matches!(err, ModeError::RecordsFileExists(p) if p == path));
    }

    #[test]
    fn prepare_records_file_creates_an_empty_file_when_absent() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join(RECORDS_FILENAME);

        prepare_records_file(&path).expect("no prior file: must succeed");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file was created"),
            ""
        );
    }

    #[test]
    fn prepare_records_file_accepts_an_existing_but_empty_file() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join(RECORDS_FILENAME);
        std::fs::write(&path, "").expect("seed an empty file");

        prepare_records_file(&path).expect("an empty prior file is not a conflict");
    }
}
