// SPDX-License-Identifier: AGPL-3.0-or-later
//! Benchmark modes and the types shared across them: a topology reference
//! CLI/library callers hand every mode, the error type every mode's
//! `run_*` function returns, and a small parse-error type for this crate's
//! own hand-rolled value parsers (durations, and later parameter grids).

use std::fmt;
use std::path::{Path, PathBuf};

use quip_proto::v1::Job;

use crate::backend::SessionError;
use crate::corpus::{CorpusError, CorpusSource};
use crate::instances::{InstanceError, InstanceSource};
use crate::problem::ProblemSource;
use crate::topology::{self, TopologyError, TopologySpec};

pub mod comparative;
pub mod compare;
pub mod reproducibility;
pub mod scaling;
pub mod sweep;
pub mod throughput;
pub mod ttt;

/// Where a mode's jobs come from: derived from a seed, replayed from a corpus
/// of chain-harvested instances, or replayed from a file of explicit
/// instances.
///
/// A derived source differs from the other two in one way that matters to a
/// caller. A derived source is endless — it will draw as many problems as
/// asked for. A corpus or an instance file is finite and yields each instance
/// exactly once, so a mode that dispatches until a wall-clock target stops
/// early when it runs out. Treat `--duration` as a cap in that case, not a
/// target.
#[derive(Debug)]
pub enum JobSource {
    /// Deterministic problems derived from `(seed, topology, index)`.
    Derived(Box<ProblemSource>),
    /// Chain-harvested instances replayed from a corpus.
    Corpus(Box<CorpusSource>),
    /// Explicit coefficients replayed from an instance file.
    Instances(Box<InstanceSource>),
}

impl JobSource {
    /// Draw the next job, or `None` when the source is exhausted. A
    /// [`Derived`](Self::Derived) source is exhausted only at the job count
    /// it was built with.
    #[must_use]
    pub fn next_job(&mut self) -> Option<Job> {
        match self {
            Self::Derived(s) => s.next_job(),
            Self::Corpus(s) => s.next_job(),
            Self::Instances(s) => s.next_job(),
        }
    }

    /// A short label naming the source for a run summary: `"seed:<n>"` for a
    /// derived source, the corpus name or path, or `"instances:<path>"`.
    #[must_use]
    pub fn label(&self, seed: u64) -> String {
        match self {
            Self::Derived(_) => format!("seed:{seed}"),
            Self::Corpus(s) => s.label().to_string(),
            Self::Instances(s) => format!("instances:{}", s.label()),
        }
    }

    /// How many instances a corpus or instance-file source holds, or `None`
    /// for a derived source, which has no fixed size.
    #[must_use]
    pub fn instances(&self) -> Option<usize> {
        match self {
            Self::Derived(_) => None,
            Self::Corpus(s) => Some(s.len()),
            Self::Instances(s) => Some(s.len()),
        }
    }
}

/// Load the finite source a run asked for, or `None` when it asked for
/// neither: `corpus` names a corpus replay, `instances` an instance file.
///
/// # Errors
///
/// Returns [`ModeError::ConflictingSources`] when both are set, or the load
/// error of whichever source was named.
pub fn load_finite_source(
    corpus: Option<&str>,
    instances: Option<&Path>,
    spec: &TopologySpec,
) -> Result<Option<JobSource>, ModeError> {
    match (corpus, instances) {
        (Some(_), Some(_)) => Err(ModeError::ConflictingSources),
        (Some(name), None) => Ok(Some(JobSource::Corpus(Box::new(CorpusSource::load(
            name, spec,
        )?)))),
        (None, Some(path)) => Ok(Some(JobSource::Instances(Box::new(InstanceSource::load(
            path, spec,
        )?)))),
        (None, None) => Ok(None),
    }
}

/// A topology to benchmark against: a bundled fixture, a JSON file, or a
/// generated graph. Every mode that needs a topology takes one of these,
/// so the CLI's mutually exclusive flags resolve once at the CLI boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyRef {
    /// A bundled fixture name, loaded via [`topology::load_fixture`].
    Preset(String),
    /// A topology-spec JSON file, loaded via [`topology::load_topology`].
    File(PathBuf),
    /// An in-memory graph and its coefficient sets.
    Generated {
        /// `complete:N` or `circulant:N:D`.
        description: String,
        /// Allowed h and J values.
        couplings: topology::Couplings,
    },
}

impl TopologyRef {
    /// Load the referenced topology.
    ///
    /// # Errors
    ///
    /// Returns the loader or generator error for an invalid or missing spec.
    pub fn load(&self) -> Result<TopologySpec, TopologyError> {
        match self {
            Self::Preset(name) => topology::load_fixture(name),
            Self::File(path) => topology::load_topology(path),
            Self::Generated {
                description,
                couplings,
            } => topology::generate_topology(description, *couplings),
        }
    }

    /// A short human-readable label for `record::TopologyInfo::name`: the
    /// preset name, file path, or generator description.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Preset(name) => name.clone(),
            Self::File(path) => path.display().to_string(),
            Self::Generated { description, .. } => description.clone(),
        }
    }
}

/// Errors common to every benchmark mode's `run_*` entry point.
#[derive(Debug)]
pub enum ModeError {
    /// The configured topology could not be loaded.
    Topology(TopologyError),
    /// The configured corpus could not be loaded, or does not belong to the
    /// configured topology.
    Corpus(CorpusError),
    /// The configured instance file could not be loaded, or does not belong
    /// to the configured topology.
    Instances(InstanceError),
    /// Both a corpus and an instance file were configured. A run replays one
    /// finite source.
    ConflictingSources,
    /// `backend` did not match any entry in `backend::builtin_registry`.
    UnknownBackend(String),
    /// The coordinator-side session loop failed to start (bind/spawn).
    Session(SessionError),
    /// Writing job records or the run summary failed.
    Io(std::io::Error),
    /// The mode's records file already contains data. Modes refuse to
    /// append into a pre-existing run's output rather than silently
    /// growing a shared file across separate invocations of the same
    /// `--output-dir` (the normal case, since it defaults to a fixed
    /// path).
    RecordsFileExists(PathBuf),
    /// `sweep::ParamGrid`'s applicable dimensions (which ones apply depends
    /// on the resolved backend's `params::BackendKind`) produced zero
    /// cells: at least one of them was an empty `[]`.
    EmptyGrid,
    /// A `sweep::ParamGrid` dimension was set that does not apply to the
    /// resolved backend's `params::BackendKind` (`anneal_time_us` for a
    /// `Sweeping` backend, or `num_sweeps` for an `Annealing` one).
    /// Rejected rather than silently ignored: a dropped sweep axis with no
    /// error would make part of the requested grid vanish with no signal,
    /// which this crate's parameter handling has repeatedly ruled against.
    InapplicableGridDimension(String),
    /// `ttt::TttConfig::energy_targets_milli` was empty.
    EmptyTargets,
    /// `ttt::TttConfig::energy_targets_milli` contained the same milli-value
    /// target more than once. Rejected rather than silently deduplicated: a
    /// duplicate would resolve to the same `ttt::target_dir_name`, and the
    /// second target's `std::fs::File::create` call would silently truncate
    /// the first target's just-written records (`File::create` truncates an
    /// existing file rather than refusing to touch it).
    DuplicateTarget(i64),
    /// `comparative::ComparativeConfig::max_iterations` was `0`. Rejected at
    /// the mode level (in addition to `cli::parse_max_iterations`'s own
    /// identical CLI-level rejection) because a zero-iteration search would
    /// otherwise return hardness `0.0` — a plausible-looking, in-range
    /// value indistinguishable from a real finding, unlike an empty result
    /// set with an obviously-zero count.
    InvalidMaxIterations,
    /// `comparative::ComparativeConfig::target_file` could not be opened or
    /// read.
    TargetFileUnreadable(PathBuf, std::io::Error),
    /// `comparative::ComparativeConfig::target_file` parsed but contained no
    /// usable (`passed`) records to compare a search evaluation against.
    EmptyTargetFile(PathBuf),
    /// `comparative::run_comparative`'s own binary search produced a
    /// non-finite or out-of-`[0.0, 1.0]` hardness. Unreachable under normal
    /// operation (the search's bracket only ever narrows within `[0.0,
    /// 1.0]`), but checked rather than trusted — see that function's own
    /// doc on why a divergent search must fail loudly rather than emit a
    /// `NaN`/out-of-range hardness into a summary that looks like every
    /// other, valid one.
    InvalidComputedHardness(f64),
    /// `scaling::ScalingConfig::sizes` was empty.
    EmptySizes,
    /// A `scaling::ScalingConfig::sizes` entry cannot support a 3-regular
    /// graph: every node needs 3 distinct neighbours, which is impossible
    /// for an odd node count or fewer than 4 nodes. Rejected at both parse
    /// time (`cli::parse_one_scaling_size`) and here, for a caller that
    /// constructs a `ScalingConfig` directly.
    InvalidSize(usize),
    /// `scaling::ScalingConfig::sizes` contained the same node count more
    /// than once. Rejected rather than silently deduplicated: a duplicate
    /// would resolve to the same `scaling::size_dir_name`, and the second
    /// size's `File::create` call would silently truncate the first size's
    /// just-written records.
    DuplicateSize(usize),
    /// `scaling::generate_topology`'s configuration-model random-graph
    /// generator did not find a valid (no self-loop, no duplicate edge)
    /// pairing within its bounded attempt budget for the given node count.
    /// Exceedingly unlikely for any size this crate's own tests or a
    /// realistic `--sizes` value would ever use, but checked rather than
    /// looping forever or returning a graph that is not actually
    /// 3-regular.
    GraphGenerationFailed(usize),
    /// `reproducibility::run_reproducibility`'s two runs produced a batch
    /// of `passed` energies that `stats::ks_distance`/`stats::ks_two_sample_p`
    /// cannot compare (empty, or containing a non-finite value -- see those
    /// functions' own documented `None` cases). Reported as an error rather
    /// than fabricating a `p_value` (e.g. defaulting to `1.0`, which would
    /// silently claim "the two runs are identical" for a run that produced
    /// no usable data at all).
    EmptyReproBatch {
        /// Which run(s) produced no usable batch: `"run_a"`, `"run_b"`, or
        /// `"run_a and run_b"`.
        which: &'static str,
    },
    /// `compare::run_compare`'s `--file-a` or `--file-b` could not be opened
    /// or read. Mirrors [`Self::TargetFileUnreadable`]: an I/O failure on a
    /// user-supplied input path is an error naming that path, not a silent
    /// empty distribution.
    CompareFileUnreadable(PathBuf, std::io::Error),
    /// `compare::run_compare`'s input file parsed but yielded no usable
    /// (`passed`) records to compare, or `stats::describe`/
    /// `stats::ks_distance`/`stats::ks_two_sample_p` returned `None` for
    /// that side. Reported as an error rather than fabricating a
    /// `CompareReport` whose `Stats`/`ks`/`p_value` would look like a real
    /// comparison of empty data — the same reason
    /// [`Self::EmptyTargetFile`] and [`Self::EmptyReproBatch`] refuse to
    /// invent a default `p_value`.
    EmptyCompareFile(PathBuf),
    /// `compare::run_compare`'s input file yielded no usable (`passed`)
    /// energies and at least one non-blank line failed to parse as a
    /// [`crate::record::JobRecord`]. Distinct from [`Self::EmptyCompareFile`]:
    /// an all-failed run has parseable records with `passed: false`, while
    /// this case means every candidate line was discarded (typically a
    /// `JobRecord` schema mismatch with this build). The `usize` is the
    /// `read_jsonl` skip count.
    CompareFileUnparseable(PathBuf, usize),
}

impl fmt::Display for ModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Topology(e) => write!(f, "failed to load topology: {e}"),
            Self::Corpus(e) => write!(f, "failed to load corpus: {e}"),
            Self::Instances(e) => write!(f, "failed to load instances: {e}"),
            Self::ConflictingSources => {
                write!(
                    f,
                    "corpus and instances are mutually exclusive: set only one"
                )
            }
            Self::UnknownBackend(name) => write!(
                f,
                "unknown backend {name:?}; see `backend::builtin_registry` for valid names"
            ),
            Self::Session(e) => write!(f, "failed to run miner session: {e}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::RecordsFileExists(path) => write!(
                f,
                "records file {} already contains data from a previous run; \
                 choose a different --output-dir or remove the file first",
                path.display()
            ),
            Self::EmptyGrid => write!(
                f,
                "--param-grid produced zero cells: every dimension it applies to \
                 (given the resolved backend's execution model) must be a nonempty list"
            ),
            Self::InapplicableGridDimension(message) => {
                write!(f, "invalid --param-grid: {message}")
            }
            Self::EmptyTargets => write!(f, "--energy-targets must not be empty"),
            Self::DuplicateTarget(milli) => write!(
                f,
                "--energy-targets contains the same milli-value target twice ({milli}); \
                 each target must resolve to its own output subdirectory"
            ),
            Self::InvalidMaxIterations => write!(
                f,
                "--max-iterations must not be 0 (a zero-iteration search would report hardness \
                 0.0 with no signal that no evaluation ever ran)"
            ),
            Self::TargetFileUnreadable(path, e) => {
                write!(f, "--target-file {} could not be read: {e}", path.display())
            }
            Self::EmptyTargetFile(path) => write!(
                f,
                "--target-file {} contains no usable (passed) records to match against",
                path.display()
            ),
            Self::InvalidComputedHardness(value) => write!(
                f,
                "comparative search produced an invalid hardness ({value}); this indicates a bug \
                 in the search's own bracket arithmetic, not a user input mistake"
            ),
            Self::EmptySizes => write!(f, "--sizes must not be empty"),
            Self::InvalidSize(n) => write!(
                f,
                "--sizes contains {n}, which cannot support a 3-regular graph (every node needs \
                 3 distinct neighbours, so a size must be even and at least 4)"
            ),
            Self::DuplicateSize(n) => write!(
                f,
                "--sizes contains the same node count twice ({n}); each size must resolve to \
                 its own output subdirectory"
            ),
            Self::GraphGenerationFailed(n) => write!(
                f,
                "failed to generate a random 3-regular graph on {n} nodes within the attempt \
                 budget; this indicates a bug in the generator, not a user input mistake"
            ),
            Self::EmptyReproBatch { which } => write!(
                f,
                "{which} produced no usable (passed) records to compare distributions against"
            ),
            Self::CompareFileUnreadable(path, e) => {
                write!(
                    f,
                    "compare input file {} could not be read: {e}",
                    path.display()
                )
            }
            Self::EmptyCompareFile(path) => write!(
                f,
                "compare input file {} contains no usable (passed) records to compare",
                path.display()
            ),
            Self::CompareFileUnparseable(path, n) => write!(
                f,
                "compare input file {} has no usable (passed) records: {n} lines failed to parse \
                 (likely a JobRecord schema mismatch)",
                path.display()
            ),
        }
    }
}

impl std::error::Error for ModeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Topology(e) => Some(e),
            Self::Corpus(e) => Some(e),
            Self::Instances(e) => Some(e),
            Self::UnknownBackend(_)
            | Self::ConflictingSources
            | Self::RecordsFileExists(_)
            | Self::EmptyGrid
            | Self::InapplicableGridDimension(_)
            | Self::EmptyTargets
            | Self::DuplicateTarget(_)
            | Self::InvalidMaxIterations
            | Self::EmptyTargetFile(_)
            | Self::InvalidComputedHardness(_)
            | Self::EmptySizes
            | Self::InvalidSize(_)
            | Self::DuplicateSize(_)
            | Self::GraphGenerationFailed(_)
            | Self::EmptyReproBatch { .. }
            | Self::EmptyCompareFile(_)
            | Self::CompareFileUnparseable(_, _) => None,
            Self::Session(e) => Some(e),
            Self::Io(e) | Self::TargetFileUnreadable(_, e) | Self::CompareFileUnreadable(_, e) => {
                Some(e)
            }
        }
    }
}

impl From<TopologyError> for ModeError {
    fn from(e: TopologyError) -> Self {
        Self::Topology(e)
    }
}

impl From<CorpusError> for ModeError {
    fn from(e: CorpusError) -> Self {
        Self::Corpus(e)
    }
}

impl From<InstanceError> for ModeError {
    fn from(e: InstanceError) -> Self {
        Self::Instances(e)
    }
}

impl From<SessionError> for ModeError {
    fn from(e: SessionError) -> Self {
        Self::Session(e)
    }
}

/// A malformed value passed to one of this crate's small hand-rolled
/// parsers (durations, and later parameter grids), carrying an actionable
/// message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Hex-encode `bytes`, lowercase, no separator. Matches the convention
/// `record::JobRecord::job_id` and `backend::session` already use for wire
/// ids and hashes.
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Milli-units to natural `f64` units (divide by 1000), for feeding
/// `stats::describe`/`stats::ks_distance`/`stats::ks_two_sample_p`.
///
/// Shared entry point for new modes (currently `compare`). Existing mode
/// modules keep their own private copies — consolidating those is a
/// separate change and must not land mixed into a feature task.
#[expect(
    clippy::cast_precision_loss,
    reason = "benchmark energies fit comfortably within f64's 52-bit exact-integer range"
)]
#[must_use]
pub(crate) fn milli_to_f64(milli: i64) -> f64 {
    milli as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{hex_encode, TopologyRef};

    #[test]
    fn hex_encode_is_lowercase_no_separator() {
        assert_eq!(hex_encode(&[0x0a, 0xff, 0x00]), "0aff00");
        assert_eq!(hex_encode(&[]), String::new());
    }

    #[test]
    fn preset_loads_the_named_fixture() {
        let spec = TopologyRef::Preset("smoke".to_string())
            .load()
            .expect("smoke fixture parses");
        assert_eq!(spec.nodes.len(), 8);
    }

    #[test]
    fn file_loads_a_topology_spec_from_disk() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("custom.spec.json");
        std::fs::write(
            &path,
            r#"{"nodes": [0, 1, 2], "edges": [[0, 1]], "allowed_h_milli": [0], "allowed_j_milli": [0]}"#,
        )
        .expect("write fixture");
        let spec = TopologyRef::File(path).load().expect("file spec parses");
        assert_eq!(spec.nodes, vec![0, 1, 2]);
    }

    #[test]
    fn label_uses_the_preset_name_or_the_files_own_path_text() {
        assert_eq!(TopologyRef::Preset("smoke".to_string()).label(), "smoke");
        let path = std::path::PathBuf::from("/tmp/custom.spec.json");
        assert_eq!(TopologyRef::File(path).label(), "/tmp/custom.spec.json");
    }
}
