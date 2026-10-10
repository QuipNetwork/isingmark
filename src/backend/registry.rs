// SPDX-License-Identifier: AGPL-3.0-or-later
//! Built-in backend registry and `--capabilities`/`--check` probing.
//!
//! Mirrors the CLI surface `quip-solver-core::session::run` dispatches on
//! (`crates/quip-solver-core/src/session.rs` in `quip-solver-core` 0.0.2):
//! `--capabilities` prints a JSON document to stdout and exits `0` without
//! opening the device; `--check` opens the device, discards the result, and
//! exits `0` on success or a backend-specific non-zero code (e.g.
//! `EnvIncompatible`) on failure.

use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::params::BackendKind;

/// A driveable backend: display name, resolved binary, extra CLI args, and
/// its execution model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSpec {
    /// Short display/config name (e.g. `"cpu-sa"`).
    pub name: String,
    /// Path to the backend binary. [`builtin_registry`] resolves this from
    /// `$PATH`; config overrides it with an explicit path.
    pub bin: PathBuf,
    /// Extra CLI args passed on every invocation, before the probe or
    /// session-mode flag.
    pub args: Vec<String>,
    /// Execution model: which job parameters this backend is driven by.
    pub kind: BackendKind,
    /// CLI flag this backend's binary accepts to size its kernel state for a
    /// topology, if it has one (`--max-nodes` on the CUDA miners). `None`
    /// means the binary does not know such a flag, and passing one would make
    /// it exit on an argument error instead of running.
    pub node_capacity_flag: Option<String>,
}

impl BackendSpec {
    /// Append this backend's node-capacity flag for a `nodes`-node topology.
    ///
    /// A backend without a flag is returned unchanged. An `args` list that
    /// already names the flag is left alone, so an explicit override from a
    /// config file wins over the derived value.
    #[must_use]
    pub fn with_node_capacity(mut self, nodes: usize) -> Self {
        let Some(flag) = self.node_capacity_flag.clone() else {
            return self;
        };
        if self.args.iter().any(|a| a == &flag) {
            return self;
        }
        self.args.push(flag);
        self.args.push(nodes.to_string());
        self
    }
}

/// Errors probing a [`BackendSpec`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// The binary could not be spawned (not on `$PATH`, not executable, does
    /// not exist, ...). The message is the underlying I/O error.
    NotFound(String),
    /// `--check` (device open) exited non-zero.
    CheckFailed {
        /// Process exit code, or `-1` if the process was terminated by a
        /// signal and reported no code.
        code: i32,
    },
    /// `--capabilities` exited non-zero, or its stdout was not valid UTF-8 /
    /// valid JSON.
    InvalidOutput(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(e) => write!(f, "backend binary not found: {e}"),
            Self::CheckFailed { code } => write!(f, "backend --check failed with exit code {code}"),
            Self::InvalidOutput(e) => {
                write!(f, "backend --capabilities returned invalid output: {e}")
            }
        }
    }
}

impl std::error::Error for ProbeError {}

/// One built-in backend's fixed identity: config-facing name, binary
/// filename resolved from `$PATH`, and execution model.
struct BuiltinEntry {
    name: &'static str,
    bin_name: &'static str,
    kind: BackendKind,
    /// See [`BackendSpec::node_capacity_flag`].
    node_capacity_flag: Option<&'static str>,
}

/// Fixed built-in backend list. Order matches display order; `builtin_registry`
/// preserves it.
const BUILTINS: &[BuiltinEntry] = &[
    BuiltinEntry {
        name: "cpu-sa",
        bin_name: "quip-cpu-sa",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-gibbs",
        bin_name: "quip-cpu-gibbs",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    // The annealing accelerations added on 2026-09-03, from Isakov et al.
    // (Comput. Phys. Commun. 192, 265 (2015)): tabulated Metropolis
    // thresholds, and the same with 64 reads packed per machine word. See
    // quip-solver-cpu/docs/sa-acceleration.md.
    BuiltinEntry {
        name: "cpu-fsa",
        bin_name: "quip-cpu-fsa",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-msa",
        bin_name: "quip-cpu-msa",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    // Simulated bifurcation, one entry per variant. Each variant is its own
    // binary rather than a flag on one binary, so each needs its own name
    // here. All nine are driven by reads and sweeps.
    //
    // Only `cpu-sb` ships in a quip-solver-cpu release. The other eight build
    // behind that crate's `experimental` feature, so their binaries reach
    // `$PATH` only when someone builds them on purpose. That costs nothing
    // here: a built-in whose binary is absent still gets an entry and fails
    // at probe time with `ProbeError::NotFound`, which is the same result as
    // naming a CUDA backend on a host with no card.
    BuiltinEntry {
        name: "cpu-sb",
        bin_name: "quip-cpu-sb",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-bsb",
        bin_name: "quip-cpu-bsb",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-hdsb",
        bin_name: "quip-cpu-hdsb",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-hbsb",
        bin_name: "quip-cpu-hbsb",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    // The variants added on 2026-09-02: edge-of-chaos control on the
    // ballistic and discrete couplings, tabu-enhanced dSB, replica-ring dSB,
    // and globally guided dSB. See quip-solver-cpu/docs/sb-variants.md.
    BuiltinEntry {
        name: "cpu-gbsb",
        bin_name: "quip-cpu-gbsb",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-gdsb",
        bin_name: "quip-cpu-gdsb",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-tedsb",
        bin_name: "quip-cpu-tedsb",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-sbqa",
        bin_name: "quip-cpu-sbqa",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-ggdsb",
        bin_name: "quip-cpu-ggdsb",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    // Tensor network and its bond-dimension-1 limit. `cpu-mps` picks its bond
    // dimension per job from a flop and memory budget, so it takes reads and
    // sweeps like the rest and needs no capacity flag of its own.
    BuiltinEntry {
        name: "cpu-mps",
        bin_name: "quip-cpu-mps",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "cpu-mfa",
        bin_name: "quip-cpu-mfa",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    // BP-gauged tensor network on the problem graph, experimental like the
    // SB variants: reads and sweeps drive it, so it is a Sweeping backend.
    BuiltinEntry {
        name: "cpu-flatiron",
        bin_name: "quip-cpu-flatiron",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    // `cuda` keeps its historical name rather than becoming `cuda-sa`:
    // renaming it would break every existing config and command line for no
    // functional gain.
    BuiltinEntry {
        name: "cuda",
        bin_name: "quip-cuda-sa",
        kind: BackendKind::Sweeping,
        node_capacity_flag: Some("--max-nodes"),
    },
    // Gibbs holds its spin state in shared memory rather than per-thread
    // local memory, so it reaches far larger topologies than `cuda`: measured
    // flat cost per node to 48000 on an A4000, against an SA ceiling of 8192.
    BuiltinEntry {
        name: "cuda-gibbs",
        bin_name: "quip-cuda-gibbs",
        kind: BackendKind::Sweeping,
        node_capacity_flag: Some("--max-nodes"),
    },
    BuiltinEntry {
        name: "metal",
        bin_name: "quip-metal-sa",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "dwave",
        bin_name: "quip-dwave-qa",
        kind: BackendKind::Annealing,
        node_capacity_flag: None,
    },
    BuiltinEntry {
        name: "beit",
        bin_name: "quip-beit-sa",
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    },
    // cuOpt solves each job as a MILP under a time budget carried on the
    // wire as `anneal_time_us`, with the same per-job/`SetTarget`/built-in
    // default precedence as `quip-dwave-qa`. It accepts `num_reads` only for
    // protocol parity — there is no MILP batch mode, so one solve returns one
    // incumbent — which makes `Annealing` (reads + anneal time) the execution
    // model that drives it, despite the solver itself being deterministic.
    BuiltinEntry {
        name: "cuopt",
        bin_name: "quip-cuopt-milp",
        kind: BackendKind::Annealing,
        node_capacity_flag: None,
    },
];

/// Build the registry of built-in backends (`cpu-sa`, `cpu-gibbs`, `cpu-fsa`,
/// `cpu-msa`, `cpu-sb`, `cpu-bsb`, `cpu-hdsb`, `cpu-hbsb`, `cpu-gbsb`,
/// `cpu-gdsb`, `cpu-tedsb`, `cpu-sbqa`, `cpu-ggdsb`, `cpu-mps`, `cpu-mfa`,
/// `cpu-flatiron`, `cuda`, `cuda-gibbs`, `metal`, `dwave`, `beit`, `cuopt`),
/// resolving each binary from `$PATH`.
///
/// A backend whose binary is not on `$PATH` still gets an entry: `bin` falls
/// back to the bare binary name, so [`probe_capabilities`]/[`probe_check`]
/// still run (and fail with [`ProbeError::NotFound`]) rather than the
/// registry silently omitting the backend. Config-driven overrides build
/// [`BackendSpec`] directly rather than going through this function.
#[must_use]
pub fn builtin_registry() -> Vec<BackendSpec> {
    let path_var = env::var_os("PATH").unwrap_or_default();
    builtin_registry_from(&path_var)
}

/// [`builtin_registry`], parameterized on the `$PATH`-style directory list
/// to search instead of reading the real environment.
///
/// This seam exists so tests can assert `builtin_registry` is actually wired
/// to `$PATH` resolution (as opposed to, say, searching on the wrong field of
/// [`BuiltinEntry`]) without mutating process-wide environment state:
/// `std::env::set_var` is `unsafe` and `unsafe-code = "deny"` forbids it
/// here.
fn builtin_registry_from(path_var: &OsStr) -> Vec<BackendSpec> {
    BUILTINS
        .iter()
        .map(|entry| BackendSpec {
            name: entry.name.to_string(),
            bin: resolve_in_paths(entry.bin_name, path_var)
                .unwrap_or_else(|| PathBuf::from(entry.bin_name)),
            args: Vec::new(),
            kind: entry.kind,
            node_capacity_flag: entry.node_capacity_flag.map(str::to_string),
        })
        .collect()
}

/// Search the `:`/`;`-separated directory list `path_var` (the same syntax
/// as `$PATH`) for an executable file named `bin_name`, returning the first
/// match in list order. Directories that do not contain a matching
/// executable file are skipped, including when they contain a
/// non-executable file of the same name.
fn resolve_in_paths(bin_name: &str, path_var: &OsStr) -> Option<PathBuf> {
    env::split_paths(path_var)
        .map(|dir| dir.join(bin_name))
        .find(|candidate| is_executable(candidate))
}

/// Whether `path` is a regular file with at least one executable bit set
/// (Unix), or simply exists as a file (other platforms, where execute
/// permission is not a separate filesystem bit).
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Run `<bin> --capabilities` and parse its stdout as JSON.
///
/// # Errors
///
/// Returns [`ProbeError::NotFound`] if the binary cannot be spawned,
/// [`ProbeError::InvalidOutput`] if it exits non-zero or its stdout is not
/// valid UTF-8/JSON.
pub fn probe_capabilities(spec: &BackendSpec) -> Result<serde_json::Value, ProbeError> {
    let output = Command::new(&spec.bin)
        .args(&spec.args)
        .arg("--capabilities")
        .output()
        .map_err(|e| ProbeError::NotFound(e.to_string()))?;

    if !output.status.success() {
        let code = output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ProbeError::InvalidOutput(format!(
            "--capabilities exited with status {code}: {stderr}"
        )));
    }

    let stdout = String::from_utf8(output.stdout).map_err(|e| {
        ProbeError::InvalidOutput(format!("--capabilities stdout is not valid UTF-8: {e}"))
    })?;
    serde_json::from_str(&stdout)
        .map_err(|e| ProbeError::InvalidOutput(format!("failed to parse --capabilities JSON: {e}")))
}

/// Run `<bin> --check`, mapping a non-zero exit to [`ProbeError::CheckFailed`].
///
/// # Errors
///
/// Returns [`ProbeError::NotFound`] if the binary cannot be spawned,
/// [`ProbeError::CheckFailed`] if it exits non-zero.
pub fn probe_check(spec: &BackendSpec) -> Result<(), ProbeError> {
    let status = Command::new(&spec.bin)
        .args(&spec.args)
        .arg("--check")
        .status()
        .map_err(|e| ProbeError::NotFound(e.to_string()))?;

    if status.success() {
        Ok(())
    } else {
        Err(ProbeError::CheckFailed {
            code: status.code().unwrap_or(-1),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::{builtin_registry, builtin_registry_from, BackendKind};

    /// Every built-in name maps to the exact expected binary and kind. Each
    /// name/binary/kind triple is distinct across entries, so a mixed-up
    /// mapping (e.g. `cuda` resolving to `quip-metal-sa`, or `dwave` tagged
    /// `Sweeping`) fails a specific assertion rather than being masked by a
    /// coincidental match.
    #[test]
    fn builtin_registry_has_the_fourteen_expected_entries_in_order() {
        let registry = builtin_registry();
        let actual: Vec<(&str, &str, BackendKind)> = registry
            .iter()
            .map(|spec| {
                (
                    spec.name.as_str(),
                    spec.bin.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                    spec.kind,
                )
            })
            .collect();
        assert_eq!(
            actual,
            vec![
                ("cpu-sa", "quip-cpu-sa", BackendKind::Sweeping),
                ("cpu-gibbs", "quip-cpu-gibbs", BackendKind::Sweeping),
                ("cpu-fsa", "quip-cpu-fsa", BackendKind::Sweeping),
                ("cpu-msa", "quip-cpu-msa", BackendKind::Sweeping),
                ("cpu-sb", "quip-cpu-sb", BackendKind::Sweeping),
                ("cpu-bsb", "quip-cpu-bsb", BackendKind::Sweeping),
                ("cpu-hdsb", "quip-cpu-hdsb", BackendKind::Sweeping),
                ("cpu-hbsb", "quip-cpu-hbsb", BackendKind::Sweeping),
                ("cpu-gbsb", "quip-cpu-gbsb", BackendKind::Sweeping),
                ("cpu-gdsb", "quip-cpu-gdsb", BackendKind::Sweeping),
                ("cpu-tedsb", "quip-cpu-tedsb", BackendKind::Sweeping),
                ("cpu-sbqa", "quip-cpu-sbqa", BackendKind::Sweeping),
                ("cpu-ggdsb", "quip-cpu-ggdsb", BackendKind::Sweeping),
                ("cpu-mps", "quip-cpu-mps", BackendKind::Sweeping),
                ("cpu-mfa", "quip-cpu-mfa", BackendKind::Sweeping),
                ("cpu-flatiron", "quip-cpu-flatiron", BackendKind::Sweeping),
                ("cuda", "quip-cuda-sa", BackendKind::Sweeping),
                ("cuda-gibbs", "quip-cuda-gibbs", BackendKind::Sweeping),
                ("metal", "quip-metal-sa", BackendKind::Sweeping),
                ("dwave", "quip-dwave-qa", BackendKind::Annealing),
                ("beit", "quip-beit-sa", BackendKind::Sweeping),
                ("cuopt", "quip-cuopt-milp", BackendKind::Annealing),
            ]
        );
    }

    /// Only backends whose binary accepts a node-capacity flag carry one.
    /// Passing `--max-nodes` to a miner that does not know the flag would
    /// make it exit on an argument error instead of running. Both CUDA
    /// miners take it; nothing else does.
    #[test]
    fn only_the_cuda_backends_declare_a_node_capacity_flag() {
        let registry = builtin_registry();
        for spec in &registry {
            let expected = if spec.name == "cuda" || spec.name == "cuda-gibbs" {
                Some("--max-nodes")
            } else {
                None
            };
            assert_eq!(
                spec.node_capacity_flag.as_deref(),
                expected,
                "backend {} has the wrong node-capacity flag",
                spec.name
            );
        }
    }

    /// A backend that declares the flag gets it appended with the topology's
    /// node count, so the miner compiles a kernel big enough for the run.
    #[test]
    fn with_node_capacity_appends_the_flag_for_cuda() {
        let cuda = builtin_registry()
            .into_iter()
            .find(|b| b.name == "cuda")
            .expect("registry always has cuda");
        let sized = cuda.with_node_capacity(5640);
        assert_eq!(
            sized.args,
            vec!["--max-nodes".to_string(), "5640".to_string()]
        );
    }

    /// A backend without the flag must be left exactly as it was.
    #[test]
    fn with_node_capacity_is_a_no_op_without_a_flag() {
        let cpu = builtin_registry()
            .into_iter()
            .find(|b| b.name == "cpu-sa")
            .expect("registry always has cpu-sa");
        let sized = cpu.clone().with_node_capacity(5640);
        assert_eq!(sized.args, cpu.args);
        assert!(sized.args.is_empty());
    }

    /// An explicit args override wins: a caller that set the flag by hand
    /// must not get a second, conflicting copy appended.
    #[test]
    fn with_node_capacity_leaves_an_explicit_flag_alone() {
        let mut cuda = builtin_registry()
            .into_iter()
            .find(|b| b.name == "cuda")
            .expect("registry always has cuda");
        cuda.args = vec!["--max-nodes".to_string(), "9000".to_string()];
        let sized = cuda.with_node_capacity(5640);
        assert_eq!(
            sized.args,
            vec!["--max-nodes".to_string(), "9000".to_string()]
        );
    }

    /// `builtin_registry_from` actually searches `path_var` for each
    /// entry's *binary name*, not some other field. Seeds `quip-cuda-sa`
    /// (and nothing else) into a synthetic `$PATH` and checks two things:
    /// the `cuda` entry resolves to that exact seeded path (not merely "some
    /// path" or the bare fallback), and every other entry — whose binary is
    /// absent from this `$PATH` — still falls back to its own bare name. A
    /// mix-up that searches on `entry.name` (e.g. `"cuda"`) instead of
    /// `entry.bin_name` (`"quip-cuda-sa"`) would never find the seeded file
    /// (nothing in this `$PATH` is literally named `cuda`), so `cuda` would
    /// wrongly fall back to the bare name too, failing the first assertion.
    #[test]
    fn builtin_registry_from_wires_path_resolution_to_each_entrys_bin_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let seeded = dir.path().join("quip-cuda-sa");
        std::fs::write(&seeded, "#!/bin/sh\nexit 0\n").expect("write fixture");
        std::fs::set_permissions(&seeded, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fixture");

        let path_var = std::env::join_paths([dir.path()]).expect("join_paths");
        let registry = builtin_registry_from(&path_var);

        let cuda = registry
            .iter()
            .find(|spec| spec.name == "cuda")
            .expect("cuda entry exists");
        assert_eq!(cuda.bin, seeded);

        for (spec, entry) in registry.iter().zip(super::BUILTINS.iter()) {
            if spec.name != "cuda" {
                assert_eq!(spec.bin, std::path::PathBuf::from(entry.bin_name));
            }
        }
    }

    /// Each entry's resolved `bin` is either a real file found on `$PATH`,
    /// or (the expected case on a machine with none of these miners
    /// installed) falls back to exactly the bare binary name — never an
    /// empty path or some other placeholder.
    #[test]
    fn missing_from_path_falls_back_to_bare_binary_name() {
        let registry = builtin_registry();
        for (spec, entry) in registry.iter().zip(super::BUILTINS.iter()) {
            let bare = std::path::PathBuf::from(entry.bin_name);
            assert!(spec.bin == bare || spec.bin.is_file());
        }
    }
}

#[cfg(test)]
mod path_resolution_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::resolve_in_paths;

    /// Create `dir/name` with the given executable bit, writing distinct,
    /// nonempty content so an accidental read of the wrong file is visible
    /// in a failure message rather than comparing two empty files.
    fn write_file(
        dir: &std::path::Path,
        name: &str,
        executable: bool,
        marker: &str,
    ) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, marker).expect("write fixture file");
        let mode = if executable { 0o755 } else { 0o644 };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .expect("chmod fixture file");
        path
    }

    /// `resolve_in_paths` picks the first directory (in `$PATH` order) that
    /// holds an *executable* file with the right name, skipping an earlier
    /// directory that holds a same-named but non-executable file. The three
    /// directories are ordered non-executable-first specifically so a
    /// mutation that drops the executable-bit check (treating any same-name
    /// file as a match) would resolve to the wrong, non-executable path
    /// instead of matching this test's expectation by coincidence.
    #[test]
    fn resolve_skips_non_executable_and_absent_dirs_to_find_first_executable_match() {
        let empty_dir = tempfile::tempdir().expect("tempdir");
        let non_exec_dir = tempfile::tempdir().expect("tempdir");
        let exec_dir = tempfile::tempdir().expect("tempdir");

        write_file(non_exec_dir.path(), "probe-me", false, "not executable");
        let expected = write_file(exec_dir.path(), "probe-me", true, "executable");

        let path_var =
            std::env::join_paths([empty_dir.path(), non_exec_dir.path(), exec_dir.path()])
                .expect("join_paths");

        assert_eq!(resolve_in_paths("probe-me", &path_var), Some(expected));
    }

    /// A name present in no directory on the search list resolves to `None`.
    #[test]
    fn resolve_returns_none_when_no_directory_has_the_binary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path_var = std::env::join_paths([dir.path()]).expect("join_paths");
        assert_eq!(resolve_in_paths("does-not-exist-anywhere", &path_var), None);
    }
}
