// SPDX-License-Identifier: AGPL-3.0-or-later
//! Integration tests for `isingmark::modes::compare::run_compare`.
//!
//! Compare is pure post-hoc analysis of two JSONL files on disk — no
//! backend is spawned. Every assertion below is designed to kill a
//! specific mutation class this project's earlier tasks failed review
//! for: constant `ks`/`p_value`, swapped `n_a`/`n_b`, dropped `passed`
//! filter, hardcoded-zero skipped counts, and `is_err()`-only error
//! checks.
//!
//! # Fixture design
//!
//! File A and file B are deliberately asymmetric in both record count
//! and energy distribution:
//!
//! - A: three `passed` energies at milli `-1000, -2000, -3000`
//!   → natural units `-1.0, -2.0, -3.0` → mean `-2.0`. Also carries one
//!   `passed: false` record at milli `1_000_000` (natural `1000.0`) and
//!   one corrupt JSONL line; if either is wrongly included the mean and
//!   sample counts move far enough that every mean/`n_a` assertion fails.
//! - B: five `passed` energies at milli `1000, 2000, 3000, 4000, 5000`
//!   → natural units `1.0..5.0` → mean `3.0`. Completely disjoint from
//!   A's range, so `ks_distance` is exactly `1.0` (see
//!   `stats::ks_distance`'s own `..._of_disjoint_samples_is_exactly_one`
//!   test) and the asymptotic p-value is tiny (~3.8e-3; hand-derived in
//!   that same module for n=m=5, and smaller here only in the effective-n
//!   sense — still well under 0.05).
//!
//! Expected numbers are hand-written or independently derived; nothing
//! under test is used to compute an expected value.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use isingmark::modes::compare::{format_comparison_table, run_compare, CompareReport};
use isingmark::modes::ModeError;
use isingmark::record::{append_jsonl, JobRecord};

/// A `JobRecord` with only the fields a given test cares about filled in.
/// Every unused field is a stable zero/false so fixtures stay short.
fn record(job_id: &str, best_energy_milli: i64, passed: bool) -> JobRecord {
    JobRecord {
        job_id: job_id.to_string(),
        backend: "cpu-sa".to_string(),
        n_solutions: 1,
        best_energy_milli,
        diversity_milli: 0,
        passed,
        rejected: !passed,
        reject_reason: if passed {
            None
        } else {
            Some("fixture_reject".to_string())
        },
        device_access_time_us: 0,
        wall_ms: 0,
        reads: 1,
        sweeps: 0,
        anneal_time_us: 0,
    }
}

/// File A: 3 passed energies at -1/-2/-3 natural units, one failed record
/// with energy 1000 (would dominate the mean if wrongly included), and
/// one corrupt line (`skipped_lines = 1`).
fn write_file_a(path: &Path) {
    for (i, milli) in [-1_000_i64, -2_000, -3_000].into_iter().enumerate() {
        append_jsonl(path, &record(&format!("a{i}"), milli, true)).expect("write passed A");
    }
    // Failed record whose energy would move the mean from -2.0 to
    // (-1-2-3+1000)/4 = 248.5 if the passed filter were removed.
    append_jsonl(path, &record("a_fail", 1_000_000, false)).expect("write failed A");
    // Genuinely corrupt line: not blank, not valid JSON for JobRecord.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open A for corrupt append");
    writeln!(file, "{{this is not valid json").expect("write corrupt line");
}

/// File B: 5 passed energies at 1..5 natural units. Different count and
/// completely disjoint from A, so a swapped-argument bug that populates
/// `n_a` from B (or vice versa) fails every asymmetric assertion, and a
/// constant-`ks = 0.0` stub fails the large-distance assertion.
fn write_file_b(path: &Path) {
    for (i, milli) in [1_000_i64, 2_000, 3_000, 4_000, 5_000]
        .into_iter()
        .enumerate()
    {
        append_jsonl(path, &record(&format!("b{i}"), milli, true)).expect("write passed B");
    }
}

fn fixture_pair() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path_a = dir.path().join("a.jsonl");
    let path_b = dir.path().join("b.jsonl");
    // Touch A empty first so append_jsonl has a place to land; write_file_a
    // creates via append_jsonl's create(true).
    write_file_a(&path_a);
    write_file_b(&path_b);
    (dir, path_a, path_b)
}

/// Comparing a file with itself: identical empirical CDFs → `ks == 0.0`
/// and a p-value near 1.0. Alone this would be satisfied by a stub that
/// always returns `ks = 0.0`; the disjoint-files test below is the other
/// bound.
#[test]
fn comparing_a_file_with_itself_gives_ks_zero_and_high_p() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("same.jsonl");
    write_file_a(&path);

    let report = run_compare(&path, &path).expect("self-compare succeeds");

    assert!(
        (report.ks - 0.0).abs() < 1e-12,
        "identical distributions must have ks exactly 0.0, got {}",
        report.ks
    );
    assert!(
        report.p_value > 0.99,
        "identical distributions must have p near 1.0, got {}",
        report.p_value
    );
    // n_a == n_b for a self-compare (both sides read the same file), but
    // the sample count must still be the *passed* count (3), not total
    // records (4 = 3 passed + 1 failed) and not including the corrupt line.
    assert_eq!(report.n_a, 3, "self-compare n_a must count only passed");
    assert_eq!(report.n_b, 3, "self-compare n_b must count only passed");
    assert_eq!(
        report.skipped_a, 1,
        "corrupt line must be counted as skipped"
    );
    assert_eq!(
        report.skipped_b, 1,
        "corrupt line must be counted as skipped"
    );
    // Hand-derived mean of [-1.0, -2.0, -3.0] = -2.0.
    assert!(
        (report.a.mean - -2.0).abs() < 1e-12,
        "mean of passed A energies must be -2.0, got {}",
        report.a.mean
    );
}

/// Asymmetric, completely disjoint files: `ks` is large (exactly 1.0 for
/// fully separated samples), `p` is tiny, and `n_a`/`n_b`/`skipped_*`
/// match the asymmetric fixture. Kills constant-`ks = 0.0`,
/// constant-`ks = 1.0` is *not* killed by the ks assertion alone (the
/// true value is 1.0) — the self-compare test above is the upper-bound
/// partner; together they bracket. The `n_a`/`n_b` assertions here kill
/// a swapped-argument population of those fields.
#[test]
fn comparing_disjoint_asymmetric_files_gives_large_ks_and_tiny_p() {
    let (_dir, path_a, path_b) = fixture_pair();

    let report = run_compare(&path_a, &path_b).expect("compare succeeds");

    // Fully disjoint ranges → KS distance is exactly 1.0 (see
    // stats::ks_distance's own unit test on this property).
    assert!(
        (report.ks - 1.0).abs() < 1e-12,
        "disjoint distributions must have ks exactly 1.0, got {}",
        report.ks
    );
    assert!(
        report.p_value < 0.05,
        "disjoint distributions must have a small p_value, got {}",
        report.p_value
    );
    // Asymmetric counts: A has 3 passed, B has 5. A swap of n_a/n_b fails.
    assert_eq!(report.n_a, 3, "n_a must come from file A (3 passed)");
    assert_eq!(report.n_b, 5, "n_b must come from file B (5 passed)");
    // Only A carries a corrupt line.
    assert_eq!(report.skipped_a, 1, "skipped_a must count A's corrupt line");
    assert_eq!(report.skipped_b, 0, "skipped_b must be 0 for clean file B");

    // Hand-derived means. The failed A record at 1000.0 is excluded.
    assert!(
        (report.a.mean - -2.0).abs() < 1e-12,
        "mean A must be -2.0 (passed only); including the failed 1000.0 \
         record would yield 248.5. got {}",
        report.a.mean
    );
    // Hand-derived: (1+2+3+4+5)/5 = 3.0.
    assert!(
        (report.b.mean - 3.0).abs() < 1e-12,
        "mean B must be 3.0, got {}",
        report.b.mean
    );
    // min/max also hand-derived, bracket the distributions from both ends.
    assert!((report.a.min - -3.0).abs() < 1e-12);
    assert!((report.a.max - -1.0).abs() < 1e-12);
    assert!((report.b.min - 1.0).abs() < 1e-12);
    assert!((report.b.max - 5.0).abs() < 1e-12);
}

/// The `passed` filter: a file whose only extreme energy is on a
/// `passed: false` record must report the mean of the *passed* subset.
/// Standalone of the mean check inside the disjoint test so a regression
/// that only re-runs a subset of tests still catches a filter drop.
#[test]
fn passed_filter_excludes_failed_records_from_the_mean() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path_a = dir.path().join("a.jsonl");
    let path_b = dir.path().join("b.jsonl");

    // Two passed at 0 and 2 → mean 1.0. One failed at 10_000 milli
    // (natural 10.0) — if included, mean becomes (0+2+10)/3 ≈ 4.0.
    append_jsonl(&path_a, &record("p0", 0, true)).expect("write");
    append_jsonl(&path_a, &record("p1", 2_000, true)).expect("write");
    append_jsonl(&path_a, &record("fail", 10_000, false)).expect("write");

    // B is a clean single-point distribution so describe/ks succeed.
    append_jsonl(&path_b, &record("b0", 1_000, true)).expect("write");
    append_jsonl(&path_b, &record("b1", 1_000, true)).expect("write");

    let report = run_compare(&path_a, &path_b).expect("compare succeeds");
    assert_eq!(report.n_a, 2, "failed record must not count toward n_a");
    assert!(
        (report.a.mean - 1.0).abs() < 1e-12,
        "mean must exclude the failed record's energy; got {}",
        report.a.mean
    );
}

/// A corrupt line written into the fixture reaches `CompareReport.skipped_*`.
#[test]
fn skipped_lines_from_corrupt_input_reach_the_report() {
    let (_dir, path_a, path_b) = fixture_pair();
    // file A has one corrupt line by construction; also poison B.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path_b)
        .expect("open B");
    writeln!(file, "not-json-at-all").expect("corrupt B");
    writeln!(file, "{{also: broken").expect("corrupt B again");

    let report = run_compare(&path_a, &path_b).expect("compare succeeds");
    assert_eq!(report.skipped_a, 1);
    assert_eq!(report.skipped_b, 2);
    // Passed counts unchanged by the corrupt lines.
    assert_eq!(report.n_a, 3);
    assert_eq!(report.n_b, 5);
}

/// A nonexistent path returns `CompareFileUnreadable`, not a panic and not
/// a generic `Io`/`EmptyCompareFile`.
#[test]
fn nonexistent_file_returns_compare_file_unreadable() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let missing = dir.path().join("does-not-exist.jsonl");
    let present = dir.path().join("present.jsonl");
    append_jsonl(&present, &record("ok", 0, true)).expect("write present");

    let err = run_compare(&missing, &present).expect_err("missing A must err");
    assert!(
        matches!(err, ModeError::CompareFileUnreadable(ref p, _) if p == &missing),
        "expected CompareFileUnreadable for missing A, got: {err}"
    );

    let err = run_compare(&present, &missing).expect_err("missing B must err");
    assert!(
        matches!(err, ModeError::CompareFileUnreadable(ref p, _) if p == &missing),
        "expected CompareFileUnreadable for missing B, got: {err}"
    );
}

/// A file whose every record has `passed: false` returns
/// `EmptyCompareFile` for that side.
#[test]
fn file_of_only_failed_records_returns_empty_compare_file() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let empty_passed = dir.path().join("empty.jsonl");
    let good = dir.path().join("good.jsonl");
    append_jsonl(&empty_passed, &record("fail0", -5_000, false)).expect("write");
    append_jsonl(&empty_passed, &record("fail1", 5_000, false)).expect("write");
    append_jsonl(&good, &record("ok", 0, true)).expect("write");

    let err = run_compare(&empty_passed, &good).expect_err("no passed A must err");
    assert!(
        matches!(err, ModeError::EmptyCompareFile(ref p) if p == &empty_passed),
        "expected EmptyCompareFile for A, got: {err}"
    );

    let err = run_compare(&good, &empty_passed).expect_err("no passed B must err");
    assert!(
        matches!(err, ModeError::EmptyCompareFile(ref p) if p == &empty_passed),
        "expected EmptyCompareFile for B, got: {err}"
    );
}

/// An empty (zero-byte) file is also `EmptyCompareFile`.
#[test]
fn empty_file_returns_empty_compare_file() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let empty = dir.path().join("empty.jsonl");
    let good = dir.path().join("good.jsonl");
    std::fs::File::create(&empty).expect("create empty");
    append_jsonl(&good, &record("ok", 0, true)).expect("write");

    let err = run_compare(&empty, &good).expect_err("empty A must err");
    assert!(
        matches!(err, ModeError::EmptyCompareFile(ref p) if p == &empty),
        "expected EmptyCompareFile for empty A, got: {err}"
    );
}

/// The rendered table carries the two sample counts and both skipped
/// counts with values a wrong-field bug would change. Not merely
/// "non-empty."
#[test]
fn format_comparison_table_carries_asymmetric_counts() {
    let (_dir, path_a, path_b) = fixture_pair();
    let report = run_compare(&path_a, &path_b).expect("compare succeeds");
    let table = format_comparison_table(&report);

    assert!(
        table.contains("a=3 passed"),
        "table must show n_a=3: {table}"
    );
    assert!(
        table.contains("b=5 passed"),
        "table must show n_b=5: {table}"
    );
    assert!(
        table.contains("a=1 lines"),
        "table must show skipped_a=1: {table}"
    );
    assert!(
        table.contains("b=0 lines"),
        "table must show skipped_b=0: {table}"
    );
    assert!(
        table.contains("failed/rejected records excluded"),
        "table must state the passed-only filter: {table}"
    );
    // Means appear so a table that only prints counts still fails if the
    // stats block is dropped.
    assert!(
        table.contains("-2.0000") || table.contains("mean: -2"),
        "A mean: {table}"
    );
    assert!(
        table.contains("3.0000") || table.contains("mean: 3"),
        "B mean: {table}"
    );
}

/// Sanity: `CompareReport` fields are all populated and consistent with
/// each other (`n_a` drives a's sample, etc.).
#[test]
fn report_fields_are_internally_consistent() {
    let (_dir, path_a, path_b) = fixture_pair();
    let report: CompareReport = run_compare(&path_a, &path_b).expect("compare succeeds");
    assert!(report.n_a > 0 && report.n_b > 0);
    assert!(report.n_a != report.n_b, "fixture must stay asymmetric");
    assert!(report.a.min <= report.a.mean && report.a.mean <= report.a.max);
    assert!(report.b.min <= report.b.mean && report.b.mean <= report.b.max);
    assert!((0.0..=1.0).contains(&report.ks));
    assert!((0.0..=1.0).contains(&report.p_value));
}
