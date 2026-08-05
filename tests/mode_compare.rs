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
        best_energy_milli: Some(best_energy_milli),
        best_spins_hex: None,
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
        requested_reads: 1,
        requested_sweeps: 0,
        requested_anneal_time_us: 0,
        measured_reads: Some(1),
        measured_sweeps: Some(0),
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
    // Labelled mean rows (not a bare number that p50 can also produce).
    // Placement under the correct header is covered by
    // `format_table_places_each_mean_under_the_right_header`.
    assert!(
        table.contains("mean: -2.0000"),
        "A mean row with mean: label: {table}"
    );
    assert!(
        table.contains("mean: 3.0000"),
        "B mean row with mean: label: {table}"
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

// ---------------------------------------------------------------------------
// Mutation-hardening fixtures (Task 14 fix round 1)
// ---------------------------------------------------------------------------

/// Partial-overlap middle case: neither identical nor fully disjoint.
///
/// Energies in natural units (milli / 1000):
/// - A = {1, 2, 3, 4, 5}
/// - B = {2, 3, 4, 5, 6}
///
/// Hand-derived two-sample KS distance (max absolute CDF gap over the
/// pooled step points; n = m = 5):
///
/// ```text
///   x=1: F_A=1/5=0.2, F_B=0/5=0.0  -> |diff|=0.2
///   x=2: F_A=2/5=0.4, F_B=1/5=0.2  -> |diff|=0.2
///   x=3: F_A=3/5=0.6, F_B=2/5=0.4  -> |diff|=0.2
///   x=4: F_A=4/5=0.8, F_B=3/5=0.6  -> |diff|=0.2
///   x=5: F_A=5/5=1.0, F_B=4/5=0.8  -> |diff|=0.2
///   x=6: F_A=5/5=1.0, F_B=5/5=1.0  -> |diff|=0.0
///   max = 0.2 exactly
/// ```
///
/// Means: A = 3.0, B = 4.0. A stub that redefines ks as
/// `|mean_a − mean_b| / (|mean_a| + |mean_b|)` yields 1/7 ≈ 0.1429, not
/// 0.2. A binary stub (0 if equal else 1) yields 1.0. Both fail the exact
/// pin below. The asymptotic p-value for D=0.2, n=m=5 is ~0.9996 (lambda
/// ≈ 0.354; Kolmogorov Q), so a binary p stub (1 if ks==0 else 0) yields
/// 0.0 and fails the lower bound.
#[test]
fn partial_overlap_ks_is_the_hand_derived_intermediate_value() {
    let dir = tempfile::tempdir().expect("temp");
    let path_a = dir.path().join("a.jsonl");
    let path_b = dir.path().join("b.jsonl");
    for (i, milli) in [1_000_i64, 2_000, 3_000, 4_000, 5_000]
        .into_iter()
        .enumerate()
    {
        append_jsonl(&path_a, &record(&format!("a{i}"), milli, true)).expect("write A");
    }
    for (i, milli) in [2_000_i64, 3_000, 4_000, 5_000, 6_000]
        .into_iter()
        .enumerate()
    {
        append_jsonl(&path_b, &record(&format!("b{i}"), milli, true)).expect("write B");
    }

    let report = run_compare(&path_a, &path_b).expect("compare");

    assert!(
        (report.ks - 0.2).abs() < 1e-12,
        "partial overlap must yield ks=0.2 (hand-derived max CDF gap), \
         not a binary 0/1 stub or a mean-difference proxy; got {}",
        report.ks
    );
    // Binary p (0 when ks != 0) fails this. Real asymptotic p ≈ 0.9996.
    assert!(
        report.p_value > 0.99,
        "partial overlap p must stay high for n=m=5, D=0.2 (real ≈0.9996), \
         not a binary 0/1 stub; got {}",
        report.p_value
    );
    // Confirm the mean-diff proxy would have differed: means are 3 and 4.
    assert!((report.a.mean - 3.0).abs() < 1e-12, "mean A");
    assert!((report.b.mean - 4.0).abs() < 1e-12, "mean B");
}

/// `Stats::std` is population std (divide by N). For A = {−1,−2,−3}:
/// mean = −2, variance = ((1)^2 + 0 + (1)^2)/3 = 2/3, std = sqrt(2/3).
/// For B = {1,2,3,4,5}: mean = 3, variance = (4+1+0+1+4)/5 = 2, std = sqrt(2).
#[test]
fn report_std_matches_hand_derived_population_std() {
    let (_dir, path_a, path_b) = fixture_pair();
    let report = run_compare(&path_a, &path_b).expect("compare");

    let expected_a = (2.0_f64 / 3.0).sqrt();
    assert!(
        (report.a.std - expected_a).abs() < 1e-12,
        "std A must be sqrt(2/3)≈{expected_a} (population /N), got {}",
        report.a.std
    );
    let expected_b = 2.0_f64.sqrt();
    assert!(
        (report.b.std - expected_b).abs() < 1e-12,
        "std B must be sqrt(2)≈{expected_b}, got {}",
        report.b.std
    );
}

/// Percentiles use linear interpolation h = q*(n−1) (numpy default / R type 7).
///
/// A sorted = [−3, −2, −1], n=3:
/// - p50: h=0.5*2=1.0 → exact index 1 → −2.0
/// - p90: h=0.9*2=1.8 → lo=1, hi=2, frac=0.8 → −2 + 0.8*(−1−(−2)) = −1.2
/// - p99: h=0.99*2=1.98 → lo=1, hi=2, frac=0.98 → −2 + 0.98 = −1.02
///
/// B sorted = [1, 2, 3, 4, 5], n=5:
/// - p50: h=0.5*4=2.0 → exact index 2 → 3.0
/// - p90: h=0.9*4=3.6 → lo=3, hi=4, frac=0.6 → 4 + 0.6*(5−4) = 4.6
/// - p99: h=0.99*4=3.96 → lo=3, hi=4, frac=0.96 → 4 + 0.96 = 4.96
#[test]
fn report_percentiles_match_hand_derived_passed_energies() {
    let (_dir, path_a, path_b) = fixture_pair();
    let report = run_compare(&path_a, &path_b).expect("compare");

    assert!(
        (report.a.p50 - -2.0).abs() < 1e-12,
        "A p50 must be -2.0, got {}",
        report.a.p50
    );
    assert!(
        (report.a.p90 - -1.2).abs() < 1e-12,
        "A p90 must be -1.2, got {}",
        report.a.p90
    );
    assert!(
        (report.a.p99 - -1.02).abs() < 1e-12,
        "A p99 must be -1.02, got {}",
        report.a.p99
    );
    assert!(
        (report.b.p50 - 3.0).abs() < 1e-12,
        "B p50 must be 3.0, got {}",
        report.b.p50
    );
    assert!(
        (report.b.p90 - 4.6).abs() < 1e-12,
        "B p90 must be 4.6, got {}",
        report.b.p90
    );
    assert!(
        (report.b.p99 - 4.96).abs() < 1e-12,
        "B p99 must be 4.96, got {}",
        report.b.p99
    );
}

/// Table means are anchored under the correct file header, not merely
/// present somewhere. A/B values differ so a swapped stats-block render
/// fails. The `mean:` label is required so a dropped mean row cannot be
/// satisfied by a `p50:` line that happens to share the same number.
#[test]
fn format_table_places_each_mean_under_the_right_header() {
    let (_dir, path_a, path_b) = fixture_pair();
    let report = run_compare(&path_a, &path_b).expect("compare");
    let table = format_comparison_table(&report);

    let a_pos = table
        .find("file A (passed energies")
        .expect("A header must be present");
    let b_pos = table
        .find("file B (passed energies")
        .expect("B header must be present");
    let mean_a = table
        .find("mean: -2.0000")
        .expect("A mean line must be present with the mean: label");
    let mean_b = table
        .find("mean: 3.0000")
        .expect("B mean line must be present with the mean: label");

    assert!(
        a_pos < mean_a && mean_a < b_pos,
        "mean: -2.0000 must sit under file A and before file B:\n{table}"
    );
    assert!(
        b_pos < mean_b,
        "mean: 3.0000 must sit under file B:\n{table}"
    );
}

/// A record with `passed == false` and `rejected == false` must still be
/// excluded. Under the correct `.filter(|r| r.passed)` it is out; under a
/// wrong `.filter(|r| !r.rejected)` it is in. The energy is far from the
/// passed sample so inclusion moves the mean and the count.
#[test]
fn passed_false_rejected_false_is_still_excluded() {
    let dir = tempfile::tempdir().expect("temp");
    let path_a = dir.path().join("a.jsonl");
    let path_b = dir.path().join("b.jsonl");

    append_jsonl(&path_a, &record("ok", 0, true)).expect("write passed A");
    // Non-complementary flags: neither accepted nor rejected.
    let mut odd = record("odd", 1_000_000, false);
    odd.rejected = false;
    append_jsonl(&path_a, &odd).expect("write odd A");

    append_jsonl(&path_b, &record("b0", 0, true)).expect("write B");
    append_jsonl(&path_b, &record("b1", 0, true)).expect("write B");

    let report = run_compare(&path_a, &path_b).expect("compare");
    assert_eq!(
        report.n_a, 1,
        "only passed==true counts; !rejected would also admit the odd record"
    );
    assert!(
        (report.a.mean - 0.0).abs() < 1e-12,
        "odd energy 1000.0 must not enter the mean; got {}",
        report.a.mean
    );
}

/// End-to-end: the built binary maps `--file-a` / `--file-b` to the
/// correct sides of the printed table. A swap in `main`'s match arm would
/// put B's mean under the file A header.
#[test]
fn compare_cli_maps_file_a_and_file_b_to_the_correct_table_sides() {
    let (_dir, path_a, path_b) = fixture_pair();
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_isingmark"));
    let output = std::process::Command::new(&bin)
        .args([
            "compare",
            "--file-a",
            path_a.to_str().expect("utf-8 path A"),
            "--file-b",
            path_b.to_str().expect("utf-8 path B"),
        ])
        .output()
        .expect("spawn isingmark compare");

    assert!(
        output.status.success(),
        "compare CLI must exit 0; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let table = String::from_utf8(output.stdout).expect("utf-8 stdout");

    let a_pos = table
        .find("file A (passed energies")
        .expect("CLI table must name file A");
    let b_pos = table
        .find("file B (passed energies")
        .expect("CLI table must name file B");
    let mean_a = table
        .find("mean: -2.0000")
        .expect("CLI must print A's mean under its label");
    let mean_b = table
        .find("mean: 3.0000")
        .expect("CLI must print B's mean under its label");

    assert!(
        a_pos < mean_a && mean_a < b_pos,
        "--file-a mean must appear under file A (not swapped):\n{table}"
    );
    assert!(
        b_pos < mean_b,
        "--file-b mean must appear under file B (not swapped):\n{table}"
    );
}
