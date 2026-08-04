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
//! ## Why there are two golden fixtures
//!
//! The report's per-job lines are in miner *completion* order, not job
//! *submission* order: `quip-coordinator drive` dispatches jobs to the miner
//! with `queue_depth = 3`, so completion order is not guaranteed to match
//! generation order, and is not reproducible run-to-run even for the same
//! seed (confirmed by running the same command three times: the `job_id`
//! *set* was identical all three times, but the order differed every time).
//! `fixtures/golden/smoke_seed42.jsonl` is one such run, used only to pin the
//! `job_id` *set*.
//!
//! `fixtures/golden/smoke_seed42_order.json` pins the true generation order
//! (index 0..7), recovered without relying on any of this crate's own code:
//! run the same drive command with `--count 1`, then `--count 2`, ...,
//! `--count 8`. Each `--count n` report's `job_id` set is a strict superset of
//! the `--count n-1` report's set (drive mode always draws indices `0..n` in
//! order and only completion order is scrambled), so the single `job_id` that
//! is new at `--count n` is generation index `n-1`. Regenerate with:
//!
//! ```sh
//! for n in 1 2 3 4 5 6 7 8; do
//!   quip-coordinator drive \
//!     --miner <path-to-a-cpu-sa-miner-binary> \
//!     --source random --topology-preset smoke --seed 42 --count "$n" \
//!     --report "fixtures/golden/smoke_seed42_count${n}.jsonl"
//! done
//! # index (n-1) = the `job_id` in count-n's report that isn't in count-(n-1)'s
//! ```
//!
//! then diff each report's `job_id` set against the previous one to read off
//! the new element. This was run three times end-to-end; every step added
//! exactly one new `job_id`, and the recovered order was identical each time
//! (and matches computing `salt_for(42, index)` directly for `index` in
//! `0..8`, which is expected since the two are the same computation done two
//! different ways). The per-count intermediate reports are not committed —
//! only the recovered order is — since they are reproducible from the
//! command above and add no information beyond it.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;

use isingmark::problem::ProblemSource;
use isingmark::topology::load_fixture;
use quip_proto::v1::ising_problem::Graph;
use quip_proto::v1::JobKind;

const GOLDEN_PATH: &str = "fixtures/golden/smoke_seed42.jsonl";
const GOLDEN_ORDER_PATH: &str = "fixtures/golden/smoke_seed42_order.json";

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

/// `job_id`s in true generation order (index 0..7), recovered via the
/// `--count 1..8` procedure documented above.
fn golden_order() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(GOLDEN_ORDER_PATH);
    let text = std::fs::read_to_string(&path)
        .expect("golden fixture fixtures/golden/smoke_seed42_order.json must be readable");
    serde_json::from_str(&text).expect("golden order fixture is a JSON array of strings")
}

/// Nonzero, so `job_shape_matches_the_upstream_pow_job_convention` can
/// distinguish `ProblemSource` forwarding this value from a hardcoded `0`.
/// The draw itself (`job_id`, `h`/`j`) does not depend on `deadline_ms`, so
/// this has no effect on the other two tests in this file.
const DEADLINE_MS: u64 = 12_345;

fn drain_computed(spec: &isingmark::topology::TopologySpec) -> Vec<quip_proto::v1::Job> {
    let mut src = ProblemSource::new(spec, [0u8; 32], 42, 8, DEADLINE_MS);
    let mut jobs = Vec::new();
    while let Some(job) = src.next_job() {
        jobs.push(job);
    }
    jobs
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
    let computed: Vec<String> = drain_computed(&spec)
        .iter()
        .map(|job| hex_encode(&job.job_id))
        .collect();

    let golden_set: BTreeSet<&str> = golden.iter().map(String::as_str).collect();
    let computed_set: BTreeSet<&str> = computed.iter().map(String::as_str).collect();
    assert_eq!(
        computed_set, golden_set,
        "computed job_id set (left) must match quip-coordinator drive's golden \
         report (right); golden order: {golden:?}, computed order: {computed:?}"
    );
}

#[test]
fn job_id_sequence_matches_recovered_upstream_generation_order() {
    let order = golden_order();
    assert_eq!(
        order.len(),
        8,
        "recovered order fixture must have exactly 8 entries"
    );

    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let computed: Vec<String> = drain_computed(&spec)
        .iter()
        .map(|job| hex_encode(&job.job_id))
        .collect();

    assert_eq!(
        computed, order,
        "ProblemSource must emit job_ids in the same order as upstream's \
         generation order (index 0..7), recovered independently via the \
         --count 1..8 procedure documented at the top of this file"
    );
}

#[test]
fn job_shape_matches_the_upstream_pow_job_convention() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    let expected_graph = Some(Graph::TopologyHash(spec.topology_hash()));

    for (index, job) in drain_computed(&spec).into_iter().enumerate() {
        let generation = u64::try_from(index).expect("index fits in u64") + 1;
        assert_eq!(job.generation, generation, "job {index}: generation");
        assert_eq!(
            job.kind,
            JobKind::IsingSample as i32,
            "job {index}: kind must be ISING_SAMPLE"
        );
        assert_eq!(job.deadline_ms, DEADLINE_MS, "job {index}: deadline_ms");
        assert!(job.provenance.is_some(), "job {index}: missing provenance");
        let provenance = job.provenance.as_ref().expect("checked above");
        assert!(provenance.is_pow, "job {index}: provenance.is_pow");
        assert!(
            provenance.order_id.is_empty(),
            "job {index}: provenance.order_id must be empty for a PoW job"
        );
        assert!(job.ising.is_some(), "job {index}: missing ising problem");
        let ising = job.ising.as_ref().expect("checked above");
        assert_eq!(
            ising.graph, expected_graph,
            "job {index}: graph must reference the topology hash"
        );
    }
}
