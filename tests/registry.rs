// SPDX-License-Identifier: AGPL-3.0-or-later
//! Behavioral tests for `isingmark::backend`: registry contents and
//! `--capabilities`/`--check` probing.
//!
//! The real `quip-*` miner binaries are not installed on every machine that
//! runs this suite, so the positive JSON-parse and exit-code-mapping paths
//! are exercised hermetically against a fake miner fixture (a small shell
//! script) in the `fake_miner` module below.
//!
//! `quip_cpu_sa_probe_succeeds_when_the_real_binary_is_installed` also drives
//! the real `quip-cpu-sa`, and fails by default if it is missing — this
//! project's tests do not silently skip real-hardware coverage. Set
//! `ISINGMARK_ALLOW_MISSING_MINERS=1` to opt out on a machine that does not
//! have the miner binaries installed (true of most development and CI
//! machines today).

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
/// order — no more, no fewer, and none renamed.
#[test]
fn builtin_registry_contains_exactly_the_five_documented_backends_in_order() {
    let registry = builtin_registry();
    let names: Vec<&str> = registry.iter().map(|spec| spec.name.as_str()).collect();
    assert_eq!(names, vec!["cpu-sa", "cpu-gibbs", "cuda", "metal", "dwave"]);
}

/// Environment variable that opts out of the real-binary requirement below.
/// Documented here (rather than only in a comment on the test) so a reader
/// who hits the failure this test produces on a machine without the miner
/// binaries installed can find the escape hatch without reading the plan.
const ALLOW_MISSING_MINERS_VAR: &str = "ISINGMARK_ALLOW_MISSING_MINERS";

/// Coverage of the real binary: `quip-cpu-sa` must be installed and pass
/// both probes by default. A missing binary FAILS this test rather than
/// skipping it — a silent skip would report a green `... ok` for a test that
/// asserted nothing, since libtest captures the output of passing tests.
/// Set [`ALLOW_MISSING_MINERS_VAR`] to opt out on a machine that does not
/// have the miner binaries installed (true of most development and CI
/// machines today; the hermetic paths in `fake_miner` give the JSON-parse
/// and exit-code-mapping logic real coverage regardless of this variable).
#[test]
fn quip_cpu_sa_probe_succeeds_when_the_real_binary_is_installed() {
    let registry = builtin_registry();
    let spec = registry
        .into_iter()
        .find(|spec| spec.name == "cpu-sa")
        .expect("builtin_registry always has a cpu-sa entry");

    if !spec.bin.is_file() {
        assert!(
            std::env::var_os(ALLOW_MISSING_MINERS_VAR).is_some(),
            "quip-cpu-sa was not found on $PATH (looked for {}). This test \
             requires the real miner binary by default; set \
             {ALLOW_MISSING_MINERS_VAR}=1 to opt out on a machine without \
             the miner binaries installed.",
            spec.bin.display()
        );
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
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::thread;
    use std::time::Duration;

    use isingmark::backend::{probe_capabilities, probe_check, BackendSpec, ProbeError};
    use isingmark::params::BackendKind;

    /// Write `script` to `path`, mark it mode `0755`, and confirm it can be
    /// exec'd before returning.
    ///
    /// ## Why confirm with a spawn (and why a bounded ETXTBSY retry)
    ///
    /// These fixtures are shell scripts written and then executed in the same
    /// process as many other test threads. On Linux, a sibling thread can
    /// `fork` while the parent still holds a write handle to the just-written
    /// file; the child inherits that descriptor, and `exec` of a file that is
    /// open for writing returns `ETXTBSY` (`ErrorKind::ExecutableFileBusy`).
    /// `probe_capabilities` then surfaces a spawn error instead of the
    /// fixture's intended stdout/exit behaviour, and the test's variant
    /// assertion flakes.
    ///
    /// Measured in the CI image (`rust:1.97.1-bookworm`): six sibling threads
    /// calling `Command::output()` in a loop while the main thread wrote,
    /// chmod'd, and exec'd a fixture produced **`ExecutableFileBusy` 2 times
    /// out of 300**. macOS does not exhibit this race (the same concurrent
    /// reproduction does not fail there), which is why the flake only appears
    /// on Linux CI.
    ///
    /// Product code never writes an executable and then execs it. This is
    /// pure fixture setup, so a bounded retry here is acceptable. The retry
    /// stays in fixture setup — not around `probe_capabilities` /
    /// `probe_check` — so each test still makes exactly one probe call and a
    /// genuine spawn failure from the code under test still fails the test.
    ///
    /// A fixture that never becomes executable is a real failure: the bound
    /// is finite and small, and exhausting it panics with a clear message.
    fn write_executable_script(path: &Path, script: impl AsRef<[u8]>) {
        std::fs::write(path, script).expect("write fixture script");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fixture script executable");
        // Fail here (not at the test's probe call) so a genuine ETXTBSY from
        // the code under test is still a hard test failure.
        try_confirm_executable(path).expect(
            "fixture still not executable after write+chmod; \
             see write_executable_script docs (Linux ETXTBSY write-handle/fork race)",
        );
    }

    /// Bounded spawn probe used only by [`write_executable_script`].
    ///
    /// Finite and small: a few hundred milliseconds worst case. The
    /// write-handle window is short; once inherited FDs close, the next
    /// attempt succeeds. Infinite retry would mask a permanently broken
    /// fixture.
    fn try_confirm_executable(path: &Path) -> std::io::Result<()> {
        const MAX_ATTEMPTS: u32 = 32;
        const RETRY_SLEEP: Duration = Duration::from_millis(5);

        let mut last_busy: Option<std::io::Error> = None;
        for _ in 0..MAX_ATTEMPTS {
            match Command::new(path).output() {
                // Spawn succeeded (exit status is irrelevant; the script may
                // exit non-zero when invoked with no probe args).
                Ok(_) => return Ok(()),
                Err(err) if err.kind() == ErrorKind::ExecutableFileBusy => {
                    last_busy = Some(err);
                    thread::sleep(RETRY_SLEEP);
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_busy.unwrap_or_else(|| {
            std::io::Error::new(
                ErrorKind::ExecutableFileBusy,
                "exhausted ETXTBSY retries without capturing an error",
            )
        }))
    }

    /// Write an executable shell-script fixture to `dir` that prints a
    /// fixed, asymmetric JSON document on `--capabilities` and exits with
    /// `check_exit_code` on `--check`.
    fn write_fake_miner(dir: &Path, check_exit_code: i32) -> PathBuf {
        let path = dir.join("fake-miner");
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\n  --capabilities)\n    printf '%s' '{{\"backend\":\"fake\",\"algorithm\":\"sim\",\"max_nodes\":257,\"max_edges\":31}}'\n    exit 0\n    ;;\n  --check)\n    exit {check_exit_code}\n    ;;\nesac\nexit 1\n"
        );
        write_executable_script(&path, script);
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

    /// Extract the message from a [`ProbeError::InvalidOutput`], or `None`
    /// for any other variant. Lets tests assert on the message text without
    /// a `match` arm that would need a denied `panic!`/`unreachable!` for
    /// the impossible case.
    fn invalid_output_message(err: &ProbeError) -> Option<&str> {
        match err {
            ProbeError::InvalidOutput(message) => Some(message.as_str()),
            ProbeError::NotFound(_) | ProbeError::CheckFailed { .. } => None,
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
        write_executable_script(&path, "#!/bin/sh\nprintf 'not json'\nexit 0\n");

        let err = probe_capabilities(&spec_for(path)).expect_err("garbage stdout is not JSON");

        assert!(matches!(err, ProbeError::InvalidOutput(_)));
    }

    /// Invalid UTF-8 stdout is its own documented `InvalidOutput` case
    /// (`registry.rs:46-47`, `:170-171`), distinct from a valid-UTF-8,
    /// invalid-JSON payload. `\xFF\xFE` is not valid UTF-8 under any
    /// interpretation (0xFF is not a valid UTF-8 leading or continuation
    /// byte), so this cannot be satisfied by a lossy-replacement decode
    /// that then happens to fail JSON parsing for an unrelated reason.
    #[test]
    fn invalid_utf8_stdout_is_reported_as_invalid_output_mentioning_utf8() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("non-utf8-miner");
        write_executable_script(&path, "#!/bin/sh\nprintf '\\377\\376'\nexit 0\n");

        let err =
            probe_capabilities(&spec_for(path)).expect_err("invalid UTF-8 stdout is an error");

        let message = invalid_output_message(&err)
            .expect("invalid UTF-8 stdout is reported as InvalidOutput");
        assert!(
            message.contains("UTF-8"),
            "message does not mention UTF-8: {message}"
        );
    }

    /// A non-zero `--capabilities` exit (a version-mismatched binary, a
    /// wrapper script, or a backend whose runtime is missing) must not
    /// discard the process's own diagnosis: the error message carries both
    /// the exit code and the stderr text, not just a generic JSON-parse
    /// failure from empty stdout.
    #[test]
    fn capabilities_nonzero_exit_reports_the_exit_code_and_stderr_reason() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("crashing-miner");
        write_executable_script(
            &path,
            "#!/bin/sh\n>&2 printf 'cuda runtime not found'\nexit 3\n",
        );

        let err = probe_capabilities(&spec_for(path)).expect_err("nonzero exit is an error");

        let message =
            invalid_output_message(&err).expect("nonzero --capabilities exit is InvalidOutput");
        assert!(
            message.contains('3'),
            "message drops the exit code: {message}"
        );
        assert!(
            message.contains("cuda runtime not found"),
            "message drops the stderr reason: {message}"
        );
    }

    /// `BackendSpec::args` must be forwarded on every invocation, and land
    /// *before* the probe flag (documented at `registry.rs:27-28`). The
    /// fixture only succeeds when `$1` is exactly `--extra`; if forwarding
    /// is dropped, `$1` becomes the probe flag itself (`--capabilities` or
    /// `--check`) instead, which does not match, so the fixture exits
    /// non-zero and both probes fail.
    #[test]
    fn extra_args_are_forwarded_before_the_probe_flag() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let path = dir.path().join("args-checking-miner");
        let script = "#!/bin/sh\nif [ \"$1\" = \"--extra\" ]; then\n  case \"$2\" in\n    --capabilities) printf '%s' '{\"ok\":true}'; exit 0 ;;\n    --check) exit 0 ;;\n  esac\nfi\nexit 9\n";
        write_executable_script(&path, script);

        let spec = BackendSpec {
            name: "fake".to_string(),
            bin: path,
            args: vec!["--extra".to_string()],
            kind: BackendKind::Sweeping,
        };

        probe_capabilities(&spec).expect("--extra forwarded before --capabilities");
        probe_check(&spec).expect("--extra forwarded before --check");
    }
}
