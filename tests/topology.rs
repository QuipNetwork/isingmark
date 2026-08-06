// SPDX-License-Identifier: AGPL-3.0-or-later
use std::path::Path;

use isingmark::topology::{load_fixture, load_topology, TopologyError};

#[test]
fn loads_smoke_fixture() {
    let spec = load_fixture("smoke").expect("smoke fixture parses");
    assert_eq!(spec.nodes.len(), 8);
    assert_eq!(spec.edges.len(), 8);
    assert_eq!(
        spec.allowed_spin_milli,
        vec![-1000, 1000],
        "allowed_spin_milli defaults to the binary set when absent from the fixture"
    );
}

#[test]
fn loads_advantage2_system1_fixture() {
    let spec = load_fixture("advantage2-system1").expect("advantage2-system1 fixture parses");
    assert_eq!(spec.nodes.len(), 4577);
    assert_eq!(spec.edges.len(), 41515);
}

/// BEIT coefficient cap: worst-case non-zero count is `edges + nodes` (every
/// `J` non-zero, every `h` non-zero). Fixtures for BEIT must stay under 5000.
const BEIT_MAX_NONZERO_COEFFICIENTS: usize = 5000;

/// Decode Chimera `C(8,16,4)` linear qubit index
/// `q = 8 * (16 * i + j) + 4 * u + k` into cell row/col, partition, offset.
fn chimera_decode(qubit: u32) -> (u32, u32, u32, u32) {
    let cell = qubit / 8;
    let rem = qubit % 8;
    let row = cell / 16;
    let col = cell % 16;
    let partition = rem / 4;
    let offset = rem % 4;
    (row, col, partition, offset)
}

/// True when `(left, right)` is an internal, vertical, or horizontal Chimera
/// `C(8,16,4)` edge under the three adjacency rules.
fn is_chimera_c8_16_4_edge(left: u32, right: u32) -> bool {
    let (row0, col0, part0, off0) = chimera_decode(left);
    let (row1, col1, part1, off1) = chimera_decode(right);
    // Internal: same cell, opposite partitions, any offsets.
    if row0 == row1 && col0 == col1 && part0 != part1 {
        return true;
    }
    // Vertical: same column, both partition 0, same offset, adjacent rows.
    if col0 == col1 && part0 == 0 && part1 == 0 && off0 == off1 && row0.abs_diff(row1) == 1 {
        return true;
    }
    // Horizontal: same row, both partition 1, same offset, adjacent columns.
    if row0 == row1 && part0 == 1 && part1 == 1 && off0 == off1 && col0.abs_diff(col1) == 1 {
        return true;
    }
    false
}

#[test]
fn chimera_c8_16_4_fixture_structure_and_beit_cap() {
    let spec = load_fixture("chimera-c8-16-4").expect("chimera-c8-16-4 fixture parses");
    assert_eq!(spec.nodes.len(), 1024);
    assert_eq!(spec.edges.len(), 2976);
    assert!(
        spec.edges.len() + spec.nodes.len() <= BEIT_MAX_NONZERO_COEFFICIENTS,
        "edges + nodes = {} exceeds BEIT cap {BEIT_MAX_NONZERO_COEFFICIENTS}",
        spec.edges.len() + spec.nodes.len()
    );
    for &(u, v) in &spec.edges {
        assert!(
            is_chimera_c8_16_4_edge(u, v),
            "edge ({u}, {v}) is not a Chimera C(8,16,4) adjacency"
        );
    }
}

#[test]
fn zephyr_538_fixture_is_induced_advantage2_subgraph_under_beit_cap() {
    let parent = load_fixture("advantage2-system1").expect("advantage2-system1 fixture parses");
    let sub = load_fixture("zephyr-538").expect("zephyr-538 fixture parses");

    assert!(
        sub.edges.len() + sub.nodes.len() <= BEIT_MAX_NONZERO_COEFFICIENTS,
        "edges + nodes = {} exceeds BEIT cap {BEIT_MAX_NONZERO_COEFFICIENTS}",
        sub.edges.len() + sub.nodes.len()
    );

    let parent_nodes: std::collections::HashSet<u32> = parent.nodes.iter().copied().collect();
    let parent_edges: std::collections::HashSet<(u32, u32)> =
        parent.edges.iter().copied().collect();
    let sub_nodes: std::collections::HashSet<u32> = sub.nodes.iter().copied().collect();

    for &n in &sub.nodes {
        assert!(
            parent_nodes.contains(&n),
            "zephyr-538 node {n} is absent from advantage2-system1"
        );
    }
    for &(u, v) in &sub.edges {
        assert!(
            parent_edges.contains(&(u, v)),
            "zephyr-538 edge ({u}, {v}) is absent from advantage2-system1"
        );
        assert!(
            sub_nodes.contains(&u) && sub_nodes.contains(&v),
            "zephyr-538 edge ({u}, {v}) endpoints must both be selected nodes"
        );
    }

    // Induced edge set: every parent edge with both ends selected is present.
    let expected_induced: std::collections::HashSet<(u32, u32)> = parent
        .edges
        .iter()
        .copied()
        .filter(|&(u, v)| sub_nodes.contains(&u) && sub_nodes.contains(&v))
        .collect();
    let actual_edges: std::collections::HashSet<(u32, u32)> = sub.edges.iter().copied().collect();
    assert_eq!(
        actual_edges, expected_induced,
        "zephyr-538 edge set must equal the induced subgraph of the selected nodes"
    );
}

#[test]
fn rejects_empty_nodes() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("empty-nodes.spec.json");
    std::fs::write(
        &path,
        r#"{"nodes":[],"edges":[],"allowed_h_milli":[1],"allowed_j_milli":[1]}"#,
    )
    .expect("write fixture");
    assert_eq!(
        load_topology(&path).expect_err("empty nodes must be rejected"),
        TopologyError::EmptyNodes
    );
}

#[test]
fn rejects_empty_allowed_h() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("empty-h.spec.json");
    std::fs::write(
        &path,
        r#"{"nodes":[0],"edges":[],"allowed_h_milli":[],"allowed_j_milli":[1]}"#,
    )
    .expect("write fixture");
    assert_eq!(
        load_topology(&path).expect_err("empty allowed_h_milli must be rejected"),
        TopologyError::EmptyAllowedH
    );
}

#[test]
fn rejects_empty_allowed_j() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("empty-j.spec.json");
    std::fs::write(
        &path,
        r#"{"nodes":[0],"edges":[],"allowed_h_milli":[1],"allowed_j_milli":[]}"#,
    )
    .expect("write fixture");
    assert_eq!(
        load_topology(&path).expect_err("empty allowed_j_milli must be rejected"),
        TopologyError::EmptyAllowedJ
    );
}

#[test]
fn rejects_edge_with_unknown_node_id() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("bad-edge.spec.json");
    std::fs::write(
        &path,
        r#"{
            "nodes": [0, 1],
            "edges": [[0, 1], [1, 5]],
            "allowed_h_milli": [1000],
            "allowed_j_milli": [1000]
        }"#,
    )
    .expect("write fixture");
    assert_eq!(
        load_topology(&path).expect_err("edge referencing unknown node must be rejected"),
        TopologyError::EdgeUnknownNode {
            edge_index: 1,
            endpoint: 5,
        }
    );
}

// The next three tests must discriminate a sanitizer rejection from a merely
// missing file: `TopologyError::Json` also carries I/O errors, so asserting
// only `matches!(err, TopologyError::Json(_))` (as an earlier version of this
// file did) passes whether or not the `[A-Za-z0-9-]` allowlist runs at all.
// Each asserts on the sanitizer's own message text instead, and the first two
// pick names that would resolve to the real `smoke` fixture if the allowlist
// were removed, so a deleted guard fails the test rather than silently
// passing.

#[test]
fn rejects_fixture_name_that_escapes_via_dotdot() {
    assert!(
        load_fixture("smoke").is_ok(),
        "the fixture this escape targets must actually exist for the test to prove anything"
    );
    // Without the allowlist this resolves to the real, existing
    // fixtures/smoke.spec.json (fixtures/../fixtures/smoke.spec.json).
    let err = load_fixture("../fixtures/smoke").expect_err("dotdot traversal must be rejected");
    assert!(
        matches!(&err, TopologyError::Json(msg) if msg.contains("must match [A-Za-z0-9-]")),
        "expected rejection by the name sanitizer, got {err:?}"
    );
}

#[test]
fn rejects_absolute_fixture_name() {
    // Without the allowlist, `PathBuf::join` on an absolute path discards the
    // fixtures-dir base entirely, so this also resolves to the real fixture.
    let absolute = format!("{}/fixtures/smoke", env!("CARGO_MANIFEST_DIR"));
    let err = load_fixture(&absolute).expect_err("absolute path must be rejected");
    assert!(
        matches!(&err, TopologyError::Json(msg) if msg.contains("must match [A-Za-z0-9-]")),
        "expected rejection by the name sanitizer, got {err:?}"
    );
}

#[test]
fn rejects_fixture_name_with_embedded_path_separator() {
    let err = load_fixture("sub/smoke").expect_err("embedded separator must be rejected");
    assert!(
        matches!(&err, TopologyError::Json(msg) if msg.contains("must match [A-Za-z0-9-]")),
        "expected rejection by the name sanitizer, got {err:?}"
    );
}

#[test]
fn load_topology_rejects_missing_file() {
    let err = load_topology(Path::new("fixtures/does-not-exist.spec.json"))
        .expect_err("missing file must be rejected");
    assert!(matches!(err, TopologyError::Json(_)));
}

#[test]
fn topology_hash_matches_upstream_golden_vector() {
    // Pinned against `quip-coordinator`'s `golden_topology_hash_known_fixture`
    // (`crates/quip-coordinator/src/topology.rs`): sorted nodes, min/max-normalized
    // sorted edges, set-canonical allowed-value bytes, SCALE-encoded tuple, blake2b-256.
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("golden.spec.json");
    std::fs::write(
        &path,
        r#"{
            "nodes": [0, 1, 2],
            "edges": [[0, 1], [1, 2]],
            "allowed_h_milli": [-1000, 0, 1000],
            "allowed_j_milli": [-1000, 1000],
            "allowed_spin_milli": [-1000, 1000]
        }"#,
    )
    .expect("write fixture");
    let spec = load_topology(&path).expect("golden fixture parses");
    let hash = spec.topology_hash();
    let expected = hex_decode("56dff5ca824517ba7f0593ec12a5dd102eb0a14f775b35c415467e0e0d6e19c2");
    assert_eq!(hash, expected);
}

#[test]
fn topology_hash_is_order_independent_for_nodes_and_edges() {
    let a = topology_from(&[2, 0, 1], &[(1, 0), (2, 1)], &[-1000, 0, 1000]).topology_hash();
    let b = topology_from(&[0, 1, 2], &[(0, 1), (1, 2)], &[-1000, 0, 1000]).topology_hash();
    assert_eq!(a, b);
}

// Ported from upstream's `hash_is_order_independent_for_set_elements`
// (`quip-coordinator/src/topology.rs`). This is the one test that exercises
// `canonical_set_bytes`'s `sort_unstable()` — the hand-reimplemented step in
// `topology_hash` — rather than `topology_hash` itself sorting nodes/edges.
#[test]
fn topology_hash_is_order_independent_for_set_elements() {
    let a = topology_from(&[0, 1], &[(0, 1)], &[1000, -1000, 0]).topology_hash();
    let b = topology_from(&[0, 1], &[(0, 1)], &[-1000, 0, 1000]).topology_hash();
    assert_eq!(a, b);
}

fn topology_from(
    nodes: &[u32],
    edges: &[(u32, u32)],
    allowed_h_milli: &[i32],
) -> isingmark::topology::TopologySpec {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("spec.json");
    let json = serde_json::json!({
        "nodes": nodes,
        "edges": edges,
        "allowed_h_milli": allowed_h_milli,
        "allowed_j_milli": [-1000, 1000],
    });
    std::fs::write(&path, json.to_string()).expect("write fixture");
    load_topology(&path).expect("fixture parses")
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| {
            #[expect(
                clippy::unwrap_used,
                reason = "fixed 64-char hex golden constant in test code"
            )]
            u8::from_str_radix(&s[i..i + 2], 16).unwrap()
        })
        .collect()
}
