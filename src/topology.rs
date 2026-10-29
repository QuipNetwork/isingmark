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

/// Allowed coefficient sets for a generated milli-valued problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Couplings {
    /// Unit couplings and ternary fields.
    Binary,
    /// Multiples of 16 milli, with 125 field levels.
    Int8,
    /// Every milli value from -1000 through 1000.
    Int32,
}

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
    /// The generated topology description or size is invalid.
    Generate(String),
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
}

impl std::fmt::Display for TopologyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Generate(e) => write!(f, "invalid generated topology: {e}"),
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
        }
    }
}

impl std::error::Error for TopologyError {}

/// Build `complete:N` or `circulant:N:D` without a topology file.
///
/// Circulant graphs connect each node to its `D/2` nearest neighbors in
/// each direction. Node ids are contiguous from zero, and edges have
/// canonical endpoints with no duplicates or self-loops.
///
/// # Errors
///
/// Returns [`TopologyError::Generate`] for malformed descriptions, zero
/// nodes, an odd degree, a degree at least `N`, or an unallocatable edge list.
pub fn generate_topology(
    description: &str,
    couplings: Couplings,
) -> Result<TopologySpec, TopologyError> {
    let invalid = || {
        TopologyError::Generate(
            "expected complete:N or circulant:N:D with N > 0 and even 0 <= D < N".to_string(),
        )
    };
    let parts: Vec<&str> = description.split(':').collect();
    let (n, degree) = match parts.as_slice() {
        ["complete", n] => (n.parse::<u32>().map_err(|_| invalid())?, None),
        ["circulant", n, d] => (
            n.parse::<u32>().map_err(|_| invalid())?,
            Some(d.parse::<u32>().map_err(|_| invalid())?),
        ),
        _ => return Err(invalid()),
    };
    if n == 0 || degree.is_some_and(|d| d >= n || d % 2 != 0) {
        return Err(invalid());
    }
    let edge_count = u64::from(n) * u64::from(degree.unwrap_or(n - 1)) / 2;
    let edge_count = usize::try_from(edge_count).map_err(|_| invalid())?;
    let mut edges = Vec::new();
    edges
        .try_reserve_exact(edge_count)
        .map_err(|e| TopologyError::Generate(format!("cannot allocate {edge_count} edges: {e}")))?;
    match degree {
        None => {
            for u in 0..n {
                for v in u + 1..n {
                    edges.push((u, v));
                }
            }
        }
        Some(d) => {
            for u in 0..n {
                for offset in 1..=d / 2 {
                    let v = if u < n - offset {
                        u + offset
                    } else {
                        u - (n - offset)
                    };
                    edges.push((u.min(v), u.max(v)));
                }
            }
        }
    }
    let fields: Vec<i32> = match couplings {
        Couplings::Binary => vec![-1000, 0, 1000],
        Couplings::Int8 => (-992..=992).step_by(16).collect(),
        Couplings::Int32 => (-1000..=1000).collect(),
    };
    let interactions = fields.iter().copied().filter(|&v| v != 0).collect();
    Ok(TopologySpec {
        nodes: (0..n).collect(),
        edges,
        allowed_h_milli: fields,
        allowed_j_milli: interactions,
        allowed_spin_milli: DEFAULT_ALLOWED_SPIN_MILLI.to_vec(),
    })
}

#[derive(Debug, Deserialize)]
struct TopologySpecJson {
    nodes: Vec<u32>,
    #[serde(default)]
    edges: Vec<(u32, u32)>,
    allowed_h_milli: Vec<i32>,
    allowed_j_milli: Vec<i32>,
    #[serde(default)]
    allowed_spin_milli: Vec<i32>,
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
/// # Errors
///
/// Returns [`TopologyError`] when the file cannot be read, its contents are
/// not valid topology-spec JSON, required fields are empty, or an edge
/// references an unknown node id.
pub fn load_topology(path: &Path) -> Result<TopologySpec, TopologyError> {
    let text = fs::read_to_string(path)
        .map_err(|e| TopologyError::Json(format!("failed to read {}: {e}", path.display())))?;
    let raw: TopologySpecJson =
        serde_json::from_str(&text).map_err(|e| TopologyError::Json(e.to_string()))?;

    if raw.nodes.is_empty() {
        return Err(TopologyError::EmptyNodes);
    }
    if raw.allowed_h_milli.is_empty() {
        return Err(TopologyError::EmptyAllowedH);
    }
    if raw.allowed_j_milli.is_empty() {
        return Err(TopologyError::EmptyAllowedJ);
    }
    validate_edges(&raw.nodes, &raw.edges)?;

    let allowed_spin_milli = if raw.allowed_spin_milli.is_empty() {
        DEFAULT_ALLOWED_SPIN_MILLI.to_vec()
    } else {
        raw.allowed_spin_milli
    };

    Ok(TopologySpec {
        nodes: raw.nodes,
        edges: raw.edges,
        allowed_h_milli: raw.allowed_h_milli,
        allowed_j_milli: raw.allowed_j_milli,
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
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(TopologyError::Json(format!(
            "fixture name {name:?} must match [A-Za-z0-9-]"
        )));
    }
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "fixtures"]
        .iter()
        .collect::<PathBuf>()
        .join(format!("{name}.spec.json"));
    load_topology(&path)
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
