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
    // Pin the finiteness check: range rejection of NaN still mentions
    // "hardness", so asserting only "hardness" would miss a deleted finite branch.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("nan.yaml");
    fs::write(&path, "hardness: .nan\n").expect("write");
    let err = load_config(&path).expect_err("nan");
    assert!(
        matches!(
            &err,
            ConfigError::Invalid { message, .. } if message.contains("finite")
        ),
        "got: {err}"
    );
}

#[test]
fn param_curve_anneal_min_gt_max_rejected_naming_anneal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("anneal_curve.yaml");
    fs::write(
        &path,
        "param_curve:\n  min_reads: 8\n  max_reads: 64\n  min_sweeps: 100\n  max_sweeps: 1000\n  \
         min_anneal_us: 200\n  max_anneal_us: 5\n",
    )
    .expect("write");
    let err = load_config(&path).expect_err("min_anneal_us > max_anneal_us");
    assert!(
        matches!(
            &err,
            ConfigError::Invalid { message, .. }
                if message.contains("anneal") && message.contains("greater")
        ),
        "got: {err}"
    );
}

#[test]
fn param_curve_unknown_nested_key_rejected_naming_the_key() {
    // Typo is not a prefix of a real field (`min_reads`), so a missing-field
    // fallback after dropping deny_unknown_fields cannot satisfy the assert.
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("curve_typo.yaml");
    fs::write(
        &path,
        "param_curve:\n  min_reed: 8\n  max_reads: 64\n  min_sweeps: 100\n  max_sweeps: 1000\n  \
         min_anneal_us: 5\n  max_anneal_us: 200\n",
    )
    .expect("write");
    let err = load_config(&path).expect_err("unknown nested key");
    let msg = err.to_string();
    assert!(
        msg.contains("min_reed"),
        "error must name the offending nested key, got: {msg}"
    );
}

/// C1: a non-default `param_curve` in YAML must reach resolved job params,
/// not merely be copied onto the mode config. The shipped example curves
/// used to match `DEFAULT_PARAM_CURVE`, so a happy-path run could never show
/// the ignore bug — this curve deliberately differs.
#[test]
fn cli_path_file_param_curve_reaches_resolved_job_params() {
    use isingmark::modes::throughput::DEFAULT_PARAM_CURVE;
    use isingmark::params::BackendKind;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("curve.yaml");
    // min_reads 16..128 at hardness 0.5 → 72; default curve is 8..64 → 36.
    // Raw string: `\` line continuation strips leading whitespace on the next
    // line, which would un-indent nested `param_curve` keys to top level.
    fs::write(
        &path,
        r"backend: cpu-sa
topology_preset: smoke
hardness: 0.5
duration: 10s
param_curve:
  min_reads: 16
  max_reads: 128
  min_sweeps: 200
  max_sweeps: 2000
  min_anneal_us: 10
  max_anneal_us: 400
",
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

    assert_ne!(
        cfg.param_curve, DEFAULT_PARAM_CURVE,
        "file curve must differ from DEFAULT_PARAM_CURVE so this test can detect ignore"
    );
    let resolved = cfg.param_curve.resolve(BackendKind::Sweeping, cfg.hardness);
    assert_eq!(
        resolved.num_reads, 72,
        "file curve at hardness 0.5 must resolve to 72 reads (default would be 36)"
    );
    assert_eq!(
        resolved.num_sweeps, 1100,
        "file curve at hardness 0.5 must resolve to 1100 sweeps (default would be 550)"
    );

    // Sanity: if into_config ignored the file curve, resolve would be default.
    let default_resolved = DEFAULT_PARAM_CURVE.resolve(BackendKind::Sweeping, 0.5);
    assert_eq!(default_resolved.num_reads, 36);
    assert_ne!(resolved.num_reads, default_resolved.num_reads);
}

/// I3: `--backend` must beat a distinct file `backend`.
#[test]
fn cli_path_flag_backend_beats_file_backend() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("backend_over.yaml");
    fs::write(
        &path,
        "backend: dwave\ntopology_preset: smoke\nduration: 10s\n",
    )
    .expect("write");

    let cli = Cli::try_parse_from([
        "isingmark",
        "throughput",
        "--config",
        path.to_str().expect("utf8"),
        "--backend",
        "cpu-sa",
    ])
    .expect("parse");
    let cfg = throughput_from(cli).into_config().expect("into_config");
    assert_eq!(
        cfg.backend, "cpu-sa",
        "flag --backend cpu-sa must beat file backend: dwave, got {}",
        cfg.backend
    );
}

/// Topology flag must beat a distinct file topology (same precedence class
/// as backend; only add if not already covered — was not).
#[test]
fn cli_path_flag_topology_beats_file_topology() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("topo_over.yaml");
    fs::write(
        &path,
        "backend: cpu-sa\ntopology_preset: smoke\nduration: 10s\n",
    )
    .expect("write");

    let cli = Cli::try_parse_from([
        "isingmark",
        "throughput",
        "--config",
        path.to_str().expect("utf8"),
        "--topology-file",
        "fixtures/smoke.spec.json",
    ])
    .expect("parse");
    let cfg = throughput_from(cli).into_config().expect("into_config");
    assert_eq!(
        cfg.topology,
        isingmark::modes::TopologyRef::File(PathBuf::from("fixtures/smoke.spec.json")),
        "flag --topology-file must beat file topology_preset: smoke"
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
