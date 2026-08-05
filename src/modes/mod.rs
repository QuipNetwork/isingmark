// SPDX-License-Identifier: AGPL-3.0-or-later
//! Benchmark modes and the types shared across them: a topology reference
//! CLI/library callers hand every mode, the error type every mode's
//! `run_*` function returns, and a small parse-error type for this crate's
//! own hand-rolled value parsers (durations, and later parameter grids).

use std::fmt;
use std::path::PathBuf;

use crate::backend::SessionError;
use crate::topology::{self, TopologyError, TopologySpec};

pub mod sweep;
pub mod throughput;
pub mod ttt;

/// A topology to benchmark against: a bundled fixture name, or a JSON file
/// path. Every mode that needs a topology takes one of these rather than a
/// raw `--topology-preset`/`--topology-file` pair, so the CLI's mutually
/// exclusive flag group is resolved once, at the CLI boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopologyRef {
    /// A bundled fixture name, loaded via [`topology::load_fixture`].
    Preset(String),
    /// A topology-spec JSON file, loaded via [`topology::load_topology`].
    File(PathBuf),
}

impl TopologyRef {
    /// Load the referenced topology.
    ///
    /// # Errors
    ///
    /// Returns whatever `topology::load_fixture`/`topology::load_topology`
    /// returns for an invalid or missing spec.
    pub fn load(&self) -> Result<TopologySpec, TopologyError> {
        match self {
            Self::Preset(name) => topology::load_fixture(name),
            Self::File(path) => topology::load_topology(path),
        }
    }

    /// A short human-readable label for `record::TopologyInfo::name`: the
    /// preset name, or the file's own path text.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Preset(name) => name.clone(),
            Self::File(path) => path.display().to_string(),
        }
    }
}

/// Errors common to every benchmark mode's `run_*` entry point.
#[derive(Debug)]
pub enum ModeError {
    /// The configured topology could not be loaded.
    Topology(TopologyError),
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
}

impl fmt::Display for ModeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Topology(e) => write!(f, "failed to load topology: {e}"),
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
        }
    }
}

impl std::error::Error for ModeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Topology(e) => Some(e),
            Self::UnknownBackend(_)
            | Self::RecordsFileExists(_)
            | Self::EmptyGrid
            | Self::InapplicableGridDimension(_)
            | Self::EmptyTargets
            | Self::DuplicateTarget(_) => None,
            Self::Session(e) => Some(e),
            Self::Io(e) => Some(e),
        }
    }
}

impl From<TopologyError> for ModeError {
    fn from(e: TopologyError) -> Self {
        Self::Topology(e)
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
