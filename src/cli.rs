// SPDX-License-Identifier: AGPL-3.0-or-later
//! Command-line argument parsing: the `isingmark` clap [`Parser`], its
//! subcommands, and the options common to every mode.

use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::backend::builtin_registry;
use crate::config::{
    self, merge, FileConfig, FileParamCurve, DEFAULT_HARDNESS, DEFAULT_MAX_ITERATIONS,
    DEFAULT_MAX_TIME_S, DEFAULT_NUM_JOBS, DEFAULT_NUM_PROBLEMS, DEFAULT_NUM_TRIALS,
    DEFAULT_OUTPUT_DIR, DEFAULT_SEED,
};
use crate::modes::comparative::{
    ComparativeConfig, DEFAULT_PARAM_CURVE as COMPARATIVE_DEFAULT_CURVE,
};
use crate::modes::reproducibility::{ReproConfig, DEFAULT_PARAM_CURVE as REPRO_DEFAULT_CURVE};
use crate::modes::scaling::{self, ScalingConfig, DEFAULT_PARAM_CURVE as SCALING_DEFAULT_CURVE};
use crate::modes::sweep::{self, ParamGrid, SweepConfig};
use crate::modes::throughput::{
    self, ThroughputConfig, DEFAULT_PARAM_CURVE as THROUGHPUT_DEFAULT_CURVE,
};
use crate::modes::ttt::{self, TttConfig};
use crate::modes::TopologyRef;
use crate::params::ParamCurve;
use crate::topology::Couplings;

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
    /// Compare two completed benchmark runs' JSONL record files and report
    /// whether their best-energy distributions differ. Pure post-hoc
    /// analysis: no backend is spawned.
    Compare(CompareArgs),
    /// Run timed trials from an explicit campaign manifest.
    Campaign(CampaignArgs),
}

#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
pub struct CampaignArgs {
    #[command(subcommand)]
    pub command: Option<CampaignCommand>,
    /// Campaign manifest, with paths relative to its directory.
    #[arg(long, required = true)]
    pub manifest: Option<PathBuf>,
    /// Select policies with this backend name or executable path.
    #[arg(long, required = true)]
    pub backend: Option<String>,
    /// Directory for trial traces and campaign results.
    #[arg(long, required = true)]
    pub output: Option<PathBuf>,
    /// Validate inputs and print resource estimates without starting a miner.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Subcommand)]
pub enum CampaignCommand {
    /// Read bounded JSON-lines commands from stdin and write JSON event replies.
    Serve(CampaignServeArgs),
}

#[derive(Debug, Args)]
pub struct CampaignServeArgs {
    /// Campaign manifest. Its input paths are relative to this file.
    #[arg(long)]
    pub manifest: PathBuf,
    /// Select policies with this backend name or executable path.
    #[arg(long)]
    pub backend: String,
    /// Directory for trial traces and campaign results.
    #[arg(long)]
    pub output: PathBuf,
}

impl CampaignServeArgs {
    #[must_use]
    pub fn into_config(self, dry_run: bool) -> crate::campaign::runner::CampaignConfig {
        crate::campaign::runner::CampaignConfig {
            manifest: self.manifest,
            backend: self.backend,
            output: self.output,
            dry_run,
        }
    }
}

/// `isingmark compare` arguments. Unlike every other mode, compare takes
/// only two input paths — no backend, topology, hardness, or output dir.
#[derive(Debug, Args)]
pub struct CompareArgs {
    /// Path to the first run's JSONL records file.
    #[arg(long)]
    pub file_a: PathBuf,
    /// Path to the second run's JSONL records file.
    #[arg(long)]
    pub file_b: PathBuf,
    /// Write an energy-CDF overlay PNG to this path. Requires a binary
    /// built with `--features plot`; without that feature the flag still
    /// parses and the process exits non-zero with a rebuild message.
    #[arg(long, value_name = "PATH")]
    pub plot: Option<PathBuf>,
}

/// Options common to every mode: which backend and topology to use, how
/// hard to drive it, and where to write results.
///
/// Fields that a YAML config file can also supply are `Option` with **no**
/// clap `default_value` / `default_value_t`: a clap default would make an
/// omitted flag indistinguishable from an explicit user value, so
/// [`merge`] would see `Some(default)` and the config file's value would
/// be silently ignored. Defaults are applied in each mode's `into_config`
/// via [`merge`] instead; help text names the default.
#[derive(Debug, Args)]
pub struct CommonArgs {
    /// Backend to benchmark. See `backend::builtin_registry` for the fixed
    /// set of names this maps to. Required from `--backend` or `--config`.
    #[arg(long, value_parser = parse_backend)]
    pub backend: Option<String>,
    /// Topology source: at most one of the three flags; required from a flag
    /// or from `--config` after merge.
    #[command(flatten)]
    pub topology_source: TopologySource,
    /// Difficulty in `[0.0, 1.0]`; interpolates between a backend's min and
    /// max sample parameters (default: 0.5).
    #[arg(long, value_parser = parse_hardness, help = "Difficulty in [0.0, 1.0] (default: 0.5)")]
    pub hardness: Option<f64>,
    /// Seed for deterministic problem generation (default: 0).
    #[arg(long, help = "Seed for deterministic problem generation (default: 0)")]
    pub seed: Option<u64>,
    /// Directory to write JSONL records and the run summary to (default: ./out).
    /// Created if it does not already exist.
    #[arg(
        long,
        help = "Output directory for JSONL records and summary (default: ./out)"
    )]
    pub output_dir: Option<PathBuf>,
    /// YAML config file. Explicit flags override values from this file;
    /// the file overrides built-in defaults. A config file can supply
    /// backend, topology, and every other option this mode accepts, so
    /// `isingmark <mode> --config FILE` is sufficient on its own.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Increase output verbosity. Repeatable. Accepted for forward
    /// compatibility; not yet wired to any logging behavior.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbosity: u8,
}

/// Topology source: a bundled fixture, a spec file, or a generated graph.
/// At most one may be set on the CLI. None is required at
/// clap parse time because a `--config` file may
/// supply the topology instead; after merge, exactly one source must
/// resolve or `into_config` errors.
#[derive(Debug, Args)]
#[group(skip)]
pub struct TopologySource {
    /// Bundled fixture name (e.g. `"smoke"`).
    #[arg(long, group = "topology_source")]
    pub topology_preset: Option<String>,
    /// Path to a topology-spec JSON file.
    #[arg(long, group = "topology_source")]
    pub topology_file: Option<PathBuf>,
    /// Generate complete:N or circulant:N:D in memory (D must be even and < N).
    #[arg(long, group = "topology_source", value_name = "SPEC")]
    pub topology_generate: Option<String>,
    /// Coefficient sets for generated topologies (default: binary).
    #[arg(long, value_enum, requires = "topology_generate", conflicts_with_all = ["topology_preset", "topology_file"])]
    pub couplings: Option<Couplings>,
}

impl TopologySource {
    /// Resolve to a [`TopologyRef`] from these flags alone (no config
    /// file). Prefer [`resolve_topology`] when a config file may also
    /// supply the topology.
    ///
    /// # Errors
    ///
    /// Returns a message if the topology source is missing or ambiguous,
    /// or couplings are set without a generated topology.
    pub fn to_ref(&self) -> Result<TopologyRef, String> {
        if self.couplings.is_some() && self.topology_generate.is_none() {
            return Err("--couplings requires --topology-generate".to_string());
        }
        match (
            &self.topology_preset,
            &self.topology_file,
            &self.topology_generate,
        ) {
            (Some(name), None, None) => Ok(TopologyRef::Preset(name.clone())),
            (None, Some(path), None) => Ok(TopologyRef::File(path.clone())),
            (None, None, Some(description)) => Ok(TopologyRef::Generated {
                description: description.clone(),
                couplings: self.couplings.unwrap_or(Couplings::Binary),
            }),
            (None, None, None) => Err(
                "one of --topology-preset, --topology-file or --topology-generate is required"
                    .to_string(),
            ),
            _ => Err(
                "--topology-preset, --topology-file and --topology-generate are mutually exclusive"
                    .to_string(),
            ),
        }
    }
}

/// Load the optional YAML config named by `path`, or return an empty
/// [`FileConfig`] when no path was given.
fn load_optional_config(path: Option<&Path>) -> Result<FileConfig, String> {
    match path {
        None => Ok(FileConfig::default()),
        Some(p) => config::load_config(p).map_err(|e| e.to_string()),
    }
}

/// Resolve backend: flag beats file; error if neither supplied.
fn resolve_backend(
    flag: Option<String>,
    file: Option<String>,
    config_path: Option<&Path>,
) -> Result<String, String> {
    match (flag, file) {
        (Some(b), _) | (None, Some(b)) => {
            // File values skip clap's value_parser; re-check the registry.
            parse_backend(&b)
        }
        (None, None) => Err(missing_required("backend", "--backend", config_path)),
    }
}

/// File `param_curve` when present, else the mode's built-in default.
/// No CLI flag exists for this field.
fn merge_param_curve(file: &FileConfig, default: ParamCurve) -> ParamCurve {
    file.param_curve
        .as_ref()
        .map_or(default, FileParamCurve::to_param_curve)
}

/// Resolve topology: flag beats file; error if neither supplied. Flag
/// mutual exclusion is clap's job; file mutual exclusion is checked here.
fn resolve_topology(
    flag: &TopologySource,
    file: &FileConfig,
    config_path: Option<&Path>,
) -> Result<TopologyRef, String> {
    if flag.topology_preset.is_some()
        || flag.topology_file.is_some()
        || flag.topology_generate.is_some()
        || flag.couplings.is_some()
    {
        return flag.to_ref();
    }
    match (&file.topology_preset, &file.topology_file) {
        (Some(name), None) => Ok(TopologyRef::Preset(name.clone())),
        (None, Some(path)) => Ok(TopologyRef::File(path.clone())),
        (Some(_), Some(_)) => Err(
            "config file sets both topology_preset and topology_file; only one is allowed"
                .to_string(),
        ),
        (None, None) => Err(missing_required(
            "topology",
            "--topology-preset/--topology-file/--topology-generate",
            config_path,
        )),
    }
}

/// Build the "not set by FLAG and absent from …" message shape used for
/// backend and topology (and any other required-after-merge field).
fn missing_required(what: &str, flag: &str, config_path: Option<&Path>) -> String {
    match config_path {
        Some(p) => format!(
            "no {what}: not set by {flag} and absent from {}",
            p.display()
        ),
        None => format!("no {what}: not set by {flag} and no --config file was given"),
    }
}

/// Merge `--corpus` and `--instances` with the config file's values, flags
/// first, and reject a result that sets both.
fn resolve_finite_source(
    corpus_flag: Option<String>,
    instances_flag: Option<PathBuf>,
    file: &FileConfig,
    config_path: Option<&Path>,
) -> Result<(Option<String>, Option<PathBuf>), String> {
    // An explicit flag for one source displaces the file's key for the other:
    // the flag is the per-invocation instruction.
    let (corpus, instances) = match (corpus_flag, instances_flag) {
        (None, None) => (file.corpus.clone(), file.instances.clone()),
        (corpus, instances) => (corpus, instances),
    };
    if corpus.is_some() && instances.is_some() {
        let origin = config_path.map_or_else(String::new, |p| format!(" (or {})", p.display()));
        return Err(format!(
            "--corpus and --instances are mutually exclusive{origin}: a run replays one finite source"
        ));
    }
    Ok((corpus, instances))
}

/// Warn when a shared config file carries `hardness` for a mode that does
/// not use it. A flat `FileConfig` union may legitimately hold keys for
/// several modes, so this is a warning rather than an error (unlike an
/// explicit `--hardness` flag, which is a per-invocation instruction).
/// Same stderr mechanism as `config::warn_zero_bounds`.
fn warn_unused_config_hardness(mode: &str, reason: &str) {
    #[expect(
        clippy::print_stderr,
        reason = "unused config-key warning is operator-facing diagnostic on stderr; \
                  not a substitute for structured logging (none wired yet)"
    )]
    {
        eprintln!("warning: config file sets hardness, which is ignored by {mode} ({reason})");
    }
}

/// `isingmark throughput` arguments.
#[derive(Debug, Args)]
pub struct ThroughputArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// How long to keep dispatching jobs, e.g. `"30s"`, `"5m"`, `"2h"`.
    /// Required from `--duration` or `--config`.
    #[arg(long, value_parser = parse_duration_arg)]
    pub duration: Option<f64>,
    /// Replay a corpus of chain-harvested instances instead of deriving
    /// problems from `--seed`. A bundled corpus name (`chain-h0`,
    /// `chain-ternary`) or a path to a JSONL file.
    ///
    /// A bundled corpus must be paired with the topology it was drawn
    /// against. A corpus is finite, so `--duration` becomes an upper bound.
    #[arg(long)]
    pub corpus: Option<String>,
    /// Replay explicit instances from a JSONL file instead of deriving
    /// problems from `--seed`. The file carries the coefficients themselves,
    /// so it can hold problems that no topology preset draws.
    ///
    /// The file's header names the topology it was built for and must match
    /// `--topology-preset`. The file is finite, so `--duration` becomes an
    /// upper bound. Cannot be combined with `--corpus`.
    #[arg(long, value_name = "PATH")]
    pub instances: Option<PathBuf>,
}

impl ThroughputArgs {
    /// Build a [`ThroughputConfig`] from parsed arguments, merging
    /// `--config` file values under explicit flags.
    ///
    /// # Errors
    ///
    /// Returns a message if the config file cannot be loaded, if a
    /// required field (backend, topology, duration) is set by neither flag
    /// nor file, or if a file value fails validation.
    pub fn into_config(self) -> Result<ThroughputConfig, String> {
        let config_path = self.common.config.as_deref();
        let file = load_optional_config(config_path)?;
        let backend = resolve_backend(self.common.backend, file.backend.clone(), config_path)?;
        let topology = resolve_topology(&self.common.topology_source, &file, config_path)?;
        let hardness = merge(self.common.hardness, file.hardness, DEFAULT_HARDNESS);
        let seed = merge(self.common.seed, file.seed, DEFAULT_SEED);
        let output_dir = merge(
            self.common.output_dir,
            file.output_dir.clone(),
            PathBuf::from(DEFAULT_OUTPUT_DIR),
        );
        let duration_s = match (self.duration, file.duration.as_deref()) {
            (Some(d), _) => d,
            (None, Some(s)) => throughput::parse_duration(s).map_err(|e| e.to_string())?,
            (None, None) => {
                return Err(missing_required("duration", "--duration", config_path));
            }
        };
        let param_curve = merge_param_curve(&file, THROUGHPUT_DEFAULT_CURVE);
        let (corpus, instances) =
            resolve_finite_source(self.corpus, self.instances, &file, config_path)?;
        Ok(ThroughputConfig {
            backend,
            topology,
            hardness,
            param_curve,
            duration_s,
            seed,
            corpus,
            instances,
            output_dir,
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
    /// cuda-gibbs, cuda-msa, metal, beit), or `num_reads` + `anneal_time_us` for an
    /// annealing one
    /// (dwave, cuopt). `--hardness` (from `CommonArgs`) is not used by this mode
    /// — cell parameters come entirely from this grid instead of a
    /// hardness curve. Required from `--param-grid` or `--config`.
    #[arg(long, value_parser = parse_param_grid_arg)]
    pub param_grid: Option<ParamGrid>,
    /// Jobs drawn per cell (default: 20). Every cell draws the identical
    /// sequence (same seed, same count) — see `modes::sweep`'s module docs
    /// for why.
    #[arg(long, value_parser = parse_num_jobs, help = "Jobs drawn per cell (default: 20)")]
    pub num_jobs: Option<u32>,
    /// Write a sweep-heatmap PNG to this path. Requires a binary built
    /// with `--features plot`; without that feature the flag still parses
    /// and the process exits non-zero with a rebuild message.
    #[arg(long, value_name = "PATH")]
    pub plot: Option<PathBuf>,
    /// Replay a corpus of chain-harvested instances instead of deriving
    /// problems from `--seed`. A bundled corpus name (`chain-h0`,
    /// `chain-ternary`) or a path to a JSONL file.
    ///
    /// A bundled corpus must be paired with the topology it was drawn
    /// against. A corpus is finite, so `--num-jobs` becomes an upper bound:
    /// every cell draws the same `min(num_jobs, corpus length)` instances.
    #[arg(long)]
    pub corpus: Option<String>,
    /// Replay explicit instances from a JSONL file instead of deriving
    /// problems from `--seed`. The file carries the coefficients themselves,
    /// so it can hold problems that no topology preset draws.
    ///
    /// The file's header names the topology it was built for and must match
    /// `--topology-preset`. The file is finite, so `--num-jobs` becomes an
    /// upper bound, as for a corpus. Cannot be combined with `--corpus`.
    #[arg(long, value_name = "PATH")]
    pub instances: Option<PathBuf>,
}

impl SweepArgs {
    /// Build a [`SweepConfig`] from parsed arguments, merging `--config`
    /// file values under explicit flags.
    ///
    /// # Errors
    ///
    /// Returns a message if the config file cannot be loaded, if a required
    /// field is set by neither flag nor file, or if `--hardness` was set
    /// (this mode takes cell parameters from the grid, not a hardness curve).
    pub fn into_config(self) -> Result<SweepConfig, String> {
        let config_path = self.common.config.as_deref();
        let file = load_optional_config(config_path)?;
        // Explicit `--hardness` is an error: the flag is a per-invocation
        // instruction and this mode never reads it. A `hardness` key in a
        // shared config file is only a warning (union schema).
        if self.common.hardness.is_some() {
            return Err(
                "--hardness is not used by sweep: cell parameters come from --param-grid \
                 (or param_grid in the config file), not from a hardness curve"
                    .to_string(),
            );
        }
        if file.hardness.is_some() {
            warn_unused_config_hardness(
                "sweep",
                "cell parameters come from param_grid, not a hardness curve",
            );
        }
        let backend = resolve_backend(self.common.backend, file.backend.clone(), config_path)?;
        let topology = resolve_topology(&self.common.topology_source, &file, config_path)?;
        let seed = merge(self.common.seed, file.seed, DEFAULT_SEED);
        let output_dir = merge(
            self.common.output_dir,
            file.output_dir.clone(),
            PathBuf::from(DEFAULT_OUTPUT_DIR),
        );
        let grid = self
            .param_grid
            .or(file.param_grid.clone())
            .ok_or_else(|| missing_required("param-grid", "--param-grid", config_path))?;
        let num_jobs = merge(self.num_jobs, file.num_jobs, DEFAULT_NUM_JOBS);
        let (corpus, instances) =
            resolve_finite_source(self.corpus, self.instances, &file, config_path)?;
        Ok(SweepConfig {
            backend,
            topology,
            grid,
            num_jobs,
            seed,
            corpus,
            instances,
            output_dir,
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
    // Not `required = true`: a `--config` file may supply the list. An
    // empty Vec after clap parse means the flag was absent; `into_config`
    // then takes the file value or errors.
    #[arg(
        long,
        allow_hyphen_values = true,
        value_delimiter = ',',
        value_parser = parse_one_energy_target_arg
    )]
    pub energy_targets: Vec<i64>,
    /// Trials attempted per target, before any `--max-time-s` cutoff
    /// (default: 20).
    #[arg(long, value_parser = parse_num_trials, help = "Trials attempted per target (default: 20)")]
    pub num_trials: Option<u32>,
    /// Wall-clock budget per target, in seconds, checked between trials
    /// (default: 30.0).
    #[arg(long, value_parser = parse_max_time_s, help = "Wall-clock budget per target in seconds (default: 30.0)")]
    pub max_time_s: Option<f64>,
    /// Write a time-to-target CDF PNG to this path. Requires a binary
    /// built with `--features plot`; without that feature the flag still
    /// parses and the process exits non-zero with a rebuild message.
    #[arg(long, value_name = "PATH")]
    pub plot: Option<PathBuf>,
}

impl TttArgs {
    /// Build a [`TttConfig`] from parsed arguments, merging `--config`
    /// file values under explicit flags.
    ///
    /// # Errors
    ///
    /// Returns a message if the config file cannot be loaded, if a required
    /// field is set by neither flag nor file, or if `--hardness` was set
    /// (this mode leaves sample parameters unset).
    pub fn into_config(self) -> Result<TttConfig, String> {
        let config_path = self.common.config.as_deref();
        let file = load_optional_config(config_path)?;
        // Explicit `--hardness` is an error: ttt dispatches with unset sample
        // parameters (backend defaults). A config-file `hardness` is only a
        // warning (shared FileConfig union).
        if self.common.hardness.is_some() {
            return Err(
                "--hardness is not used by ttt: sample parameters are left unset so \
                 each trial defers to the backend's own defaults, not a hardness curve"
                    .to_string(),
            );
        }
        if file.hardness.is_some() {
            warn_unused_config_hardness(
                "ttt",
                "sample parameters are left unset (backend defaults), not taken from hardness",
            );
        }
        let backend = resolve_backend(self.common.backend, file.backend.clone(), config_path)?;
        let topology = resolve_topology(&self.common.topology_source, &file, config_path)?;
        let seed = merge(self.common.seed, file.seed, DEFAULT_SEED);
        let output_dir = merge(
            self.common.output_dir,
            file.output_dir.clone(),
            PathBuf::from(DEFAULT_OUTPUT_DIR),
        );
        let energy_targets_milli = if self.energy_targets.is_empty() {
            match &file.energy_targets {
                Some(vals) if !vals.is_empty() => vals
                    .iter()
                    .map(|v| {
                        // Reuse the shared natural-to-milli conversion so
                        // file and flag paths cannot drift on rounding.
                        ttt::parse_one_energy_target(&v.to_string()).map_err(|e| e.to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                _ => {
                    return Err(missing_required(
                        "energy-targets",
                        "--energy-targets",
                        config_path,
                    ));
                }
            }
        } else {
            self.energy_targets
        };
        let num_trials = merge(self.num_trials, file.num_trials, DEFAULT_NUM_TRIALS);
        let max_time_s = merge(self.max_time_s, file.max_time_s, DEFAULT_MAX_TIME_S);
        Ok(TttConfig {
            backend,
            topology,
            energy_targets_milli,
            num_trials,
            max_time_s,
            seed,
            output_dir,
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
    /// is the value this search computes, not an input to it. Required from
    /// `--target-file` or `--config`.
    #[arg(long)]
    pub target_file: Option<PathBuf>,
    /// Jobs drawn per hardness evaluation (default: 20). Every evaluation
    /// draws the identical sequence (same seed, same count) — see
    /// `modes::comparative`'s module docs for why.
    #[arg(long, value_parser = parse_num_jobs, help = "Jobs drawn per hardness evaluation (default: 20)")]
    pub num_jobs: Option<u32>,
    /// Maximum number of evaluations to run before reporting whatever
    /// hardness the bracket has narrowed to (default: 10). The search may
    /// stop earlier once the bracket narrows below `0.01`.
    #[arg(long, value_parser = parse_max_iterations, help = "Max hardness-search evaluations (default: 10)")]
    pub max_iterations: Option<u32>,
}

impl ComparativeArgs {
    /// Build a [`ComparativeConfig`] from parsed arguments, merging
    /// `--config` file values under explicit flags.
    ///
    /// # Errors
    ///
    /// Returns a message if the config file cannot be loaded, if a required
    /// field is set by neither flag nor file, or if `--hardness` was set
    /// (hardness is an output of this mode's search, not an input).
    pub fn into_config(self) -> Result<ComparativeConfig, String> {
        let config_path = self.common.config.as_deref();
        let file = load_optional_config(config_path)?;
        // Explicit `--hardness` is an error: comparative *searches for* the
        // hardness that matches the reference, so hardness is an output, not
        // an input. A config-file `hardness` is only a warning (union schema).
        if self.common.hardness.is_some() {
            return Err(
                "--hardness is not used by comparative: hardness is an *output* of this \
                 mode (the search reports the hardness matching the reference distribution), \
                 not an input"
                    .to_string(),
            );
        }
        if file.hardness.is_some() {
            warn_unused_config_hardness(
                "comparative",
                "hardness is searched for against the reference file, not taken from config",
            );
        }
        let backend = resolve_backend(self.common.backend, file.backend.clone(), config_path)?;
        let topology = resolve_topology(&self.common.topology_source, &file, config_path)?;
        let seed = merge(self.common.seed, file.seed, DEFAULT_SEED);
        let output_dir = merge(
            self.common.output_dir,
            file.output_dir.clone(),
            PathBuf::from(DEFAULT_OUTPUT_DIR),
        );
        let target_file = self
            .target_file
            .or(file.target_file.clone())
            .ok_or_else(|| missing_required("target-file", "--target-file", config_path))?;
        let num_jobs = merge(self.num_jobs, file.num_jobs, DEFAULT_NUM_JOBS);
        let max_iterations = merge(
            self.max_iterations,
            file.max_iterations,
            DEFAULT_MAX_ITERATIONS,
        );
        let param_curve = merge_param_curve(&file, COMPARATIVE_DEFAULT_CURVE);
        Ok(ComparativeConfig {
            backend,
            topology,
            target_file,
            num_jobs,
            max_iterations,
            seed,
            output_dir,
            param_curve,
            backend_bin_override: None,
            backend_args_override: None,
        })
    }
}

/// `isingmark scaling` arguments. Does not flatten [`CommonArgs`]: unlike
/// every other mode, scaling generates its own topology per requested node
/// count (see `modes::scaling`'s module doc) rather than accepting one via
/// `--topology-preset`/`--topology-file`, so `CommonArgs::topology_source`
/// does not apply here. Every other `CommonArgs` field this mode does use
/// (`--backend`, `--hardness`, `--seed`, `--output-dir`, `--config`,
/// `--verbose`) is repeated directly below instead, with the identical
/// value parsers and the same no-clap-default rule (see [`CommonArgs`]).
#[derive(Debug, Args)]
pub struct ScalingArgs {
    /// Backend to benchmark. Required from `--backend` or `--config`.
    #[arg(long, value_parser = parse_backend)]
    pub backend: Option<String>,
    /// Node counts to generate a random 3-regular topology for,
    /// comma-separated (e.g. `"8,16,32"`). Each must be even and at least 4
    /// (a 3-regular graph is impossible otherwise — every node needs 3
    /// distinct neighbours), and the list must not repeat a value (two
    /// identical sizes would collide on one output subdirectory). Required
    /// from `--sizes` or `--config`.
    #[arg(
        long,
        value_delimiter = ',',
        value_parser = parse_one_scaling_size_arg
    )]
    pub sizes: Vec<usize>,
    /// Difficulty in `[0.0, 1.0]`, held fixed across every size (default: 0.5).
    #[arg(long, value_parser = parse_hardness, help = "Difficulty in [0.0, 1.0] (default: 0.5)")]
    pub hardness: Option<f64>,
    /// Jobs drawn per size (default: 20).
    #[arg(long, value_parser = parse_num_jobs, help = "Jobs drawn per size (default: 20)")]
    pub num_jobs: Option<u32>,
    /// Seed for deterministic problem generation and deterministic
    /// topology generation (default: 0).
    #[arg(long, help = "Seed for problem and topology generation (default: 0)")]
    pub seed: Option<u64>,
    /// Directory to write JSONL records and the run summary to (default: ./out).
    #[arg(
        long,
        help = "Output directory for JSONL records and summary (default: ./out)"
    )]
    pub output_dir: Option<PathBuf>,
    /// YAML config file. Explicit flags override values from this file;
    /// the file overrides built-in defaults.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Increase output verbosity. Repeatable. Accepted for forward
    /// compatibility; not yet wired to any logging behavior.
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    pub verbosity: u8,
}

impl ScalingArgs {
    /// Build a [`ScalingConfig`] from parsed arguments, merging `--config`
    /// file values under explicit flags.
    ///
    /// # Errors
    ///
    /// Returns a message if the config file cannot be loaded, or if a
    /// required field is set by neither flag nor file.
    pub fn into_config(self) -> Result<ScalingConfig, String> {
        let config_path = self.config.as_deref();
        let file = load_optional_config(config_path)?;
        let backend = resolve_backend(self.backend, file.backend.clone(), config_path)?;
        let hardness = merge(self.hardness, file.hardness, DEFAULT_HARDNESS);
        let seed = merge(self.seed, file.seed, DEFAULT_SEED);
        let output_dir = merge(
            self.output_dir,
            file.output_dir.clone(),
            PathBuf::from(DEFAULT_OUTPUT_DIR),
        );
        let num_jobs = merge(self.num_jobs, file.num_jobs, DEFAULT_NUM_JOBS);
        let sizes = if self.sizes.is_empty() {
            match &file.sizes {
                Some(s) if !s.is_empty() => {
                    // Re-validate each size with the same rules as the flag
                    // value_parser so a file cannot smuggle an odd/too-small
                    // size past clap.
                    for &n in s {
                        scaling::parse_one_scaling_size(&n.to_string())
                            .map_err(|e| e.to_string())?;
                    }
                    s.clone()
                }
                _ => return Err(missing_required("sizes", "--sizes", config_path)),
            }
        } else {
            self.sizes
        };
        let param_curve = merge_param_curve(&file, SCALING_DEFAULT_CURVE);
        Ok(ScalingConfig {
            backend,
            sizes,
            hardness,
            num_jobs,
            seed,
            output_dir,
            param_curve,
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
    /// Jobs drawn per run (default: 20). Both runs draw the identical
    /// sequence (same seed, same count) against the same topology — see
    /// `modes::reproducibility`'s module doc for why.
    #[arg(long, value_parser = parse_num_jobs, help = "Jobs drawn per run (default: 20)")]
    pub num_problems: Option<u32>,
    /// Difficulty in `[0.0, 1.0]` for the second run. Defaults to
    /// `--hardness` (the ordinary "is this backend reproducible at a fixed
    /// effort" use case — see `modes::reproducibility`'s module doc); set
    /// this to a different value only to intentionally compare two
    /// different sampling efforts against the same problems.
    #[arg(long, value_parser = parse_hardness)]
    pub hardness_b: Option<f64>,
}

impl ReproducibilityArgs {
    /// Build a [`ReproConfig`] from parsed arguments, merging `--config`
    /// file values under explicit flags.
    ///
    /// # Errors
    ///
    /// Returns a message if the config file cannot be loaded, or if a
    /// required field is set by neither flag nor file.
    pub fn into_config(self) -> Result<ReproConfig, String> {
        let config_path = self.common.config.as_deref();
        let file = load_optional_config(config_path)?;
        let backend = resolve_backend(self.common.backend, file.backend.clone(), config_path)?;
        let topology = resolve_topology(&self.common.topology_source, &file, config_path)?;
        let hardness_a = merge(self.common.hardness, file.hardness, DEFAULT_HARDNESS);
        // hardness_b: flag > file > hardness_a (not the global default).
        let hardness_b = self.hardness_b.or(file.hardness_b).unwrap_or(hardness_a);
        let seed = merge(self.common.seed, file.seed, DEFAULT_SEED);
        let output_dir = merge(
            self.common.output_dir,
            file.output_dir.clone(),
            PathBuf::from(DEFAULT_OUTPUT_DIR),
        );
        let num_problems = merge(self.num_problems, file.num_problems, DEFAULT_NUM_PROBLEMS);
        let param_curve = merge_param_curve(&file, REPRO_DEFAULT_CURVE);
        Ok(ReproConfig {
            backend,
            topology,
            num_problems,
            hardness_a,
            hardness_b,
            seed,
            output_dir,
            param_curve,
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

/// `--hardness` / `--hardness-b` value parser. clap's default `f64` parser
/// is `f64::from_str`, which accepts `nan`/`inf`/`-inf`;
/// `params::ParamCurve::resolve` is total and silently maps `NaN` to the
/// curve minimum, so an un-guarded `--hardness nan` would exit cleanly with
/// a full, successful-looking result set while every job silently ran at
/// the curve's easiest setting. Reject anything non-finite or outside
/// `[0.0, 1.0]` here, at parse time, instead — reusing
/// [`config::validate_hardness`] so the file-config path cannot diverge.
fn parse_hardness(s: &str) -> Result<f64, String> {
    let value: f64 = s
        .parse()
        .map_err(|_| format!("{s:?} is not a valid number"))?;
    // Prefer the raw token in the non-finite message (matches prior CLI
    // wording that quoted `nan`/`inf` as the user typed them).
    if !value.is_finite() {
        return Err(format!("--hardness must be a finite number, got {s:?}"));
    }
    config::validate_hardness(value, "--hardness")
}

/// `--duration` value parser, delegating to `throughput::parse_duration`.
fn parse_duration_arg(s: &str) -> Result<f64, String> {
    throughput::parse_duration(s).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{
        Cli, Command, ComparativeArgs, CompareArgs, ReproducibilityArgs, ScalingArgs, SweepArgs,
        ThroughputArgs, TttArgs,
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
            Command::Campaign(_)
            | Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_)
            | Command::Compare(_) => {
                unreachable!("test only ever parses a throughput invocation")
            }
        }
    }

    /// Extract the `Sweep` variant. See [`throughput_args`].
    fn sweep_args(cli: Cli) -> SweepArgs {
        match cli.command {
            Command::Sweep(args) => args,
            Command::Campaign(_)
            | Command::Throughput(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_)
            | Command::Compare(_) => {
                unreachable!("test only ever parses a sweep invocation")
            }
        }
    }

    /// Extract the `Ttt` variant. See [`throughput_args`].
    fn ttt_args(cli: Cli) -> TttArgs {
        match cli.command {
            Command::Ttt(args) => args,
            Command::Campaign(_)
            | Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Comparative(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_)
            | Command::Compare(_) => {
                unreachable!("test only ever parses a ttt invocation")
            }
        }
    }

    /// Extract the `Comparative` variant. See [`throughput_args`].
    fn comparative_args(cli: Cli) -> ComparativeArgs {
        match cli.command {
            Command::Comparative(args) => args,
            Command::Campaign(_)
            | Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_)
            | Command::Compare(_) => {
                unreachable!("test only ever parses a comparative invocation")
            }
        }
    }

    /// Extract the `Scaling` variant. See [`throughput_args`].
    fn scaling_args(cli: Cli) -> ScalingArgs {
        match cli.command {
            Command::Scaling(args) => args,
            Command::Campaign(_)
            | Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Reproducibility(_)
            | Command::Compare(_) => {
                unreachable!("test only ever parses a scaling invocation")
            }
        }
    }

    /// Extract the `Reproducibility` variant. See [`throughput_args`].
    fn reproducibility_args(cli: Cli) -> ReproducibilityArgs {
        match cli.command {
            Command::Reproducibility(args) => args,
            Command::Campaign(_)
            | Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Scaling(_)
            | Command::Compare(_) => {
                unreachable!("test only ever parses a reproducibility invocation")
            }
        }
    }

    /// Extract the `Compare` variant. See [`throughput_args`].
    fn compare_args(cli: Cli) -> CompareArgs {
        match cli.command {
            Command::Compare(args) => args,
            Command::Campaign(_)
            | Command::Throughput(_)
            | Command::Sweep(_)
            | Command::Ttt(_)
            | Command::Comparative(_)
            | Command::Scaling(_)
            | Command::Reproducibility(_) => {
                unreachable!("test only ever parses a compare invocation")
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
        assert_eq!(args.common.backend.as_deref(), Some("cpu-sa"));
        assert_eq!(
            args.common.topology_source.topology_preset.as_deref(),
            Some("smoke")
        );
        assert_eq!(args.common.hardness, Some(0.5));
        assert_eq!(args.duration, Some(30.0));
        assert_eq!(args.common.seed, Some(42));
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
        // Topology is no longer clap-required (a --config file may supply
        // it); into_config must still reject a run with no topology from
        // either source.
        let cli = parse(&["throughput", "--backend", "cpu-sa", "--duration", "30s"])
            .expect("clap accepts missing topology when no --config");
        let err = throughput_args(cli)
            .into_config()
            .expect_err("topology required after merge");
        assert!(
            err.contains("topology"),
            "error must name topology, got: {err}"
        );
        assert!(
            err.contains("no --config file was given"),
            "error must note absence of config file, got: {err}"
        );
    }

    #[test]
    fn missing_backend_is_rejected_naming_flag_and_no_config() {
        let cli = parse(&[
            "throughput",
            "--topology-preset",
            "smoke",
            "--duration",
            "30s",
        ])
        .expect("clap accepts missing backend when no --config");
        let err = throughput_args(cli)
            .into_config()
            .expect_err("backend required after merge");
        assert!(err.contains("backend"), "got: {err}");
        assert!(err.contains("--backend"), "got: {err}");
        assert!(err.contains("no --config file was given"), "got: {err}");
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
        assert_eq!(
            cfg.param_curve,
            crate::modes::throughput::DEFAULT_PARAM_CURVE,
            "no file/flag curve must fall through to DEFAULT_PARAM_CURVE"
        );
    }

    /// End-to-end: a real config file supplies hardness when the flag is
    /// absent, and that value reaches the resulting mode config. Distinct
    /// from the default (0.5) so a merge that ignores the file cannot pass.
    #[test]
    fn throughput_config_file_supplies_hardness_when_flag_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("t.yaml");
        std::fs::write(
            &path,
            "backend: cpu-sa\ntopology_preset: smoke\nhardness: 0.7\nduration: 30s\n",
        )
        .expect("write");
        let cli = parse(&["throughput", "--config", path.to_str().expect("utf8 path")])
            .expect("clap accepts --config alone");
        let cfg = throughput_args(cli)
            .into_config()
            .expect("complete config file must build ThroughputConfig");
        assert_eq!(cfg.backend, "cpu-sa");
        assert_eq!(
            cfg.topology,
            crate::modes::TopologyRef::Preset("smoke".to_string())
        );
        assert!(
            (cfg.hardness - 0.7).abs() < f64::EPSILON,
            "file hardness 0.7 must reach mode config, got {}",
            cfg.hardness
        );
        assert!((cfg.duration_s - 30.0).abs() < f64::EPSILON);
    }

    /// Explicit flag overrides the config file (distinct values).
    #[test]
    fn throughput_explicit_flag_overrides_config_file_hardness() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("t.yaml");
        std::fs::write(
            &path,
            "backend: cpu-sa\ntopology_preset: smoke\nhardness: 0.7\nduration: 30s\n",
        )
        .expect("write");
        let cli = parse(&[
            "throughput",
            "--config",
            path.to_str().expect("utf8 path"),
            "--hardness",
            "0.9",
        ])
        .expect("parse");
        let cfg = throughput_args(cli).into_config().expect("merge");
        assert!(
            (cfg.hardness - 0.9).abs() < f64::EPSILON,
            "flag 0.9 must beat file 0.7, got {}",
            cfg.hardness
        );
    }

    /// Flag `--backend` beats a distinct file `backend` (I3).
    #[test]
    fn throughput_explicit_flag_overrides_config_file_backend() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("backend.yaml");
        std::fs::write(
            &path,
            "backend: dwave\ntopology_preset: smoke\nduration: 30s\n",
        )
        .expect("write");
        let cli = parse(&[
            "throughput",
            "--config",
            path.to_str().expect("utf8 path"),
            "--backend",
            "cpu-sa",
        ])
        .expect("parse");
        let cfg = throughput_args(cli).into_config().expect("merge");
        assert_eq!(
            cfg.backend, "cpu-sa",
            "flag --backend must beat file backend: dwave, got {}",
            cfg.backend
        );
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
        assert_eq!(args.common.backend.as_deref(), Some("cpu-sa"));
        let grid = args.param_grid.as_ref().expect("param_grid set");
        assert_eq!(grid.num_reads, vec![64, 128]);
        assert_eq!(grid.num_sweeps, vec![100, 500]);
        assert_eq!(args.num_jobs, Some(10));
        assert_eq!(args.common.seed, Some(42));
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
    fn sweep_config_file_supplies_param_grid_when_flag_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("s.yaml");
        std::fs::write(
            &path,
            "backend: cpu-sa\ntopology_preset: smoke\nparam_grid:\n  num_reads: [64, 128]\n  num_sweeps: [100]\nnum_jobs: 5\n",
        )
        .expect("write");
        let cli = parse(&["sweep", "--config", path.to_str().expect("utf8")]).expect("parse");
        let cfg = sweep_args(cli).into_config().expect("complete config");
        assert_eq!(cfg.grid.num_reads, vec![64, 128]);
        assert_eq!(cfg.grid.num_sweeps, vec![100]);
        assert_eq!(cfg.num_jobs, 5);
    }

    /// Explicit `--hardness` on sweep is an error: cell params come from the
    /// grid, not a hardness curve (F2).
    #[test]
    fn sweep_explicit_hardness_flag_is_rejected() {
        let cli = parse(&[
            "sweep",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--param-grid",
            r#"{"num_reads":[64],"num_sweeps":[100]}"#,
            "--hardness",
            "0.9",
        ])
        .expect("clap still accepts CommonArgs hardness");
        let message = sweep_args(cli)
            .into_config()
            .expect_err("--hardness must be rejected for sweep");
        assert!(
            message.contains("--hardness") && message.contains("param-grid"),
            "got: {message}"
        );
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
        assert_eq!(args.common.backend.as_deref(), Some("cpu-sa"));
        assert_eq!(args.energy_targets, vec![-5_000, -10_000, -15_000]);
        assert_eq!(args.num_trials, Some(10));
        assert_eq!(args.max_time_s, Some(5.0));
        assert_eq!(args.common.seed, Some(42));
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
    fn ttt_config_file_supplies_energy_targets_when_flag_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ttt.yaml");
        std::fs::write(
            &path,
            "backend: cpu-sa\ntopology_preset: smoke\nenergy_targets: [-5.0, -10.0]\nnum_trials: 7\nmax_time_s: 12.5\n",
        )
        .expect("write");
        let cli = parse(&["ttt", "--config", path.to_str().expect("utf8")]).expect("parse");
        let cfg = ttt_args(cli).into_config().expect("complete config");
        assert_eq!(cfg.energy_targets_milli, vec![-5_000, -10_000]);
        assert_eq!(cfg.num_trials, 7);
        assert!((cfg.max_time_s - 12.5).abs() < f64::EPSILON);
    }

    /// Explicit `--hardness` on ttt is an error: sample params stay unset (F2).
    #[test]
    fn ttt_explicit_hardness_flag_is_rejected() {
        let cli = parse(&[
            "ttt",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--energy-targets",
            "-5",
            "--hardness",
            "0.9",
        ])
        .expect("clap still accepts CommonArgs hardness");
        let message = ttt_args(cli)
            .into_config()
            .expect_err("--hardness must be rejected for ttt");
        assert!(
            message.contains("--hardness") && message.contains("unset"),
            "got: {message}"
        );
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
        assert_eq!(args.common.backend.as_deref(), Some("cpu-sa"));
        assert_eq!(
            args.target_file.as_deref(),
            Some(std::path::Path::new("reference.jsonl"))
        );
        assert_eq!(args.num_jobs, Some(15));
        assert_eq!(args.max_iterations, Some(5));
        assert_eq!(args.common.seed, Some(42));
    }

    #[test]
    fn comparative_target_file_is_required() {
        let cli = parse(&[
            "comparative",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
        ])
        .expect("clap accepts missing target-file when no --config");
        let err = comparative_args(cli)
            .into_config()
            .expect_err("--target-file required after merge");
        assert!(err.contains("target-file"), "got: {err}");
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

    /// Explicit `--hardness` on comparative is an error: hardness is an
    /// output of the search, not an input (F2).
    #[test]
    fn comparative_explicit_hardness_flag_is_rejected() {
        let cli = parse(&[
            "comparative",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--target-file",
            "reference.jsonl",
            "--hardness",
            "0.9",
        ])
        .expect("clap still accepts CommonArgs hardness");
        let message = comparative_args(cli)
            .into_config()
            .expect_err("--hardness must be rejected for comparative");
        assert!(
            message.contains("--hardness") && message.contains("output"),
            "got: {message}"
        );
    }

    #[test]
    fn comparative_config_file_supplies_target_file_when_flag_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("c.yaml");
        std::fs::write(
            &path,
            "backend: dwave\ntopology_preset: smoke\ntarget_file: reference.jsonl\nnum_jobs: 12\nmax_iterations: 6\n",
        )
        .expect("write");
        let cli = parse(&["comparative", "--config", path.to_str().expect("utf8")]).expect("parse");
        let cfg = comparative_args(cli)
            .into_config()
            .expect("complete config");
        assert_eq!(cfg.backend, "dwave");
        assert_eq!(cfg.target_file, std::path::PathBuf::from("reference.jsonl"));
        assert_eq!(cfg.num_jobs, 12);
        assert_eq!(cfg.max_iterations, 6);
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
        assert_eq!(args.backend.as_deref(), Some("cpu-sa"));
        assert_eq!(args.sizes, vec![8, 16, 32]);
        assert_eq!(args.hardness, Some(0.5));
        assert_eq!(args.num_jobs, Some(10));
        assert_eq!(args.seed, Some(42));
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
        let cli = parse(&["scaling", "--backend", "cpu-sa"])
            .expect("clap accepts missing sizes when no --config");
        let err = scaling_args(cli)
            .into_config()
            .expect_err("--sizes required after merge");
        assert!(err.contains("sizes"), "got: {err}");
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
    fn scaling_config_file_supplies_sizes_when_flag_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("sc.yaml");
        std::fs::write(
            &path,
            "backend: cpu-sa\nsizes: [8, 16]\nhardness: 0.75\nnum_jobs: 5\n",
        )
        .expect("write");
        let cli = parse(&["scaling", "--config", path.to_str().expect("utf8")]).expect("parse");
        let cfg = scaling_args(cli).into_config().expect("complete config");
        assert_eq!(cfg.sizes, vec![8, 16]);
        assert!((cfg.hardness - 0.75).abs() < f64::EPSILON);
        assert_eq!(cfg.num_jobs, 5);
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
        assert_eq!(args.common.backend.as_deref(), Some("cpu-sa"));
        assert_eq!(args.num_problems, Some(15));
        assert_eq!(args.common.hardness, Some(0.4));
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
    fn reproducibility_config_file_supplies_fields_when_flags_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("r.yaml");
        std::fs::write(
            &path,
            "backend: cpu-sa\ntopology_preset: smoke\nhardness: 0.6\nnum_problems: 7\n",
        )
        .expect("write");
        let cli =
            parse(&["reproducibility", "--config", path.to_str().expect("utf8")]).expect("parse");
        let cfg = reproducibility_args(cli)
            .into_config()
            .expect("complete config");
        assert!((cfg.hardness_a - 0.6).abs() < f64::EPSILON);
        assert!(
            (cfg.hardness_b - 0.6).abs() < f64::EPSILON,
            "hardness_b defaults to hardness_a from file"
        );
        assert_eq!(cfg.num_problems, 7);
    }

    #[test]
    fn a_full_valid_compare_invocation_parses() {
        let cli = parse(&[
            "compare",
            "--file-a",
            "/tmp/a.jsonl",
            "--file-b",
            "/tmp/b.jsonl",
        ])
        .expect("valid invocation parses");
        let args = compare_args(cli);
        assert_eq!(args.file_a, std::path::PathBuf::from("/tmp/a.jsonl"));
        assert_eq!(args.file_b, std::path::PathBuf::from("/tmp/b.jsonl"));
    }

    #[test]
    fn compare_requires_both_file_flags() {
        let err =
            parse(&["compare", "--file-a", "/tmp/a.jsonl"]).expect_err("--file-b is required");
        let message = err.to_string();
        assert!(
            message.contains("file-b") || message.contains("required"),
            "missing --file-b must be rejected: {message}"
        );
        let err =
            parse(&["compare", "--file-b", "/tmp/b.jsonl"]).expect_err("--file-a is required");
        let message = err.to_string();
        assert!(
            message.contains("file-a") || message.contains("required"),
            "missing --file-a must be rejected: {message}"
        );
    }

    const THROUGHPUT_BASE: [&str; 7] = [
        "throughput",
        "--backend",
        "cpu-sa",
        "--topology-preset",
        "smoke",
        "--duration",
        "30s",
    ];

    #[test]
    fn throughput_instances_flag_reaches_the_config() {
        let mut argv = THROUGHPUT_BASE.to_vec();
        argv.extend(["--instances", "/tmp/family.jsonl"]);
        let cfg = throughput_args(parse(&argv).expect("parses"))
            .into_config()
            .expect("builds");
        assert_eq!(
            cfg.instances,
            Some(std::path::PathBuf::from("/tmp/family.jsonl"))
        );
        assert_eq!(cfg.corpus, None);
    }

    #[test]
    fn throughput_corpus_and_instances_together_is_a_usage_error() {
        let mut argv = THROUGHPUT_BASE.to_vec();
        argv.extend(["--corpus", "chain-h0", "--instances", "/tmp/family.jsonl"]);
        let err = throughput_args(parse(&argv).expect("clap accepts both"))
            .into_config()
            .expect_err("both sources must be rejected");
        assert!(err.contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn throughput_config_file_supplies_instances_and_a_flag_displaces_the_files_corpus() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("t.yaml");
        std::fs::write(
            &path,
            "backend: cpu-sa\ntopology_preset: smoke\nduration: 30s\ncorpus: chain-h0\n",
        )
        .expect("write");
        let config = path.to_str().expect("utf8 path");
        let cfg = throughput_args(
            parse(&[
                "throughput",
                "--config",
                config,
                "--instances",
                "/tmp/f.jsonl",
            ])
            .expect("parses"),
        )
        .into_config()
        .expect("a flag for one source displaces the file's other source");
        assert_eq!(cfg.corpus, None);
        assert_eq!(
            cfg.instances,
            Some(std::path::PathBuf::from("/tmp/f.jsonl"))
        );

        std::fs::write(
            &path,
            "backend: cpu-sa\ntopology_preset: smoke\nduration: 30s\ncorpus: chain-h0\ninstances: /tmp/f.jsonl\n",
        )
        .expect("write");
        let err = throughput_args(parse(&["throughput", "--config", config]).expect("parses"))
            .into_config()
            .expect_err("a file that sets both must be rejected");
        assert!(err.contains("mutually exclusive"), "{err}");
    }

    #[test]
    fn sweep_instances_flag_reaches_the_config_and_conflicts_with_corpus() {
        let base = [
            "sweep",
            "--backend",
            "cpu-sa",
            "--topology-preset",
            "smoke",
            "--param-grid",
            r#"{"num_reads":[8],"num_sweeps":[10]}"#,
        ];
        let mut argv = base.to_vec();
        argv.extend(["--instances", "/tmp/family.jsonl"]);
        let cfg = sweep_args(parse(&argv).expect("parses"))
            .into_config()
            .expect("builds");
        assert_eq!(
            cfg.instances,
            Some(std::path::PathBuf::from("/tmp/family.jsonl"))
        );

        argv.extend(["--corpus", "chain-h0"]);
        let err = sweep_args(parse(&argv).expect("clap accepts both"))
            .into_config()
            .expect_err("both sources must be rejected");
        assert!(err.contains("mutually exclusive"), "{err}");
    }
}
