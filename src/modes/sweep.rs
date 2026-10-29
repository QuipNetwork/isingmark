// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sweep mode: run every point ("cell") of a `--param-grid` Cartesian
//! product against the identical set of drawn problems, so a difference
//! between cells is attributable to the swept parameters rather than to
//! which problems each cell happened to draw (a paired design).
//!
//! # Per-job parameters, not `SetTarget`
//!
//! `throughput` (Task 8) conveys its sample parameters via a session-level
//! `SetTarget`, sent once right after the handshake. A sweep instead sets
//! `num_reads`/`num_sweeps`/`anneal_time_us` directly on each dispatched
//! job's own `IsingProblem` fields (`problem::ProblemSource::next_job`
//! otherwise always leaves these `0`, i.e. "unset" — see that module's
//! docs). Which fields apply depends on the resolved backend's
//! [`crate::params::BackendKind`]: `Sweeping` backends use
//! `num_reads`/`num_sweeps` and leave `anneal_time_us` at `0`; `Annealing`
//! backends use `num_reads`/`anneal_time_us` and leave `num_sweeps` at `0`.
//! A grid that sets the *inapplicable* dimension for the resolved backend is
//! rejected at the start of [`run_sweep`] ([`ModeError::InapplicableGridDimension`])
//! rather than silently ignored — silently dropping a sweep axis would give
//! back a result set that looks complete but measured fewer dimensions than
//! requested, with no signal that anything was left out.
//!
//! # Output layout: one file per cell, all cells over the same problems
//!
//! `Job.job_id` is `nonce.to_vec()`, derived only from `(seed, index)`
//! (`problem::ProblemSource`), never from job parameters. Every cell in a
//! sweep is built from a *fresh* `ProblemSource` constructed with the same
//! `(seed, num_jobs)`, so every cell draws the byte-identical sequence of
//! `job_id`s (the paired design). Each cell's records are written to their
//! own `records.jsonl` inside a per-cell subdirectory (see
//! [`cell_dir_name`]), rather than one shared file, so a reader never sees
//! two same-`job_id` records competing for one line.
//!
//! # Why this runs one `run_session` call per cell, not one for the whole grid
//!
//! Sending every cell's jobs through a single, shared
//! `backend::run_session` call would seem to save a subprocess spawn per
//! cell, but it would break per-cell attribution. The paired design makes
//! `job_id` byte-identical across cells (same seed and index; parameters
//! never enter the id). With one shared session, every cell's terminal
//! records land in a single `RunReport.records` list, and
//! [`crate::record::JobRecord`] carries no cell identifier — only
//! `job_id`, which is the same for the corresponding problem in every
//! cell. The session would keep every record (`backend::session` tracks
//! in-flight work as a per-id FIFO queue, so duplicate ids do not
//! overwrite each other), but nothing in the record would say which grid
//! cell produced it. Per-cell sessions remain the boundary that keeps
//! per-cell attribution possible: each cell's records are written under
//! that cell's own subdirectory (see [`cell_dir_name`]) after that cell's
//! `run_session` completes.
//!
//! One consequence: the backend subprocess is still spawned fresh per
//! cell, the same cost `throughput` pays per batch — not to change a
//! session-level `SetTarget` (parameters ride the per-job wire fields
//! instead), but as the attribution boundary described above.
//!
//! # A known gap: `anneal_time_us` cannot be verified from a `JobRecord`
//!
//! [`crate::record::JobRecord`] carries three `requested_*` fields
//! (`requested_reads`, `requested_sweeps`, `requested_anneal_time_us`)
//! filled from the session-level `SetTarget` when one was sent, and
//! `measured_reads`/`measured_sweeps` filled from the miner's
//! `SamplerMeta` when present. This mode passes `target: None` to every
//! session and puts cell parameters on each job's own `IsingProblem`
//! fields instead, so a reader of a sweep's JSONL can and cannot verify
//! the following:
//!
//! - `measured_reads` / `measured_sweeps` carry the cell's actual values
//!   for a backend that reports `SamplerMeta`, so a `Sweeping` sweep's
//!   parameters are verifiable per cell from its output.
//! - All three `requested_*` fields are `None` for every sweep record,
//!   because no `SetTarget` was sent. `None` means "no target was sent",
//!   not "zero was requested".
//! - There is no `measured_anneal_time_us` — `SamplerMeta` does not carry
//!   one — so an `Annealing` sweep's per-cell `anneal_time_us` cannot be
//!   verified from a `JobRecord` at all. This is the residual gap.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Deserialize;

use crate::backend::{builtin_registry, run_session, BackendSpec, RunParams, RunReport};
use crate::modes::{hex_encode, load_finite_source, JobSource, ModeError, ParseError, TopologyRef};
use crate::params::BackendKind;
use crate::problem::ProblemSource;
use crate::record::{append_jsonl, write_summary, HardwareInfo, Summary, TopologyInfo, Totals};
use crate::stats::{describe, Stats};
use crate::topology::TopologySpec;

/// Basename of a sweep cell's JSONL record file within its own
/// subdirectory of [`SweepConfig::output_dir`] (see [`cell_dir_name`]).
pub const RECORDS_FILENAME: &str = "records.jsonl";
/// Basename of the sweep's closing grid summary within
/// [`SweepConfig::output_dir`].
pub const SUMMARY_FILENAME: &str = "summary.json";

/// Miner account every cell derives jobs against. Fixed for the same reason
/// `throughput` fixes its own copy of this constant: it only affects the
/// derived nonce, and every cell must derive against the same value for the
/// paired design (same `(seed, index)` -> same job) to hold.
const MINER_ACCOUNT: [u8; 32] = [0u8; 32];

/// No wire deadline: like `throughput`, this mode measures its own
/// completion and does not need the miner to expire jobs on its own clock.
const NO_DEADLINE_MS: u64 = 0;

/// A parameter-sweep grid: the Cartesian dimensions `--param-grid` sets.
/// Which two of the three fields are used depends on the resolved
/// backend's [`BackendKind`] (see the module docs); the unused one for a
/// given backend must be left empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ParamGrid {
    /// Reads to sweep. Used by every backend kind.
    pub num_reads: Vec<u32>,
    /// Sweeps to sweep. Used only by [`BackendKind::Sweeping`] backends.
    pub num_sweeps: Vec<u32>,
    /// Anneal times (microseconds) to sweep. Used only by
    /// [`BackendKind::Annealing`] backends.
    pub anneal_time_us: Vec<u32>,
}

/// Parse a `--param-grid` JSON string into a [`ParamGrid`].
///
/// # Errors
///
/// Returns [`ParseError`] if `json` does not parse, contains an unknown
/// field, or any dimension contains a `0` — `0` means "unset" on the wire
/// (see `params::ParamCurve`'s own documented 0-means-unset contract), so a
/// grid cell that resolved to `0` would silently defer to the miner's own
/// default instead of running the value the grid actually requested, with
/// no error and a misleading label on that cell's output directory. An
/// empty dimension (`[]`) is *not* rejected here: whether that is valid
/// depends on which dimensions the resolved backend actually uses, which
/// this function does not know — see [`build_cells`]'s
/// [`ModeError::EmptyGrid`].
pub fn parse_param_grid(json: &str) -> Result<ParamGrid, ParseError> {
    let grid: ParamGrid =
        serde_json::from_str(json).map_err(|e| ParseError(format!("invalid --param-grid: {e}")))?;
    for (name, values) in [
        ("num_reads", &grid.num_reads),
        ("num_sweeps", &grid.num_sweeps),
        ("anneal_time_us", &grid.anneal_time_us),
    ] {
        if values.contains(&0) {
            return Err(ParseError(format!(
                "--param-grid {name} must not contain 0 (0 means \"unset\" on the wire, which \
                 would silently fall back to the miner's own default instead of running the \
                 requested value); got {values:?}"
            )));
        }
    }
    Ok(grid)
}

/// Configuration for [`run_sweep`].
#[derive(Debug, Clone)]
pub struct SweepConfig {
    /// `backend::BackendSpec::name` to benchmark (e.g. `"cpu-sa"`).
    pub backend: String,
    /// Topology to draw jobs against. Shared by every cell.
    pub topology: TopologyRef,
    /// The parameter grid to sweep.
    pub grid: ParamGrid,
    /// Jobs drawn per cell. Every cell draws the identical `num_jobs`-job
    /// sequence (same seed, same count) — see the module docs on the
    /// paired design this establishes.
    pub num_jobs: u32,
    /// Seed for deterministic problem generation, shared by every cell.
    /// Ignored when [`Self::corpus`] is set, because a corpus names its
    /// problems rather than deriving them.
    pub seed: u64,
    /// Replay a corpus of chain-harvested instances instead of deriving
    /// problems from [`Self::seed`]. A bundled corpus name
    /// (`corpus::bundled_names`) or a path to a JSONL file; a bundled name
    /// must be paired with the topology it was drawn against.
    ///
    /// A corpus is finite, so it caps [`Self::num_jobs`]: a cell draws
    /// `min(num_jobs, corpus length)` problems. Every cell reloads the
    /// corpus from the start, so the paired design holds exactly as it does
    /// for a derived source.
    pub corpus: Option<String>,
    /// Replay explicit instances from a JSONL file instead of deriving
    /// problems from [`Self::seed`]. Mutually exclusive with
    /// [`Self::corpus`]; setting both fails with
    /// [`ModeError::ConflictingSources`].
    ///
    /// The file's header must carry the `topology_hash` of the run's
    /// topology (see [`crate::instances`]). The file is finite, so it caps
    /// [`Self::num_jobs`] exactly as a corpus does, and every cell reloads it
    /// from the start.
    pub instances: Option<PathBuf>,
    /// Directory to write one subdirectory per cell into, plus a top-level
    /// [`SUMMARY_FILENAME`]. Created if it does not already exist. Refuses
    /// to start (see [`ModeError::RecordsFileExists`]) if *any* cell's
    /// [`RECORDS_FILENAME`] already contains data from an earlier run —
    /// checked for every cell before any of them run, so a conflict on a
    /// later cell cannot leave earlier cells' output half-written.
    pub output_dir: PathBuf,
    /// Explicit backend binary path, bypassing `$PATH` resolution. See
    /// `throughput::ThroughputConfig::backend_bin_override`.
    pub backend_bin_override: Option<PathBuf>,
    /// Explicit backend CLI args, replacing `backend::BackendSpec::args`.
    /// See `throughput::ThroughputConfig::backend_args_override`.
    pub backend_args_override: Option<Vec<String>>,
}

/// One dispatched-and-completed cell of a sweep's grid.
///
/// Does not derive `Debug`: `backend::RunReport` (Task 7) does not
/// implement it, and this task does not modify already-reviewed code to
/// add a derive elsewhere.
pub struct CellOutcome {
    /// This cell's subdirectory name under `SweepConfig::output_dir` (see
    /// [`cell_dir_name`]).
    pub dir_name: String,
    /// Reads this cell requested on every job.
    pub num_reads: u32,
    /// Sweeps this cell requested on every job (`0` for `Annealing`
    /// backends).
    pub num_sweeps: u32,
    /// Anneal time (microseconds) this cell requested on every job (`0`
    /// for `Sweeping` backends).
    pub anneal_time_us: u32,
    /// This cell's own session outcome.
    pub report: RunReport,
}

/// Outcome of [`run_sweep`].
pub struct SweepReport {
    /// One entry per grid cell, in Cartesian-product generation order.
    pub cells: Vec<CellOutcome>,
    /// The grid summary also written to [`SUMMARY_FILENAME`], returned here
    /// too so a caller (namely `main.rs`) does not have to read that file
    /// back off disk and re-parse it.
    pub summary: Summary,
}

/// Run sweep mode: for every cell in `cfg.grid`'s Cartesian product, draw
/// `cfg.num_jobs` deterministic jobs from the identical `(cfg.seed,
/// cfg.num_jobs)` draw, set the cell's parameters directly on each job's
/// `IsingProblem` fields, and dispatch them in their own session against
/// `cfg.backend`. Writes each cell's records to its own
/// `<cell dir>/records.jsonl` as its session completes, and a closing
/// [`SUMMARY_FILENAME`] once every cell is done.
///
/// # Errors
///
/// Returns [`ModeError::Topology`] if `cfg.topology` fails to load,
/// [`ModeError::UnknownBackend`] if `cfg.backend` matches no entry in
/// `backend::builtin_registry`, [`ModeError::InapplicableGridDimension`] if
/// `cfg.grid` sets a dimension the resolved backend's
/// `params::BackendKind` does not use, [`ModeError::EmptyGrid`] if the
/// applicable dimensions' Cartesian product has zero cells,
/// [`ModeError::RecordsFileExists`] if any cell's records file already
/// contains data, [`ModeError::Session`] if a cell's session fails to bind
/// or spawn, and [`ModeError::Io`] if the output directory or any file
/// cannot be written.
pub async fn run_sweep(cfg: &SweepConfig) -> Result<SweepReport, ModeError> {
    let spec = cfg.topology.load()?;
    // The miner opens its device before any job arrives, so the kernel
    // capacity must be on its argv, derived from the topology we are about
    // to run.
    let backend = resolve_backend(cfg)?.with_node_capacity(spec.nodes.len());
    let cells = build_cells(&cfg.grid, backend.kind, &cfg.backend)?;

    std::fs::create_dir_all(&cfg.output_dir).map_err(ModeError::Io)?;

    let cell_paths: Vec<(PathBuf, PathBuf, String)> = cells
        .iter()
        .map(|cell| {
            let dir_name = cell_dir_name(cell, backend.kind);
            let dir = cfg.output_dir.join(&dir_name);
            let records_path = dir.join(RECORDS_FILENAME);
            (dir, records_path, dir_name)
        })
        .collect();

    // Refuse to start if *any* cell already has data, before creating or
    // writing anything: a conflict discovered partway through would
    // otherwise leave earlier cells' output sitting next to a run that
    // never finished, with nothing on disk distinguishing it from a
    // complete sweep.
    for (_, records_path, _) in &cell_paths {
        if std::fs::metadata(records_path).is_ok_and(|m| m.len() > 0) {
            return Err(ModeError::RecordsFileExists(records_path.clone()));
        }
    }

    let mut outcomes = Vec::with_capacity(cells.len());
    for (cell, (dir, records_path, dir_name)) in cells.iter().zip(&cell_paths) {
        std::fs::create_dir_all(dir).map_err(ModeError::Io)?;
        std::fs::File::create(records_path).map_err(ModeError::Io)?;

        let jobs = cell.draw_jobs(
            &spec,
            cfg.seed,
            cfg.num_jobs,
            cfg.corpus.as_deref(),
            cfg.instances.as_deref(),
        )?;

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

        outcomes.push(CellOutcome {
            dir_name: dir_name.clone(),
            num_reads: cell.num_reads,
            num_sweeps: cell.num_sweeps,
            anneal_time_us: cell.anneal_time_us,
            report,
        });
    }

    let summary = build_summary(cfg, &spec, &outcomes);
    write_summary(&cfg.output_dir.join(SUMMARY_FILENAME), &summary).map_err(ModeError::Io)?;

    Ok(SweepReport {
        cells: outcomes,
        summary,
    })
}

/// Resolve `cfg.backend` against the built-in registry, applying
/// `backend_bin_override`/`backend_args_override` if set. Mirrors
/// `throughput::resolve_backend` exactly; kept as its own copy rather than
/// shared, matching this crate's existing per-mode-module convention for
/// small helpers like this one.
fn resolve_backend(cfg: &SweepConfig) -> Result<BackendSpec, ModeError> {
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

/// One point in a [`ParamGrid`]'s Cartesian product, resolved to concrete
/// per-job sample parameters and its generation index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cell {
    /// Zero-based index into the Cartesian product, in generation order
    /// (outer loop over `num_reads`, inner loop over the backend's second
    /// applicable dimension). Used to keep [`cell_dir_name`] unique even if
    /// a grid repeats a value within one dimension.
    index: usize,
    num_reads: u32,
    num_sweeps: u32,
    anneal_time_us: u32,
}

impl Cell {
    /// Draw `count` deterministic jobs from a *fresh* [`ProblemSource`]
    /// built from `(spec, seed, count)`, setting this cell's parameters
    /// directly on each job's `IsingProblem` fields.
    ///
    /// A fresh source per call (rather than one shared across cells) is the
    /// paired design's core mechanism: the same `(seed, count)` always
    /// yields the same job sequence (`ProblemSource`'s own documented
    /// contract), so every cell draws byte-identical jobs regardless of
    /// which cell ran first.
    fn draw_jobs(
        &self,
        spec: &TopologySpec,
        seed: u64,
        count: u32,
        corpus: Option<&str>,
        instances: Option<&Path>,
    ) -> Result<Vec<quip_proto::v1::Job>, ModeError> {
        let mut source = load_finite_source(corpus, instances, spec)?.unwrap_or_else(|| {
            JobSource::Derived(Box::new(ProblemSource::new(
                spec,
                MINER_ACCOUNT,
                seed,
                count,
                NO_DEADLINE_MS,
            )))
        });
        let mut jobs = Vec::with_capacity(count as usize);
        while jobs.len() < count as usize {
            let Some(mut job) = source.next_job() else {
                break;
            };
            if let Some(ising) = job.ising.as_mut() {
                ising.num_reads = self.num_reads;
                ising.num_sweeps = self.num_sweeps;
                ising.anneal_time_us = self.anneal_time_us;
            }
            jobs.push(job);
        }
        Ok(jobs)
    }
}

/// Build every cell of `grid`'s Cartesian product for `kind`'s two
/// applicable dimensions (`num_reads` plus `num_sweeps` for
/// [`BackendKind::Sweeping`], or `num_reads` plus `anneal_time_us` for
/// [`BackendKind::Annealing`]), in outer-`num_reads`/inner-second-dimension
/// order.
///
/// # Errors
///
/// Returns [`ModeError::InapplicableGridDimension`] if `grid` sets the
/// dimension `kind` does not use, or [`ModeError::EmptyGrid`] if either
/// applicable dimension is empty (so the product has zero points).
fn build_cells(
    grid: &ParamGrid,
    kind: BackendKind,
    backend_name: &str,
) -> Result<Vec<Cell>, ModeError> {
    let second_dim = match kind {
        BackendKind::Sweeping => {
            if !grid.anneal_time_us.is_empty() {
                return Err(ModeError::InapplicableGridDimension(format!(
                    "backend {backend_name:?} is a Sweeping backend (num_reads + num_sweeps); \
                     anneal_time_us does not apply to it and must be left out of --param-grid"
                )));
            }
            &grid.num_sweeps
        }
        BackendKind::Annealing => {
            if !grid.num_sweeps.is_empty() {
                return Err(ModeError::InapplicableGridDimension(format!(
                    "backend {backend_name:?} is an Annealing backend (num_reads + \
                     anneal_time_us); num_sweeps does not apply to it and must be left out of \
                     --param-grid"
                )));
            }
            &grid.anneal_time_us
        }
    };

    if grid.num_reads.is_empty() || second_dim.is_empty() {
        return Err(ModeError::EmptyGrid);
    }

    let mut cells = Vec::with_capacity(grid.num_reads.len() * second_dim.len());
    for &num_reads in &grid.num_reads {
        for &second_value in second_dim {
            let (num_sweeps, anneal_time_us) = match kind {
                BackendKind::Sweeping => (second_value, 0),
                BackendKind::Annealing => (0, second_value),
            };
            let index = cells.len();
            cells.push(Cell {
                index,
                num_reads,
                num_sweeps,
                anneal_time_us,
            });
        }
    }
    Ok(cells)
}

/// This cell's subdirectory name: `cell_<index>_r<num_reads>_s<num_sweeps>`
/// for a `Sweeping` backend, `cell_<index>_r<num_reads>_a<anneal_time_us>`
/// for an `Annealing` one — deterministic (a rerun of the same grid
/// against the same backend kind reproduces the same names) and readable
/// (a directory listing sorts cells by generation order and shows every
/// cell's own parameters without opening it).
fn cell_dir_name(cell: &Cell, kind: BackendKind) -> String {
    match kind {
        BackendKind::Sweeping => format!(
            "cell_{}_r{}_s{}",
            cell.index, cell.num_reads, cell.num_sweeps
        ),
        BackendKind::Annealing => {
            format!(
                "cell_{}_r{}_a{}",
                cell.index, cell.num_reads, cell.anneal_time_us
            )
        }
    }
}

/// Build the closing grid [`Summary`] from every cell's completed
/// [`CellOutcome`]. `totals` aggregates across every cell; `config` carries
/// the requested grid alongside each cell's own directory, parameters, and
/// per-cell stats (via `stats::describe` over that cell's passed jobs'
/// `best_energy_milli` converted to **natural units** (`/ 1000`), and
/// separately over every one of that cell's records' `wall_ms`), plus that
/// cell's own miner failure state — so a cell a backend failed partway
/// through is distinguishable from a complete one without needing the raw
/// `RunReport`.
///
/// The JSON key for energy stats is `best_energy_stats` (natural units), not
/// `best_energy_milli_stats`: the values are already divided by 1000 to match
/// the human-facing table units, and a `_milli` suffix would invite a silent
/// 1000× misread against `JobRecord::best_energy_milli`.
///
/// `session_wall_ms` (one number per cell) and `wall_ms_stats` (a
/// distribution, one entry per record) answer different questions and must
/// not be confused: `session_wall_ms` is `RunReport::run_wall_ms`, measured
/// across the *entire* `run_session` call for that cell — subprocess spawn,
/// handshake, every job's dispatch, and up to `SHUTDOWN_GRACE_MS * 3` of
/// teardown (`backend::session::run_session_inner`) — fixed overhead that
/// has nothing to do with the swept parameters. `wall_ms_stats` is built
/// from each record's own `JobRecord::wall_ms` (dispatch-to-result for that
/// one job), which is what actually varies with `num_reads`/`num_sweeps`/
/// `anneal_time_us`. On a fast backend with a small `num_jobs`, fixed
/// per-session overhead can dwarf the per-job effect a sweep exists to
/// measure, so a cell-to-cell comparison must use `wall_ms_stats`, not
/// `session_wall_ms`.
fn build_summary(cfg: &SweepConfig, spec: &TopologySpec, outcomes: &[CellOutcome]) -> Summary {
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
        // Every cell's records file is created empty and exclusively
        // written by this call, same reasoning as `throughput::build_summary`.
        skipped_lines: 0,
    };

    let cells_json: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|o| {
            let energies: Vec<f64> = o
                .report
                .records
                .iter()
                .filter(|r| r.passed)
                .filter_map(|r| r.best_energy_milli.map(milli_to_f64))
                .collect();
            // Every record's own dispatch-to-result wall time, regardless of
            // pass/fail/reject: unlike `best_energy_milli` (None when a job
            // never produced a solution), `JobRecord::wall_ms` is
            // well-defined for every terminal record.
            let wall_mss: Vec<f64> = o
                .report
                .records
                .iter()
                .map(|r| wall_ms_to_f64(r.wall_ms))
                .collect();
            serde_json::json!({
                "dir": o.dir_name,
                "num_reads": o.num_reads,
                "num_sweeps": o.num_sweeps,
                "anneal_time_us": o.anneal_time_us,
                "jobs": o.report.records.len(),
                "passed": o.report.records.iter().filter(|r| r.passed).count(),
                "rejected": o.report.records.iter().filter(|r| r.rejected).count(),
                "session_wall_ms": o.report.run_wall_ms,
                "miner_exit_code": o.report.miner_exit_code,
                "miner_unresponsive": o.report.miner_unresponsive,
                "miner_stderr_tail": o.report.miner_stderr_tail,
                "orphan_results": o.report.orphan_results,
                // Natural units (best_energy_milli / 1000), not milli — see
                // `build_summary`'s doc.
                "best_energy_stats": stats_json(describe(&energies)),
                "wall_ms_stats": stats_json(describe(&wall_mss)),
            })
        })
        .collect();

    let config = serde_json::json!({
        "backend": cfg.backend,
        "topology": cfg.topology.label(),
        "seed": cfg.seed,
        "corpus": cfg.corpus.clone(),
        "instances": cfg.instances.as_ref().map(|p| p.display().to_string()),
        "num_jobs_per_cell": cfg.num_jobs,
        "grid": {
            "num_reads": cfg.grid.num_reads,
            "num_sweeps": cfg.grid.num_sweeps,
            "anneal_time_us": cfg.grid.anneal_time_us,
        },
        "measured_wall_ms": outcomes.iter().map(|o| o.report.run_wall_ms).sum::<u64>(),
        "cells": cells_json,
    });

    Summary {
        mode: "sweep".to_string(),
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

/// Render a `stats::describe` result as JSON. `stats::Stats` (Task 9) does
/// not derive `Serialize`, and this task does not modify already-reviewed
/// code to add one; `None` (an empty or non-finite sample, neither of which
/// occurs for a cell with any passed record) renders as JSON `null`.
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
/// `stats::describe`.
#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark energies fit comfortably within f64's 52-bit exact-integer range"
)]
fn milli_to_f64(milli: i64) -> f64 {
    milli as f64 / 1000.0
}

/// `JobRecord::wall_ms` (a `u64` count of milliseconds) to `f64`, for
/// feeding `stats::describe`.
#[expect(
    clippy::cast_precision_loss,
    reason = "a single job's wall time in milliseconds fits comfortably within f64's 52-bit \
              exact-integer range"
)]
fn wall_ms_to_f64(wall_ms: u64) -> f64 {
    wall_ms as f64
}

/// A process-wide counter backing [`unique_sock_path`], mirroring
/// `throughput`'s own (separately-scoped) counter of the same purpose. Not
/// shared with `throughput`'s: this module's own `isingmark-sweep-`
/// filename prefix already makes the two namespaces disjoint, so a shared
/// counter would add coupling to already-reviewed code (`throughput.rs`,
/// Task 8) for no correctness benefit.
static SOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Build a coordinator socket path under the system temp directory, unique
/// across every call in this process (including concurrent ones) and
/// disjoint from `throughput`'s own socket paths (distinct filename
/// prefix), so the two modes can never collide even if a future caller
/// somehow drove both from the same process.
fn unique_sock_path() -> PathBuf {
    let n = SOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("isingmark-sweep-{}-{n}.sock", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::{build_cells, cell_dir_name, parse_param_grid, Cell, ParamGrid};
    use crate::modes::ModeError;
    use crate::params::BackendKind;

    fn grid(num_reads: &[u32], num_sweeps: &[u32], anneal_time_us: &[u32]) -> ParamGrid {
        ParamGrid {
            num_reads: num_reads.to_vec(),
            num_sweeps: num_sweeps.to_vec(),
            anneal_time_us: anneal_time_us.to_vec(),
        }
    }

    #[test]
    fn parse_param_grid_parses_a_full_valid_grid() {
        let parsed = parse_param_grid(r#"{"num_reads":[64,128],"num_sweeps":[100,500]}"#)
            .expect("valid grid parses");
        assert_eq!(parsed.num_reads, vec![64, 128]);
        assert_eq!(parsed.num_sweeps, vec![100, 500]);
        assert!(parsed.anneal_time_us.is_empty());
    }

    #[test]
    fn parse_param_grid_rejects_malformed_json() {
        assert!(parse_param_grid("{not json}").is_err());
    }

    #[test]
    fn parse_param_grid_rejects_an_unknown_key() {
        let err = parse_param_grid(r#"{"num_reads":[64],"bogus_field":[1]}"#)
            .expect_err("unknown key must be rejected");
        assert!(err.0.contains("bogus_field") || err.0.to_lowercase().contains("unknown"));
    }

    #[test]
    fn parse_param_grid_accepts_an_empty_dimension() {
        // Whether an empty dimension is valid depends on the resolved
        // backend, which parse_param_grid does not know about — that
        // check happens in `build_cells`. This must still parse.
        let parsed = parse_param_grid(r#"{"num_reads":[64],"num_sweeps":[]}"#).expect("parses");
        assert!(parsed.num_sweeps.is_empty());
    }

    #[test]
    fn parse_param_grid_rejects_a_zero_value_in_any_dimension() {
        assert!(parse_param_grid(r#"{"num_reads":[0,64]}"#).is_err());
        assert!(parse_param_grid(r#"{"num_reads":[64],"num_sweeps":[0]}"#).is_err());
        assert!(parse_param_grid(r#"{"num_reads":[64],"anneal_time_us":[0]}"#).is_err());
    }

    #[test]
    fn parse_param_grid_rejects_a_value_exceeding_u32() {
        assert!(parse_param_grid(r#"{"num_reads":[4294967296]}"#).is_err());
    }

    #[test]
    fn parse_param_grid_rejects_a_negative_value() {
        assert!(parse_param_grid(r#"{"num_reads":[-1]}"#).is_err());
    }

    #[test]
    fn build_cells_computes_the_full_cartesian_product_in_generation_order() {
        let g = grid(&[64, 128], &[100, 500, 900], &[]);
        let cells = build_cells(&g, BackendKind::Sweeping, "cpu-sa").expect("builds");
        assert_eq!(cells.len(), 6);
        let expected: Vec<(u32, u32)> = vec![
            (64, 100),
            (64, 500),
            (64, 900),
            (128, 100),
            (128, 500),
            (128, 900),
        ];
        let actual: Vec<(u32, u32)> = cells.iter().map(|c| (c.num_reads, c.num_sweeps)).collect();
        assert_eq!(actual, expected);
        assert!(cells.iter().all(|c| c.anneal_time_us == 0));
        let indices: Vec<usize> = cells.iter().map(|c| c.index).collect();
        assert_eq!(indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn build_cells_uses_num_reads_and_anneal_time_us_for_an_annealing_backend() {
        let g = grid(&[10, 20], &[], &[5, 50]);
        let cells = build_cells(&g, BackendKind::Annealing, "dwave").expect("builds");
        assert_eq!(cells.len(), 4);
        assert!(cells.iter().all(|c| c.num_sweeps == 0));
        let actual: Vec<(u32, u32)> = cells
            .iter()
            .map(|c| (c.num_reads, c.anneal_time_us))
            .collect();
        assert_eq!(actual, vec![(10, 5), (10, 50), (20, 5), (20, 50)]);
    }

    #[test]
    fn build_cells_rejects_anneal_time_us_for_a_sweeping_backend() {
        let g = grid(&[64], &[100], &[5]);
        let err = build_cells(&g, BackendKind::Sweeping, "cpu-sa")
            .expect_err("anneal_time_us must be rejected for a Sweeping backend");
        assert!(matches!(err, ModeError::InapplicableGridDimension(_)));
    }

    #[test]
    fn build_cells_rejects_num_sweeps_for_an_annealing_backend() {
        let g = grid(&[64], &[100], &[]);
        let err = build_cells(&g, BackendKind::Annealing, "dwave")
            .expect_err("num_sweeps must be rejected for an Annealing backend");
        assert!(matches!(err, ModeError::InapplicableGridDimension(_)));
    }

    #[test]
    fn build_cells_rejects_an_empty_num_reads_dimension() {
        let g = grid(&[], &[100], &[]);
        let err = build_cells(&g, BackendKind::Sweeping, "cpu-sa")
            .expect_err("empty num_reads must collapse the product to nothing");
        assert!(matches!(err, ModeError::EmptyGrid));
    }

    #[test]
    fn build_cells_rejects_an_empty_second_dimension() {
        let g = grid(&[64], &[], &[]);
        let err = build_cells(&g, BackendKind::Sweeping, "cpu-sa")
            .expect_err("empty num_sweeps must collapse the product to nothing");
        assert!(matches!(err, ModeError::EmptyGrid));
    }

    /// Two cells with the same parameters (a grid that repeats a value in
    /// one dimension) must still get distinct directory names — the
    /// generation index disambiguates them.
    #[test]
    fn cell_dir_name_disambiguates_cells_sharing_a_second_dimension_value_via_index() {
        let a = Cell {
            index: 0,
            num_reads: 64,
            num_sweeps: 100,
            anneal_time_us: 0,
        };
        let b = Cell {
            index: 1,
            num_reads: 64,
            num_sweeps: 100,
            anneal_time_us: 0,
        };
        let name_a = cell_dir_name(&a, BackendKind::Sweeping);
        let name_b = cell_dir_name(&b, BackendKind::Sweeping);
        assert_ne!(name_a, name_b);
        assert_eq!(name_a, "cell_0_r64_s100");
        assert_eq!(name_b, "cell_1_r64_s100");
    }

    #[test]
    fn cell_dir_name_for_annealing_uses_anneal_time_us_not_sweeps() {
        let cell = Cell {
            index: 2,
            num_reads: 10,
            num_sweeps: 0,
            anneal_time_us: 30,
        };
        assert_eq!(
            cell_dir_name(&cell, BackendKind::Annealing),
            "cell_2_r10_a30"
        );
    }
}
