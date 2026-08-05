// SPDX-License-Identifier: AGPL-3.0-or-later
//! Plot output tests and the `--plot` missing-feature rejection path.
//!
//! With `--features plot`, each plot function must write a real PNG (file
//! exists, size above a floor, correct eight-byte signature). Without the
//! feature, the CLI still accepts `--plot PATH` and exits non-zero with a
//! message that names the `plot` feature.

#[cfg(feature = "plot")]
use std::path::Path;

#[cfg(not(feature = "plot"))]
use std::io::Write as _;
#[cfg(not(feature = "plot"))]
use std::path::{Path, PathBuf};

/// PNG file signature (ISO/IEC 15948).
#[cfg(feature = "plot")]
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Minimum acceptable PNG size: a real chart is hundreds of bytes; a single
/// byte would still satisfy a non-empty check.
#[cfg(feature = "plot")]
const MIN_PNG_BYTES: u64 = 200;

/// Assert `path` is a non-trivial PNG (exists, large enough, correct magic).
#[cfg(feature = "plot")]
fn assert_is_png(path: &Path) {
    assert!(path.is_file(), "plot file must exist at {}", path.display());
    let meta = std::fs::metadata(path).expect("stat plot file");
    assert!(
        meta.len() > MIN_PNG_BYTES,
        "plot file too small to be a real PNG: {} bytes at {}",
        meta.len(),
        path.display()
    );
    let bytes = std::fs::read(path).expect("read plot file");
    assert!(
        bytes.len() >= 8 && bytes[..8] == PNG_SIGNATURE,
        "plot file must start with the PNG signature; first bytes = {:02x?}",
        &bytes[..bytes.len().min(8)]
    );
}

#[cfg(feature = "plot")]
mod with_plot_feature {
    use super::{assert_is_png, MIN_PNG_BYTES};
    use std::io::Write as _;
    use std::path::{Path, PathBuf};

    use isingmark::backend::RunReport;
    use isingmark::modes::sweep::{CellOutcome, SweepReport};
    use isingmark::modes::ttt::TttReport;
    use isingmark::record::{HardwareInfo, JobRecord, Summary, TopologyInfo, Totals};
    use isingmark::report::{plot_energy_cdfs, plot_sweep_heatmap, plot_ttt_cdf};

    fn dummy_summary(mode: &str) -> Summary {
        Summary {
            mode: mode.to_string(),
            backend: "cpu-sa".to_string(),
            topology: TopologyInfo {
                name: "smoke".to_string(),
                num_nodes: 8,
                num_edges: 8,
                topology_hash: "00".to_string(),
            },
            hardware: HardwareInfo {
                platform: "test".to_string(),
                cpu: "test".to_string(),
                ram_gb: 1.0,
            },
            config: serde_json::json!({}),
            totals: Totals {
                jobs: 4,
                passed: 4,
                rejected: 0,
                skipped_lines: 0,
            },
        }
    }

    fn passed_record(energy_milli: i64) -> JobRecord {
        JobRecord {
            job_id: "00".to_string(),
            backend: "cpu-sa".to_string(),
            n_solutions: 1,
            best_energy_milli: Some(energy_milli),
            best_spins_hex: None,
            diversity_milli: 0,
            passed: true,
            rejected: false,
            reject_reason: None,
            device_access_time_us: 0,
            wall_ms: 10,
            requested_reads: Some(64),
            requested_sweeps: Some(100),
            requested_anneal_time_us: Some(0),
            measured_reads: Some(64),
            measured_sweeps: Some(100),
        }
    }

    fn cell(dir: &str, num_reads: u32, num_sweeps: u32, energy_milli: i64) -> CellOutcome {
        CellOutcome {
            dir_name: dir.to_string(),
            num_reads,
            num_sweeps,
            anneal_time_us: 0,
            report: RunReport {
                handshake_ok: true,
                records: vec![passed_record(energy_milli)],
                miner_exit_code: 0,
                miner_unresponsive: false,
                miner_stderr_tail: None,
                orphan_results: 0,
                run_wall_ms: 100,
            },
        }
    }

    #[test]
    fn plot_sweep_heatmap_writes_a_png() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("sweep.png");
        let report = SweepReport {
            cells: vec![
                cell("cell_0", 64, 100, -1000),
                cell("cell_1", 64, 500, -2000),
                cell("cell_2", 128, 100, -1500),
                cell("cell_3", 128, 500, -3000),
            ],
            summary: dummy_summary("sweep"),
        };
        plot_sweep_heatmap(&report, &out).expect("plot_sweep_heatmap succeeds");
        assert_is_png(&out);
        assert!(std::fs::metadata(&out).expect("stat").len() > MIN_PNG_BYTES);
    }

    #[test]
    fn plot_ttt_cdf_writes_a_png() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("ttt.png");
        let mut per_target = std::collections::BTreeMap::new();
        per_target.insert(-5000, vec![0.1, 0.2, 0.4, 0.8]);
        per_target.insert(-10_000, vec![0.5, 1.0, 1.5]);
        let report = TttReport {
            per_target,
            summary: dummy_summary("ttt"),
        };
        plot_ttt_cdf(&report, &out).expect("plot_ttt_cdf succeeds");
        assert_is_png(&out);
    }

    #[test]
    fn plot_energy_cdfs_writes_a_png() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("energy.png");
        let a = vec![-3.0, -2.0, -1.0];
        let b = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        plot_energy_cdfs(&a, &b, &out).expect("plot_energy_cdfs succeeds");
        assert_is_png(&out);
    }

    /// End-to-end: `isingmark compare --plot` writes a PNG next to two
    /// real JSONL files when the plot feature is compiled in.
    #[test]
    fn compare_cli_plot_writes_a_png() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_a = dir.path().join("a.jsonl");
        let path_b = dir.path().join("b.jsonl");
        let plot_path = dir.path().join("compare.png");

        write_energy_jsonl(&path_a, &[-1000, -2000, -3000]);
        write_energy_jsonl(&path_b, &[1000, 2000, 3000, 4000, 5000]);

        let bin = PathBuf::from(env!("CARGO_BIN_EXE_isingmark"));
        let output = std::process::Command::new(&bin)
            .args([
                "compare",
                "--file-a",
                path_a.to_str().expect("utf-8 a"),
                "--file-b",
                path_b.to_str().expect("utf-8 b"),
                "--plot",
                plot_path.to_str().expect("utf-8 plot"),
            ])
            .output()
            .expect("spawn isingmark compare --plot");

        assert!(
            output.status.success(),
            "compare --plot must exit 0 with plot feature; stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_is_png(&plot_path);
    }

    fn write_energy_jsonl(path: &Path, energies_milli: &[i64]) {
        let mut file = std::fs::File::create(path).expect("create jsonl");
        for (i, &e) in energies_milli.iter().enumerate() {
            let rec = JobRecord {
                job_id: format!("{i:02x}"),
                backend: "cpu-sa".to_string(),
                n_solutions: 1,
                best_energy_milli: Some(e),
                best_spins_hex: None,
                diversity_milli: 0,
                passed: true,
                rejected: false,
                reject_reason: None,
                device_access_time_us: 0,
                wall_ms: 10,
                requested_reads: Some(64),
                requested_sweeps: Some(100),
                requested_anneal_time_us: Some(0),
                measured_reads: Some(64),
                measured_sweeps: Some(100),
            };
            let line = serde_json::to_string(&rec).expect("serialize");
            writeln!(file, "{line}").expect("write line");
        }
    }
}

/// Without the `plot` feature, `--plot` still parses and exits non-zero
/// with a message that names the feature and the rebuild command.
#[cfg(not(feature = "plot"))]
#[test]
fn plot_flag_without_feature_exits_non_zero_and_names_feature() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path_a = dir.path().join("a.jsonl");
    let path_b = dir.path().join("b.jsonl");
    let plot_path = dir.path().join("out.png");

    // Minimal passed records so compare would otherwise succeed.
    write_minimal_jsonl(&path_a, -1000);
    write_minimal_jsonl(&path_b, 1000);

    let bin = PathBuf::from(env!("CARGO_BIN_EXE_isingmark"));
    let output = std::process::Command::new(&bin)
        .args([
            "compare",
            "--file-a",
            path_a.to_str().expect("utf-8 a"),
            "--file-b",
            path_b.to_str().expect("utf-8 b"),
            "--plot",
            plot_path.to_str().expect("utf-8 plot"),
        ])
        .output()
        .expect("spawn isingmark compare --plot without feature");

    assert!(
        !output.status.success(),
        "--plot without feature must exit non-zero; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("plot"),
        "error must name the plot feature; got: {combined}"
    );
    assert!(
        combined.contains("features plot") || combined.contains("--features plot"),
        "error must name the rebuild command; got: {combined}"
    );
    assert!(
        !plot_path.exists(),
        "no plot file must be written when the feature is absent"
    );
}

#[cfg(not(feature = "plot"))]
fn write_minimal_jsonl(path: &Path, energy_milli: i64) {
    let rec = serde_json::json!({
        "job_id": "00",
        "backend": "cpu-sa",
        "n_solutions": 1,
        "best_energy_milli": energy_milli,
        "best_spins_hex": null,
        "diversity_milli": 0,
        "passed": true,
        "rejected": false,
        "reject_reason": null,
        "device_access_time_us": 0,
        "wall_ms": 10,
        "requested_reads": 64,
        "requested_sweeps": 100,
        "requested_anneal_time_us": 0,
        "measured_reads": 64,
        "measured_sweeps": 100,
    });
    let mut file = std::fs::File::create(path).expect("create jsonl");
    writeln!(file, "{rec}").expect("write line");
}
