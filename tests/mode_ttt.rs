// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for `isingmark::modes::ttt::run_ttt`.
//!
//! Mirrors `tests/mode_sweep.rs`'s pattern: a hermetic run against
//! `isingmark-fake-miner` (via `TttConfig::backend_bin_override`) gives the
//! whole time-to-target loop — target validation, the eager output-dir
//! pre-pass, per-target/per-trial session dispatch, the `<=` reach check,
//! and the `max_time_s` trial budget — real coverage on a machine with no
//! miner binaries installed.
//!
//! `isingmark-fake-miner`'s default behavior (one solution, all `+1` spins,
//! no energy lie) makes every trial's `best_energy_milli` a deterministic
//! function of `(seed, trial index)` alone, independent of which target(s)
//! it is being checked against — exactly what the paired-design and
//! asymmetric-comparison tests below rely on.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use isingmark::modes::ttt::{run_ttt, TttConfig, RECORDS_FILENAME, SUMMARY_FILENAME};
use isingmark::modes::{ModeError, TopologyRef};
use isingmark::record::{read_jsonl, Summary};

fn fake_miner_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner"))
}

/// A `TttConfig` pointed at the hermetic fake miner, with every field set to
/// an explicit, distinctive value — callers override only what a given test
/// cares about via `..base()`.
fn base_config(output_dir: PathBuf) -> TttConfig {
    TttConfig {
        backend: "cpu-sa".to_string(),
        topology: TopologyRef::Preset("smoke".to_string()),
        energy_targets_milli: vec![1_000_000_000],
        num_trials: 3,
        max_time_s: 30.0,
        seed: 42,
        output_dir,
        backend_bin_override: Some(fake_miner_bin()),
        backend_args_override: None,
    }
}

fn target_dir(output_dir: &Path, target_milli: i64) -> PathBuf {
    output_dir.join(format!("target_{target_milli}"))
}

fn read_summary(output_dir: &Path) -> Summary {
    let text = std::fs::read_to_string(output_dir.join(SUMMARY_FILENAME))
        .expect("summary.json is readable");
    serde_json::from_str(&text).expect("summary.json parses as Summary")
}

/// A target so far above any energy the smoke fixture's tiny (8-node,
/// 8-edge, `|h|,|j| <= 1.0` natural) problems can produce that every trial
/// reaches it — used whenever a test needs "always reached" rather than
/// caring about the exact value.
const EASILY_REACHABLE_TARGET_MILLI: i64 = 1_000_000_000;

/// A target so far below any energy the smoke fixture can produce that no
/// trial ever reaches it.
const UNREACHABLE_TARGET_MILLI: i64 = -1_000_000_000;

/// An easily reachable target must yield a time for every trial: `<=`
/// against a huge target is satisfied immediately, so `per_target`'s vector
/// for it must have exactly `num_trials` entries, one per trial, and
/// `records.jsonl` must hold exactly that many records.
#[tokio::test]
async fn hermetic_ttt_easily_reachable_target_yields_a_time_for_every_trial() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = TttConfig {
        num_trials: 4,
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_ttt(&cfg).await.expect("ttt run succeeds");

    let times = report
        .per_target
        .get(&EASILY_REACHABLE_TARGET_MILLI)
        .expect("the requested target has an entry");
    assert_eq!(
        times.len(),
        cfg.num_trials as usize,
        "every trial must reach an easily reachable target"
    );

    let jsonl =
        read_jsonl(&target_dir(dir.path(), EASILY_REACHABLE_TARGET_MILLI).join(RECORDS_FILENAME))
            .expect("records.jsonl is readable");
    assert_eq!(jsonl.records.len(), cfg.num_trials as usize);
    assert!(jsonl.records.iter().all(|r| r.passed));
}

/// An unreachable target must yield an empty vector rather than hanging —
/// the `#[tokio::test]` completing at all within its default timeout is
/// itself part of what this test proves, alongside the explicit assertion
/// that no trial was recorded as having reached it. Every trial is still
/// dispatched and written to `records.jsonl` (this mode does not skip
/// dispatch just because a target looks hard), only none of them satisfy
/// `<=` against it.
#[tokio::test]
async fn hermetic_ttt_unreachable_target_yields_empty_vector_without_hanging() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = TttConfig {
        energy_targets_milli: vec![UNREACHABLE_TARGET_MILLI],
        num_trials: 3,
        ..base_config(dir.path().to_path_buf())
    };

    let report = run_ttt(&cfg).await.expect("ttt run succeeds");

    let times = report
        .per_target
        .get(&UNREACHABLE_TARGET_MILLI)
        .expect("the requested target has an entry, even though nothing reached it");
    assert!(
        times.is_empty(),
        "an unreachable target must map to an empty Vec, got {times:?}"
    );

    let jsonl =
        read_jsonl(&target_dir(dir.path(), UNREACHABLE_TARGET_MILLI).join(RECORDS_FILENAME))
            .expect("records.jsonl is readable");
    assert_eq!(
        jsonl.records.len(),
        cfg.num_trials as usize,
        "every trial must still be attempted and recorded even though none reach the target"
    );

    // The summary must render "not reached" explicitly (a present `null`,
    // not a missing key, `NaN`, or `0.0`), not merely "empty" via the report.
    let summary = read_summary(dir.path());
    let targets = summary.config["targets"]
        .as_array()
        .expect("config carries a targets array");
    let entry = targets
        .iter()
        .find(|t| t["target_milli"].as_i64() == Some(UNREACHABLE_TARGET_MILLI))
        .expect("the unreachable target has its own summary entry");
    assert_eq!(entry["reached"].as_u64(), Some(0));
    assert!(
        entry["median_reached_at_s"].is_null(),
        "an unreached target's median must be an explicit null, got {:?}",
        entry["median_reached_at_s"]
    );
    assert!(entry["p99_reached_at_s"].is_null());
    assert!(entry["times_stats"].is_null());
}

/// Pins the `<=` direction (not `>=`): captures one trial's real,
/// deterministically-derived `best_energy_milli` from a first exploratory
/// run (against a target guaranteed to be reached), then re-runs the exact
/// same `(seed, trial 0)` draw against two targets straddling that value —
/// the captured value itself (must reach under `<=`) and one milli-unit
/// below it (must NOT reach under `<=`, but WOULD incorrectly reach under
/// `>=`, since `energy >= energy - 1` is always true). A mutation that
/// flips the comparison to `>=` makes both targets "reached", so the second
/// assertion is what actually distinguishes the two directions.
#[tokio::test]
async fn hermetic_ttt_uses_less_than_or_equal_not_greater_than_or_equal() {
    let probe_dir = tempfile::tempdir().expect("create temp dir");
    let probe_cfg = TttConfig {
        num_trials: 1,
        ..base_config(probe_dir.path().to_path_buf())
    };
    run_ttt(&probe_cfg).await.expect("probe run succeeds");
    let probe_jsonl = read_jsonl(
        &target_dir(probe_dir.path(), EASILY_REACHABLE_TARGET_MILLI).join(RECORDS_FILENAME),
    )
    .expect("probe records.jsonl is readable");
    let actual_energy_milli = probe_jsonl.records[0].best_energy_milli;

    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = TttConfig {
        energy_targets_milli: vec![actual_energy_milli, actual_energy_milli - 1],
        num_trials: 1,
        ..base_config(dir.path().to_path_buf())
    };
    let report = run_ttt(&cfg).await.expect("ttt run succeeds");

    assert_eq!(
        report.per_target[&actual_energy_milli].len(),
        1,
        "a target exactly equal to the trial's own energy must be reached under <="
    );
    assert!(
        report.per_target[&(actual_energy_milli - 1)].is_empty(),
        "a target one milli-unit below the trial's own energy must NOT be reached under <= \
         (it would incorrectly be reached under >=, since energy >= energy - 1 is always true)"
    );
}

/// Every target must replay the identical `(seed, num_trials)` job
/// sequence — the paired design that makes their reach times comparable at
/// all. Uses three distinct, all-easily-reachable targets (so every trial
/// is attempted in full for every target, with no `max_time_s` cutoff to
/// complicate the comparison) and checks that every target's `records.jsonl`
/// holds the exact same `job_id` set.
#[tokio::test]
async fn hermetic_ttt_every_target_replays_the_identical_job_sequence() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let targets = vec![1_000_000_000, 2_000_000_000, 3_000_000_000];
    let cfg = TttConfig {
        energy_targets_milli: targets.clone(),
        num_trials: 3,
        ..base_config(dir.path().to_path_buf())
    };

    run_ttt(&cfg).await.expect("ttt run succeeds");

    let mut job_id_sets: Vec<HashSet<String>> = Vec::new();
    for &target_milli in &targets {
        let jsonl = read_jsonl(&target_dir(dir.path(), target_milli).join(RECORDS_FILENAME))
            .expect("records.jsonl is readable");
        assert_eq!(jsonl.records.len(), cfg.num_trials as usize);
        job_id_sets.push(jsonl.records.iter().map(|r| r.job_id.clone()).collect());
    }

    let first = &job_id_sets[0];
    assert_eq!(
        first.len(),
        cfg.num_trials as usize,
        "no duplicate ids within one target"
    );
    for (i, set) in job_id_sets.iter().enumerate().skip(1) {
        assert_eq!(
            set, first,
            "target {} (index {i}) must draw the identical job_id set as target 0 (the paired design)",
            targets[i]
        );
    }
}

/// `max_time_s` must be honored within tolerance: a target with far more
/// requested trials than the budget allows must stop early, not run every
/// trial. A single cheap warm-up call absorbs this test binary's one-time
/// first-subprocess-spawn cost (measured separately at several times any
/// later spawn's cost — OS/dynamic-linker caching, not something this mode
/// controls) so the timed call's budget isn't dominated by an unrelated,
/// one-off cost.
#[tokio::test]
async fn hermetic_ttt_max_time_s_is_honored_within_tolerance() {
    let warm_dir = tempfile::tempdir().expect("create temp dir");
    run_ttt(&TttConfig {
        num_trials: 2,
        ..base_config(warm_dir.path().to_path_buf())
    })
    .await
    .expect("warm-up run succeeds");

    let dir = tempfile::tempdir().expect("create temp dir");
    let max_time_s = 0.1;
    let cfg = TttConfig {
        num_trials: 1_000,
        max_time_s,
        ..base_config(dir.path().to_path_buf())
    };

    let start = Instant::now();
    let _report = run_ttt(&cfg).await.expect("ttt run succeeds");
    let elapsed = start.elapsed();

    let jsonl =
        read_jsonl(&target_dir(dir.path(), EASILY_REACHABLE_TARGET_MILLI).join(RECORDS_FILENAME))
            .expect("records.jsonl is readable");
    assert!(
        jsonl.records.len() < cfg.num_trials as usize,
        "a 0.1s budget must not allow all 1000 requested trials to run, got {}",
        jsonl.records.len()
    );
    assert!(
        !jsonl.records.is_empty(),
        "the budget must allow at least one trial to run"
    );
    // Generous upper bound: this mode's own loop only checks the budget
    // *between* trials, so one trial already in flight when the budget is
    // exceeded is allowed to finish (see `modes::ttt`'s module doc); a 5x
    // margin absorbs ordinary scheduler/CI jitter without being loose enough
    // to pass if the budget were ignored outright (which would cost roughly
    // 1000 trials' worth of wall time, far beyond this bound).
    assert!(
        elapsed.as_secs_f64() < max_time_s * 5.0,
        "max_time_s={max_time_s}s must be honored within tolerance, took {elapsed:?}"
    );
}

/// A records file left over from an earlier run must not be silently
/// appended to, for any target — checked for every target before any of
/// them run, so a conflict on a later target cannot leave earlier targets'
/// (real, from-this-run) data sitting next to a run that was refused.
/// Mirrors `tests/mode_sweep.rs`'s identical test for sweep cells.
#[tokio::test]
async fn hermetic_ttt_refuses_to_reuse_an_output_dir_with_existing_target_records() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = base_config(dir.path().to_path_buf());

    run_ttt(&cfg).await.expect("first run succeeds");
    // `backend::RunReport` (transitively, via `record::JobRecord`, which
    // does implement `Debug`) is not embedded in `TttReport`, but
    // `Result::expect_err` still needs the `Ok` type to implement `Debug`;
    // `TttReport` does not derive it (its `Summary` field would need to,
    // and this task does not add that derive), so check the `Result`
    // directly instead, matching `tests/mode_sweep.rs`'s own workaround.
    let second_run = run_ttt(&cfg).await;
    assert!(
        second_run.is_err(),
        "a second run into the same output dir must be refused"
    );
    if let Err(e) = second_run {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }

    let jsonl =
        read_jsonl(&target_dir(dir.path(), EASILY_REACHABLE_TARGET_MILLI).join(RECORDS_FILENAME))
            .expect("first run's records survive");
    assert_eq!(jsonl.records.len(), cfg.num_trials as usize);
}

/// The output-dir conflict check must be eager — checked for *every* target
/// before *any* target is dispatched — not lazy. Seeds a stale records file
/// in only the *second* (out of two) target's directory and nothing in the
/// first target's directory (which does not even exist yet). A lazy
/// implementation would dispatch the first target's real trials before
/// discovering the conflict on the second, leaving one good target sitting
/// next to a refused run with no `summary.json`. The decisive assertion is
/// that the first target's directory was never created at all.
#[tokio::test]
async fn hermetic_ttt_output_dir_conflict_check_is_eager_before_any_target_runs() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = TttConfig {
        energy_targets_milli: vec![1_000_000_000, 2_000_000_000],
        ..base_config(dir.path().to_path_buf())
    };

    let conflicting_dir = target_dir(dir.path(), 2_000_000_000);
    std::fs::create_dir_all(&conflicting_dir).expect("create the conflicting target's dir");
    std::fs::write(conflicting_dir.join(RECORDS_FILENAME), "{\"stale\":true}\n")
        .expect("seed a stale records file in the second target only");

    let result = run_ttt(&cfg).await;
    assert!(
        result.is_err(),
        "a conflict on either target must refuse the whole run"
    );
    if let Err(e) = result {
        assert!(matches!(e, ModeError::RecordsFileExists(_)), "got: {e}");
    }

    let first_target_dir = target_dir(dir.path(), 1_000_000_000);
    assert!(
        !first_target_dir.exists(),
        "the first target must never be created when the second target's conflict is detected eagerly"
    );
}

/// An empty target list must be rejected rather than silently producing a
/// run with no output.
#[tokio::test]
async fn hermetic_ttt_rejects_an_empty_target_list() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = TttConfig {
        energy_targets_milli: vec![],
        ..base_config(dir.path().to_path_buf())
    };

    let result = run_ttt(&cfg).await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e, ModeError::EmptyTargets), "got: {e}");
    }
}

/// Two targets that convert to the same milli-value must be rejected: they
/// would collide on the same output subdirectory, and the second target's
/// `File::create` would silently truncate the first target's records.
#[tokio::test]
async fn hermetic_ttt_rejects_a_duplicate_target() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let cfg = TttConfig {
        energy_targets_milli: vec![1_000_000_000, 2_000_000_000, 1_000_000_000],
        ..base_config(dir.path().to_path_buf())
    };

    let result = run_ttt(&cfg).await;
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(
            matches!(e, ModeError::DuplicateTarget(1_000_000_000)),
            "got: {e}"
        );
    }
    assert!(
        std::fs::read_dir(dir.path()).map_or(true, |mut entries| entries.next().is_none()),
        "a rejected target list must not create any target output"
    );
}
