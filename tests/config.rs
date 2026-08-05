// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for YAML config loading: shipped example configs,
//! merge precedence through the real CLI path, and validation.

use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser as _;
use isingmark::cli::{Cli, Command};
use isingmark::config::{load_config, merge, ConfigError, FileConfig, DEFAULT_HARDNESS};

/// Directory holding the shipped example configs. Enumerated at runtime
/// (not a hardcoded list of filenames) so a fourth example cannot be added
/// without being covered.
fn examples_configs_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/configs")
}

fn throughput_from(cli: Cli) -> isingmark::cli::ThroughputArgs {
    match cli.command {
        Command::Throughput(args) => args,
        Command::Sweep(_)
        | Command::Ttt(_)
        | Command::Comparative(_)
        | Command::Scaling(_)
        | Command::Reproducibility(_)
        | Command::Compare(_) => {
            // Match the in-tree cli unit-test style: unreachable! is accepted
            // for test-only extractors that only ever see one variant.
            unreachable!("test only ever parses a throughput invocation")
        }
    }
}

#[test]
fn every_shipped_example_config_parses() {
    let dir = examples_configs_dir();
    assert!(
        dir.is_dir(),
        "examples/configs must exist at {}",
        dir.display()
    );
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("read_dir examples/configs")
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("yaml"))
        .collect();
    entries.sort();
    assert!(
        !entries.is_empty(),
        "examples/configs must contain at least one .yaml file (empty or wrongly-pathed \
         directory must not pass as \"all zero configs parsed\")"
    );
    for path in &entries {
        let cfg =
            load_config(path).expect("shipped example config must parse (see path in display)");
        // Every complete example is expected to name a backend (R2).
        assert!(
            cfg.backend.is_some(),
            "{} must set backend so it is runnable with only --config",
            path.display()
        );
        // Surface the path on failure above; also pin that load succeeded.
        let _ = path;
    }
}

#[test]
fn merge_three_layers_with_distinct_values() {
    // flag 0.9, file 0.7, default 0.5 — all distinct so each layer's win is
    // unambiguous.
    assert!((merge(Some(0.9_f64), Some(0.7_f64), 0.5_f64) - 0.9).abs() < f64::EPSILON);
    assert!((merge(None, Some(0.7_f64), 0.5_f64) - 0.7).abs() < f64::EPSILON);
    assert!((merge::<f64>(None, None, 0.5) - 0.5).abs() < f64::EPSILON);
    assert!((DEFAULT_HARDNESS - 0.5).abs() < f64::EPSILON);
}

/// End-to-end through the real CLI path: file value reaches mode config
/// when the flag is absent. Proves `into_config` actually calls `merge`,
/// not merely that `merge` itself is correct.
#[test]
fn cli_path_file_hardness_reaches_throughput_config() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("only_file.yaml");
    fs::write(
        &path,
        "backend: cpu-sa\n\
         topology_preset: smoke\n\
         hardness: 0.7\n\
         duration: 10s\n\
         seed: 99\n\
         output_dir: /tmp/isingmark-cfg-e2e\n",
    )
    .expect("write");

    let cli = Cli::try_parse_from([
        "isingmark",
        "throughput",
        "--config",
        path.to_str().expect("utf8"),
    ])
    .expect("parse");
    let cfg = throughput_from(cli).into_config().expect("into_config");
    assert!(
        (cfg.hardness - 0.7).abs() < f64::EPSILON,
        "file hardness 0.7 must reach ThroughputConfig, got {}",
        cfg.hardness
    );
    assert_eq!(cfg.seed, 99);
    assert_eq!(cfg.output_dir, PathBuf::from("/tmp/isingmark-cfg-e2e"));
    assert!((cfg.duration_s - 10.0).abs() < f64::EPSILON);
}

#[test]
fn cli_path_flag_overrides_file_hardness() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("flag_over_file.yaml");
    fs::write(
        &path,
        "backend: cpu-sa\ntopology_preset: smoke\nhardness: 0.7\nduration: 10s\n",
    )
    .expect("write");

    let cli = Cli::try_parse_from([
        "isingmark",
        "throughput",
        "--config",
        path.to_str().expect("utf8"),
        "--hardness",
        "0.9",
    ])
    .expect("parse");
    let cfg = throughput_from(cli).into_config().expect("into_config");
    assert!(
        (cfg.hardness - 0.9).abs() < f64::EPSILON,
        "flag 0.9 must beat file 0.7, got {}",
        cfg.hardness
    );
}

#[test]
fn cli_path_default_hardness_when_neither_flag_nor_file() {
    let cli = Cli::try_parse_from([
        "isingmark",
        "throughput",
        "--backend",
        "cpu-sa",
        "--topology-preset",
        "smoke",
        "--duration",
        "10s",
    ])
    .expect("parse");
    let args = throughput_from(cli);
    // Flag is None (no clap default injection). Merge must yield 0.5.
    assert_eq!(args.common.hardness, None, "clap must not inject hardness");
    let cfg = args.into_config().expect("into_config");
    assert!(
        (cfg.hardness - 0.5).abs() < f64::EPSILON,
        "default hardness must be 0.5, got {}",
        cfg.hardness
    );
}

#[test]
fn unknown_key_error_contains_the_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("typo.yaml");
    fs::write(&path, "hardnes: 0.5\n").expect("write");
    let err = load_config(&path).expect_err("unknown key");
    let msg = err.to_string();
    assert!(
        msg.contains("hardnes"),
        "error must name the offending key, got: {msg}"
    );
}

#[test]
fn non_finite_hardness_rejected_by_variant() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("nan.yaml");
    fs::write(&path, "hardness: .nan\n").expect("write");
    let err = load_config(&path).expect_err("nan");
    assert!(
        matches!(
            &err,
            ConfigError::Invalid { message, .. }
                if message.contains("finite") || message.contains("hardness")
        ),
        "got: {err}"
    );
}

#[test]
fn param_curve_min_gt_max_rejected_by_variant() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("curve.yaml");
    fs::write(
        &path,
        "param_curve:\n  min_reads: 100\n  max_reads: 10\n  min_sweeps: 1\n  max_sweeps: 2\n  \
         min_anneal_us: 1\n  max_anneal_us: 2\n",
    )
    .expect("write");
    let err = load_config(&path).expect_err("min > max");
    assert!(
        matches!(
            &err,
            ConfigError::Invalid { message, .. }
                if message.contains("reads") && message.contains("greater")
        ),
        "got: {err}"
    );
}

#[test]
fn zero_bound_warns_and_load_succeeds() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("zero.yaml");
    fs::write(
        &path,
        "param_curve:\n  min_reads: 0\n  max_reads: 64\n  min_sweeps: 100\n  max_sweeps: 1000\n  \
         min_anneal_us: 5\n  max_anneal_us: 200\n",
    )
    .expect("write");
    let cfg = load_config(&path).expect("zero bound must succeed (warn only)");
    assert_eq!(
        cfg.param_curve.as_ref().map(|c| c.min_reads),
        Some(0),
        "zero bound must be preserved after successful load"
    );
}

#[test]
fn missing_backend_after_merge_names_flag_and_config_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("no_backend.yaml");
    fs::write(&path, "topology_preset: smoke\nduration: 10s\n").expect("write");
    let cli = Cli::try_parse_from([
        "isingmark",
        "throughput",
        "--config",
        path.to_str().expect("utf8"),
    ])
    .expect("parse");
    let err = throughput_from(cli).into_config().expect_err("no backend");
    assert!(err.contains("backend"), "got: {err}");
    assert!(err.contains("--backend"), "got: {err}");
    assert!(
        err.contains("no_backend.yaml"),
        "error must name the config path, got: {err}"
    );
}

/// Regression for the clap-default trap (R3): if `default_value_t = 0.5`
/// were restored on `hardness`, clap would inject `Some(0.5)` whenever the
/// flag is absent, and a config file's distinct hardness would be ignored.
#[test]
fn clap_does_not_inject_default_hardness_so_file_can_win() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("r3.yaml");
    // File hardness deliberately ≠ default 0.5.
    fs::write(
        &path,
        "backend: cpu-sa\ntopology_preset: smoke\nhardness: 0.3\nduration: 5s\n",
    )
    .expect("write");
    let cli = Cli::try_parse_from([
        "isingmark",
        "throughput",
        "--config",
        path.to_str().expect("utf8"),
    ])
    .expect("parse");
    let args = throughput_from(cli);
    assert_eq!(
        args.common.hardness, None,
        "hardness flag must be None when omitted — a clap default would break file precedence"
    );
    let cfg = args.into_config().expect("into_config");
    assert!(
        (cfg.hardness - 0.3).abs() < f64::EPSILON,
        "file hardness 0.3 must win over built-in default 0.5, got {}",
        cfg.hardness
    );
}

#[test]
fn empty_file_config_deserializes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("empty.yaml");
    fs::write(&path, "").expect("write");
    let cfg = load_config(&path).expect("empty YAML mapping/document is ok");
    assert_eq!(cfg, FileConfig::default());
}

#[test]
fn load_config_io_error_for_missing_path() {
    let err = load_config(Path::new("/definitely/not/a/real/isingmark-config.yaml"))
        .expect_err("missing path");
    assert!(matches!(err, ConfigError::Io { .. }));
}
