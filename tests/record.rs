// SPDX-License-Identifier: AGPL-3.0-or-later
//! Round-trip and shape tests for `isingmark::record`.

use std::fmt::Write as _;

use isingmark::record::{
    append_jsonl, read_jsonl, write_summary, HardwareInfo, JobRecord, JsonlRead, Summary,
    TopologyInfo, Totals,
};
use serde_json::json;

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// A record with `rejected = true` and `reject_reason = Some(..)`. Every
/// numeric/bool field is distinct and nonzero (and distinct from every
/// field in [`record_without_reject_reason`]) so a mutation that swaps two
/// same-typed fields during (de)serialization is detectable.
fn record_with_reject_reason() -> JobRecord {
    JobRecord {
        job_id: hex_encode(&[
            0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x60, 0x71, 0x82, 0x93, 0xa4, 0xb5, 0xc6, 0xd7,
            0xe8, 0xf9,
        ]),
        backend: "cpu-sa".to_string(),
        n_solutions: 3,
        best_energy_milli: Some(-15250),
        best_spins_hex: Some("01ff01".to_string()),
        diversity_milli: 275,
        passed: false,
        rejected: true,
        reject_reason: Some("energy_above_threshold".to_string()),
        device_access_time_us: 812,
        wall_ms: 40_150,
        requested_reads: 64,
        requested_sweeps: 500,
        requested_anneal_time_us: 90,
        measured_reads: Some(32),
        measured_sweeps: Some(250),
    }
}

/// A record with `rejected = false` and `reject_reason = None`. See
/// [`record_with_reject_reason`] for why every field value is distinct.
fn record_without_reject_reason() -> JobRecord {
    JobRecord {
        job_id: hex_encode(&[
            0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a,
            0x0b, 0x0c,
        ]),
        backend: "cuda".to_string(),
        n_solutions: 8,
        best_energy_milli: Some(-9375),
        best_spins_hex: None,
        diversity_milli: 611,
        passed: true,
        rejected: false,
        reject_reason: None,
        device_access_time_us: 333,
        wall_ms: 5_222,
        requested_reads: 128,
        requested_sweeps: 733,
        requested_anneal_time_us: 17,
        measured_reads: Some(128),
        measured_sweeps: Some(733),
    }
}

#[test]
fn round_trip_preserves_all_fields_for_both_reject_reason_cases() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("records.jsonl");

    let rec_a = record_with_reject_reason();
    let rec_b = record_without_reject_reason();
    append_jsonl(&path, &rec_a).expect("append first record");
    append_jsonl(&path, &rec_b).expect("append second record");

    let read_back = read_jsonl(&path).expect("read records back");
    assert_eq!(
        read_back,
        JsonlRead {
            records: vec![rec_a, rec_b],
            skipped_lines: 0,
        }
    );
}

#[test]
fn append_jsonl_writes_exactly_one_line_per_record() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("records.jsonl");

    append_jsonl(&path, &record_with_reject_reason()).expect("append first record");
    append_jsonl(&path, &record_without_reject_reason()).expect("append second record");
    append_jsonl(&path, &record_with_reject_reason()).expect("append third record");

    let raw = std::fs::read_to_string(&path).expect("read raw file");
    let lines: Vec<&str> = raw.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(raw.ends_with('\n'));
    for line in &lines {
        serde_json::from_str::<serde_json::Value>(line).expect("each line is one JSON object");
    }
}

#[test]
fn read_jsonl_skips_a_blank_trailing_line() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("records.jsonl");

    let rec_a = record_with_reject_reason();
    let rec_b = record_without_reject_reason();
    let content = format!(
        "{}\n{}\n\n",
        serde_json::to_string(&rec_a).expect("serialize rec_a"),
        serde_json::to_string(&rec_b).expect("serialize rec_b"),
    );
    std::fs::write(&path, content).expect("write file with blank trailing line");

    let read_back = read_jsonl(&path).expect("read records back");
    assert_eq!(
        read_back,
        JsonlRead {
            records: vec![rec_a, rec_b],
            skipped_lines: 0,
        }
    );
}

#[test]
fn read_jsonl_on_empty_file_returns_empty_vec_and_zero_skipped() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("records.jsonl");
    std::fs::write(&path, "").expect("write empty file");

    let read_back = read_jsonl(&path).expect("read_jsonl succeeds on an empty file");
    assert!(read_back.records.is_empty());
    assert_eq!(read_back.skipped_lines, 0);
}

/// Review finding 2: a malformed line used to void every record in the
/// file, not just itself. A single corrupt line (of the kind a partial
/// write could leave behind) must not cost the records before or after it,
/// and must be counted rather than silently dropped.
#[test]
fn read_jsonl_skips_a_malformed_middle_line_and_counts_it() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("records.jsonl");

    let rec_a = record_with_reject_reason();
    let rec_b = record_without_reject_reason();
    let content = format!(
        "{}\nnot valid json\n{}\n",
        serde_json::to_string(&rec_a).expect("serialize rec_a"),
        serde_json::to_string(&rec_b).expect("serialize rec_b"),
    );
    std::fs::write(&path, content).expect("write file with a malformed middle line");

    let read_back = read_jsonl(&path).expect("read_jsonl must not fail on one bad line");
    assert_eq!(read_back.records, vec![rec_a, rec_b]);
    assert_eq!(read_back.skipped_lines, 1);
}

/// Distinguishes a real counter from a hardcoded constant: two malformed
/// lines here must count as `2`, not the `1` the single-bad-line test above
/// would also accept.
#[test]
fn read_jsonl_counts_multiple_skipped_lines_distinctly() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("records.jsonl");

    let rec_a = record_with_reject_reason();
    let content = format!(
        "not valid json either\n{}\nstill not valid json\n",
        serde_json::to_string(&rec_a).expect("serialize rec_a"),
    );
    std::fs::write(&path, content).expect("write file with two malformed lines");

    let read_back = read_jsonl(&path).expect("read_jsonl must not fail on bad lines");
    assert_eq!(read_back.records, vec![rec_a]);
    assert_eq!(read_back.skipped_lines, 2);
}

/// Review finding 6: a process killed mid-`append_jsonl` can leave a
/// newline-less partial line at the end of the file. The next
/// `append_jsonl` call has no separator to write after, so its record lands
/// on the same physical line as the leftover fragment, producing one
/// unparseable merged line. That must cost only the record that collided
/// with the fragment — every prior record must survive and the file must
/// stay readable, not become permanently corrupt.
#[test]
fn read_jsonl_recovers_prior_records_after_a_crash_truncated_tail_line() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("records.jsonl");

    let good = record_with_reject_reason();
    let mut content = serde_json::to_string(&good).expect("serialize good record");
    content.push('\n');
    // Simulates a crash mid-write: a syntactically incomplete JSON value
    // with no trailing newline, as `append_jsonl` would leave behind if the
    // process died after writing only part of a record.
    content.push_str(r#"{"job_id":"crash-truncated-fragmen"#);
    std::fs::write(&path, &content).expect("write file with a crash-truncated tail");

    append_jsonl(&path, &record_without_reject_reason())
        .expect("append after a crash-truncated tail");

    let read_back = read_jsonl(&path).expect("read_jsonl must not fail on the corrupt tail");
    assert_eq!(read_back.records, vec![good]);
    assert_eq!(read_back.skipped_lines, 1);
}

fn summary() -> Summary {
    Summary {
        mode: "throughput".to_string(),
        backend: "cpu-sa".to_string(),
        topology: TopologyInfo {
            name: "smoke".to_string(),
            num_nodes: 8,
            num_edges: 5,
            topology_hash: hex_encode(&[
                0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
                0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b,
                0x1c, 0x1d, 0x1e, 0x1f,
            ]),
        },
        hardware: HardwareInfo {
            platform: "macos".to_string(),
            cpu: "Apple M4 Max".to_string(),
            ram_gb: 64.0,
        },
        config: json!({"num_reads": 64, "num_sweeps": 500}),
        totals: Totals {
            jobs: 10,
            passed: 9,
            rejected: 1,
            skipped_lines: 2,
        },
    }
}

#[test]
fn write_summary_round_trips_through_typed_deserialize() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("summary.json");

    let original = summary();
    write_summary(&path, &original).expect("write summary");

    let raw = std::fs::read_to_string(&path).expect("read summary file");
    let read_back: Summary = serde_json::from_str(&raw).expect("summary parses as Summary");
    assert_eq!(read_back, original);
}

#[test]
fn summary_json_contains_topology_hardware_config_and_totals_keys() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("summary.json");

    write_summary(&path, &summary()).expect("write summary");
    let raw = std::fs::read_to_string(&path).expect("read summary file");
    let value: serde_json::Value = serde_json::from_str(&raw).expect("summary is valid JSON");

    assert_eq!(value["mode"], "throughput");
    assert_eq!(value["backend"], "cpu-sa");

    assert_eq!(value["topology"]["name"], "smoke");
    assert_eq!(value["topology"]["num_nodes"], 8);
    assert_eq!(value["topology"]["num_edges"], 5);
    assert_eq!(
        value["topology"]["topology_hash"],
        "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
    );

    assert_eq!(value["hardware"]["platform"], "macos");
    assert_eq!(value["hardware"]["cpu"], "Apple M4 Max");
    assert_eq!(value["hardware"]["ram_gb"], 64.0);

    assert_eq!(value["config"]["num_reads"], 64);
    assert_eq!(value["config"]["num_sweeps"], 500);

    assert_eq!(value["totals"]["jobs"], 10);
    assert_eq!(value["totals"]["passed"], 9);
    assert_eq!(value["totals"]["rejected"], 1);
    assert_eq!(value["totals"]["skipped_lines"], 2);
}
