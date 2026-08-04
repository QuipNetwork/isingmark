// SPDX-License-Identifier: AGPL-3.0-or-later
//! Behavioral tests for `isingmark::backend`: registry contents and
//! `--capabilities`/`--check` probing.
//!
//! The real `quip-*` miner binaries are not installed on every machine that
//! runs this suite, so the positive JSON-parse and exit-code-mapping paths
//! are exercised hermetically against a fake miner fixture (a small shell
//! script) in the `fake_miner` module below. Probing the real `quip-cpu-sa`
//! is kept as an opportunistic extra that skips when the binary is absent.

use isingmark::backend::{
    builtin_registry, probe_capabilities, probe_check, BackendSpec, ProbeError,
};
use isingmark::params::BackendKind;

/// A `BackendSpec` pointing at a path with no file at all never spawns
/// anything real, so probing it must fail with `NotFound` rather than
/// panicking (e.g. on an `unwrap`-ed spawn error).
#[test]
fn missing_binary_probe_returns_not_found_and_never_panics() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let spec = BackendSpec {
        name: "nonexistent".to_string(),
        bin: dir.path().join("this-binary-does-not-exist-anywhere"),
        args: Vec::new(),
        kind: BackendKind::Sweeping,
    };

    let capabilities_err =
        probe_capabilities(&spec).expect_err("a nonexistent binary cannot be spawned");
    assert!(matches!(capabilities_err, ProbeError::NotFound(_)));

    let check_err = probe_check(&spec).expect_err("a nonexistent binary cannot be spawned");
    assert!(matches!(check_err, ProbeError::NotFound(_)));
}

/// The built-in registry produces exactly the five documented backends, in
/// order, and fabricates no entry for a name it does not recognize.
#[test]
fn unknown_backend_name_is_not_present_in_the_builtin_registry() {
    let registry = builtin_registry();
    let names: Vec<&str> = registry.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(names, vec!["cpu-sa", "cpu-gibbs", "cuda", "metal", "dwave"]);
    assert!(!names.contains(&"quantum-flux-capacitor"));
}

/// Opportunistic coverage of the real binary: if `quip-cpu-sa` is not
/// installed on this machine (true on most development and CI machines,
/// since none of the five miner binaries ship with this repo), skip rather
/// than fail. The hermetic paths below in `fake_miner` give the JSON-parse
/// and exit-code-mapping logic real coverage regardless of whether this
/// skips.
#[test]
fn quip_cpu_sa_probe_succeeds_when_the_real_binary_is_installed() {
    let registry = builtin_registry();
    let spec = registry
        .into_iter()
        .find(|spec| spec.name == "cpu-sa")
        .expect("builtin_registry always has a cpu-sa entry");

    if !spec.bin.is_file() {
        #[expect(
            clippy::print_stderr,
            reason = "opportunistic test skip notice: quip-cpu-sa is not installed on this machine"
        )]
        {
            eprintln!("skipping quip_cpu_sa_probe_succeeds_when_the_real_binary_is_installed: quip-cpu-sa not found on $PATH");
        }
        return;
    }

    let capabilities =
        probe_capabilities(&spec).expect("--capabilities succeeds for an installed real miner");
    assert!(capabilities.is_object());
    probe_check(&spec).expect("--check succeeds for an installed real miner");
}

/// Hermetic coverage of `probe_capabilities`/`probe_check` against a fake
/// miner fixture: a small, asymmetric shell script that answers
/// `--capabilities` with known JSON and `--check` with a chosen exit code.
/// This runs unconditionally (no real miner required), so the JSON-parse
/// path and the exit-code mapping always get exercised, not just when the
/// real binaries happen to be installed.
#[cfg(unix)]
mod fake_miner {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use isingmark::backend::{probe_capabilities, probe_check, BackendSpec, ProbeError};
    use isingmark::params::BackendKind;

    /// Write an executable shell-script fixture to `dir` that prints a
    /// fixed, asymmetric JSON document on `--capabilities` and exits with
    /// `check_exit_code` on `--check`.
    fn write_fake_miner(dir: &Path, check_exit_code: i32) -> PathBuf {
        let path = dir.join("fake-miner");
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n  --capabilities)\n    printf '%s' '{{\"backend\":\"fake\",\"algorithm\":\"sim\",\"max_nodes\":257,\"max_edges\":31}}'\n    exit 0\n    ;;\n  --check)\n    exit {check_exit_code}\n    ;;\nesac\nexit 1\n"
        );
        std::fs::write(&path, script).expect("write fake miner script");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake miner script executable");
        path
    }

    fn spec_for(bin: PathBuf) -> BackendSpec {
        BackendSpec {
            name: "fake".to_string(),
            bin,
            args: Vec::new(),
            kind: BackendKind::Sweeping,
        }
    }

    /// The four JSON fields are distinct types/values (string, string,
    /// two differently-valued numbers) so a parser that returns an empty
    /// object, a default value, or mixes up fields fails a specific
    /// assertion rather than passing by coincidence.
    #[test]
    fn capabilities_parses_the_exact_json_the_fixture_prints() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let bin = write_fake_miner(dir.path(), 0);

        let value =
            probe_capabilities(&spec_for(bin)).expect("--capabilities succeeds for the fixture");

        assert_eq!(value["backend"], "fake");
        assert_eq!(value["algorithm"], "sim");
        assert_eq!(value["max_nodes"], 257);
        assert_eq!(value["max_edges"], 31);
    }

    #[test]
    fn check_success_returns_ok() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let bin = write_fake_miner(dir.path(), 0);
        probe_check(&spec_for(bin)).expect("--check exit 0 maps to Ok");
    }

    /// `42` is deliberately not `0` (success) and not `1` (the exit code a
    /// broken stub would most plausibly hardcode for "failure"), so this
    /// assertion rules out both a stub that always reports success and one
    /// that always reports a generic failure code.
    #[test]
    fn check_nonzero_exit_maps_to_the_exact_code() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let bin = write_fake_miner(dir.path(), 42);

        let err = probe_check(&spec_for(bin)).expect_err("--check exit 42 is an error");

        assert_eq!(err, ProbeError::CheckFailed { code: 42 });
    }

    #[test]
    fn invalid_json_stdout_is_reported_as_invalid_output() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("garbage-miner");
        std::fs::write(&path, "#!/bin/sh\nprintf 'not json'\nexit 0\n")
            .expect("write garbage fixture");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod garbage fixture executable");

        let err = probe_capabilities(&spec_for(path)).expect_err("garbage stdout is not JSON");

        assert!(matches!(err, ProbeError::InvalidOutput(_)));
    }
}
