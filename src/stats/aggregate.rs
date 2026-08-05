// SPDX-License-Identifier: AGPL-3.0-or-later
//! Summary statistics (`describe`) and linear-interpolation percentiles.

use super::contains_non_finite;

/// Summary statistics over a slice of samples (e.g. energies from a
/// benchmark run).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    /// Smallest sample.
    pub min: f64,
    /// Largest sample.
    pub max: f64,
    /// Arithmetic mean.
    pub mean: f64,
    /// **Population** standard deviation (divides the sum of squared
    /// deviations by `N`, not `N - 1`). This matches the Python
    /// implementation this crate replaces, which used `numpy.std`'s
    /// default `ddof=0`. Do not "fix" this to a sample standard
    /// deviation (`N - 1`) — that would silently change every reported
    /// value and break comparability with prior benchmark runs. For a
    /// single-sample input this is `0.0`, not `NaN`.
    pub std: f64,
    /// 50th percentile (median), linearly interpolated.
    pub p50: f64,
    /// 90th percentile, linearly interpolated.
    pub p90: f64,
    /// 99th percentile, linearly interpolated.
    pub p99: f64,
}

/// Compute [`Stats`] over `values`.
///
/// Returns `None` if `values` is empty or contains a non-finite value
/// (see [`super::contains_non_finite`]). A single-element input is valid
/// and produces `std: 0.0` with every percentile equal to that element.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts in a benchmark run are far below f64's 52-bit exact-integer range"
)]
#[must_use]
pub fn describe(values: &[f64]) -> Option<Stats> {
    if values.is_empty() || contains_non_finite(values) {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);

    let n = sorted.len() as f64;
    let mean = sorted.iter().sum::<f64>() / n;
    // Population variance: divide by N, not N - 1 — see `Stats::std`.
    let variance = sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;

    Some(Stats {
        min: sorted[0],
        max: sorted[sorted.len() - 1],
        mean,
        std: variance.sqrt(),
        p50: percentile(&sorted, 0.5)?,
        p90: percentile(&sorted, 0.9)?,
        p99: percentile(&sorted, 0.99)?,
    })
}

/// Percentile `q` (in `[0.0, 1.0]`) of `sorted`, by linear interpolation
/// between closest ranks — numpy's default `percentile` method
/// (equivalently, R's quantile type 7). For a sorted slice of length `n`:
///
/// ```text
/// h  = q * (n - 1)          // fractional index, NOT q * n
/// lo = floor(h)
/// hi = ceil(h)
/// result = v[lo] + (h - lo) * (v[hi] - v[lo])
/// ```
///
/// `sorted` must already be sorted in non-decreasing order; this is a
/// precondition checked with `debug_assert!` in debug builds, not
/// silently tolerated (an unsorted input would return a plausible-looking
/// but meaningless number).
///
/// Returns `None` if `sorted` is empty, if `q` is `NaN` or outside
/// `[0.0, 1.0]`, or if `sorted` contains a non-finite value (see
/// [`super::contains_non_finite`]). Out-of-range `q` is rejected rather
/// than clamped: clamping would silently turn a caller's typo (e.g.
/// `q = 1.5`) into a quietly-wrong "max" result instead of surfacing the
/// mistake.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts in a benchmark run are far below f64's 52-bit exact-integer range"
)]
#[expect(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "h = q * (n - 1) with q clamped to [0.0, 1.0] and n > 0 is always non-negative; \
              the explicit .min(n - 1) below guards the only case where floating-point \
              rounding could push a truncated ceiling one index past the last valid slot"
)]
#[must_use]
pub fn percentile(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    if !(0.0..=1.0).contains(&q) {
        return None;
    }
    if contains_non_finite(sorted) {
        return None;
    }
    debug_assert!(
        sorted.windows(2).all(|w| w[0] <= w[1]),
        "percentile requires a sorted (non-decreasing) slice"
    );

    let n = sorted.len();
    let h = q * (n - 1) as f64;
    let lo = (h.floor() as usize).min(n - 1);
    let hi = (h.ceil() as usize).min(n - 1);
    let frac = h - lo as f64;
    Some(sorted[lo] + frac * (sorted[hi] - sorted[lo]))
}

#[cfg(test)]
mod tests {
    use super::{describe, percentile};

    /// A set with every statistic distinct, so a mutation that swaps two
    /// fields (e.g. `std`/`mean`, or `p50`/`p90`) is detectable.
    ///
    /// Hand-derived expected values for `[2.0, 4.0, 4.0, 9.0, 11.0]`
    /// (n = 5, already sorted):
    ///
    /// ```text
    /// min = 2.0, max = 11.0
    /// mean = (2+4+4+9+11)/5 = 30/5 = 6.0
    /// population variance = ((2-6)^2 + (4-6)^2 + (4-6)^2 + (9-6)^2 + (11-6)^2) / 5
    ///                      = (16 + 4 + 4 + 9 + 25) / 5 = 58/5 = 11.6
    /// std = sqrt(11.6) = 3.405877273619807...
    /// p50: h = 0.5*4 = 2.0 -> exact index 2 -> 4.0
    /// p90: h = 0.9*4 = 3.6 -> lo=3 (9.0), hi=4 (11.0), frac=0.6
    ///      -> 9.0 + 0.6*(11.0-9.0) = 9.0 + 1.2 = 10.2
    /// p99: h = 0.99*4 = 3.96 -> lo=3 (9.0), hi=4 (11.0), frac=0.96
    ///      -> 9.0 + 0.96*2.0 = 9.0 + 1.92 = 10.92
    /// ```
    fn sample() -> Vec<f64> {
        vec![2.0, 4.0, 4.0, 9.0, 11.0]
    }

    #[test]
    fn describe_matches_hand_derived_statistics() {
        let stats = describe(&sample()).expect("non-empty finite input");
        assert!((stats.min - 2.0).abs() < 1e-12);
        assert!((stats.max - 11.0).abs() < 1e-12);
        assert!((stats.mean - 6.0).abs() < 1e-12);
        assert!((stats.std - 11.6_f64.sqrt()).abs() < 1e-12);
        assert!((stats.p50 - 4.0).abs() < 1e-12);
        assert!((stats.p90 - 10.2).abs() < 1e-12);
        assert!((stats.p99 - 10.92).abs() < 1e-12);
    }

    #[test]
    fn describe_of_empty_slice_is_none() {
        assert_eq!(describe(&[]), None);
    }

    #[test]
    // Population std of a single sample is (x - x)^2 / 1 = 0.0 exactly —
    // no rounding can occur, so an exact comparison is intentional here,
    // not a fuzzy "close enough" check the float_cmp lint is meant to
    // guard against.
    #[expect(
        clippy::float_cmp,
        reason = "std of a single sample is exactly 0.0 by construction, not approximately so"
    )]
    fn describe_of_single_element_is_degenerate_not_a_panic() {
        let stats = describe(&[7.0]).expect("single element is valid");
        assert!((stats.min - 7.0).abs() < 1e-12);
        assert!((stats.max - 7.0).abs() < 1e-12);
        assert!((stats.mean - 7.0).abs() < 1e-12);
        assert_eq!(stats.std, 0.0);
        assert!((stats.p50 - 7.0).abs() < 1e-12);
        assert!((stats.p90 - 7.0).abs() < 1e-12);
        assert!((stats.p99 - 7.0).abs() < 1e-12);
    }

    #[test]
    fn describe_rejects_nan() {
        assert_eq!(describe(&[1.0, f64::NAN, 3.0]), None);
    }

    #[test]
    fn describe_rejects_infinity() {
        assert_eq!(describe(&[1.0, f64::INFINITY, 3.0]), None);
        assert_eq!(describe(&[1.0, f64::NEG_INFINITY, 3.0]), None);
    }

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "test fixture value; exactness of the round-trip is irrelevant, only that \
                  the resulting f64 stays finite"
    )]
    #[expect(
        clippy::float_cmp,
        reason = "min/max are copied through from the sorted input with no arithmetic, so an \
                  exact match is guaranteed, not approximate"
    )]
    fn describe_accepts_extreme_but_finite_sentinels() {
        // f64::MAX and the 1 << 62 overflow sentinel used upstream in
        // quip_protocol::scoring are realistic, not hypothetical, and
        // must not be rejected as if they were non-finite.
        let big = (1_i64 << 62) as f64;
        let stats = describe(&[0.0, big, f64::MAX]).expect("finite extremes are valid");
        assert_eq!(stats.max, f64::MAX);
        assert_eq!(stats.min, 0.0);
    }

    /// The numpy-documented example: percentile(&[1,2,3,4], 0.5) == 2.5.
    /// h = 0.5 * (4 - 1) = 1.5, lo = 1 (2.0), hi = 2 (3.0), frac = 0.5
    /// -> 2.0 + 0.5 * (3.0 - 2.0) = 2.5. Nearest-rank would give 2.0 or
    /// 3.0 instead — a wrong-but-plausible value this test would catch.
    #[test]
    fn percentile_median_of_four_matches_numpy() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.5), Some(2.5));
    }

    /// percentile(&[1,2,3,4], 0.25) == 1.75.
    /// h = 0.25 * 3 = 0.75, lo = 0 (1.0), hi = 1 (2.0), frac = 0.75
    /// -> 1.0 + 0.75 * (2.0 - 1.0) = 1.75.
    #[test]
    fn percentile_quarter_of_four_matches_numpy() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.25), Some(1.75));
    }

    #[test]
    fn percentile_at_zero_and_one_returns_the_endpoints() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.0), Some(1.0));
        assert_eq!(percentile(&v, 1.0), Some(4.0));
    }

    #[test]
    fn percentile_of_empty_slice_is_none() {
        assert_eq!(percentile(&[], 0.5), None);
    }

    #[test]
    fn percentile_rejects_q_outside_unit_range() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, -0.1), None);
        assert_eq!(percentile(&v, 1.1), None);
        assert_eq!(percentile(&v, f64::NAN), None);
    }

    #[test]
    fn percentile_rejects_non_finite_values_in_input() {
        assert_eq!(percentile(&[1.0, f64::NAN, 3.0], 0.5), None);
        assert_eq!(percentile(&[1.0, f64::INFINITY, 3.0], 0.5), None);
    }

    #[test]
    #[should_panic(expected = "percentile requires a sorted")]
    fn percentile_debug_asserts_on_an_unsorted_slice() {
        let _ = percentile(&[3.0, 1.0, 2.0], 0.5);
    }
}
