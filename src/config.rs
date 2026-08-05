// SPDX-License-Identifier: AGPL-3.0-or-later
//! YAML config file loading and flag-over-file-over-default merge.
//!
//! Precedence for every overridable field: explicit CLI flag, then a value
//! from the file named by `--config`, then a built-in default (when one
//! exists). See [`merge`].

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::modes::sweep::ParamGrid;
use crate::params::ParamCurve;

/// Reject non-finite or out-of-range hardness values.
///
/// Shared by the CLI `--hardness` / `--hardness-b` value parsers and by
/// [`load_config`] so flag and file paths cannot drift apart on the bounds.
///
/// # Errors
///
/// Returns a message naming `source` (e.g. `"--hardness"` or `"hardness"`)
/// when `value` is non-finite or outside `[0.0, 1.0]`.
pub fn validate_hardness(value: f64, source: &str) -> Result<f64, String> {
    if !value.is_finite() {
        return Err(format!("{source} must be a finite number, got {value}"));
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(format!("{source} must be in [0.0, 1.0], got {value}"));
    }
    Ok(value)
}

/// Defaults for fields the CLI historically injected via clap
/// `default_value` / `default_value_t`. Kept here so `merge` and help text
/// share one source of truth; changing any of these silently changes every
/// existing user's runs that omit the flag.
pub const DEFAULT_HARDNESS: f64 = 0.5;
/// Default problem-generation seed.
pub const DEFAULT_SEED: u64 = 0;
/// Default output directory for JSONL records and the run summary.
pub const DEFAULT_OUTPUT_DIR: &str = "./out";
/// Default jobs-per-cell / jobs-per-size / jobs-per-evaluation count.
pub const DEFAULT_NUM_JOBS: u32 = 20;
/// Default trials-per-target for time-to-target mode.
pub const DEFAULT_NUM_TRIALS: u32 = 20;
/// Default wall-clock budget per target, in seconds, for time-to-target.
pub const DEFAULT_MAX_TIME_S: f64 = 30.0;
/// Default maximum hardness-search iterations for comparative mode.
pub const DEFAULT_MAX_ITERATIONS: u32 = 10;
/// Default problems-per-run for reproducibility mode.
pub const DEFAULT_NUM_PROBLEMS: u32 = 20;

/// Errors produced while reading or validating a YAML config file.
#[derive(Debug)]
pub enum ConfigError {
    /// The file could not be read.
    Io {
        /// Path that failed to open or read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The file was not valid YAML for [`FileConfig`], or failed a
    /// post-deserialize validation check (hardness range, `min > max`
    /// curve bounds, unknown key, …). The message already names the
    /// offending field where possible.
    Invalid {
        /// Path of the config file that failed validation.
        path: PathBuf,
        /// Human-readable reason.
        message: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(f, "failed to read config file {}: {source}", path.display())
            }
            Self::Invalid { path, message } => {
                write!(f, "invalid config file {}: {message}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Invalid { .. } => None,
        }
    }
}

/// Every option a YAML config file may supply. All fields are `Option` so
/// a partial file only overrides what it names; unset fields fall through
/// to the CLI flag or the built-in default via [`merge`].
///
/// Unknown keys are rejected ([`serde(deny_unknown_fields)`]) so a typo
/// like `hardnes:` is an error rather than a silent no-op.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    /// Backend name (e.g. `"cpu-sa"`). See `backend::builtin_registry`.
    pub backend: Option<String>,
    /// Bundled topology fixture name (e.g. `"smoke"`). Mutually exclusive
    /// with [`Self::topology_file`].
    pub topology_preset: Option<String>,
    /// Path to a topology-spec JSON file. Mutually exclusive with
    /// [`Self::topology_preset`].
    pub topology_file: Option<PathBuf>,
    /// Difficulty in `[0.0, 1.0]`.
    pub hardness: Option<f64>,
    /// Seed for deterministic problem generation.
    pub seed: Option<u64>,
    /// Directory for JSONL records and the run summary.
    pub output_dir: Option<PathBuf>,
    /// Hardness-to-parameter curve bounds. Validated on load (see
    /// [`load_config`]); modes that still hardcode their own
    /// `DEFAULT_PARAM_CURVE` do not yet consume this override at runtime.
    pub param_curve: Option<FileParamCurve>,
    /// Throughput wall-clock duration, e.g. `"30s"`, `"5m"`, `"2h"`.
    pub duration: Option<String>,
    /// Sweep parameter grid (YAML mapping of the same shape as the
    /// `--param-grid` JSON object).
    pub param_grid: Option<ParamGrid>,
    /// Jobs per cell / size / evaluation (sweep, comparative, scaling).
    pub num_jobs: Option<u32>,
    /// Time-to-target energy targets in **natural units** (not milli).
    pub energy_targets: Option<Vec<f64>>,
    /// Trials attempted per target (ttt).
    pub num_trials: Option<u32>,
    /// Wall-clock budget per target in seconds (ttt).
    pub max_time_s: Option<f64>,
    /// Reference JSONL path for comparative mode.
    pub target_file: Option<PathBuf>,
    /// Maximum hardness-search iterations (comparative).
    pub max_iterations: Option<u32>,
    /// Node counts for scaling mode.
    pub sizes: Option<Vec<usize>>,
    /// Jobs per run for reproducibility mode.
    pub num_problems: Option<u32>,
    /// Second-run hardness for reproducibility mode.
    pub hardness_b: Option<f64>,
}

/// YAML form of [`ParamCurve`]. All six bounds are required when the
/// `param_curve` key is present — a partial curve that silently half-defaulted
/// would be this project's recurring defect class.
///
/// A bound of `0` is accepted but warned on: on the wire `0` means UNSET, so
/// the miner ignores it and runs its own adaptation logic — the easiest
/// point on the curve would then measure miner adaptation instead of the
/// requested parameter.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileParamCurve {
    /// Reads at `hardness = 0.0`.
    pub min_reads: u32,
    /// Reads at `hardness = 1.0`.
    pub max_reads: u32,
    /// Sweeps at `hardness = 0.0` (sweeping backends).
    pub min_sweeps: u32,
    /// Sweeps at `hardness = 1.0` (sweeping backends).
    pub max_sweeps: u32,
    /// Anneal time (µs) at `hardness = 0.0` (annealing backends).
    pub min_anneal_us: u32,
    /// Anneal time (µs) at `hardness = 1.0` (annealing backends).
    pub max_anneal_us: u32,
}

impl FileParamCurve {
    /// Convert to the runtime [`ParamCurve`] type.
    #[must_use]
    pub fn to_param_curve(&self) -> ParamCurve {
        ParamCurve {
            min_reads: self.min_reads,
            max_reads: self.max_reads,
            min_sweeps: self.min_sweeps,
            max_sweeps: self.max_sweeps,
            min_anneal_us: self.min_anneal_us,
            max_anneal_us: self.max_anneal_us,
        }
    }
}

/// Three-way precedence: explicit flag beats file beats built-in default.
///
/// Callers that want "no default, required from flag or file" should not
/// call this — check `flag.or(file)` and error if both are `None`.
#[must_use]
pub fn merge<T>(flag: Option<T>, file: Option<T>, default: T) -> T {
    match flag {
        Some(v) => v,
        None => match file {
            Some(v) => v,
            None => default,
        },
    }
}

/// Read `path` as YAML into a [`FileConfig`], then validate fields the type
/// system cannot check (hardness finite and in `[0.0, 1.0]`, `ParamCurve`
/// min ≤ max, zero-bound warning).
///
/// # Errors
///
/// Returns [`ConfigError::Io`] if the file cannot be read, or
/// [`ConfigError::Invalid`] if YAML parsing or validation fails. An
/// unknown key is rejected and the error message names the key.
pub fn load_config(path: &Path) -> Result<FileConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut cfg: FileConfig = serde_saphyr::from_str(&text).map_err(|e| ConfigError::Invalid {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    validate_file_config(&mut cfg, path)?;
    Ok(cfg)
}

/// Post-deserialize validation shared by [`load_config`]. Separated so unit
/// tests can exercise the same checks against an already-built
/// [`FileConfig`] without going through the filesystem.
fn validate_file_config(cfg: &mut FileConfig, path: &Path) -> Result<(), ConfigError> {
    if let Some(h) = cfg.hardness {
        validate_hardness(h, "hardness").map_err(|message| ConfigError::Invalid {
            path: path.to_path_buf(),
            message,
        })?;
    }
    if let Some(h) = cfg.hardness_b {
        validate_hardness(h, "hardness_b").map_err(|message| ConfigError::Invalid {
            path: path.to_path_buf(),
            message,
        })?;
    }
    if let Some(curve) = &cfg.param_curve {
        validate_param_curve(curve).map_err(|message| ConfigError::Invalid {
            path: path.to_path_buf(),
            message,
        })?;
        warn_zero_bounds(curve);
    }
    if let Some(n) = cfg.num_jobs {
        if n == 0 {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: "num_jobs must not be 0 (a zero-job run would exit successfully having \
                          tested nothing)"
                    .to_string(),
            });
        }
    }
    if let Some(n) = cfg.num_trials {
        if n == 0 {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: "num_trials must not be 0 (a zero-trial target would report an empty \
                          result with no signal that it never ran)"
                    .to_string(),
            });
        }
    }
    if let Some(n) = cfg.max_iterations {
        if n == 0 {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: "max_iterations must not be 0 (a zero-iteration search would report \
                          hardness 0.0 with no signal that no evaluation ever ran)"
                    .to_string(),
            });
        }
    }
    if let Some(n) = cfg.num_problems {
        if n == 0 {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: "num_problems must not be 0 (a zero-problem run would exit successfully \
                          having tested nothing)"
                    .to_string(),
            });
        }
    }
    if let Some(t) = cfg.max_time_s {
        if !t.is_finite() {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!("max_time_s must be a finite number, got {t}"),
            });
        }
        if t <= 0.0 {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                message: format!("max_time_s must be greater than 0, got {t}"),
            });
        }
    }
    if let Some(grid) = &cfg.param_grid {
        for (name, values) in [
            ("num_reads", &grid.num_reads),
            ("num_sweeps", &grid.num_sweeps),
            ("anneal_time_us", &grid.anneal_time_us),
        ] {
            if values.contains(&0) {
                return Err(ConfigError::Invalid {
                    path: path.to_path_buf(),
                    message: format!(
                        "param_grid {name} must not contain 0 (0 means \"unset\" on the wire)"
                    ),
                });
            }
        }
    }
    // Silence the unused-mut lint if no field needs mutation; kept `&mut`
    // so a future check can rewrite a field in place without signature churn.
    let _ = cfg;
    Ok(())
}

/// Reject any `ParamCurve` pair where `min > max`, naming which pair.
fn validate_param_curve(curve: &FileParamCurve) -> Result<(), String> {
    for (name, min, max) in [
        ("reads", curve.min_reads, curve.max_reads),
        ("sweeps", curve.min_sweeps, curve.max_sweeps),
        ("anneal_us", curve.min_anneal_us, curve.max_anneal_us),
    ] {
        if min > max {
            return Err(format!(
                "param_curve min_{name} ({min}) is greater than max_{name} ({max})"
            ));
        }
    }
    Ok(())
}

/// WARN (do not reject) when any of the six bounds is `0`. On the wire `0`
/// means UNSET, so the miner ignores it and runs its own adaptation logic —
/// the easiest point on the curve would then measure miner adaptation
/// instead of the requested parameter. That is a silently misleading
/// benchmark, not a crash.
fn warn_zero_bounds(curve: &FileParamCurve) {
    let zeros: Vec<&str> = [
        ("min_reads", curve.min_reads),
        ("max_reads", curve.max_reads),
        ("min_sweeps", curve.min_sweeps),
        ("max_sweeps", curve.max_sweeps),
        ("min_anneal_us", curve.min_anneal_us),
        ("max_anneal_us", curve.max_anneal_us),
    ]
    .into_iter()
    .filter_map(|(name, v)| if v == 0 { Some(name) } else { None })
    .collect();
    if zeros.is_empty() {
        return;
    }
    #[expect(
        clippy::print_stderr,
        reason = "config zero-bound warning is operator-facing diagnostic on stderr; \
                  not a substitute for structured logging (none wired yet)"
    )]
    {
        eprintln!(
            "warning: param_curve bound(s) {} are 0; on the wire 0 means UNSET, so the miner \
             ignores that bound and runs its own adaptation logic — the easiest point on the \
             curve would then measure miner adaptation instead of the requested parameter",
            zeros.join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        load_config, merge, validate_file_config, validate_param_curve, ConfigError, FileConfig,
        FileParamCurve, DEFAULT_HARDNESS,
    };
    use std::io::Write;
    use std::path::Path;

    #[test]
    fn merge_flag_beats_file_beats_default_with_distinct_values() {
        // Three distinct values so "file wins" cannot be confused with
        // "default wins" and "flag wins" cannot be confused with either.
        let flag = Some(0.9_f64);
        let file = Some(0.7_f64);
        let default = 0.5_f64;
        assert!((merge(flag, file, default) - 0.9).abs() < f64::EPSILON);
    }

    #[test]
    fn merge_file_beats_default_when_flag_absent() {
        let flag: Option<f64> = None;
        let file = Some(0.7_f64);
        let default = 0.5_f64;
        assert!((merge(flag, file, default) - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn merge_default_when_flag_and_file_absent() {
        let flag: Option<f64> = None;
        let file: Option<f64> = None;
        let default = 0.5_f64;
        assert!((merge(flag, file, default) - 0.5).abs() < f64::EPSILON);
        // Pin the named constant too so a drift between DEFAULT_HARDNESS and
        // the literal used in merge call sites is not invisible here.
        assert!((DEFAULT_HARDNESS - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn merge_does_not_prefer_file_over_flag() {
        // Mutation gate: if merge returned `file` when `flag` is Some, this
        // fails. Values deliberately distinct from each other and from the
        // default.
        assert_eq!(merge(Some(3_u32), Some(2_u32), 1_u32), 3);
    }

    fn write_temp(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cfg.yaml");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(contents.as_bytes()).expect("write");
        (dir, path)
    }

    #[test]
    fn load_config_accepts_a_partial_file() {
        let (_dir, path) = write_temp("hardness: 0.7\nbackend: cpu-sa\n");
        let cfg = load_config(&path).expect("partial file loads");
        assert_eq!(cfg.hardness, Some(0.7));
        assert_eq!(cfg.backend.as_deref(), Some("cpu-sa"));
        assert_eq!(cfg.seed, None);
    }

    #[test]
    fn load_config_rejects_unknown_key_naming_the_key() {
        let (_dir, path) = write_temp("hardnes: 0.5\n");
        let err = load_config(&path).expect_err("typo key must be rejected");
        assert!(
            matches!(
                &err,
                ConfigError::Invalid { message, .. } if message.contains("hardnes")
            ),
            "error must be Invalid naming the offending key, got: {err}"
        );
    }

    #[test]
    fn load_config_rejects_non_finite_hardness() {
        for bad in [".nan", ".inf", "-.inf"] {
            let (_dir, path) = write_temp(&format!("hardness: {bad}\n"));
            let err = load_config(&path).expect_err("non-finite hardness must be rejected");
            assert!(
                matches!(
                    &err,
                    ConfigError::Invalid { message, .. }
                        if message.contains("finite") || message.contains("hardness")
                ),
                "expected Invalid with hardness/finite mention for {bad}, got: {err}"
            );
        }
    }

    #[test]
    fn load_config_rejects_hardness_outside_unit_interval() {
        let (_dir, path) = write_temp("hardness: 1.5\n");
        let err = load_config(&path).expect_err("out-of-range hardness must be rejected");
        assert!(
            matches!(
                &err,
                ConfigError::Invalid { message, .. } if message.contains("[0.0, 1.0]")
            ),
            "expected Invalid with range mention, got: {err}"
        );
    }

    #[test]
    fn load_config_rejects_param_curve_min_greater_than_max_naming_pair() {
        let yaml = r"
param_curve:
  min_reads: 64
  max_reads: 8
  min_sweeps: 100
  max_sweeps: 1000
  min_anneal_us: 5
  max_anneal_us: 200
";
        let (_dir, path) = write_temp(yaml);
        let err = load_config(&path).expect_err("min > max must be rejected");
        assert!(
            matches!(
                &err,
                ConfigError::Invalid { message, .. }
                    if message.contains("reads") && message.contains("greater")
            ),
            "error must name the pair and say min > max, got: {err}"
        );
    }

    #[test]
    fn validate_param_curve_accepts_equal_bounds() {
        let curve = FileParamCurve {
            min_reads: 8,
            max_reads: 8,
            min_sweeps: 100,
            max_sweeps: 100,
            min_anneal_us: 5,
            max_anneal_us: 5,
        };
        validate_param_curve(&curve).expect("equal bounds are valid");
    }

    #[test]
    fn zero_bound_warns_but_load_succeeds() {
        let yaml = r"
param_curve:
  min_reads: 0
  max_reads: 64
  min_sweeps: 100
  max_sweeps: 1000
  min_anneal_us: 5
  max_anneal_us: 200
";
        let (_dir, path) = write_temp(yaml);
        // Must SUCCEED (warn, do not reject). A test that only checked a
        // warning was emitted would pass on an implementation that rejected.
        let cfg = load_config(&path).expect("zero bound must warn and still succeed");
        assert_eq!(cfg.param_curve.as_ref().map(|c| c.min_reads), Some(0));
    }

    #[test]
    fn missing_config_file_is_io_error() {
        let err = load_config(Path::new("/no/such/config-file-isingmark.yaml"))
            .expect_err("missing file must be Io");
        assert!(matches!(err, ConfigError::Io { .. }));
    }

    #[test]
    fn validate_file_config_rejects_min_gt_max_by_variant() {
        let mut cfg = FileConfig {
            param_curve: Some(FileParamCurve {
                min_reads: 1,
                max_reads: 2,
                min_sweeps: 50,
                max_sweeps: 10,
                min_anneal_us: 1,
                max_anneal_us: 2,
            }),
            ..FileConfig::default()
        };
        let err = validate_file_config(&mut cfg, Path::new("x.yaml"))
            .expect_err("min_sweeps > max_sweeps");
        assert!(
            matches!(
                &err,
                ConfigError::Invalid { message, .. } if message.contains("sweeps")
            ),
            "got: {err}"
        );
    }
}
