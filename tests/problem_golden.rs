// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cross-implementation golden check for `problem::ProblemSource` against a
//! real `quip-coordinator drive --source random` run.
//!
//! Regenerate `fixtures/golden/smoke_seed42.jsonl` with:
//!
//! ```sh
//! mkdir -p fixtures/golden
//! quip-coordinator drive \
//!   --miner <path-to-a-cpu-sa-miner-binary> \
//!   --source random --topology-preset smoke --seed 42 --count 8 \
//!   --report fixtures/golden/smoke_seed42.jsonl
//! ```
//!
//! (The fixture committed here was generated with `--topology
//! fixtures/smoke.spec.json` instead of `--topology-preset smoke`, because
//! the release binary used to generate it had `CARGO_MANIFEST_DIR` baked in
//! from a stale checkout path and could not resolve the preset fixture. That
//! file is byte-identical to `quip-coordinator`'s own
//! `fixtures/drive/smoke.spec.json` — verified with `diff` before use — so
//! this is equivalent to the command above.)
//!
//! The report's per-job lines are in miner *completion* order, not job
//! *submission* order: `quip-coordinator drive` dispatches jobs to the miner
//! with `queue_depth = 3`, so completion order is not guaranteed to match
//! generation order, and is not reproducible run-to-run even for the same
//! seed (confirmed by running the same command three times: the `job_id`
//! *set* was identical all three times, but the order differed every time).
//! The draw itself does not depend on ordering — `salt_for(seed, index)` is a
//! pure function of `(seed, index)` — so this test compares job-id *sets*,
//! sorted, rather than asserting a specific sequence.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use isingmark::problem::ProblemSource;
use isingmark::topology::load_fixture;

const GOLDEN_PATH: &str = "fixtures/golden/smoke_seed42.jsonl";

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// `job_id`s from the golden report's per-job lines, in file order. The
/// aggregate summary line (no `job_id` field) is skipped.
fn golden_job_ids() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_PATH);
    let text = std::fs::read_to_string(&path)
        .expect("golden fixture fixtures/golden/smoke_seed42.jsonl must be readable");
    text.lines()
        .filter_map(|line| {
            let value: serde_json::Value =
                serde_json::from_str(line).expect("golden fixture line is valid JSON");
            value
                .get("job_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect()
}

#[test]
fn job_id_set_matches_real_quip_coordinator_drive_output() {
    let golden = golden_job_ids();
    assert_eq!(
        golden.len(),
        8,
        "golden fixture must have exactly 8 per-job lines"
    );

    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let mut src = ProblemSource::new(&spec, [0u8; 32], 42, 8, 0);
    let mut computed = Vec::new();
    while let Some(job) = src.next_job() {
        computed.push(hex_encode(&job.job_id));
    }

    let golden_set: BTreeSet<&str> = golden.iter().map(String::as_str).collect();
    let computed_set: BTreeSet<&str> = computed.iter().map(String::as_str).collect();
    assert_eq!(
        computed_set, golden_set,
        "computed job_id set (left) must match quip-coordinator drive's golden \
         report (right); golden order: {golden:?}, computed order: {computed:?}"
    );
}
