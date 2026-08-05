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
