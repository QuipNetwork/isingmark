// SPDX-License-Identifier: AGPL-3.0-or-later
//! Command-line argument parsing: the `isingmark` clap [`Parser`], its
//! subcommands, and the options common to every mode.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::backend::builtin_registry;
use crate::modes::sweep::{self, ParamGrid, SweepConfig};
use crate::modes::throughput::{self, ThroughputConfig};
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
    #[arg(long, default_value_t = 20)]
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

/// `--param-grid` value parser, delegating to `sweep::parse_param_grid`.
fn parse_param_grid_arg(s: &str) -> Result<ParamGrid, String> {
    sweep::parse_param_grid(s).map_err(|e| e.to_string())
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

    use super::{Cli, Command, SweepArgs, ThroughputArgs};

    fn parse(extra_args: &[&str]) -> Result<Cli, clap::Error> {
        let mut full_argv = vec!["isingmark"];
        full_argv.extend_from_slice(extra_args);
        Cli::try_parse_from(full_argv)
    }

    /// Extract the `Throughput` variant, for tests that parsed a throughput
    /// invocation. `Command` has more than one variant (as of `sweep`), so
    /// `let Command::Throughput(args) = cli.command;` is no longer an
    /// irrefutable pattern and cannot be used directly.
    fn throughput_args(cli: Cli) -> ThroughputArgs {
        match cli.command {
            Command::Throughput(args) => args,
            Command::Sweep(_) => unreachable!("test only ever parses a throughput invocation"),
        }
    }

    /// Extract the `Sweep` variant. See [`throughput_args`].
    fn sweep_args(cli: Cli) -> SweepArgs {
        match cli.command {
            Command::Sweep(args) => args,
            Command::Throughput(_) => unreachable!("test only ever parses a sweep invocation"),
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
}
