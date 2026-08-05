// SPDX-License-Identifier: AGPL-3.0-or-later
//! Command-line argument parsing: the `isingmark` clap [`Parser`], its
//! subcommands, and the options common to every mode.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::backend::builtin_registry;
use crate::modes::comparative::ComparativeConfig;
use crate::modes::reproducibility::ReproConfig;
use crate::modes::scaling::{self, ScalingConfig};
use crate::modes::sweep::{self, ParamGrid, SweepConfig};
use crate::modes::throughput::{self, ThroughputConfig};
use crate::modes::ttt::{self, TttConfig};
use crate::modes::TopologyRef;

/// `isingmark`: an annealer-agnostic Ising model benchmarking tool.
#[derive(Debug, Parser)]
#[command(name = "isingmark", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Benchmark modes.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Dispatch jobs against a backend for a fixed duration and report
    /// throughput.
    Throughput(ThroughputArgs),
    /// Run every cell of a `--param-grid` Cartesian product against the
    /// identical set of drawn problems.
    Sweep(SweepArgs),
    /// For each `--energy-targets` value, run trials until it is reached or
    /// `--max-time-s` elapses, and report time-to-target.
    Ttt(TttArgs),
    /// Search hardness in `[0.0, 1.0]` for the best-energy distribution
    /// that best matches a reference `--target-file`'s own distribution.
    Comparative(ComparativeArgs),
    /// Benchmark a fixed hardness against a random 3-regular topology at
    /// each of several requested node counts.
    Scaling(ScalingArgs),
    /// Run two batches of jobs against the identical problem sequence and
    /// report whether their best-energy distributions are statistically
    /// distinguishable.
    Reproducibility(ReproducibilityArgs),
}

/// Options common to every mode: which backend and topology to use, how
/// hard to drive it, and where to write results.
#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Backend to benchmark. See `backend::builtin_registry` for the fixed
    /// set of names this maps to.
    #[arg(long, value_parser = parse_backend)]
    pub backend: String,
    /// Exactly one of `--topology-preset`/`--topology-file` is required.
    #[command(flatten)]
    pub topology_source: TopologySource,
    /// Difficulty in `[0.0, 1.0]`; interpolates between a backend's min and
    /// max sample parameters.
    #[arg(long, value_parser = parse_hardness, default_value_t = 0.5)]
    pub hardness: f64,
    /// Seed for deterministic problem generation.
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Directory to write JSONL records and the run summary to. Created if
    /// it does not already exist.
    #[arg(long, default_value = "./out")]
    pub output_dir: PathBuf,
    /// YAML config file. Flags override values from this file. The flag
    /// exists for forward compatibility with a later config-loading task,
    /// but loading is not implemented yet: every mode's `into_config`
    /// rejects this with an explicit error rather than silently ignoring
    /// it, since a run at hardcoded defaults with no signal the file was
    /// never read is worse than refusing to start.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Increase output verbosity. Repeatable. Accepted for forward
    /// compatibility; not yet wired to any logging behavior.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbosity: u8,
}

/// Exactly one topology source is required: a bundled fixture name, or a
/// spec file path.
#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct TopologySource {
    /// Bundled fixture name (e.g. `"smoke"`).
    #[arg(long)]
    pub topology_preset: Option<String>,
    /// Path to a topology-spec JSON file.
    #[arg(long)]
    pub topology_file: Option<PathBuf>,
}

impl TopologySource {
    /// Resolve to a [`TopologyRef`].
    ///
    /// Clap's `required(true), multiple(false)` group on this struct
    /// already prevents both fields being `None` or both `Some` when
    /// parsed from `argv`, but this stays a total, error-returning
    /// function (rather than trusting that invariant with an
    /// `unreachable!()`) since nothing stops a caller from constructing
    /// this struct directly, bypassing clap entirely.
    ///
    /// # Errors
    ///
    /// Returns a message naming the problem if both or neither field is
    /// set.
    pub fn to_ref(&self) -> Result<TopologyRef, String> {
        match (&self.topology_preset, &self.topology_file) {
            (Some(name), None) => Ok(TopologyRef::Preset(name.clone())),
            (None, Some(path)) => Ok(TopologyRef::File(path.clone())),
            (Some(_), Some(_)) => {
                Err("--topology-preset and --topology-file are mutually exclusive".to_string())
            }
            (None, None) => {
                Err("one of --topology-preset or --topology-file is required".to_string())
            }
        }
    }
}

/// `isingmark throughput` arguments.
#[derive(Debug, Args)]
pub struct ThroughputArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// How long to keep dispatching jobs, e.g. `"30s"`, `"5m"`, `"2h"`.
    #[arg(long, value_parser = parse_duration_arg)]
    pub duration: f64,
}

impl ThroughputArgs {
    /// Build a [`ThroughputConfig`] from parsed arguments.
    ///
    /// # Errors
    ///
    /// Returns a message if the mutually exclusive topology group was
    /// somehow left unresolved (see [`TopologySource::to_ref`]), or if
    /// `--config` was given (see [`CommonArgs::config`]'s doc comment:
    /// config loading is not implemented yet, and silently ignoring the
    /// flag would run at hardcoded defaults with no signal that the file
    /// was never read).
    pub fn into_config(self) -> Result<ThroughputConfig, String> {
        if self.common.config.is_some() {
            return Err(
                "--config is not implemented yet; YAML config loading lands in a later task \
                 (drop the flag and pass every option on the command line for now)"
                    .to_string(),
            );
        }
        let topology = self.common.topology_source.to_ref()?;
        Ok(ThroughputConfig {
            backend: self.common.backend,
            topology,
            hardness: self.common.hardness,
            duration_s: self.duration,
            seed: self.common.seed,
            output_dir: self.common.output_dir,
            backend_bin_override: None,
            backend_args_override: None,
        })
    }
}

/// `isingmark sweep` arguments.
#[derive(Debug, Args)]
pub struct SweepArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Parameter grid to sweep, e.g.
    /// `'{"num_reads":[64,128],"num_sweeps":[100,500]}'`. Which fields
    /// apply depends on `--backend`'s execution model: `num_reads` +
    /// `num_sweeps` for a sweeping backend (cpu-sa, cpu-gibbs, cuda,
    /// metal), or `num_reads` + `anneal_time_us` for an annealing one
    /// (dwave). `--hardness` (from `CommonArgs`) is not used by this mode
    /// — cell parameters come entirely from this grid instead of a
    /// hardness curve.
    #[arg(long, value_parser = parse_param_grid_arg)]
    pub param_grid: ParamGrid,
    /// Jobs drawn per cell. Every cell draws the identical sequence (same
    /// seed, same count) — see `modes::sweep`'s module docs for why.
    #[arg(long, value_parser = parse_num_jobs, default_value_t = 20)]
    pub num_jobs: u32,
}

impl SweepArgs {
    /// Build a [`SweepConfig`] from parsed arguments.
    ///
    /// # Errors
    ///
    /// Same conditions as [`ThroughputArgs::into_config`]: an unresolved
    /// mutually exclusive topology group, or `--config` (not implemented
    /// yet).
    pub fn into_config(self) -> Result<SweepConfig, String> {
        if self.common.config.is_some() {
            return Err(
                "--config is not implemented yet; YAML config loading lands in a later task \
                 (drop the flag and pass every option on the command line for now)"
                    .to_string(),
            );
        }
        let topology = self.common.topology_source.to_ref()?;
        Ok(SweepConfig {
            backend: self.common.backend,
            topology,
            grid: self.param_grid,
            num_jobs: self.num_jobs,
            seed: self.common.seed,
            output_dir: self.common.output_dir,
            backend_bin_override: None,
            backend_args_override: None,
        })
    }
}

/// `isingmark ttt` arguments.
#[derive(Debug, Args)]
pub struct TttArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Energy targets to reach, in **natural units** (not milli — e.g.
    /// `-5.0`, not `-5000`), comma-separated, e.g. `"-5.0,-10.0,-15.0"`.
    /// Converted internally to milli-units (multiplied by 1000, rounded) to
    /// match `record::JobRecord::best_energy_milli`; see `modes::ttt`'s
    /// module doc for why this boundary is natural units while every
    /// internal type is milli. `--hardness` (from `CommonArgs`) is not used
    /// by this mode — every trial dispatches with unset sample parameters,
    /// deferring entirely to the backend's own default, the same choice
    /// `modes::sweep` documents for its own `target: None`.
    // `value_delimiter`, not a whole-string parser returning `Vec<i64>`:
    // clap's derive macro treats a `Vec<T>`-typed field as "one value of
    // type `T` per occurrence" purely from the field's own syntactic type,
    // regardless of what a custom `value_parser` returns — a parser
    // returning `Vec<i64>` here mismatches what the derived
    // `FromArgMatches` expects to downcast (`i64`, once per occurrence) and
    // panics at parse time. `value_delimiter = ','` instead lets clap split
    // `"-5,-10,-15"` into three occurrences itself, each run through
    // `parse_one_energy_target_arg` (returning a single `i64`), which is
    // exactly what a `Vec<i64>` field expects. `required = true`: an empty
    // list would make every subsequent trial dispatch pointless, so this is
    // enforced the same way `TopologySource`'s own required group is,
    // rather than deferring to `modes::ttt::run_ttt`'s own
    // `ModeError::EmptyTargets` for a mistake catchable this early.
    #[arg(
        long,
        required = true,
        allow_hyphen_values = true,
        value_delimiter = ',',
        value_parser = parse_one_energy_target_arg
    )]
    pub energy_targets: Vec<i64>,
    /// Trials attempted per target, before any `--max-time-s` cutoff.
    #[arg(long, value_parser = parse_num_trials, default_value_t = 20)]
    pub num_trials: u32,
    /// Wall-clock budget per target, in seconds, checked between trials.
    #[arg(long, value_parser = parse_max_time_s, default_value_t = 30.0)]
    pub max_time_s: f64,
}

impl TttArgs {
    /// Build a [`TttConfig`] from parsed arguments.
    ///
    /// # Errors
    ///
    /// Same conditions as [`ThroughputArgs::into_config`]: an unresolved
    /// mutually exclusive topology group, or `--config` (not implemented
    /// yet).
    pub fn into_config(self) -> Result<TttConfig, String> {
        if self.common.config.is_some() {
            return Err(
                "--config is not implemented yet; YAML config loading lands in a later task \
                 (drop the flag and pass every option on the command line for now)"
                    .to_string(),
            );
        }
        let topology = self.common.topology_source.to_ref()?;
        Ok(TttConfig {
            backend: self.common.backend,
            topology,
            energy_targets_milli: self.energy_targets,
            num_trials: self.num_trials,
            max_time_s: self.max_time_s,
            seed: self.common.seed,
            output_dir: self.common.output_dir,
            backend_bin_override: None,
            backend_args_override: None,
        })
    }
}

/// `isingmark comparative` arguments.
#[derive(Debug, Args)]
pub struct ComparativeArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Reference benchmark JSONL file (e.g. another run's `records.jsonl`)
    /// to match this backend's best-energy distribution against.
    /// `--hardness` (from `CommonArgs`) is not used by this mode — hardness
    /// is the value this search computes, not an input to it.
    #[arg(long)]
    pub target_file: PathBuf,
    /// Jobs drawn per hardness evaluation. Every evaluation draws the
    /// identical sequence (same seed, same count) — see
    /// `modes::comparative`'s module docs for why.
    #[arg(long, value_parser = parse_num_jobs, default_value_t = 20)]
    pub num_jobs: u32,
    /// Maximum number of evaluations to run before reporting whatever
    /// hardness the bracket has narrowed to. The search may stop earlier
    /// once the bracket narrows below `0.01`.
    #[arg(long, value_parser = parse_max_iterations, default_value_t = 10)]
    pub max_iterations: u32,
}

impl ComparativeArgs {
    /// Build a [`ComparativeConfig`] from parsed arguments.
    ///
    /// # Errors
    ///
    /// Same conditions as [`ThroughputArgs::into_config`]: an unresolved
    /// mutually exclusive topology group, or `--config` (not implemented
    /// yet).
    pub fn into_config(self) -> Result<ComparativeConfig, String> {
        if self.common.config.is_some() {
            return Err(
                "--config is not implemented yet; YAML config loading lands in a later task \
                 (drop the flag and pass every option on the command line for now)"
                    .to_string(),
            );
        }
        let topology = self.common.topology_source.to_ref()?;
        Ok(ComparativeConfig {
            backend: self.common.backend,
            topology,
            target_file: self.target_file,
            num_jobs: self.num_jobs,
            max_iterations: self.max_iterations,
            seed: self.common.seed,
            output_dir: self.common.output_dir,
            backend_bin_override: None,
            backend_args_override: None,
        })
    }
}

/// `isingmark scaling` arguments. Does not flatten [`CommonArgs`]: unlike
/// every other mode, scaling generates its own topology per requested node
/// count (see `modes::scaling`'s module doc) rather than accepting one via
/// `--topology-preset`/`--topology-file`, so `CommonArgs::topology_source`'s
/// required mutually exclusive group does not apply here. Every other
/// `CommonArgs` field this mode does use (`--backend`, `--hardness`,
/// `--seed`, `--output-dir`, `--config`, `--verbose`) is repeated directly
/// below instead, with the identical value parsers.
#[derive(Debug, Args)]
pub struct ScalingArgs {
    /// Backend to benchmark. See `backend::builtin_registry` for the fixed
    /// set of names this maps to.
    #[arg(long, value_parser = parse_backend)]
    pub backend: String,
    /// Node counts to generate a random 3-regular topology for,
    /// comma-separated (e.g. `"8,16,32"`). Each must be even and at least 4
    /// (a 3-regular graph is impossible otherwise — every node needs 3
    /// distinct neighbours), and the list must not repeat a value (two
    /// identical sizes would collide on one output subdirectory).
    #[arg(
        long,
        required = true,
        value_delimiter = ',',
        value_parser = parse_one_scaling_size_arg
    )]
    pub sizes: Vec<usize>,
    /// Difficulty in `[0.0, 1.0]`, held fixed across every size.
    #[arg(long, value_parser = parse_hardness, default_value_t = 0.5)]
    pub hardness: f64,
    /// Jobs drawn per size.
    #[arg(long, value_parser = parse_num_jobs, default_value_t = 20)]
    pub num_jobs: u32,
    /// Seed for deterministic problem generation and deterministic
    /// topology generation (see `modes::scaling`'s module doc).
    #[arg(long, default_value_t = 0)]
    pub seed: u64,
    /// Directory to write JSONL records and the run summary to. Created if
    /// it does not already exist.
    #[arg(long, default_value = "./out")]
    pub output_dir: PathBuf,
    /// YAML config file. See `CommonArgs::config`'s doc: not implemented
    /// yet.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Increase output verbosity. Repeatable. Accepted for forward
    /// compatibility; not yet wired to any logging behavior.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbosity: u8,
}

impl ScalingArgs {
    /// Build a [`ScalingConfig`] from parsed arguments.
    ///
    /// # Errors
    ///
    /// Returns a message if `--config` was given (see `CommonArgs::config`'s
    /// doc comment: config loading is not implemented yet).
    pub fn into_config(self) -> Result<ScalingConfig, String> {
        if self.config.is_some() {
            return Err(
                "--config is not implemented yet; YAML config loading lands in a later task \
                 (drop the flag and pass every option on the command line for now)"
                    .to_string(),
            );
        }
        Ok(ScalingConfig {
            backend: self.backend,
            sizes: self.sizes,
            hardness: self.hardness,
            num_jobs: self.num_jobs,
            seed: self.seed,
            output_dir: self.output_dir,
            backend_bin_override: None,
            backend_args_override: None,
        })
    }
}

/// `isingmark reproducibility` arguments.
#[derive(Debug, Args)]
pub struct ReproducibilityArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Jobs drawn per run. Both runs draw the identical sequence (same
    /// seed, same count) against the same topology — see
    /// `modes::reproducibility`'s module doc for why.
    #[arg(long, value_parser = parse_num_jobs, default_value_t = 20)]
    pub num_problems: u32,
    /// Difficulty in `[0.0, 1.0]` for the second run. Defaults to
    /// `--hardness` (the ordinary "is this backend reproducible at a fixed
    /// effort" use case — see `modes::reproducibility`'s module doc); set
    /// this to a different value only to intentionally compare two
    /// different sampling efforts against the same problems.
    #[arg(long, value_parser = parse_hardness)]
    pub hardness_b: Option<f64>,
}

impl ReproducibilityArgs {
    /// Build a [`ReproConfig`] from parsed arguments.
    ///
    /// # Errors
    ///
    /// Same conditions as [`ThroughputArgs::into_config`]: an unresolved
    /// mutually exclusive topology group, or `--config` (not implemented
    /// yet).
    pub fn into_config(self) -> Result<ReproConfig, String> {
        if self.common.config.is_some() {
            return Err(
                "--config is not implemented yet; YAML config loading lands in a later task \
                 (drop the flag and pass every option on the command line for now)"
                    .to_string(),
            );
        }
        let topology = self.common.topology_source.to_ref()?;
        let hardness_a = self.common.hardness;
        let hardness_b = self.hardness_b.unwrap_or(hardness_a);
        Ok(ReproConfig {
            backend: self.common.backend,
            topology,
            num_problems: self.num_problems,
            hardness_a,
            hardness_b,
            seed: self.common.seed,
            output_dir: self.common.output_dir,
            backend_bin_override: None,
            backend_args_override: None,
        })
    }
}

/// `--sizes` per-token value parser (see `ScalingArgs::sizes`'s comment for
/// why this is per-token, not whole-string), delegating to
/// `scaling::parse_one_scaling_size`.
fn parse_one_scaling_size_arg(s: &str) -> Result<usize, String> {
    scaling::parse_one_scaling_size(s).map_err(|e| e.to_string())
}

/// `--max-iterations` value parser. `0` would silently produce a
/// comparative run that reports hardness `0.0` — a plausible-looking,
/// in-range value indistinguishable from a real finding — having run zero
/// evaluations, with no error and `ExitCode::SUCCESS`; see
/// `modes::ModeError::InvalidMaxIterations`'s own doc, which
/// `comparative::run_comparative` additionally checks for callers that
/// bypass this parser. Reject it here for the same reason
/// `parse_num_trials`/`parse_num_jobs` reject `0`.
fn parse_max_iterations(s: &str) -> Result<u32, String> {
    let value: u32 = s.parse().map_err(|_| format!("{s:?} is not a valid u32"))?;
    if value == 0 {
        return Err(
            "--max-iterations must not be 0 (a zero-iteration search would report hardness 0.0 \
             with no signal that no evaluation ever ran)"
                .to_string(),
        );
    }
    Ok(value)
}

/// `--param-grid` value parser, delegating to `sweep::parse_param_grid`.
fn parse_param_grid_arg(s: &str) -> Result<ParamGrid, String> {
    sweep::parse_param_grid(s).map_err(|e| e.to_string())
}

/// `--energy-targets` per-token value parser (see `TttArgs::energy_targets`'s
/// comment for why this is per-token, not whole-string), delegating to
/// `ttt::parse_one_energy_target`. Duplicate-target detection (two entries
/// converting to the same milli-value target) is not repeated here: it
/// needs the full parsed list at once, which this per-token parser never
/// sees, so it is left to `modes::ttt::run_ttt`'s own
/// `ModeError::DuplicateTarget` — the same "mode-level, not CLI-level"
/// placement `sweep::ModeError::EmptyGrid` already uses for a check that
/// similarly cannot be resolved from one flag's text alone.
fn parse_one_energy_target_arg(s: &str) -> Result<i64, String> {
    ttt::parse_one_energy_target(s).map_err(|e| e.to_string())
}

/// `--num-trials` value parser. `0` would silently produce a "successful"
/// run that attempted nothing for every target — every target's
/// `summary.json` entry would report `trials_attempted: 0` with no error and
/// `ExitCode::SUCCESS` — the same failure mode `sweep::parse_num_jobs`
/// rejects `0` for. Reject it here for the same reason.
fn parse_num_trials(s: &str) -> Result<u32, String> {
    let value: u32 = s.parse().map_err(|_| format!("{s:?} is not a valid u32"))?;
    if value == 0 {
        return Err(
            "--num-trials must not be 0 (a zero-trial target would report an empty result with \
             no signal that it never ran)"
                .to_string(),
        );
    }
    Ok(value)
}

/// `--max-time-s` value parser. clap's default `f64` parser accepts
/// `nan`/`inf`/`-inf`/negative values and `0.0`; `modes::ttt::run_ttt`
/// treats any of those as an already-exhausted budget (`modes::ttt::target_wall`
/// degrades them to `Duration::ZERO`), which would silently attempt zero
/// trials for every target — the same "plausible-looking, all-zero result"
/// failure mode `--hardness`'s own `nan` guard and `--num-trials 0`'s
/// rejection above both exist to catch. Reject anything non-finite or `<= 0`
/// here, at parse time, instead.
fn parse_max_time_s(s: &str) -> Result<f64, String> {
    let value: f64 = s
        .parse()
        .map_err(|_| format!("{s:?} is not a valid number"))?;
    if !value.is_finite() {
        return Err(format!("--max-time-s must be a finite number, got {s:?}"));
    }
    if value <= 0.0 {
        return Err(format!("--max-time-s must be greater than 0, got {value}"));
    }
    Ok(value)
}

/// `--num-jobs` value parser. `0` would silently produce a "successful" run
/// that dispatched nothing (`run_session` shuts down immediately after the
/// handshake) — every cell's `summary.json` entry would report `jobs: 0`
/// with no error and `ExitCode::SUCCESS`, the same "your input does not
/// mean what you think" failure mode `--param-grid`'s own `0` values are
/// rejected for (see `sweep::parse_param_grid`) and `--hardness` is
/// rejected for when non-finite. Reject it here for the same reason.
fn parse_num_jobs(s: &str) -> Result<u32, String> {
    let value: u32 = s.parse().map_err(|_| format!("{s:?} is not a valid u32"))?;
    if value == 0 {
        return Err(
            "--num-jobs must not be 0 (a zero-job sweep would exit successfully having tested \
             nothing, with no signal that it did not)"
                .to_string(),
        );
    }
    Ok(value)
}

/// `--backend` value parser, checked against the live built-in registry so
/// an unknown name produces an error listing the exact valid names rather
/// than a generic "invalid value" message.
///
/// Not built with `clap::builder::PossibleValuesParser`: that type only
/// accepts owned `String`s when clap's `string` feature is enabled, which
/// this crate's pinned `clap = { features = ["derive"] }` dependency does
/// not turn on, and this task must not change the dependency block.
fn parse_backend(s: &str) -> Result<String, String> {
    let names: Vec<String> = builtin_registry()
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    if names.iter().any(|name| name == s) {
        Ok(s.to_string())
    } else {
        Err(format!(
            "invalid backend {s:?}; valid values: {}",
            names.join(", ")
        ))
    }
}

/// `--hardness` value parser. clap's default `f64` parser is
/// `f64::from_str`, which accepts `nan`/`inf`/`-inf`;
/// `params::ParamCurve::resolve` is total and silently maps `NaN` to the
/// curve minimum, so an un-guarded `--hardness nan` would exit cleanly with
/// a full, successful-looking result set while every job silently ran at
/// the curve's easiest setting. Reject anything non-finite or outside
/// `[0.0, 1.0]` here, at parse time, instead.
fn parse_hardness(s: &str) -> Result<f64, String> {
    let value: f64 = s
        .parse()
        .map_err(|_| format!("{s:?} is not a valid number"))?;
    if !value.is_finite() {
        return Err(format!("--hardness must be a finite number, got {s:?}"));
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(format!("--hardness must be in [0.0, 1.0], got {value}"));
    }
    Ok(value)
}

/// `--duration` value parser, delegating to `throughput::parse_duration`.
fn parse_duration_arg(s: &str) -> Result<f64, String> {
    throughput::parse_duration(s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{
        Cli, Command, ComparativeArgs, ReproducibilityArgs, ScalingArgs, SweepArgs, ThroughputArgs,
        TttArgs,
    };

    fn parse(extra_args: &[&str]) -> Result<Cli, clap::Error> {
        let mut full_argv = vec!["isingmark"];
        full_argv.extend_from_slice(extra_args);
        Cli::try_parse_from(full_argv)
    }

    /// Extract the `Throughput` variant, for tests that parsed a throughput
    /// invocation. `Command` has more than one variant (as of
    /// `sweep`/`ttt`/`comparative`/`scaling`/`reproducibility`), so `let
    /// Command::Throughput(args) = cli.command;` is no longer an irrefutable
    /// pattern and cannot be used directly.
    fn throughput_args(cli: Cli) -> ThroughputArgs {
        match cli.command {
            Command::Throughput(args) => args,
            Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_) => {
                unreachable!("test only ever parses a throughput invocation")
            }
        }
    }

    /// Extract the `Sweep` variant. See [`throughput_args`].
    fn sweep_args(cli: Cli) -> SweepArgs {
        match cli.command {
            Command::Sweep(args) => args,
            Command::Throughput(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_) => {
                unreachable!("test only ever parses a sweep invocation")
            }
        }
    }

    /// Extract the `Ttt` variant. See [`throughput_args`].
    fn ttt_args(cli: Cli) -> TttArgs {
        match cli.command {
            Command::Ttt(args) => args,
            Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Comparative(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_) => {
                unreachable!("test only ever parses a ttt invocation")
            }
        }
    }

    /// Extract the `Comparative` variant. See [`throughput_args`].
    fn comparative_args(cli: Cli) -> ComparativeArgs {
        match cli.command {
            Command::Comparative(args) => args,
            Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_) => {
                unreachable!("test only ever parses a comparative invocation")
            }
        }
    }

    /// Extract the `Scaling` variant. See [`throughput_args`].
    fn scaling_args(cli: Cli) -> ScalingArgs {
        match cli.command {
            Command::Scaling(args) => args,
            Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Reproducibility(_) => {
                unreachable!("test only ever parses a scaling invocation")
            }
        }
    }

    /// Extract the `Reproducibility` variant. See [`throughput_args`].
    fn reproducibility_args(cli: Cli) -> ReproducibilityArgs {
        match cli.command {
            Command::Reproducibility(args) => args,
            Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Scaling(_) => {
                unreachable!("test only ever parses a reproducibility invocation")
            }
        }
    }

    #[test]
    fn a_full_valid_throughput_invocation_parses() {
        let cli = parse(&[
            "throughput",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--hardness",
            "0.5",
            "--duration",
            "30s",
            "--seed",
            "42",
            "--output-dir",
            "./out",
        ])
        .expect("valid invocation parses");
        let args = throughput_args(cli);
        assert_eq!(args.common.backend, "cpu-sa");
        assert_eq!(
            args.common.topology_source.topology_preset.as_deref(),
            Some("smoke")
        );
        assert!((args.common.hardness - 0.5).abs() < f64::EPSILON);
        assert!((args.duration - 30.0).abs() < f64::EPSILON);
        assert_eq!(args.common.seed, 42);
    }

    #[test]
    fn hardness_nan_is_rejected() {
        let err = parse(&[
            "throughput",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--hardness",
            "nan",
            "--duration",
            "30s",
        ])
        .expect_err("NaN hardness must be rejected at parse time");
        assert!(err.to_string().contains("finite"));
    }

    #[test]
    fn hardness_above_one_is_rejected() {
        let err = parse(&[
            "throughput",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--hardness",
            "2.0",
            "--duration",
            "30s",
        ])
        .expect_err("out-of-range hardness must be rejected at parse time");
        assert!(err.to_string().contains("[0.0, 1.0]"));
    }

    #[test]
    fn unknown_backend_is_rejected_and_lists_valid_names() {
        let err = parse(&[
            "throughput",
            "--backend",
            "not-a-real-backend",
            "--topology-preset",
            "smoke",
            "--duration",
            "30s",
        ])
        .expect_err("unknown backend must be rejected");
        let message = err.to_string();
        assert!(message.contains("cpu-sa"));
        assert!(message.contains("dwave"));
    }

    #[test]
    fn missing_topology_source_is_rejected() {
        parse(&["throughput", "--backend", "cpu-sa", "--duration", "30s"])
            .expect_err("one of --topology-preset/--topology-file is required");
    }

    #[test]
    fn both_topology_sources_together_are_rejected() {
        parse(&[
            "throughput",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--topology-file",
            "some.spec.json",
            "--duration",
            "30s",
        ])
        .expect_err("--topology-preset and --topology-file are mutually exclusive");
    }

    #[test]
    fn invalid_duration_is_rejected() {
        let err = parse(&[
            "throughput",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--duration",
            "5x",
        ])
        .expect_err("an invalid duration unit must be rejected");
        assert!(err.to_string().contains("5x"));
    }

    #[test]
    fn into_config_carries_every_field_through() {
        let cli = parse(&[
            "throughput",
            "--backend",
            "cpu-sa",
            "--topology-file",
            "fixtures/smoke.spec.json",
            "--hardness",
            "0.75",
            "--duration",
            "1m",
            "--seed",
            "7",
            "--output-dir",
            "/tmp/isingmark-out",
        ])
        .expect("valid invocation parses");
        let args = throughput_args(cli);
        let cfg = args.into_config().expect("topology group resolves");
        assert_eq!(cfg.backend, "cpu-sa");
        assert_eq!(
            cfg.topology,
            crate::modes::TopologyRef::File(std::path::PathBuf::from("fixtures/smoke.spec.json"))
        );
        assert!((cfg.hardness - 0.75).abs() < f64::EPSILON);
        assert!((cfg.duration_s - 60.0).abs() < f64::EPSILON);
        assert_eq!(cfg.seed, 7);
        assert_eq!(
            cfg.output_dir,
            std::path::PathBuf::from("/tmp/isingmark-out")
        );
        assert_eq!(cfg.backend_bin_override, None);
        assert_eq!(cfg.backend_args_override, None);
    }

    /// `--config` is plan-mandated to exist as a flag, but loading is not
    /// implemented yet; `into_config` must reject it explicitly rather than
    /// silently running at hardcoded defaults with no signal the file was
    /// never read.
    #[test]
    fn config_flag_is_rejected_until_config_loading_is_implemented() {
        let cli = parse(&[
            "throughput",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--duration",
            "30s",
            "--config",
            "bench.yaml",
        ])
        .expect("clap itself accepts --config; rejection happens in into_config");
        let args = throughput_args(cli);
        let err = args
            .into_config()
            .expect_err("--config must be rejected until it is implemented");
        assert!(err.contains("--config"));
        assert!(err.contains("not implemented"));
    }

    #[test]
    fn a_full_valid_sweep_invocation_parses() {
        let cli = parse(&[
            "sweep",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--param-grid",
            r#"{"num_reads":[64,128],"num_sweeps":[100,500]}"#,
            "--num-jobs",
            "10",
            "--seed",
            "42",
            "--output-dir",
            "./out",
        ])
        .expect("valid invocation parses");
        let args = sweep_args(cli);
        assert_eq!(args.common.backend, "cpu-sa");
        assert_eq!(args.param_grid.num_reads, vec![64, 128]);
        assert_eq!(args.param_grid.num_sweeps, vec![100, 500]);
        assert_eq!(args.num_jobs, 10);
        assert_eq!(args.common.seed, 42);
    }

    /// A malformed `--param-grid` (here, a `0` value) must be rejected at
    /// parse time, not deferred to `into_config`/`run_sweep`.
    #[test]
    fn sweep_param_grid_zero_value_is_rejected_at_parse_time() {
        let err = parse(&[
            "sweep",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--param-grid",
            r#"{"num_reads":[0,64]}"#,
        ])
        .expect_err("a 0 grid value must be rejected at parse time");
        assert!(err.to_string().contains('0'));
    }

    /// `--num-jobs 0` would silently produce a sweep that dispatches
    /// nothing and still exits successfully; must be rejected at parse
    /// time, matching the `--param-grid` zero-value convention above.
    #[test]
    fn sweep_num_jobs_zero_is_rejected_at_parse_time() {
        let err = parse(&[
            "sweep",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--param-grid",
            r#"{"num_reads":[64]}"#,
            "--num-jobs",
            "0",
        ])
        .expect_err("--num-jobs 0 must be rejected at parse time");
        assert!(err.to_string().contains("--num-jobs"));
    }

    #[test]
    fn sweep_into_config_carries_every_field_through() {
        let cli = parse(&[
            "sweep",
            "--backend",
            "cpu-sa",
            "--topology-file",
            "fixtures/smoke.spec.json",
            "--param-grid",
            r#"{"num_reads":[8,16],"num_sweeps":[50]}"#,
            "--num-jobs",
            "5",
            "--seed",
            "7",
            "--output-dir",
            "/tmp/isingmark-sweep-out",
        ])
        .expect("valid invocation parses");
        let args = sweep_args(cli);
        let cfg = args.into_config().expect("topology group resolves");
        assert_eq!(cfg.backend, "cpu-sa");
        assert_eq!(
            cfg.topology,
            crate::modes::TopologyRef::File(std::path::PathBuf::from("fixtures/smoke.spec.json"))
        );
        assert_eq!(cfg.grid.num_reads, vec![8, 16]);
        assert_eq!(cfg.grid.num_sweeps, vec![50]);
        assert_eq!(cfg.num_jobs, 5);
        assert_eq!(cfg.seed, 7);
        assert_eq!(
            cfg.output_dir,
            std::path::PathBuf::from("/tmp/isingmark-sweep-out")
        );
        assert_eq!(cfg.backend_bin_override, None);
        assert_eq!(cfg.backend_args_override, None);
    }

    #[test]
    fn sweep_config_flag_is_rejected_until_config_loading_is_implemented() {
        let cli = parse(&[
            "sweep",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--param-grid",
            r#"{"num_reads":[64]}"#,
            "--config",
            "bench.yaml",
        ])
        .expect("clap itself accepts --config; rejection happens in into_config");
        let args = sweep_args(cli);
        let err = args
            .into_config()
            .expect_err("--config must be rejected until it is implemented");
        assert!(err.contains("--config"));
        assert!(err.contains("not implemented"));
    }

    #[test]
    fn a_full_valid_ttt_invocation_parses() {
        let cli = parse(&[
            "ttt",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--energy-targets",
            "-5,-10,-15",
            "--num-trials",
            "10",
            "--max-time-s",
            "5",
            "--seed",
            "42",
            "--output-dir",
            "./out",
        ])
        .expect("valid invocation parses");
        let args = ttt_args(cli);
        assert_eq!(args.common.backend, "cpu-sa");
        assert_eq!(args.energy_targets, vec![-5_000, -10_000, -15_000]);
        assert_eq!(args.num_trials, 10);
        assert!((args.max_time_s - 5.0).abs() < f64::EPSILON);
        assert_eq!(args.common.seed, 42);
    }

    /// `--energy-targets` accepts natural units, not milli — pinned here at
    /// the CLI boundary (see `modes::ttt::parse_energy_targets`'s own tests
    /// for the unit-level pin) so a regression in the CLI wiring itself
    /// (e.g. swapping in a parser that skips the natural-to-milli
    /// conversion) is caught independently of that module's own tests.
    #[test]
    fn ttt_energy_targets_are_converted_from_natural_to_milli_units() {
        let cli = parse(&[
            "ttt",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--energy-targets",
            "-5",
        ])
        .expect("valid invocation parses");
        let args = ttt_args(cli);
        assert_eq!(
            args.energy_targets,
            vec![-5_000],
            "-5 (natural units) must become -5000 (milli), not -5"
        );
    }

    #[test]
    fn ttt_malformed_energy_targets_is_rejected_at_parse_time() {
        let err = parse(&[
            "ttt",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--energy-targets",
            "not-a-number",
        ])
        .expect_err("a malformed --energy-targets must be rejected at parse time");
        assert!(err.to_string().contains("not-a-number"));
    }

    /// `--num-trials 0` would silently produce a ttt run that attempted
    /// nothing for every target and still exits successfully; must be
    /// rejected at parse time, matching `--num-jobs 0`'s convention for
    /// `sweep`.
    #[test]
    fn ttt_num_trials_zero_is_rejected_at_parse_time() {
        let err = parse(&[
            "ttt",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--energy-targets",
            "-5",
            "--num-trials",
            "0",
        ])
        .expect_err("--num-trials 0 must be rejected at parse time");
        assert!(err.to_string().contains("--num-trials"));
    }

    #[test]
    fn ttt_max_time_s_zero_is_rejected_at_parse_time() {
        let err = parse(&[
            "ttt",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--energy-targets",
            "-5",
            "--max-time-s",
            "0",
        ])
        .expect_err("--max-time-s 0 must be rejected at parse time");
        assert!(err.to_string().contains("--max-time-s"));
    }

    #[test]
    fn ttt_max_time_s_negative_and_non_finite_are_rejected_at_parse_time() {
        for bad in ["-1", "nan", "inf", "-inf"] {
            parse(&[
                "ttt",
                "--backend",
                "cpu-sa",
                "--topology-preset",
                "smoke",
                "--energy-targets",
                "-5",
                "--max-time-s",
                bad,
            ])
            .expect_err(&format!(
                "--max-time-s {bad} must be rejected at parse time"
            ));
        }
    }

    #[test]
    fn ttt_into_config_carries_every_field_through() {
        let cli = parse(&[
            "ttt",
            "--backend",
            "cpu-sa",
            "--topology-file",
            "fixtures/smoke.spec.json",
            "--energy-targets",
            "-5,-10",
            "--num-trials",
            "7",
            "--max-time-s",
            "12.5",
            "--seed",
            "9",
            "--output-dir",
            "/tmp/isingmark-ttt-out",
        ])
        .expect("valid invocation parses");
        let args = ttt_args(cli);
        let cfg = args.into_config().expect("topology group resolves");
        assert_eq!(cfg.backend, "cpu-sa");
        assert_eq!(
            cfg.topology,
            crate::modes::TopologyRef::File(std::path::PathBuf::from("fixtures/smoke.spec.json"))
        );
        assert_eq!(cfg.energy_targets_milli, vec![-5_000, -10_000]);
        assert_eq!(cfg.num_trials, 7);
        assert!((cfg.max_time_s - 12.5).abs() < f64::EPSILON);
        assert_eq!(cfg.seed, 9);
        assert_eq!(
            cfg.output_dir,
            std::path::PathBuf::from("/tmp/isingmark-ttt-out")
        );
        assert_eq!(cfg.backend_bin_override, None);
        assert_eq!(cfg.backend_args_override, None);
    }

    #[test]
    fn ttt_config_flag_is_rejected_until_config_loading_is_implemented() {
        let cli = parse(&[
            "ttt",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--energy-targets",
            "-5",
            "--config",
            "bench.yaml",
        ])
        .expect("clap itself accepts --config; rejection happens in into_config");
        let args = ttt_args(cli);
        let err = args
            .into_config()
            .expect_err("--config must be rejected until it is implemented");
        assert!(err.contains("--config"));
        assert!(err.contains("not implemented"));
    }

    #[test]
    fn a_full_valid_comparative_invocation_parses() {
        let cli = parse(&[
            "comparative",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--target-file",
            "reference.jsonl",
            "--num-jobs",
            "15",
            "--max-iterations",
            "5",
            "--seed",
            "42",
            "--output-dir",
            "./out",
        ])
        .expect("valid invocation parses");
        let args = comparative_args(cli);
        assert_eq!(args.common.backend, "cpu-sa");
        assert_eq!(
            args.target_file,
            std::path::PathBuf::from("reference.jsonl")
        );
        assert_eq!(args.num_jobs, 15);
        assert_eq!(args.max_iterations, 5);
        assert_eq!(args.common.seed, 42);
    }

    #[test]
    fn comparative_target_file_is_required() {
        parse(&[
            "comparative",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
        ])
        .expect_err("--target-file is required");
    }

    /// `--max-iterations 0` would silently produce a search that reports
    /// hardness `0.0` — a plausible-looking, in-range value — having run
    /// zero evaluations; must be rejected at parse time, matching
    /// `--num-jobs 0`'s convention for `sweep`/`ttt`'s own zero-value
    /// rejections.
    #[test]
    fn comparative_max_iterations_zero_is_rejected_at_parse_time() {
        let err = parse(&[
            "comparative",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--target-file",
            "reference.jsonl",
            "--max-iterations",
            "0",
        ])
        .expect_err("--max-iterations 0 must be rejected at parse time");
        assert!(err.to_string().contains("--max-iterations"));
    }

    #[test]
    fn comparative_num_jobs_zero_is_rejected_at_parse_time() {
        let err = parse(&[
            "comparative",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--target-file",
            "reference.jsonl",
            "--num-jobs",
            "0",
        ])
        .expect_err("--num-jobs 0 must be rejected at parse time");
        assert!(err.to_string().contains("--num-jobs"));
    }

    #[test]
    fn comparative_into_config_carries_every_field_through() {
        let cli = parse(&[
            "comparative",
            "--backend",
            "cpu-sa",
            "--topology-file",
            "fixtures/smoke.spec.json",
            "--target-file",
            "reference.jsonl",
            "--num-jobs",
            "12",
            "--max-iterations",
            "6",
            "--seed",
            "9",
            "--output-dir",
            "/tmp/isingmark-comparative-out",
        ])
        .expect("valid invocation parses");
        let args = comparative_args(cli);
        let cfg = args.into_config().expect("topology group resolves");
        assert_eq!(cfg.backend, "cpu-sa");
        assert_eq!(
            cfg.topology,
            crate::modes::TopologyRef::File(std::path::PathBuf::from("fixtures/smoke.spec.json"))
        );
        assert_eq!(cfg.target_file, std::path::PathBuf::from("reference.jsonl"));
        assert_eq!(cfg.num_jobs, 12);
        assert_eq!(cfg.max_iterations, 6);
        assert_eq!(cfg.seed, 9);
        assert_eq!(
            cfg.output_dir,
            std::path::PathBuf::from("/tmp/isingmark-comparative-out")
        );
        assert_eq!(cfg.backend_bin_override, None);
        assert_eq!(cfg.backend_args_override, None);
    }

    #[test]
    fn comparative_config_flag_is_rejected_until_config_loading_is_implemented() {
        let cli = parse(&[
            "comparative",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--target-file",
            "reference.jsonl",
            "--config",
            "bench.yaml",
        ])
        .expect("clap itself accepts --config; rejection happens in into_config");
        let args = comparative_args(cli);
        let err = args
            .into_config()
            .expect_err("--config must be rejected until it is implemented");
        assert!(err.contains("--config"));
        assert!(err.contains("not implemented"));
    }

    #[test]
    fn a_full_valid_scaling_invocation_parses() {
        let cli = parse(&[
            "scaling",
            "--backend",
            "cpu-sa",
            "--sizes",
            "8,16,32",
            "--hardness",
            "0.5",
            "--num-jobs",
            "10",
            "--seed",
            "42",
            "--output-dir",
            "./out",
        ])
        .expect("valid invocation parses");
        let args = scaling_args(cli);
        assert_eq!(args.backend, "cpu-sa");
        assert_eq!(args.sizes, vec![8, 16, 32]);
        assert!((args.hardness - 0.5).abs() < f64::EPSILON);
        assert_eq!(args.num_jobs, 10);
        assert_eq!(args.seed, 42);
    }

    /// Scaling does not take a topology source at all — unlike every other
    /// mode, it generates its own per requested size (see
    /// `modes::scaling`'s module doc). Confirms `--topology-preset` is not
    /// even a recognized flag for this subcommand.
    #[test]
    fn scaling_does_not_accept_a_topology_source() {
        parse(&[
            "scaling",
            "--backend",
            "cpu-sa",
            "--sizes",
            "8",
            "--topology-preset",
            "smoke",
        ])
        .expect_err("--topology-preset must not be a recognized flag for scaling");
    }

    /// An odd size, or a size below 4, must be rejected at parse time,
    /// naming the offending value.
    #[test]
    fn scaling_invalid_sizes_are_rejected_at_parse_time_naming_the_value() {
        let err = parse(&["scaling", "--backend", "cpu-sa", "--sizes", "8,7"])
            .expect_err("an odd size must be rejected at parse time");
        assert!(err.to_string().contains('7'));

        let err = parse(&["scaling", "--backend", "cpu-sa", "--sizes", "3"])
            .expect_err("a size below 4 must be rejected at parse time");
        assert!(err.to_string().contains('3'));
    }

    #[test]
    fn scaling_sizes_is_required() {
        parse(&["scaling", "--backend", "cpu-sa"]).expect_err("--sizes is required");
    }

    #[test]
    fn scaling_into_config_carries_every_field_through() {
        let cli = parse(&[
            "scaling",
            "--backend",
            "cpu-sa",
            "--sizes",
            "8,16",
            "--hardness",
            "0.75",
            "--num-jobs",
            "5",
            "--seed",
            "9",
            "--output-dir",
            "/tmp/isingmark-scaling-out",
        ])
        .expect("valid invocation parses");
        let args = scaling_args(cli);
        let cfg = args.into_config().expect("config builds");
        assert_eq!(cfg.backend, "cpu-sa");
        assert_eq!(cfg.sizes, vec![8, 16]);
        assert!((cfg.hardness - 0.75).abs() < f64::EPSILON);
        assert_eq!(cfg.num_jobs, 5);
        assert_eq!(cfg.seed, 9);
        assert_eq!(
            cfg.output_dir,
            std::path::PathBuf::from("/tmp/isingmark-scaling-out")
        );
        assert_eq!(cfg.backend_bin_override, None);
        assert_eq!(cfg.backend_args_override, None);
    }

    #[test]
    fn scaling_config_flag_is_rejected_until_config_loading_is_implemented() {
        let cli = parse(&[
            "scaling",
            "--backend",
            "cpu-sa",
            "--sizes",
            "8",
            "--config",
            "bench.yaml",
        ])
        .expect("clap itself accepts --config; rejection happens in into_config");
        let args = scaling_args(cli);
        let err = args
            .into_config()
            .expect_err("--config must be rejected until it is implemented");
        assert!(err.contains("--config"));
        assert!(err.contains("not implemented"));
    }

    #[test]
    fn a_full_valid_reproducibility_invocation_parses() {
        let cli = parse(&[
            "reproducibility",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--num-problems",
            "15",
            "--hardness",
            "0.4",
            "--seed",
            "42",
            "--output-dir",
            "./out",
        ])
        .expect("valid invocation parses");
        let args = reproducibility_args(cli);
        assert_eq!(args.common.backend, "cpu-sa");
        assert_eq!(args.num_problems, 15);
        assert!((args.common.hardness - 0.4).abs() < f64::EPSILON);
        assert_eq!(args.hardness_b, None);
    }

    /// `--hardness-b` defaults to `None` (resolved to `--hardness` in
    /// `into_config`) but can be set independently.
    #[test]
    fn reproducibility_hardness_b_can_be_set_independently() {
        let cli = parse(&[
            "reproducibility",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--hardness",
            "0.0",
            "--hardness-b",
            "1.0",
        ])
        .expect("valid invocation parses");
        let args = reproducibility_args(cli);
        assert_eq!(args.hardness_b, Some(1.0));
    }

    #[test]
    fn reproducibility_into_config_defaults_hardness_b_to_hardness() {
        let cli = parse(&[
            "reproducibility",
            "--backend",
            "cpu-sa",
            "--topology-file",
            "fixtures/smoke.spec.json",
            "--hardness",
            "0.6",
            "--num-problems",
            "7",
            "--seed",
            "9",
            "--output-dir",
            "/tmp/isingmark-repro-out",
        ])
        .expect("valid invocation parses");
        let args = reproducibility_args(cli);
        let cfg = args.into_config().expect("topology group resolves");
        assert_eq!(cfg.backend, "cpu-sa");
        assert_eq!(
            cfg.topology,
            crate::modes::TopologyRef::File(std::path::PathBuf::from("fixtures/smoke.spec.json"))
        );
        assert_eq!(cfg.num_problems, 7);
        assert!((cfg.hardness_a - 0.6).abs() < f64::EPSILON);
        assert!(
            (cfg.hardness_b - 0.6).abs() < f64::EPSILON,
            "hardness_b must default to hardness_a (--hardness) when --hardness-b is not given"
        );
        assert_eq!(cfg.seed, 9);
        assert_eq!(
            cfg.output_dir,
            std::path::PathBuf::from("/tmp/isingmark-repro-out")
        );
        assert_eq!(cfg.backend_bin_override, None);
        assert_eq!(cfg.backend_args_override, None);
    }

    #[test]
    fn reproducibility_into_config_carries_an_explicit_hardness_b_through() {
        let cli = parse(&[
            "reproducibility",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--hardness",
            "0.0",
            "--hardness-b",
            "1.0",
        ])
        .expect("valid invocation parses");
        let args = reproducibility_args(cli);
        let cfg = args.into_config().expect("topology group resolves");
        assert!((cfg.hardness_a - 0.0).abs() < f64::EPSILON);
        assert!((cfg.hardness_b - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reproducibility_config_flag_is_rejected_until_config_loading_is_implemented() {
        let cli = parse(&[
            "reproducibility",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--config",
            "bench.yaml",
        ])
        .expect("clap itself accepts --config; rejection happens in into_config");
        let args = reproducibility_args(cli);
        let err = args
            .into_config()
            .expect_err("--config must be rejected until it is implemented");
        assert!(err.contains("--config"));
        assert!(err.contains("not implemented"));
    }
}
