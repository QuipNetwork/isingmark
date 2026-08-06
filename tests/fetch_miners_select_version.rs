// SPDX-License-Identifier: AGPL-3.0-or-later
//! Network-free checks of `tools/fetch-miners.sh --select-version`.
//!
//! Shells out to the script located via `CARGO_MANIFEST_DIR` so the working
//! directory of the test runner does not matter.

use std::process::Command;

fn script_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/fetch-miners.sh")
}

fn run_select(allow_prerelease: &str, candidates: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("bash");
    cmd.arg(script_path())
        .arg("--select-version")
        .args(candidates)
        .env("MINERS_ALLOW_PRERELEASE", allow_prerelease)
        // Clear any inherited tag so the script path under test is pure.
        .env_remove("MINERS_TAG");
    cmd.output().expect("failed to spawn fetch-miners.sh")
}

fn assert_selects(allow: &str, candidates: &[&str], expected: &str) {
    let out = run_select(allow, candidates);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "expected success for candidates {candidates:?} allow={allow}; status={}; stdout={stdout}; stderr={stderr}",
        out.status
    );
    let got = stdout.trim();
    assert_eq!(
        got, expected,
        "candidates {candidates:?} allow={allow}: stderr={stderr}"
    );
}

fn assert_fails(allow: &str, candidates: &[&str]) {
    let out = run_select(allow, candidates);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "expected non-zero for candidates {candidates:?} allow={allow}; stdout={stdout}; stderr={stderr}"
    );
    assert!(
        stderr.contains("no qualifying version"),
        "stderr should explain failure; got: {stderr}"
    );
}

#[test]
fn select_version_prefers_stable_over_prerelease_when_both_allowed() {
    assert_selects("1", &["v0.3.0", "v0.3.0-rc1"], "v0.3.0");
}

#[test]
fn select_version_stable_only_still_picks_stable_when_present() {
    assert_selects("0", &["v0.3.0", "v0.3.0-rc1"], "v0.3.0");
}

#[test]
fn select_version_prerelease_alone_allowed() {
    assert_selects("1", &["v0.3.0-rc1"], "v0.3.0-rc1");
}

#[test]
fn select_version_prerelease_alone_rejected_when_stable_only() {
    assert_fails("0", &["v0.3.0-rc1"]);
}

#[test]
fn select_version_numeric_prerelease_suffix_not_lexical() {
    // Lexical ASCII would rank rc9 above rc54; numeric suffix compare does not.
    assert_selects("1", &["v0.2.1-rc9", "v0.2.1-rc54"], "v0.2.1-rc54");
}

#[test]
fn select_version_minor_is_numeric_not_lexical() {
    // Lexical would rank v0.4.0 above v0.10.0; numeric minor does not.
    assert_selects("1", &["v0.4.0", "v0.10.0"], "v0.10.0");
}

#[test]
fn select_version_release_outranks_prerelease_on_same_core() {
    assert_selects("1", &["v0.4.0-rc2", "v0.4.0"], "v0.4.0");
}

// A prerelease on a higher core version outranks a lower stable version; a
// stable-first implementation (all stables above all prereleases) gets this wrong.
#[test]
fn select_version_higher_core_prerelease_beats_lower_stable() {
    assert_selects("1", &["v0.3.1-rc1", "v0.3.0"], "v0.3.1-rc1");
}

#[test]
fn select_version_higher_core_prerelease_beats_lower_stable_arg_order() {
    // Same pair as above, reversed arguments: catches first-or-last-wins bugs.
    assert_selects("1", &["v0.3.0", "v0.3.1-rc1"], "v0.3.1-rc1");
}

#[test]
fn select_version_stable_only_excludes_prerelease_not_demotes() {
    // MINERS_ALLOW_PRERELEASE=0 removes prereleases from candidacy; it does not
    // keep them ranked below stables. Here only v0.3.0 qualifies.
    assert_selects("0", &["v0.3.1-rc1", "v0.3.0"], "v0.3.0");
}

#[test]
fn select_version_same_core_stable_still_beats_prerelease_when_allowed() {
    assert_selects("1", &["v0.3.1-rc1", "v0.3.1"], "v0.3.1");
}

#[test]
fn select_version_major_prerelease_beats_older_stable() {
    assert_selects("1", &["v1.0.0-rc1", "v0.9.9"], "v1.0.0-rc1");
}

#[test]
fn select_version_prerelease_suffix_numeric_across_double_digits() {
    assert_selects("1", &["v0.3.0-rc2", "v0.3.0-rc10"], "v0.3.0-rc10");
}

#[test]
fn select_version_mixed_set_picks_highest_core_prerelease() {
    assert_selects(
        "1",
        &["v0.3.0-rc1", "v0.3.0", "v0.3.1-rc1", "v0.2.9"],
        "v0.3.1-rc1",
    );
}
