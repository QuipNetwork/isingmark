// SPDX-License-Identifier: AGPL-3.0-or-later
//! Replay of explicit Ising instances from a JSONL file.
//!
//! [`corpus::CorpusSource`](crate::corpus::CorpusSource) replays *nonces*: the
//! problem is re-drawn through the chain's `draw_ising_milli`. A structured
//! family (corner-sharing tetrahedra, twisted gauges, and the rest) is a
//! function of a nonce too, but through its own generator, so it cannot be
//! re-drawn from a topology preset. This source instead reads the coefficients
//! themselves and binds them to the topology the run reports.
//!
//! File format. Line 1 is a header, every later line one instance:
//!
//! ```json
//! {"kind":"header","format":1,"topology_hash":"<64 hex>","nodes":8,"edges":8,"family":"..."}
//! {"nonce":"<64 hex>","h_milli":[...],"j_milli":[...],"reference":{...}}
//! ```
//!
//! `j_milli` follows the topology's edge order and has exactly one entry per
//! edge. `h_milli` has one entry per node, or is absent, which means all zero.
//! Every value lies in `[-1000, 1000]`. Header fields beyond the four checked
//! here, and the per-instance `reference` object, are informational and
//! ignored.
//!
//! The header's `topology_hash` must equal the hash of the topology the run
//! supplies. [`InstanceSource::load`] refuses a mismatch: the coefficients are
//! indexed by that topology's edge order, so any other graph would make them
//! meaningless.

use std::fs;
use std::path::Path;

use quip_proto::v1::{ising_problem, CoefficientEncoding, IsingProblem, Job, JobKind, Provenance};
use quip_protocol::wire::encode_i32_le;
use serde::Deserialize;

use crate::corpus::decode_nonce;
use crate::modes::hex_encode;
use crate::problem::MILLI_SCALE;
use crate::topology::TopologySpec;

/// The only file format revision this reader understands.
const FORMAT: u64 = 1;

/// Largest coefficient magnitude, in milli-units.
const MAX_MILLI: i64 = 1000;

/// Errors loading or validating an instance file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceError {
    /// The file could not be read. The message is the underlying I/O error.
    Io(String),
    /// A line is not valid JSON of the expected shape. `line` is 1-based.
    Parse {
        /// 1-based line number in the JSONL file.
        line: usize,
        /// Decode failure detail.
        reason: String,
    },
    /// The first non-blank line is not a `{"kind":"header",...}` object, or
    /// the file is empty.
    MissingHeader,
    /// The header names a format revision this reader does not understand.
    UnsupportedFormat {
        /// The `format` value the header carried.
        found: u64,
    },
    /// The file was built for a different topology than the run supplies.
    TopologyMismatch {
        /// Hex `topology_hash` the header declares.
        expected: String,
        /// Hex `topology_hash` of the topology actually supplied.
        actual: String,
    },
    /// The header's node or edge count disagrees with the supplied topology.
    ShapeMismatch {
        /// `"nodes"` or `"edges"`.
        what: &'static str,
        /// Count the header declares.
        header: usize,
        /// Count the supplied topology has.
        topology: usize,
    },
    /// A nonce is not 32 bytes of hex. `line` is 1-based.
    BadNonce {
        /// 1-based line number in the JSONL file.
        line: usize,
        /// What was wrong with the nonce.
        reason: String,
    },
    /// A coefficient array has the wrong number of entries. `line` is 1-based.
    BadLength {
        /// 1-based line number in the JSONL file.
        line: usize,
        /// `"h_milli"` or `"j_milli"`.
        field: &'static str,
        /// Entry count the topology requires.
        expected: usize,
        /// Entry count the line carries.
        actual: usize,
    },
    /// A coefficient lies outside `[-1000, 1000]`. `line` is 1-based.
    OutOfRange {
        /// 1-based line number in the JSONL file.
        line: usize,
        /// `"h_milli"` or `"j_milli"`.
        field: &'static str,
        /// 0-based position of the offending entry.
        index: usize,
        /// The offending value.
        value: i64,
    },
    /// The file held a header and no instances.
    Empty,
}

impl std::fmt::Display for InstanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot read instance file: {e}"),
            Self::Parse { line, reason } => write!(f, "instance file line {line}: {reason}"),
            Self::MissingHeader => write!(
                f,
                "instance file has no header: line 1 must be \
                 {{\"kind\":\"header\",\"format\":1,...}}"
            ),
            Self::UnsupportedFormat { found } => write!(
                f,
                "instance file format {found} is not supported; this reader handles format {FORMAT}"
            ),
            Self::TopologyMismatch { expected, actual } => write!(
                f,
                "instance file was built for topology {expected}, but the run supplies \
                 {actual}. Pair it with the matching --topology-preset, or its \
                 coefficients do not line up with the graph's edges"
            ),
            Self::ShapeMismatch {
                what,
                header,
                topology,
            } => write!(
                f,
                "instance file header declares {header} {what}, but the topology has {topology}"
            ),
            Self::BadNonce { line, reason } => {
                write!(f, "instance file line {line}: bad nonce: {reason}")
            }
            Self::BadLength {
                line,
                field,
                expected,
                actual,
            } => write!(
                f,
                "instance file line {line}: {field} has {actual} entries, expected {expected}"
            ),
            Self::OutOfRange {
                line,
                field,
                index,
                value,
            } => write!(
                f,
                "instance file line {line}: {field}[{index}] = {value} is outside \
                 [-{MAX_MILLI}, {MAX_MILLI}]"
            ),
            Self::Empty => write!(f, "instance file holds no instances"),
        }
    }
}

impl std::error::Error for InstanceError {}

/// The header fields this reader checks. Unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct HeaderJson {
    format: u64,
    topology_hash: String,
    nodes: usize,
    edges: usize,
}

/// One instance line. `reference` and any other extra field are ignored.
#[derive(Debug, Deserialize)]
struct InstanceJson {
    nonce: String,
    #[serde(default)]
    h_milli: Option<Vec<i64>>,
    j_milli: Vec<i64>,
}

/// One validated instance, coefficients ready for the wire.
#[derive(Debug)]
struct Instance {
    nonce: [u8; 32],
    h: Vec<i32>,
    j: Vec<i32>,
}

/// A finite sequence of explicit instances, replayed in file order.
///
/// Like [`CorpusSource`](crate::corpus::CorpusSource), it yields each instance
/// once and then returns `None`, so a mode that runs to a wall-clock target
/// stops when the file runs out. Unlike it, every instance is parsed at load
/// time and held in memory: an instance is one `i32` per node and per edge, so
/// 24 instances over the 4577-node chip are about 4 MB. A file with thousands
/// of instances over that chip would hold hundreds of megabytes.
#[derive(Debug)]
pub struct InstanceSource {
    label: String,
    topology_hash: Vec<u8>,
    instances: std::vec::IntoIter<Instance>,
    total: usize,
    next_index: usize,
}

impl InstanceSource {
    /// Load the JSONL file at `path` and validate every instance against
    /// `spec`.
    ///
    /// # Errors
    ///
    /// Returns [`InstanceError`] when the file cannot be read, the header is
    /// missing, malformed, of another format, or built for another topology,
    /// any instance line fails validation, or the file holds no instances.
    pub fn load(path: &Path, spec: &TopologySpec) -> Result<Self, InstanceError> {
        let text = fs::read_to_string(path)
            .map_err(|e| InstanceError::Io(format!("{}: {e}", path.display())))?;
        Self::parse(&path.display().to_string(), &text, spec)
    }

    /// Parse already-loaded JSONL `text` against `spec`. `label` names the
    /// source in run summaries. Used directly by tests and by
    /// [`load`](Self::load).
    ///
    /// # Errors
    ///
    /// Same as [`load`](Self::load), except the I/O error.
    pub fn parse(label: &str, text: &str, spec: &TopologySpec) -> Result<Self, InstanceError> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(i, line)| (i + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty());

        let (header_no, header_line) = lines.next().ok_or(InstanceError::MissingHeader)?;
        check_header(header_no, header_line, spec)?;

        let mut instances = Vec::new();
        for (line_no, line) in lines {
            instances.push(parse_instance(line_no, line, spec)?);
        }
        if instances.is_empty() {
            return Err(InstanceError::Empty);
        }
        let total = instances.len();
        Ok(Self {
            label: label.to_string(),
            topology_hash: spec.topology_hash(),
            instances: instances.into_iter(),
            total,
            next_index: 0,
        })
    }

    /// The path this source was loaded from.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// How many instances the file holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.total
    }

    /// Whether the file holds no instances. Always false:
    /// [`parse`](Self::parse) rejects an empty file.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// Build the next instance's job, or `None` once the file is exhausted.
    ///
    /// Same job shape as the corpus replay: the job id is the nonce, the graph
    /// travels as a topology hash, and `generation` is the instance's 1-based
    /// position in the file. The coefficients are the file's, not drawn.
    #[must_use]
    pub fn next_job(&mut self) -> Option<Job> {
        let instance = self.instances.next()?;
        self.next_index += 1;
        Some(Job {
            job_id: instance.nonce.to_vec(),
            kind: JobKind::IsingSample as i32,
            generation: self.next_index as u64,
            deadline_ms: 0,
            ising: Some(IsingProblem {
                graph: Some(ising_problem::Graph::TopologyHash(
                    self.topology_hash.clone(),
                )),
                encoding: CoefficientEncoding::I32 as i32,
                scale: MILLI_SCALE,
                h: encode_i32_le(&instance.h),
                j: encode_i32_le(&instance.j),
                ..IsingProblem::default()
            }),
            provenance: Some(Provenance {
                is_pow: true,
                order_id: vec![],
            }),
            generator: None,
        })
    }
}

/// Validate the header line against `spec`.
fn check_header(line_no: usize, line: &str, spec: &TopologySpec) -> Result<(), InstanceError> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|e| InstanceError::Parse {
            line: line_no,
            reason: e.to_string(),
        })?;
    if value.get("kind").and_then(serde_json::Value::as_str) != Some("header") {
        return Err(InstanceError::MissingHeader);
    }
    let header: HeaderJson = serde_json::from_value(value).map_err(|e| InstanceError::Parse {
        line: line_no,
        reason: e.to_string(),
    })?;
    if header.format != FORMAT {
        return Err(InstanceError::UnsupportedFormat {
            found: header.format,
        });
    }
    let actual = hex_encode(&spec.topology_hash());
    let expected = header.topology_hash.trim().to_ascii_lowercase();
    if expected != actual {
        return Err(InstanceError::TopologyMismatch { expected, actual });
    }
    for (what, header_count, topology) in [
        ("nodes", header.nodes, spec.nodes.len()),
        ("edges", header.edges, spec.edges.len()),
    ] {
        if header_count != topology {
            return Err(InstanceError::ShapeMismatch {
                what,
                header: header_count,
                topology,
            });
        }
    }
    Ok(())
}

/// Parse and validate one instance line.
fn parse_instance(
    line_no: usize,
    line: &str,
    spec: &TopologySpec,
) -> Result<Instance, InstanceError> {
    let entry: InstanceJson = serde_json::from_str(line).map_err(|e| InstanceError::Parse {
        line: line_no,
        reason: e.to_string(),
    })?;
    let nonce = decode_nonce(&entry.nonce).map_err(|reason| InstanceError::BadNonce {
        line: line_no,
        reason,
    })?;
    let h = match entry.h_milli {
        Some(values) => checked_coefficients(line_no, "h_milli", &values, spec.nodes.len())?,
        None => vec![0; spec.nodes.len()],
    };
    let j = checked_coefficients(line_no, "j_milli", &entry.j_milli, spec.edges.len())?;
    Ok(Instance { nonce, h, j })
}

/// Check that `values` has `expected` entries, all within `[-1000, 1000]`.
fn checked_coefficients(
    line: usize,
    field: &'static str,
    values: &[i64],
    expected: usize,
) -> Result<Vec<i32>, InstanceError> {
    if values.len() != expected {
        return Err(InstanceError::BadLength {
            line,
            field,
            expected,
            actual: values.len(),
        });
    }
    values
        .iter()
        .enumerate()
        .map(|(index, &value)| {
            if value.abs() > MAX_MILLI {
                return Err(InstanceError::OutOfRange {
                    line,
                    field,
                    index,
                    value,
                });
            }
            // The range check above bounds `value` well inside `i32`.
            i32::try_from(value).map_err(|_| InstanceError::OutOfRange {
                line,
                field,
                index,
                value,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::load_fixture;
    use quip_protocol::wire::decode_i32_le;

    const NONCE_A: &str = "0cf67b0f85e37ddd3dda2ac11685c7ad857d069eda939ae3f2d6d93e3d5b8864";
    const NONCE_B: &str = "22681ca9153f4f30a0d5af9da319e2b93848c182a7584a541d52db2214a08d54";

    fn smoke() -> TopologySpec {
        load_fixture("smoke").expect("smoke fixture parses")
    }

    fn header_line(spec: &TopologySpec) -> String {
        format!(
            r#"{{"kind":"header","format":1,"family":"unit","topology_hash":"{}","nodes":{},"edges":{},"extra":{{"a":1}}}}"#,
            hex_encode(&spec.topology_hash()),
            spec.nodes.len(),
            spec.edges.len(),
        )
    }

    /// A `j_milli` JSON array with one entry per smoke edge.
    fn j_json(spec: &TopologySpec, value: i64) -> String {
        format!("[{}]", vec![value.to_string(); spec.edges.len()].join(","))
    }

    #[test]
    fn round_trip_through_next_job_preserves_the_coefficients() {
        let spec = smoke();
        let h: Vec<i32> = (0..8).map(|i| i * 125 - 500).collect();
        let j: Vec<i32> = (0..8).map(|i| 1000 - i * 250).collect();
        assert_eq!((h.len(), j.len()), (spec.nodes.len(), spec.edges.len()));
        let text = format!(
            "{}\n{{\"nonce\":\"{NONCE_A}\",\"h_milli\":{h:?},\"j_milli\":{j:?},\"reference\":{{\"x\":1}}}}\n{{\"nonce\":\"{NONCE_B}\",\"j_milli\":{}}}\n",
            header_line(&spec),
            j_json(&spec, -125),
        );
        let mut src = InstanceSource::parse("inline", &text, &spec).expect("parses");
        assert_eq!(src.len(), 2);

        let first = src.next_job().expect("first job");
        assert_eq!(hex_encode(&first.job_id), NONCE_A);
        assert_eq!(first.generation, 1);
        assert!(first.provenance.expect("provenance").is_pow);
        let ising = first.ising.expect("ising");
        assert_eq!(
            ising.graph,
            Some(ising_problem::Graph::TopologyHash(spec.topology_hash()))
        );
        assert_eq!(ising.scale, MILLI_SCALE);
        assert_eq!(decode_i32_le(&ising.h).expect("h decodes"), h);
        assert_eq!(decode_i32_le(&ising.j).expect("j decodes"), j);

        // An absent `h_milli` means all zero.
        let second = src.next_job().expect("second job");
        assert_eq!(second.generation, 2);
        let ising = second.ising.expect("ising");
        assert_eq!(
            decode_i32_le(&ising.h).expect("h decodes"),
            vec![0; spec.nodes.len()]
        );
        assert_eq!(
            decode_i32_le(&ising.j).expect("j decodes"),
            vec![-125; spec.edges.len()]
        );
        assert!(src.next_job().is_none());
    }

    #[test]
    fn a_header_for_another_topology_is_rejected() {
        let spec = smoke();
        let other = load_fixture("chain-ternary").expect("fixture parses");
        let text = format!(
            "{}\n{{\"nonce\":\"{NONCE_A}\",\"j_milli\":{}}}\n",
            header_line(&other),
            j_json(&spec, 0),
        );
        let err = InstanceSource::parse("inline", &text, &spec).expect_err("must reject");
        assert!(
            matches!(err, InstanceError::TopologyMismatch { .. }),
            "{err}"
        );
        assert!(err.to_string().contains("--topology-preset"));
    }

    #[test]
    fn a_header_node_or_edge_count_that_disagrees_is_rejected() {
        let spec = smoke();
        let hash = hex_encode(&spec.topology_hash());
        let text = format!(
            "{{\"kind\":\"header\",\"format\":1,\"topology_hash\":\"{hash}\",\"nodes\":{},\"edges\":{}}}\n{{\"nonce\":\"{NONCE_A}\",\"j_milli\":{}}}\n",
            spec.nodes.len(),
            spec.edges.len() + 1,
            j_json(&spec, 0),
        );
        let err = InstanceSource::parse("inline", &text, &spec).expect_err("must reject");
        assert!(
            matches!(err, InstanceError::ShapeMismatch { what: "edges", .. }),
            "{err}"
        );
    }

    #[test]
    fn an_unsupported_format_is_rejected() {
        let spec = smoke();
        let text = header_line(&spec).replace("\"format\":1", "\"format\":2");
        assert_eq!(
            InstanceSource::parse("inline", &text, &spec).expect_err("must reject"),
            InstanceError::UnsupportedFormat { found: 2 }
        );
    }

    #[test]
    fn a_file_without_a_header_is_rejected() {
        let spec = smoke();
        let text = format!(
            "{{\"nonce\":\"{NONCE_A}\",\"j_milli\":{}}}\n",
            j_json(&spec, 0)
        );
        assert_eq!(
            InstanceSource::parse("inline", &text, &spec).expect_err("must reject"),
            InstanceError::MissingHeader
        );
        assert_eq!(
            InstanceSource::parse("inline", "\n\n", &spec).expect_err("must reject"),
            InstanceError::MissingHeader
        );
    }

    #[test]
    fn a_header_with_no_instances_is_rejected() {
        let spec = smoke();
        assert_eq!(
            InstanceSource::parse("inline", &header_line(&spec), &spec).expect_err("must reject"),
            InstanceError::Empty
        );
    }

    #[test]
    fn a_wrong_j_milli_length_names_the_line() {
        let spec = smoke();
        let text = format!(
            "{}\n{{\"nonce\":\"{NONCE_A}\",\"j_milli\":{}}}\n{{\"nonce\":\"{NONCE_B}\",\"j_milli\":[1,2,3]}}\n",
            header_line(&spec),
            j_json(&spec, 0),
        );
        let err = InstanceSource::parse("inline", &text, &spec).expect_err("must reject");
        assert_eq!(
            err,
            InstanceError::BadLength {
                line: 3,
                field: "j_milli",
                expected: spec.edges.len(),
                actual: 3,
            }
        );
    }

    #[test]
    fn a_wrong_h_milli_length_names_the_line() {
        let spec = smoke();
        let text = format!(
            "{}\n{{\"nonce\":\"{NONCE_A}\",\"h_milli\":[0],\"j_milli\":{}}}\n",
            header_line(&spec),
            j_json(&spec, 0),
        );
        assert!(matches!(
            InstanceSource::parse("inline", &text, &spec).expect_err("must reject"),
            InstanceError::BadLength {
                line: 2,
                field: "h_milli",
                ..
            }
        ));
    }

    #[test]
    fn an_out_of_range_value_names_the_line_and_index() {
        let spec = smoke();
        let mut j = vec![0_i64; spec.edges.len()];
        j[2] = 1001;
        let text = format!(
            "{}\n{{\"nonce\":\"{NONCE_A}\",\"j_milli\":{j:?}}}\n",
            header_line(&spec),
        );
        assert_eq!(
            InstanceSource::parse("inline", &text, &spec).expect_err("must reject"),
            InstanceError::OutOfRange {
                line: 2,
                field: "j_milli",
                index: 2,
                value: 1001,
            }
        );
    }

    #[test]
    fn a_value_beyond_i32_is_out_of_range_not_a_parse_error() {
        let spec = smoke();
        let mut j = vec![0_i64; spec.edges.len()];
        j[0] = -5_000_000_000;
        let text = format!(
            "{}\n{{\"nonce\":\"{NONCE_A}\",\"j_milli\":{j:?}}}\n",
            header_line(&spec),
        );
        assert!(matches!(
            InstanceSource::parse("inline", &text, &spec).expect_err("must reject"),
            InstanceError::OutOfRange { index: 0, .. }
        ));
    }

    #[test]
    fn a_bad_nonce_names_the_line() {
        let spec = smoke();
        let text = format!(
            "{}\n{{\"nonce\":\"abcd\",\"j_milli\":{}}}\n",
            header_line(&spec),
            j_json(&spec, 0),
        );
        assert!(matches!(
            InstanceSource::parse("inline", &text, &spec).expect_err("must reject"),
            InstanceError::BadNonce { line: 2, .. }
        ));
    }

    #[test]
    fn malformed_json_names_the_line() {
        let spec = smoke();
        let text = format!("{}\nnot json\n", header_line(&spec));
        assert!(matches!(
            InstanceSource::parse("inline", &text, &spec).expect_err("must reject"),
            InstanceError::Parse { line: 2, .. }
        ));
    }

    #[test]
    fn the_smoke_file_loads_against_the_smoke_fixture() {
        let spec = smoke();
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("corpora")
            .join("structured")
            .join("smoke-ice-glass.jsonl");
        let mut src = InstanceSource::load(&path, &spec).expect("smoke file loads");
        assert_eq!(src.len(), 2);
        assert_eq!(src.label(), path.display().to_string());
        let first = src.next_job().expect("first job").ising.expect("ising");
        assert_eq!(
            decode_i32_le(&first.j).expect("j decodes"),
            vec![125, -125, 1000, -1000, 0, 125, -125, 0]
        );
        assert_eq!(
            decode_i32_le(&first.h).expect("h decodes"),
            vec![0; spec.nodes.len()]
        );
        let second = src.next_job().expect("second job").ising.expect("ising");
        assert_eq!(
            decode_i32_le(&second.h).expect("h decodes"),
            vec![250, 0, -250, 0, 0, 0, 0, 1000]
        );
        assert!(src.next_job().is_none());
    }

    #[test]
    fn the_smoke_file_is_rejected_against_another_topology() {
        let other = load_fixture("chain-ternary").expect("fixture parses");
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("corpora")
            .join("structured")
            .join("smoke-ice-glass.jsonl");
        assert!(matches!(
            InstanceSource::load(&path, &other).expect_err("must reject"),
            InstanceError::TopologyMismatch { .. }
        ));
    }

    #[test]
    fn a_missing_file_is_an_io_error() {
        let err = InstanceSource::load(Path::new("/no/such/instances.jsonl"), &smoke())
            .expect_err("must reject");
        assert!(matches!(err, InstanceError::Io(_)), "{err}");
    }
}
