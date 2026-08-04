// SPDX-License-Identifier: AGPL-3.0-or-later
//! Built-in backend registry and `--capabilities`/`--check` probing.
//!
//! Mirrors the CLI surface `quip-miner-core::session::run` dispatches on
//! (`crates/quip-miner-core/src/session.rs` at the upstream rev this crate
//! pins): `--capabilities` prints a JSON document to stdout and exits `0`
//! without opening the device; `--check` opens the device, discards the
//! result, and exits `0` on success or a backend-specific non-zero code
//! (e.g. `EnvIncompatible`) on failure.

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
}

/// Fixed built-in backend list. Order matches display order; `builtin_registry`
/// preserves it.
const BUILTINS: &[BuiltinEntry] = &[
    BuiltinEntry {
        name: "cpu-sa",
        bin_name: "quip-cpu-sa",
        kind: BackendKind::Sweeping,
    },
    BuiltinEntry {
        name: "cpu-gibbs",
        bin_name: "quip-cpu-gibbs",
        kind: BackendKind::Sweeping,
    },
    BuiltinEntry {
        name: "cuda",
        bin_name: "quip-cuda-sa",
        kind: BackendKind::Sweeping,
    },
    BuiltinEntry {
        name: "metal",
        bin_name: "quip-metal-sa",
        kind: BackendKind::Sweeping,
    },
    BuiltinEntry {
        name: "dwave",
        bin_name: "quip-dwave-qa",
        kind: BackendKind::Annealing,
    },
];

/// Build the registry of built-in backends (`cpu-sa`, `cpu-gibbs`, `cuda`,
/// `metal`, `dwave`), resolving each binary from `$PATH`.
///
/// A backend whose binary is not on `$PATH` still gets an entry: `bin` falls
/// back to the bare binary name, so [`probe_capabilities`]/[`probe_check`]
/// still run (and fail with [`ProbeError::NotFound`]) rather than the
/// registry silently omitting the backend. Config-driven overrides build
/// [`BackendSpec`] directly rather than going through this function.
#[must_use]
pub fn builtin_registry() -> Vec<BackendSpec> {
    BUILTINS
        .iter()
        .map(|entry| BackendSpec {
            name: entry.name.to_string(),
            bin: resolve_on_path(entry.bin_name).unwrap_or_else(|| PathBuf::from(entry.bin_name)),
            args: Vec::new(),
            kind: entry.kind,
        })
        .collect()
}

/// Search `$PATH` for an executable file named `bin_name`, returning the
/// first match in `$PATH` order.
fn resolve_on_path(bin_name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    resolve_in_paths(bin_name, &path_var)
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

    let stdout = String::from_utf8_lossy(&output.stdout);
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
    use super::{builtin_registry, BackendKind};

    /// Every built-in name maps to the exact expected binary and kind. Each
    /// name/binary/kind triple is distinct across entries, so a mixed-up
    /// mapping (e.g. `cuda` resolving to `quip-metal-sa`, or `dwave` tagged
    /// `Sweeping`) fails a specific assertion rather than being masked by a
    /// coincidental match.
    #[test]
    fn builtin_registry_has_the_five_expected_entries_in_order() {
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
                ("cuda", "quip-cuda-sa", BackendKind::Sweeping),
                ("metal", "quip-metal-sa", BackendKind::Sweeping),
                ("dwave", "quip-dwave-qa", BackendKind::Annealing),
            ]
        );
    }

    /// The registry only ever produces the five built-in names: an unknown
    /// backend name is absent, not fabricated as a zero-valued or default
    /// entry.
    #[test]
    fn unknown_backend_name_is_absent_from_the_registry() {
        let registry = builtin_registry();
        assert!(registry
            .iter()
            .all(|spec| spec.name != "not-a-real-backend"));
        assert_eq!(registry.len(), 5);
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
