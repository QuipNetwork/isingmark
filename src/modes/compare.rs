// SPDX-License-Identifier: AGPL-3.0-or-later
//! Compare mode: post-hoc two-sample comparison of two completed benchmark
//! runs' JSONL record files.
//!
//! Unlike every other mode, compare spawns no backend and writes no output
//! files. It reads `--file-a` and `--file-b`, extracts the
//! `passed == true` records' `best_energy_milli` values (converted to
//! natural units), summarises each side with `stats::describe`, and reports
//! the two-sample Kolmogorov-Smirnov distance and asymptotic p-value.
//!
//! # `passed`-only energy extraction
//!
//! Only records with `passed == true` enter the comparison — the same
//! convention every other mode uses when it feeds energies into
//! `stats::describe`/`stats::ks_distance` (see `sweep`, `comparative`,
//! `reproducibility`). Failed/rejected jobs either have no meaningful
//! energy or would otherwise pull a distribution that is not the one the
//! run claimed to have measured. The rendered table states the passed-only
//! sample counts explicitly so a low `n` is not mistaken for "few jobs ran"
//! when it may mean "many failed."
//!
//! # Empty or unreadable input is an error
//!
//! [`CompareReport`]'s `a`/`b` fields are bare [`Stats`], not `Option`, so
//! there is no honest way to represent "no usable samples." An unreadable
//! path returns [`ModeError::CompareFileUnreadable`]; a file that parses
//! but yields no `passed` records (or for which `describe`/`ks_distance`/
//! `ks_two_sample_p` returns `None`) returns
//! [`ModeError::EmptyCompareFile`]. Fabricating a default `p_value` (e.g.
//! `1.0`) would silently claim "the two files are identical" for data that
//! was never compared — the failure mode
//! [`ModeError::EmptyReproBatch`] already documents and refuses.

use std::fmt::Write as _;
use std::path::Path;

use crate::modes::{milli_to_f64, ModeError};
use crate::record::{read_jsonl, JobRecord};
use crate::stats::{describe, ks_distance, ks_two_sample_p, Stats};

/// Outcome of [`run_compare`]: descriptive stats for each file's `passed`
/// energy distribution, the two-sample KS distance and asymptotic p-value,
/// plus the sample sizes and skipped-line counts that make those numbers
/// interpretable.
///
/// A KS distance without sample sizes is uninterpretable (`ks = 0.3` at
/// `n = 5` and at `n = 5000` mean opposite things), and a silently-dropped
/// corrupt line would let a file whose records were 90% unparseable report
/// confident statistics over the surviving 10%. Both counts are therefore
/// first-class fields on the report, not optional table footnotes.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareReport {
    /// Descriptive stats over file A's `passed` energies (natural units).
    pub a: Stats,
    /// Descriptive stats over file B's `passed` energies (natural units).
    pub b: Stats,
    /// Two-sample Kolmogorov-Smirnov statistic (`stats::ks_distance`).
    /// `0.0` for identical distributions, up to `1.0` for completely
    /// disjoint ones.
    pub ks: f64,
    /// Asymptotic two-sided KS p-value (`stats::ks_two_sample_p`).
    pub p_value: f64,
    /// Number of `passed` records compared from file A.
    pub n_a: usize,
    /// Number of `passed` records compared from file B.
    pub n_b: usize,
    /// Lines [`read_jsonl`] skipped as unparseable in file A.
    pub skipped_a: usize,
    /// Lines [`read_jsonl`] skipped as unparseable in file B.
    pub skipped_b: usize,
}

/// Read two JSONL record files and compare their `passed` energy
/// distributions via a two-sample Kolmogorov-Smirnov test. Pure post-hoc
/// analysis: no backend is spawned and nothing is written to disk.
///
/// # Errors
///
/// Returns [`ModeError::CompareFileUnreadable`] if either path cannot be
/// opened or read, and [`ModeError::EmptyCompareFile`] if either file
/// yields no usable (`passed`) energies or if `stats::describe`/
/// `stats::ks_distance`/`stats::ks_two_sample_p` returns `None` for that
/// side (see this module's own doc on why that is an error rather than a
/// fabricated report).
pub fn run_compare(file_a: &Path, file_b: &Path) -> Result<CompareReport, ModeError> {
    let jsonl_a = read_jsonl(file_a)
        .map_err(|e| ModeError::CompareFileUnreadable(file_a.to_path_buf(), e))?;
    let jsonl_b = read_jsonl(file_b)
        .map_err(|e| ModeError::CompareFileUnreadable(file_b.to_path_buf(), e))?;

    let energies_a = passed_energies(&jsonl_a.records);
    let energies_b = passed_energies(&jsonl_b.records);

    // Emptiness first: a bare `Stats` cannot represent "no samples," and
    // defaulting `p_value` would invent a comparison that never ran.
    if energies_a.is_empty() {
        return Err(ModeError::EmptyCompareFile(file_a.to_path_buf()));
    }
    if energies_b.is_empty() {
        return Err(ModeError::EmptyCompareFile(file_b.to_path_buf()));
    }

    let a =
        describe(&energies_a).ok_or_else(|| ModeError::EmptyCompareFile(file_a.to_path_buf()))?;
    let b =
        describe(&energies_b).ok_or_else(|| ModeError::EmptyCompareFile(file_b.to_path_buf()))?;

    let Some(ks) = ks_distance(&energies_a, &energies_b) else {
        // Unreachable for non-empty finite i64-derived energies (see
        // `stats::ks_distance`'s documented `None` cases), but fail loud
        // rather than invent a distance — same precedent as
        // `EmptyReproBatch`.
        return Err(ModeError::EmptyCompareFile(file_a.to_path_buf()));
    };
    let Some(p_value) = ks_two_sample_p(&energies_a, &energies_b) else {
        return Err(ModeError::EmptyCompareFile(file_a.to_path_buf()));
    };

    Ok(CompareReport {
        a,
        b,
        ks,
        p_value,
        n_a: energies_a.len(),
        n_b: energies_b.len(),
        skipped_a: jsonl_a.skipped_lines,
        skipped_b: jsonl_b.skipped_lines,
    })
}

/// Render `report` as a plain-text table matching
/// `report::format_summary_table`'s visual conventions (label columns,
/// fixed-width alignment, decimal formatting). States the passed-only
/// sample counts and skipped-line counts so a low `n` is not misread.
///
/// Returns a `String` rather than printing: this crate's `print-stdout`
/// lint denies scattered `println!` calls, so the CLI routes the result
/// through `src/main.rs`'s single output boundary.
#[must_use]
pub fn format_comparison_table(r: &CompareReport) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "samples:   a={} passed, b={} passed (failed/rejected records excluded)",
        r.n_a, r.n_b
    );
    let _ = writeln!(
        out,
        "skipped:   a={} lines, b={} lines",
        r.skipped_a, r.skipped_b
    );
    let _ = writeln!(out, "ks:        {:.4}", r.ks);
    // Scientific notation for p_value: a genuinely tiny p (e.g. ~3e-8 for
    // disjoint samples) would print as `0.0000` in fixed-point, the same
    // misleading rendering `main::format_reproducibility_message` already
    // documents and refuses.
    let _ = writeln!(out, "p_value:   {:.4e}", r.p_value);
    let _ = writeln!(out);
    let _ = writeln!(out, "file A (passed energies, natural units):");
    write_stats_block(&mut out, &r.a);
    let _ = writeln!(out, "file B (passed energies, natural units):");
    write_stats_block(&mut out, &r.b);
    out
}

/// One side's `Stats` block under a `file A`/`file B` heading.
fn write_stats_block(out: &mut String, s: &Stats) {
    let _ = writeln!(out, "  min:  {:.4}", s.min);
    let _ = writeln!(out, "  max:  {:.4}", s.max);
    let _ = writeln!(out, "  mean: {:.4}", s.mean);
    let _ = writeln!(out, "  std:  {:.4}", s.std);
    let _ = writeln!(out, "  p50:  {:.4}", s.p50);
    let _ = writeln!(out, "  p90:  {:.4}", s.p90);
    let _ = writeln!(out, "  p99:  {:.4}", s.p99);
}

/// `passed` records' `best_energy_milli`, converted to natural `f64` units.
fn passed_energies(records: &[JobRecord]) -> Vec<f64> {
    records
        .iter()
        .filter(|r| r.passed)
        .map(|r| milli_to_f64(r.best_energy_milli))
        .collect()
}

/// Load the `passed` energies (natural units) from a JSONL records file.
/// Used by the CLI `--plot` path so the energy CDF chart can reuse the same
/// extraction rules as [`run_compare`] without expanding [`CompareReport`].
///
/// # Errors
///
/// Returns [`ModeError::CompareFileUnreadable`] if `path` cannot be read, or
/// [`ModeError::EmptyCompareFile`] if no `passed` records are present.
pub fn load_passed_energies(path: &Path) -> Result<Vec<f64>, ModeError> {
    let jsonl =
        read_jsonl(path).map_err(|e| ModeError::CompareFileUnreadable(path.to_path_buf(), e))?;
    let energies = passed_energies(&jsonl.records);
    if energies.is_empty() {
        return Err(ModeError::EmptyCompareFile(path.to_path_buf()));
    }
    Ok(energies)
}

#[cfg(test)]
mod tests {
    use super::format_comparison_table;
    use crate::stats::Stats;

    fn sample_report() -> super::CompareReport {
        super::CompareReport {
            a: Stats {
                min: -3.0,
                max: -1.0,
                mean: -2.0,
                std: 0.8165,
                p50: -2.0,
                p90: -1.2,
                p99: -1.02,
            },
            b: Stats {
                min: 1.0,
                max: 5.0,
                mean: 3.0,
                std: std::f64::consts::SQRT_2,
                p50: 3.0,
                p90: 4.6,
                p99: 4.96,
            },
            ks: 1.0,
            p_value: 0.003_781_354,
            n_a: 3,
            n_b: 5,
            skipped_a: 2,
            skipped_b: 7,
        }
    }

    #[test]
    fn format_comparison_table_contains_sample_and_skipped_counts() {
        let table = format_comparison_table(&sample_report());
        assert!(
            table.contains("a=3 passed"),
            "table must carry n_a: {table}"
        );
        assert!(
            table.contains("b=5 passed"),
            "table must carry n_b: {table}"
        );
        assert!(
            table.contains("a=2 lines"),
            "table must carry skipped_a: {table}"
        );
        assert!(
            table.contains("b=7 lines"),
            "table must carry skipped_b: {table}"
        );
        assert!(table.contains("failed/rejected records excluded"));
        assert!(table.contains("1.0000"), "ks must appear: {table}");
        assert!(
            table.contains("3.7814e-3") || table.contains("3.7814e-03"),
            "p_value must appear in scientific notation: {table}"
        );
    }

    /// Means under the correct A/B header with the `mean:` label. Sample
    /// report uses distinct means (−2 vs 3) so a swapped stats block fails,
    /// and the label so a dropped mean row cannot hide behind `p50:`.
    #[test]
    fn format_comparison_table_places_means_under_correct_headers() {
        let table = format_comparison_table(&sample_report());
        let a_pos = table.find("file A (passed energies").expect("A header");
        let b_pos = table.find("file B (passed energies").expect("B header");
        let mean_a = table.find("mean: -2.0000").expect("A mean line");
        let mean_b = table.find("mean: 3.0000").expect("B mean line");
        assert!(
            a_pos < mean_a && mean_a < b_pos,
            "A mean under A before B: {table}"
        );
        assert!(b_pos < mean_b, "B mean under B: {table}");
    }
}
