// SPDX-License-Identifier: AGPL-3.0-or-later
//! Replay of a fixed corpus of chain-harvested problem instances.
//!
//! [`problem::ProblemSource`](crate::problem::ProblemSource) *derives* its
//! problems from a seed. A corpus instead *replays* named ones: each line
//! carries a nonce that some miner already solved on chain, and the problem
//! is re-drawn from that nonce against a topology spec. Same nonce plus same
//! topology gives back the same `(h, J)` the miner saw.
//!
//! Mirrors the nonce-ref half of `quip-coordinator`'s drive-mode list source
//! (`crates/quip-coordinator/src/drive/list_source.rs`, reached as
//! `quip-coordinator drive --source list`), which is what produced the
//! reference results these corpora are paired with. The explicit-problem half
//! of that format (`h_milli`/`j_milli`/`edges` inline) has no user here and is
//! not implemented: an explicit entry carries its own graph, which would
//! silently bypass the topology the run reports.
//!
//! The corpus does not carry the topology. A corpus and a topology are paired
//! by `topology_hash`, and [`CorpusSource::load`] refuses a pair whose hashes
//! disagree — replaying against the wrong graph would draw valid-looking
//! problems that are not the ones the chain recorded.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use quip_proto::v1::{ising_problem, CoefficientEncoding, IsingProblem, Job, JobKind, Provenance};
use quip_protocol::chacha8::draw_ising_milli;
use quip_protocol::wire::encode_i32_le;
use serde::Deserialize;

use crate::modes::hex_encode;
use crate::problem::MILLI_SCALE;
use crate::topology::TopologySpec;

/// Bundled corpora, each paired with the fixture whose `topology_hash` its
/// instances were drawn against. Both are outputs of
/// `quip-coordinator download`, bucketed by topology.
const BUNDLED: [(&str, &str); 2] = [("chain-h0", "chain-h0"), ("chain-ternary", "chain-ternary")];

/// One corpus line: a chain-harvested instance.
#[derive(Debug, Clone, Deserialize)]
struct CorpusEntryJson {
    /// Hex problem nonce. The sole entropy for the drawn `(h, J)`.
    nonce: String,
    /// Best energy any miner achieved on chain for this instance, in
    /// milli-units. The bar a backend is measured against.
    #[serde(default)]
    energy_milli: Option<i64>,
    /// The chain's difficulty gate for this instance, in milli-units. A
    /// solution at or below this value would have been accepted.
    #[serde(default)]
    max_energy_milli: Option<i64>,
    /// Block the winning solution was submitted in. Provenance only.
    #[serde(default)]
    qblock_id: Option<u64>,
}

/// What the chain recorded for one replayed instance, keyed by nonce so a
/// caller can score a run against the bar the chain already set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainResult {
    /// Best energy any miner achieved on chain, in milli-units.
    pub energy_milli: Option<i64>,
    /// The difficulty gate that applied, in milli-units.
    pub max_energy_milli: Option<i64>,
    /// Block the winning solution was submitted in.
    pub qblock_id: Option<u64>,
}

/// Errors loading or validating a corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusError {
    /// The file could not be read. The message is the underlying I/O error.
    Io(String),
    /// A line is not valid corpus JSON. `line` is 1-based.
    Parse {
        /// 1-based line number in the JSONL file.
        line: usize,
        /// Decode failure detail.
        reason: String,
    },
    /// A nonce is not 32 bytes of hex. `line` is 1-based.
    BadNonce {
        /// 1-based line number in the JSONL file.
        line: usize,
        /// What was wrong with the nonce.
        reason: String,
    },
    /// The file held no instances.
    Empty,
    /// `name` is not a bundled corpus, and was not a readable path either.
    UnknownName {
        /// The name the caller asked for.
        name: String,
        /// The bundled names that do exist.
        known: Vec<String>,
    },
    /// The corpus was paired with a topology it was not drawn against.
    /// Replaying would draw the wrong problems.
    TopologyMismatch {
        /// The corpus name, for the message.
        corpus: String,
        /// Hex `topology_hash` the corpus expects.
        expected: String,
        /// Hex `topology_hash` of the topology actually supplied.
        actual: String,
    },
}

impl std::fmt::Display for CorpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot read corpus: {e}"),
            Self::Parse { line, reason } => write!(f, "corpus line {line}: {reason}"),
            Self::BadNonce { line, reason } => {
                write!(f, "corpus line {line}: bad nonce: {reason}")
            }
            Self::Empty => write!(f, "corpus holds no instances"),
            Self::UnknownName { name, known } => write!(
                f,
                "unknown corpus {name:?}; bundled corpora are {}, \
                 or pass a path to a JSONL file",
                known.join(", ")
            ),
            Self::TopologyMismatch {
                corpus,
                expected,
                actual,
            } => write!(
                f,
                "corpus {corpus:?} was drawn against topology {expected}, \
                 but the run supplies {actual}. Pair it with the matching \
                 --topology-preset, or the replayed problems are not the \
                 ones the chain recorded"
            ),
        }
    }
}

impl std::error::Error for CorpusError {}

/// Names of the corpora bundled with this crate.
#[must_use]
pub fn bundled_names() -> Vec<String> {
    BUNDLED.iter().map(|(n, _)| (*n).to_string()).collect()
}

/// The fixture name a bundled corpus must be paired with, if `name` names a
/// bundled corpus.
#[must_use]
pub fn paired_fixture(name: &str) -> Option<&'static str> {
    BUNDLED
        .iter()
        .find(|(corpus, _)| *corpus == name)
        .map(|(_, fixture)| *fixture)
}

/// Resolve a bundled corpus name to its path, rejecting any name that could
/// escape the `corpora/` directory.
fn bundled_path(name: &str) -> Option<PathBuf> {
    if !BUNDLED.iter().any(|(corpus, _)| *corpus == name) {
        return None;
    }
    Some(
        [env!("CARGO_MANIFEST_DIR"), "corpora"]
            .iter()
            .collect::<PathBuf>()
            .join(format!("{name}.jsonl")),
    )
}

fn decode_nonce(hex: &str) -> Result<[u8; 32], String> {
    let hex = hex.trim();
    if hex.len() != 64 {
        return Err(format!("expected 64 hex chars, got {}", hex.len()));
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

/// A finite sequence of chain-harvested problems, replayed in file order.
///
/// Unlike [`ProblemSource`](crate::problem::ProblemSource), this source is
/// finite: it yields each instance once and then returns `None`. A mode that
/// runs to a wall-clock target therefore stops when the corpus runs out.
///
/// Instances are drawn lazily, one per [`next_job`](Self::next_job). Loading
/// keeps only the nonces, which is 32 bytes each. Drawing eagerly would
/// materialize one `h` vector per node and one `J` vector per edge for every
/// instance at once — for the bundled `chain-h0` corpus that is 4658
/// instances over 46092 coefficients, on the order of a gigabyte of problem
/// data held before the first job ever reaches a miner.
#[derive(Debug)]
pub struct CorpusSource {
    label: String,
    spec: TopologySpec,
    nonces: std::vec::IntoIter<[u8; 32]>,
    total: usize,
    next_index: usize,
    chain: HashMap<Vec<u8>, ChainResult>,
}

impl CorpusSource {
    /// Load `name_or_path` as a corpus and draw every instance against
    /// `spec`.
    ///
    /// `name_or_path` is a bundled corpus name (see [`bundled_names`]) or a
    /// path to a JSONL file. A bundled name is checked against the fixture it
    /// is paired with, so a mismatched topology fails here rather than
    /// producing quietly wrong problems.
    ///
    /// # Errors
    ///
    /// Returns [`CorpusError`] when the name resolves to nothing, the file
    /// cannot be read, any line fails to parse, any nonce is malformed, the
    /// file is empty, or the corpus does not belong to `spec`.
    pub fn load(name_or_path: &str, spec: &TopologySpec) -> Result<Self, CorpusError> {
        // A bundled name wins over a same-named file on disk, so a run that
        // says `--corpus chain-h0` always means the shipped corpus.
        let path = match bundled_path(name_or_path) {
            Some(p) => p,
            None => PathBuf::from(name_or_path),
        };
        if !path.is_file() {
            return Err(CorpusError::UnknownName {
                name: name_or_path.to_string(),
                known: bundled_names(),
            });
        }
        let label = name_or_path.to_string();
        let text = fs::read_to_string(&path)
            .map_err(|e| CorpusError::Io(format!("{}: {e}", path.display())))?;
        Self::parse(&label, &text, spec)
    }

    /// Parse already-loaded JSONL `text` against `spec`. Used directly by
    /// tests and by [`load`](Self::load).
    ///
    /// # Errors
    ///
    /// Same as [`load`](Self::load), except the I/O and name errors.
    pub fn parse(label: &str, text: &str, spec: &TopologySpec) -> Result<Self, CorpusError> {
        check_pairing(label, spec)?;
        let mut nonces = Vec::new();
        let mut chain = HashMap::new();
        for (i, line) in text.lines().enumerate() {
            let line_no = i + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let entry: CorpusEntryJson =
                serde_json::from_str(trimmed).map_err(|e| CorpusError::Parse {
                    line: line_no,
                    reason: e.to_string(),
                })?;
            let nonce = decode_nonce(&entry.nonce).map_err(|reason| CorpusError::BadNonce {
                line: line_no,
                reason,
            })?;
            nonces.push(nonce);
            let _ = chain.insert(
                nonce.to_vec(),
                ChainResult {
                    energy_milli: entry.energy_milli,
                    max_energy_milli: entry.max_energy_milli,
                    qblock_id: entry.qblock_id,
                },
            );
        }
        if nonces.is_empty() {
            return Err(CorpusError::Empty);
        }
        let total = nonces.len();
        Ok(Self {
            label: label.to_string(),
            spec: spec.clone(),
            nonces: nonces.into_iter(),
            total,
            next_index: 0,
            chain,
        })
    }

    /// The corpus name or path this source was loaded from.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// How many instances the corpus holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.total
    }

    /// Whether the corpus holds no instances. Always false: [`parse`](Self::parse)
    /// rejects an empty corpus.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }

    /// What the chain recorded for the instance with this `job_id` (which is
    /// its nonce), if the corpus carried those fields.
    #[must_use]
    pub fn chain_result(&self, job_id: &[u8]) -> Option<ChainResult> {
        self.chain.get(job_id).copied()
    }

    /// Draw the next instance, or `None` once the corpus is exhausted.
    ///
    /// The draw itself cannot fail for any topology this crate can build:
    /// `draw_ising_milli` only errors on an empty allowed-value set, and
    /// `topology::load_topology` rejects those. This mirrors
    /// `problem::ProblemSource::next_job` and fails closed rather than
    /// panicking.
    #[must_use]
    pub fn next_job(&mut self) -> Option<Job> {
        let nonce = self.nonces.next()?;
        self.next_index += 1;
        draw_job(&nonce, &self.spec, self.next_index)
    }
}

/// Reject a bundled corpus paired with a topology it was not drawn against.
/// A corpus loaded from a path carries no declared pairing, so it is the
/// caller's responsibility and passes through.
fn check_pairing(label: &str, spec: &TopologySpec) -> Result<(), CorpusError> {
    let Some(fixture) = paired_fixture(label) else {
        return Ok(());
    };
    let Ok(expected) = crate::topology::load_fixture(fixture) else {
        // The paired fixture ships alongside the corpus in this crate. If it
        // cannot load, the install is broken in a way this check cannot
        // diagnose; let the run proceed and fail on the topology itself.
        return Ok(());
    };
    let expected_hash = expected.topology_hash();
    let actual_hash = spec.topology_hash();
    if expected_hash == actual_hash {
        return Ok(());
    }
    Err(CorpusError::TopologyMismatch {
        corpus: label.to_string(),
        expected: hex_encode(&expected_hash),
        actual: hex_encode(&actual_hash),
    })
}

/// Build the job a nonce names, drawn against `spec`.
///
/// Mirrors upstream's `build_ising_job_from_nonce`: the job id *is* the
/// nonce, and the graph travels as a topology hash rather than an edge list.
/// `generation` is the instance's 1-based position in the corpus.
fn draw_job(nonce: &[u8; 32], spec: &TopologySpec, generation: usize) -> Option<Job> {
    let (h, j) = draw_ising_milli(
        *nonce,
        spec.nodes.len(),
        spec.edges.len(),
        &spec.allowed_h_milli,
        &spec.allowed_j_milli,
    )
    .ok()?;
    Some(Job {
        job_id: nonce.to_vec(),
        kind: JobKind::IsingSample as i32,
        generation: generation as u64,
        deadline_ms: 0,
        ising: Some(IsingProblem {
            graph: Some(ising_problem::Graph::TopologyHash(spec.topology_hash())),
            encoding: CoefficientEncoding::I32 as i32,
            scale: MILLI_SCALE,
            h: encode_i32_le(&h),
            j: encode_i32_le(&j),
            ..IsingProblem::default()
        }),
        provenance: Some(Provenance {
            is_pow: true,
            order_id: vec![],
        }),
        generator: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::load_fixture;
    use quip_protocol::wire::decode_i32_le;

    fn h0() -> TopologySpec {
        load_fixture("chain-h0").expect("chain-h0 fixture parses")
    }

    const NONCE_A: &str = "0cf67b0f85e37ddd3dda2ac11685c7ad857d069eda939ae3f2d6d93e3d5b8864";

    #[test]
    fn job_id_is_the_nonce_and_graph_travels_as_a_topology_hash() {
        let spec = h0();
        let text = format!(r#"{{"nonce":"{NONCE_A}"}}"#);
        let mut src = CorpusSource::parse("inline", &text, &spec).expect("parses");
        let job = src.next_job().expect("one job");
        assert_eq!(hex_encode(&job.job_id), NONCE_A);
        let ising = job.ising.expect("job has an ising problem");
        assert_eq!(
            ising.graph,
            Some(ising_problem::Graph::TopologyHash(spec.topology_hash()))
        );
        assert!(job.provenance.expect("provenance").is_pow);
    }

    #[test]
    fn draw_lengths_and_values_follow_the_paired_topology() {
        let spec = h0();
        let text = format!(r#"{{"nonce":"{NONCE_A}"}}"#);
        let mut src = CorpusSource::parse("inline", &text, &spec).expect("parses");
        let ising = src.next_job().expect("one job").ising.expect("ising");
        let h = decode_i32_le(&ising.h).expect("h decodes");
        let j = decode_i32_le(&ising.j).expect("j decodes");
        assert_eq!(h.len(), spec.nodes.len());
        assert_eq!(j.len(), spec.edges.len());
        assert!(
            h.iter().all(|v| *v == 0),
            "the chain-h0 bucket allows only h = 0"
        );
        assert!(j.iter().all(|v| spec.allowed_j_milli.contains(v)));
    }

    /// The same nonce over two different topologies is two different
    /// problems. This is why the pairing check exists.
    #[test]
    fn the_same_nonce_draws_a_different_problem_per_topology() {
        let text = format!(r#"{{"nonce":"{NONCE_A}"}}"#);
        let h0_spec = h0();
        let ternary = load_fixture("chain-ternary").expect("fixture parses");
        let mut a = CorpusSource::parse("inline", &text, &h0_spec).expect("parses");
        let mut b = CorpusSource::parse("inline", &text, &ternary).expect("parses");
        let ha = a.next_job().expect("job").ising.expect("ising").h;
        let hb = b.next_job().expect("job").ising.expect("ising").h;
        assert_ne!(ha, hb);
    }

    #[test]
    fn replay_is_deterministic_across_instances() {
        let spec = h0();
        let text = format!(r#"{{"nonce":"{NONCE_A}"}}"#);
        let mut a = CorpusSource::parse("inline", &text, &spec).expect("parses");
        let mut b = CorpusSource::parse("inline", &text, &spec).expect("parses");
        assert_eq!(a.next_job(), b.next_job());
    }

    #[test]
    fn yields_each_instance_once_in_file_order_then_none() {
        let spec = h0();
        let other = "22681ca9153f4f30a0d5af9da319e2b93848c182a7584a541d52db2214a08d54";
        let text = format!("{{\"nonce\":\"{NONCE_A}\"}}\n{{\"nonce\":\"{other}\"}}\n");
        let mut src = CorpusSource::parse("inline", &text, &spec).expect("parses");
        assert_eq!(src.len(), 2);
        assert_eq!(hex_encode(&src.next_job().expect("first").job_id), NONCE_A);
        assert_eq!(hex_encode(&src.next_job().expect("second").job_id), other);
        assert!(src.next_job().is_none());
    }

    #[test]
    fn chain_result_is_keyed_by_job_id() {
        let spec = h0();
        let text = format!(
            r#"{{"nonce":"{NONCE_A}","energy_milli":-14650000,"max_energy_milli":-14620736,"qblock_id":629}}"#
        );
        let mut src = CorpusSource::parse("inline", &text, &spec).expect("parses");
        let job = src.next_job().expect("one job");
        assert_eq!(
            src.chain_result(&job.job_id),
            Some(ChainResult {
                energy_milli: Some(-14_650_000),
                max_energy_milli: Some(-14_620_736),
                qblock_id: Some(629),
            })
        );
    }

    #[test]
    fn blank_lines_are_skipped() {
        let spec = h0();
        let other = "22681ca9153f4f30a0d5af9da319e2b93848c182a7584a541d52db2214a08d54";
        let text = format!("{{\"nonce\":\"{NONCE_A}\"}}\n\n{{\"nonce\":\"{other}\"}}\n");
        assert_eq!(
            CorpusSource::parse("inline", &text, &spec)
                .expect("parses")
                .len(),
            2
        );
    }

    #[test]
    fn a_bundled_corpus_paired_with_the_wrong_topology_is_rejected() {
        let text = format!(r#"{{"nonce":"{NONCE_A}"}}"#);
        let wrong = load_fixture("chain-ternary").expect("fixture parses");
        let err = CorpusSource::parse("chain-h0", &text, &wrong)
            .expect_err("a mismatched pairing must be rejected");
        assert!(matches!(err, CorpusError::TopologyMismatch { .. }), "{err}");
        assert!(err.to_string().contains("--topology-preset"));
    }

    #[test]
    fn a_bundled_corpus_paired_with_its_own_topology_is_accepted() {
        let text = format!(r#"{{"nonce":"{NONCE_A}"}}"#);
        assert!(CorpusSource::parse("chain-h0", &text, &h0()).is_ok());
    }

    #[test]
    fn malformed_json_names_the_line() {
        let spec = h0();
        let text = format!("{{\"nonce\":\"{NONCE_A}\"}}\nnot json");
        assert!(matches!(
            CorpusSource::parse("inline", &text, &spec).expect_err("must reject"),
            CorpusError::Parse { line: 2, .. }
        ));
    }

    #[test]
    fn a_short_nonce_names_the_line() {
        let spec = h0();
        let err = CorpusSource::parse("inline", r#"{"nonce":"abcd"}"#, &spec)
            .expect_err("must reject a short nonce");
        assert!(
            matches!(err, CorpusError::BadNonce { line: 1, .. }),
            "{err}"
        );
    }

    #[test]
    fn a_non_hex_nonce_names_the_line() {
        let spec = h0();
        let text = format!(r#"{{"nonce":"zz{}"}}"#, &NONCE_A[2..]);
        assert!(matches!(
            CorpusSource::parse("inline", &text, &spec).expect_err("must reject"),
            CorpusError::BadNonce { line: 1, .. }
        ));
    }

    #[test]
    fn an_empty_corpus_is_rejected() {
        assert_eq!(
            CorpusSource::parse("inline", "\n\n", &h0()).expect_err("must reject"),
            CorpusError::Empty
        );
    }

    #[test]
    fn an_unknown_name_lists_the_bundled_corpora() {
        let err = CorpusSource::load("no-such-corpus", &h0()).expect_err("must reject");
        let msg = err.to_string();
        assert!(msg.contains("chain-h0"), "{msg}");
        assert!(msg.contains("chain-ternary"), "{msg}");
    }

    #[test]
    fn every_bundled_corpus_loads_against_its_paired_fixture() {
        for name in bundled_names() {
            let fixture = paired_fixture(&name).expect("bundled corpus has a paired fixture");
            let spec = load_fixture(fixture).expect("paired fixture parses");
            let src = CorpusSource::load(&name, &spec).expect("bundled corpus loads");
            assert!(src.len() > 1000, "{name} holds {} instances", src.len());
        }
    }
}
