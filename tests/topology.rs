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

/// The two chain-corpus fixtures must hash to the `topology_hash` the chain
/// recorded for their bucket, or the corpora in `corpora/` replay against the
/// wrong graph and every drawn problem is silently different from the one the
/// miner actually solved. The expected values are the bucket directory names
/// under the `quip-coordinator download` output that produced the corpora.
#[test]
fn chain_fixtures_hash_to_their_recorded_bucket() {
    for (name, nodes, edges, bucket_hash) in [
        (
            "chain-h0",
            4577,
            41515,
            "e66d3dfa3c9c6afb15efe29891cf9412498d94692ab5956af3ad98ba3693a02d",
        ),
        (
            "chain-ternary",
            4578,
            41531,
            "6eae38a92b27e8d353ea3b0fa311952c06e4f0a0ba6e5d705ca116b44a42c4e5",
        ),
    ] {
        let spec = load_fixture(name).expect("chain fixture parses");
        assert_eq!(spec.nodes.len(), nodes, "{name} node count");
        assert_eq!(spec.edges.len(), edges, "{name} edge count");
        assert_eq!(
            spec.topology_hash(),
            hex_decode(bucket_hash),
            "{name} must hash to the chain bucket it claims to be"
        );
    }
}

/// `advantage2-system1` carries the `chain-h0` graph with the `chain-ternary`
/// allowed-h set, so it matches neither chain bucket. It stays as a synthetic
/// benchmark topology; this pins the fact so nobody replays a corpus against
/// it by mistake.
#[test]
fn advantage2_system1_is_not_a_chain_bucket() {
    let synthetic = load_fixture("advantage2-system1").expect("fixture parses");
    for name in ["chain-h0", "chain-ternary"] {
        let chain = load_fixture(name).expect("chain fixture parses");
        assert_ne!(
            synthetic.topology_hash(),
            chain.topology_hash(),
            "advantage2-system1 must not be mistaken for {name}"
        );
    }
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

/// Every graph this crate ships, paired with its `-h0` variant. The h0
/// regime (no local fields) is the one the chain mines in, so every
/// topology needs an h0 counterpart to benchmark against.
const H0_PAIRS: [(&str, &str); 7] = [
    ("smoke", "smoke-h0"),
    ("cuopt-small", "cuopt-small-h0"),
    ("common-small", "common-small-h0"),
    ("chimera-c8-16-4", "chimera-c8-16-4-h0"),
    ("pegasus-p16", "pegasus-p16-h0"),
    ("advantage-system6", "advantage-system6-h0"),
    ("chain-ternary", "chain-ternary-h0"),
];

/// An h0 variant must be the same graph as its base with the fields removed.
/// If the two ever describe different graphs, a comparison between the h0 and
/// non-h0 regimes stops being a controlled experiment.
#[test]
fn every_h0_variant_is_its_base_graph_with_no_fields() {
    for (base_name, h0_name) in H0_PAIRS {
        let base = load_fixture(base_name).expect("base fixture parses");
        let h0 = load_fixture(h0_name).expect("h0 fixture parses");
        assert_eq!(h0.nodes, base.nodes, "{h0_name} node list");
        assert_eq!(h0.edges, base.edges, "{h0_name} edge list");
        assert_eq!(
            h0.allowed_j_milli, base.allowed_j_milli,
            "{h0_name} allowed_j"
        );
        assert_eq!(
            h0.allowed_spin_milli, base.allowed_spin_milli,
            "{h0_name} allowed_spin"
        );
        assert_eq!(
            h0.allowed_h_milli,
            vec![0],
            "{h0_name} must zero the fields"
        );
        assert_ne!(
            h0.topology_hash(),
            base.topology_hash(),
            "{h0_name} is a distinct topology from {base_name}"
        );
    }
}

/// `chain-h0` already is the h0 form of the Advantage2 machine graph, and
/// `advantage2-system1` is that same graph with ternary fields. They are the
/// h0/non-h0 pair for that machine, so no separate variant is needed.
#[test]
fn advantage2_system1_is_the_chain_h0_graph_with_ternary_fields() {
    let ternary = load_fixture("advantage2-system1").expect("fixture parses");
    let h0 = load_fixture("chain-h0").expect("fixture parses");
    assert_eq!(ternary.nodes, h0.nodes);
    assert_eq!(ternary.edges, h0.edges);
    assert_eq!(ternary.allowed_h_milli, vec![-1000, 0, 1000]);
    assert_eq!(h0.allowed_h_milli, vec![0]);
}

/// Working-graph sizes read from the live QPUs on 2026-08-07. These are
/// machine graphs, not idealized ones, so the counts are the record of what
/// the hardware actually had.
#[test]
fn machine_fixtures_match_their_recorded_working_graphs() {
    for (name, nodes, edges) in [
        // Advantage2_system1, Zephyr [12, 4], 4800 nominal qubits.
        ("chain-h0", 4577, 41515),
        // Advantage_system6, Pegasus [16], 5760 nominal qubits.
        ("advantage-system6", 5612, 40088),
    ] {
        let spec = load_fixture(name).expect("fixture parses");
        assert_eq!(spec.nodes.len(), nodes, "{name} qubits");
        assert_eq!(spec.edges.len(), edges, "{name} couplers");
    }
}

#[test]
fn extends_inherits_the_base_graph_and_overrides_only_what_it_sets() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("derived.spec.json");
    std::fs::write(&path, r#"{"extends":"smoke","allowed_h_milli":[0]}"#).expect("write");
    let derived = load_topology(&path).expect("derived spec parses");
    let base = load_fixture("smoke").expect("smoke parses");
    assert_eq!(derived.nodes, base.nodes);
    assert_eq!(derived.edges, base.edges);
    assert_eq!(derived.allowed_j_milli, base.allowed_j_milli);
    assert_eq!(derived.allowed_h_milli, vec![0]);
}

#[test]
fn extends_with_its_own_graph_is_rejected() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("both.spec.json");
    std::fs::write(
        &path,
        r#"{"extends":"smoke","nodes":[0,1],"allowed_h_milli":[0]}"#,
    )
    .expect("write");
    assert_eq!(
        load_topology(&path).expect_err("must reject"),
        TopologyError::ExtendsWithOwnGraph
    );
}

#[test]
fn extends_of_an_extends_spec_is_rejected() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("deep.spec.json");
    // `smoke-h0` is itself an `extends` spec, so this is two levels.
    std::fs::write(&path, r#"{"extends":"smoke-h0"}"#).expect("write");
    assert_eq!(
        load_topology(&path).expect_err("must reject"),
        TopologyError::ExtendsNotABase("smoke-h0".to_string())
    );
}

#[test]
fn extends_naming_a_missing_fixture_is_rejected() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("missing.spec.json");
    std::fs::write(&path, r#"{"extends":"no-such-fixture"}"#).expect("write");
    let err = load_topology(&path).expect_err("must reject");
    assert!(
        matches!(err, TopologyError::ExtendsUnloadable(ref n, _) if n == "no-such-fixture"),
        "{err}"
    );
}

/// `extends` resolves through the same name guard as `--topology-preset`, so
/// it cannot be used to read a file outside `fixtures/`.
#[test]
fn extends_cannot_escape_the_fixtures_directory() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("escape.spec.json");
    std::fs::write(&path, r#"{"extends":"../../../etc/passwd"}"#).expect("write");
    let err = load_topology(&path).expect_err("must reject");
    // The name guard fires before any file is opened, so this surfaces as the
    // guard's own message rather than a wrapped load failure.
    assert!(matches!(err, TopologyError::Json(_)), "{err}");
    assert!(err.to_string().contains("must match [A-Za-z0-9-]"), "{err}");
}
