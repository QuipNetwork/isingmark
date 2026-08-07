// SPDX-License-Identifier: AGPL-3.0-or-later
//! Topology-spec loading and validation.
//!
//! Mirrors the JSON shape and validation of `quip-coordinator`'s drive-mode
//! topology spec (`crates/quip-coordinator/src/drive/topology_spec.rs`): a
//! node list, an edge list referencing node ids, and the allowed h/J/spin
//! value sets used to draw a problem. `allowed_spin_milli` defaults to the
//! binary set `[-1000, 1000]` when the JSON omits it.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use blake2::digest::consts::U32;
use blake2::{Blake2b, Digest};
use parity_scale_codec::Encode;
use serde::Deserialize;

/// Binary spin set used when `allowed_spin_milli` is absent from the spec.
const DEFAULT_ALLOWED_SPIN_MILLI: [i32; 2] = [-1000, 1000];

type Blake2b256 = Blake2b<U32>;

/// A validated topology spec: nodes, edges, and the allowed value sets used
/// to draw an Ising problem over that graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopologySpec {
    /// Node ids in the graph (order preserved; ids may be sparse).
    pub nodes: Vec<u32>,
    /// Edges as `(u, v)` node-id pairs.
    pub edges: Vec<(u32, u32)>,
    /// Allowed h-field values (milli).
    pub allowed_h_milli: Vec<i32>,
    /// Allowed J-coupling values (milli).
    pub allowed_j_milli: Vec<i32>,
    /// Allowed solution-spin values (milli); defaults to `[-1000, 1000]`
    /// when absent from the JSON.
    pub allowed_spin_milli: Vec<i32>,
}

/// Errors parsing or validating a topology-spec JSON document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyError {
    /// The file could not be read, or its contents could not be decoded as
    /// the topology-spec JSON shape. The message is the underlying I/O or
    /// serde error.
    Json(String),
    /// `nodes` was empty.
    EmptyNodes,
    /// `allowed_h_milli` was empty.
    EmptyAllowedH,
    /// `allowed_j_milli` was empty.
    EmptyAllowedJ,
    /// An `edges` endpoint is a node id not present in `nodes`.
    EdgeUnknownNode {
        /// Zero-based index of the bad edge in the `edges` array.
        edge_index: usize,
        /// Node id that was not present in `nodes`.
        endpoint: u32,
    },
    /// The spec set `extends` and also carried its own `nodes` or `edges`.
    /// Rejected rather than picking one, because the two could disagree and
    /// the graph a run actually used would depend on that choice.
    ExtendsWithOwnGraph,
    /// `extends` named a fixture that itself sets `extends`. Inheritance is
    /// one level deep, so a base must carry a real graph.
    ExtendsNotABase(String),
    /// `extends` named a fixture that could not be loaded. The message is
    /// the underlying error.
    ExtendsUnloadable(String, Box<TopologyError>),
}

impl std::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(e) => write!(f, "invalid topology spec: {e}"),
            Self::EmptyNodes => write!(f, "topology spec: nodes must be non-empty"),
            Self::EmptyAllowedH => {
                write!(f, "topology spec: allowed_h_milli must be non-empty")
            }
            Self::EmptyAllowedJ => {
                write!(f, "topology spec: allowed_j_milli must be non-empty")
            }
            Self::EdgeUnknownNode {
                edge_index,
                endpoint,
            } => write!(
                f,
                "topology spec: edge {edge_index} references node id {endpoint}, \
                 which is not in the topology"
            ),
            Self::ExtendsWithOwnGraph => write!(
                f,
                "topology spec: `extends` inherits nodes and edges, so the spec \
                 must not also set `nodes` or `edges`"
            ),
            Self::ExtendsNotABase(name) => write!(
                f,
                "topology spec: `extends` names {name:?}, which itself uses \
                 `extends`; inheritance is one level deep, so extend its base \
                 directly"
            ),
            Self::ExtendsUnloadable(name, e) => write!(
                f,
                "topology spec: `extends` names {name:?}, which failed to load: {e}"
            ),
        }
    }
}

impl std::error::Error for TopologyError {}

#[derive(Debug, Deserialize)]
struct TopologySpecJson {
    /// Bundled fixture to inherit `nodes` and `edges` from. When set, this
    /// spec must not carry its own graph, and any allowed-value set it omits
    /// is inherited too.
    #[serde(default)]
    extends: Option<String>,
    #[serde(default)]
    nodes: Option<Vec<u32>>,
    #[serde(default)]
    edges: Option<Vec<(u32, u32)>>,
    #[serde(default)]
    allowed_h_milli: Option<Vec<i32>>,
    #[serde(default)]
    allowed_j_milli: Option<Vec<i32>>,
    #[serde(default)]
    allowed_spin_milli: Option<Vec<i32>>,
}

fn validate_edges(nodes: &[u32], edges: &[(u32, u32)]) -> Result<(), TopologyError> {
    let node_set: HashSet<u32> = nodes.iter().copied().collect();
    for (edge_index, &(u, v)) in edges.iter().enumerate() {
        for endpoint in [u, v] {
            if !node_set.contains(&endpoint) {
                return Err(TopologyError::EdgeUnknownNode {
                    edge_index,
                    endpoint,
                });
            }
        }
    }
    Ok(())
}

/// Load and validate a topology spec from a JSON file at `path`.
///
/// A spec either carries its own `nodes`/`edges`, or sets `extends` to the
/// name of a bundled fixture and inherits that fixture's graph. An `extends`
/// spec may override any allowed-value set and inherits the ones it omits.
/// This exists so a topology and its `allowed_h_milli: [0]` variant cannot
/// drift apart: they are one graph, described once.
///
/// # Errors
///
/// Returns [`TopologyError`] when the file cannot be read, its contents are
/// not valid topology-spec JSON, required fields are empty or absent, an edge
/// references an unknown node id, or `extends` names a fixture that is
/// missing, unloadable, or itself an `extends` spec.
pub fn load_topology(path: &Path) -> Result<TopologySpec, TopologyError> {
    let raw = read_spec(path)?;
    resolve_spec(raw)
}

/// Read and JSON-decode a spec file without resolving `extends`.
fn read_spec(path: &Path) -> Result<TopologySpecJson, TopologyError> {
    let text = fs::read_to_string(path)
        .map_err(|e| TopologyError::Json(format!("failed to read {}: {e}", path.display())))?;
    serde_json::from_str(&text).map_err(|e| TopologyError::Json(e.to_string()))
}

/// Resolve a decoded spec into a validated [`TopologySpec`], following
/// `extends` exactly one level.
#[expect(
    clippy::similar_names,
    reason = "allowed_h_milli and allowed_j_milli mirror the spec field names; \
              renaming them here would obscure which JSON key each one carries"
)]
fn resolve_spec(raw: TopologySpecJson) -> Result<TopologySpec, TopologyError> {
    let base = match &raw.extends {
        None => None,
        Some(name) => {
            if raw.nodes.is_some() || raw.edges.is_some() {
                return Err(TopologyError::ExtendsWithOwnGraph);
            }
            let base_path = fixture_path(name)?;
            let base_raw = read_spec(&base_path)
                .map_err(|e| TopologyError::ExtendsUnloadable(name.clone(), Box::new(e)))?;
            // One level only. A chain would work, but it would also make the
            // graph a spec resolves to depend on a file two hops away.
            if base_raw.extends.is_some() {
                return Err(TopologyError::ExtendsNotABase(name.clone()));
            }
            let base = resolve_spec(base_raw)
                .map_err(|e| TopologyError::ExtendsUnloadable(name.clone(), Box::new(e)))?;
            Some(base)
        }
    };

    let (nodes, edges) = match base.as_ref() {
        Some(b) => (b.nodes.clone(), b.edges.clone()),
        None => (raw.nodes.unwrap_or_default(), raw.edges.unwrap_or_default()),
    };
    let allowed_h_milli = raw
        .allowed_h_milli
        .or_else(|| base.as_ref().map(|b| b.allowed_h_milli.clone()))
        .unwrap_or_default();
    let allowed_j_milli = raw
        .allowed_j_milli
        .or_else(|| base.as_ref().map(|b| b.allowed_j_milli.clone()))
        .unwrap_or_default();
    let allowed_spin_milli = raw
        .allowed_spin_milli
        .filter(|v| !v.is_empty())
        .or_else(|| base.as_ref().map(|b| b.allowed_spin_milli.clone()))
        .unwrap_or_else(|| DEFAULT_ALLOWED_SPIN_MILLI.to_vec());

    if nodes.is_empty() {
        return Err(TopologyError::EmptyNodes);
    }
    if allowed_h_milli.is_empty() {
        return Err(TopologyError::EmptyAllowedH);
    }
    if allowed_j_milli.is_empty() {
        return Err(TopologyError::EmptyAllowedJ);
    }
    validate_edges(&nodes, &edges)?;

    Ok(TopologySpec {
        nodes,
        edges,
        allowed_h_milli,
        allowed_j_milli,
        allowed_spin_milli,
    })
}

/// Load and validate a bundled fixture by name, resolving to
/// `fixtures/<name>.spec.json` relative to this crate's manifest directory.
///
/// `name` must match `[A-Za-z0-9-]` so it cannot escape the fixtures
/// directory (rejects path separators and `..`).
///
/// # Errors
///
/// Returns [`TopologyError::Json`] when `name` contains characters outside
/// `[A-Za-z0-9-]`, and otherwise the same errors as [`load_topology`].
pub fn load_fixture(name: &str) -> Result<TopologySpec, TopologyError> {
    load_topology(&fixture_path(name)?)
}

/// Resolve a bundled fixture name to its path.
///
/// `name` must match `[A-Za-z0-9-]` so it cannot escape the fixtures
/// directory (rejects path separators and `..`). This guard also covers
/// `extends`, which resolves through here.
fn fixture_path(name: &str) -> Result<PathBuf, TopologyError> {
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(TopologyError::Json(format!(
            "fixture name {name:?} must match [A-Za-z0-9-]"
        )));
    }
    Ok([env!("CARGO_MANIFEST_DIR"), "fixtures"]
        .iter()
        .collect::<PathBuf>()
        .join(format!("{name}.spec.json")))
}

impl TopologySpec {
    /// Chain-canonical topology hash.
    ///
    /// Mirrors `quip-coordinator`'s `topology_hash_sets`
    /// (`crates/quip-coordinator/src/topology.rs` at the upstream rev this
    /// crate pins), which in turn matches
    /// `pallet_quantum_pow::topology::hash_topology`:
    /// `blake2_256(SCALE.encode((sorted_nodes, canonical_sorted_edges,
    /// allowed_h.canonical_bytes(), allowed_j.canonical_bytes(),
    /// allowed_spin.canonical_bytes())))`, where `canonical_bytes()` for a
    /// `Set` spec (the only variant this crate produces) is a `0x00`
    /// discriminant byte followed by the values sorted ascending, each
    /// encoded big-endian.
    ///
    /// `topology_hash_sets` is only reachable through `quip-coordinator`,
    /// which this crate does not depend on (it drags in the `QuipNetwork`
    /// Substrate fork via `sp-core`; see the design doc). `blake2_256` is
    /// plain `BLAKE2b` with a 32-byte digest and no key, so this calls the
    /// standalone `blake2` crate directly instead of `sp-core`, and
    /// `parity-scale-codec` (upstream's own SCALE implementation, with no
    /// Substrate dependency chain) performs the encoding. Pinned against
    /// upstream's `golden_topology_hash_known_fixture` test vector by
    /// `topology_hash_matches_upstream_golden_vector` in `tests/topology.rs`.
    #[must_use]
    pub fn topology_hash(&self) -> Vec<u8> {
        let mut nodes = self.nodes.clone();
        nodes.sort_unstable();

        let mut edges: Vec<(u32, u32)> = self
            .edges
            .iter()
            .map(|&(u, v)| if u <= v { (u, v) } else { (v, u) })
            .collect();
        edges.sort_unstable();

        let encoded = (
            nodes,
            edges,
            canonical_set_bytes(&self.allowed_h_milli),
            canonical_set_bytes(&self.allowed_j_milli),
            canonical_set_bytes(&self.allowed_spin_milli),
        )
            .encode();

        let mut hasher = Blake2b256::new();
        hasher.update(&encoded);
        hasher.finalize().to_vec()
    }
}

/// `AllowedValueSpec::Set(values).canonical_bytes()` for the `Set` variant:
/// a `0x00` discriminant byte followed by the values sorted ascending, each
/// big-endian `i32`.
fn canonical_set_bytes(values: &[i32]) -> Vec<u8> {
    let mut out = vec![0u8];
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    for v in sorted {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out
}
