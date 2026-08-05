// SPDX-License-Identifier: AGPL-3.0-or-later
//! Two-sample Kolmogorov-Smirnov distance and asymptotic p-value.

use super::contains_non_finite;

/// Two-sample Kolmogorov-Smirnov statistic: the maximum absolute
/// difference between the empirical CDFs of `a` and `b`, evaluated at
/// every sample point in either slice (the empirical CDF only changes at
/// sample points, so no other point can be the maximizer).
///
/// Returns `None` if either slice is empty, or if either contains a
/// non-finite value (see [`super::contains_non_finite`]). `a` and `b`
/// need not be pre-sorted; this function sorts its own copies.
///
/// `ks_distance(a, a)` is exactly `0.0` for any non-empty finite `a`: the
/// two empirical CDFs are identical at every sample point.
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts in a benchmark run are far below f64's 52-bit exact-integer range"
)]
#[expect(
    clippy::float_cmp,
    reason = "tie detection needs the literal same sample value, not an approximate match; \
              an epsilon here would merge distinct nearby samples into one CDF step"
)]
#[must_use]
pub fn ks_distance(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    if contains_non_finite(a) || contains_non_finite(b) {
        return None;
    }

    let mut sorted_a = a.to_vec();
    let mut sorted_b = b.to_vec();
    sorted_a.sort_by(f64::total_cmp);
    sorted_b.sort_by(f64::total_cmp);

    let len_a = sorted_a.len();
    let len_b = sorted_b.len();

    // Merge-scan both sorted samples, advancing past every tied value at
    // once so a repeated value is not counted as multiple crossings.
    // Empirical CDFs are step functions that only change at sample
    // points, so the maximum |Fa - Fb| is found by checking after every
    // such step.
    let mut idx_a = 0usize;
    let mut idx_b = 0usize;
    let mut max_diff = 0.0_f64;
    while idx_a < len_a && idx_b < len_b {
        if sorted_a[idx_a] < sorted_b[idx_b] {
            idx_a += 1;
        } else if sorted_b[idx_b] < sorted_a[idx_a] {
            idx_b += 1;
        } else {
            let tied_value = sorted_a[idx_a];
            while idx_a < len_a && sorted_a[idx_a] == tied_value {
                idx_a += 1;
            }
            while idx_b < len_b && sorted_b[idx_b] == tied_value {
                idx_b += 1;
            }
        }
        let cdf_a = idx_a as f64 / len_a as f64;
        let cdf_b = idx_b as f64 / len_b as f64;
        max_diff = max_diff.max((cdf_a - cdf_b).abs());
    }
    Some(max_diff)
}

/// Asymptotic two-sided p-value for the two-sample Kolmogorov-Smirnov
/// test, following the classic Kolmogorov (1933) limiting distribution
/// with the small-sample correction factor from:
///
/// Press, Teukolsky, Vetterling, Flannery, *Numerical Recipes: The Art of
/// Scientific Computing*, 3rd ed. (Cambridge University Press, 2007),
/// section 14.3.2 "Kolmogorov-Smirnov Test" (`KSdist::pks`/`ks2test`).
///
/// The effective sample size `en = n*m / (n+m)` and the statistic is
/// rescaled as `lambda = (sqrt(en) + 0.12 + 0.11/sqrt(en)) * D` before
/// evaluating the limiting distribution `Q(lambda)`; Numerical Recipes
/// attributes the `0.12 + 0.11/sqrt(en)` correction term to Stephens
/// (1970). This is the **asymptotic** p-value, not the exact small-sample
/// distribution that `scipy.stats.ks_2samp(method="exact")` computes.
///
/// Returns `None` under the same conditions as [`ks_distance`] (either
/// slice empty, or either containing a non-finite value).
#[expect(
    clippy::cast_precision_loss,
    reason = "sample counts in a benchmark run are far below f64's 52-bit exact-integer range"
)]
#[must_use]
pub fn ks_two_sample_p(a: &[f64], b: &[f64]) -> Option<f64> {
    let distance = ks_distance(a, b)?;
    let len_a = a.len() as f64;
    let len_b = b.len() as f64;
    let effective_n = (len_a * len_b) / (len_a + len_b);
    let effective_n_sqrt = effective_n.sqrt();
    let lambda = (effective_n_sqrt + 0.12 + 0.11 / effective_n_sqrt) * distance;
    Some(kolmogorov_q(lambda))
}

/// The Kolmogorov distribution's survival function `Q(lambda)`, via the
/// alternating series `Q(lambda) = 2 * sum_{j=1..inf} (-1)^(j-1) *
/// exp(-2 * j^2 * lambda^2)`. Ported from the `probks`/`ks_prob`
/// reference implementation in Numerical Recipes 3rd ed., section 14.3.2:
/// terms are summed until either the current term is negligible relative
/// to the previous term's magnitude or relative to the running sum, with
/// a hard cap of 100 terms (the series converges within a handful of
/// terms for any `lambda` that arises from real sample sizes; the cap
/// only guards against non-termination). Failing to converge within the
/// cap returns `1.0`, matching the reference implementation — this also
/// happens to be the correct limit as `lambda -> 0` (fully overlapping
/// samples), where the alternating series does not converge term-by-term.
fn kolmogorov_q(lambda: f64) -> f64 {
    const EPS1: f64 = 0.001;
    const EPS2: f64 = 1.0e-8;

    let a2 = -2.0 * lambda * lambda;
    let mut sign = 2.0_f64;
    let mut sum = 0.0_f64;
    let mut prev_term_abs = 0.0_f64;
    for j in 1..=100_i32 {
        let term = sign * (a2 * f64::from(j * j)).exp();
        sum += term;
        if term.abs() <= EPS1 * prev_term_abs || term.abs() <= EPS2 * sum.abs() {
            return sum.clamp(0.0, 1.0);
        }
        sign = -sign;
        prev_term_abs = term.abs();
    }
    1.0
}

#[cfg(test)]
mod tests {
    use super::{ks_distance, ks_two_sample_p};

    #[test]
    fn ks_distance_of_a_sample_against_itself_is_exactly_zero() {
        let a = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
        assert_eq!(ks_distance(&a, &a), Some(0.0));
    }

    #[test]
    fn ks_distance_of_disjoint_samples_is_exactly_one() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [100.0, 101.0, 102.0, 103.0, 104.0];
        assert_eq!(ks_distance(&a, &b), Some(1.0));
    }

    /// Hand-derived partial overlap: a = [1,2,3,4,5], b = [2,3,4,5,6].
    /// Combined empirical CDF steps (n = m = 5):
    ///   x=1: Fa=1/5=0.2, Fb=0/5=0.0   -> |diff|=0.2
    ///   x=2: Fa=2/5=0.4, Fb=1/5=0.2   -> |diff|=0.2
    ///   x=3: Fa=3/5=0.6, Fb=2/5=0.4   -> |diff|=0.2
    ///   x=4: Fa=4/5=0.8, Fb=3/5=0.6   -> |diff|=0.2
    ///   x=5: Fa=5/5=1.0, Fb=4/5=0.8   -> |diff|=0.2
    /// max = 0.2 exactly. This also proves the implementation takes the
    /// *maximum* difference, not e.g. the mean of the per-step
    /// differences (which here would coincidentally also be 0.2 — see
    /// the asymmetric case below for a test that tells them apart).
    #[test]
    fn ks_distance_partial_overlap_matches_hand_derivation() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [2.0, 3.0, 4.0, 5.0, 6.0];
        let d = ks_distance(&a, &b).expect("non-empty finite inputs");
        // 1/5 accumulates a float rounding residual (0.2000000000000007)
        // through the repeated integer-division steps; compare with an
        // epsilon rather than exact equality, which is not guaranteed
        // here (unlike `ks_distance(a, a) == 0.0`, where both sides
        // reduce to the same computation and cancel exactly).
        assert!((d - 0.2).abs() < 1e-12, "expected 0.2, got {d}");
    }

    /// A case where the maximum step difference and the mean step
    /// difference are clearly different values, so a mutation that
    /// returns the mean instead of the max is caught. a = [1..20]
    /// (n=20), b = nineteen copies of 1.0 plus one 21.0 (m=20).
    /// Step diffs, computed by the same merge algorithm by hand:
    /// at x=1 (a's first point ties b's 19 copies of 1): Fa=1/20=0.05,
    /// Fb=19/20=0.95 -> diff=0.90 (the maximum: this is the largest gap
    /// in the whole run). From there Fb stays at 0.95 while Fa climbs
    /// one step at a time (0.10, 0.15, ..., 1.0), shrinking the diff
    /// step by step down through 0.85, 0.80, ..., 0.0, then a final
    /// step once b's remaining value (21.0) is consumed after a is
    /// exhausted brings it back up slightly. The full sequence of 20
    /// per-step diffs averages to 0.43 (verified independently in
    /// Python), a full 0.47 away from the true maximum of 0.90 — not a
    /// value a max/mean mix-up could produce by coincidence.
    #[test]
    fn ks_distance_picks_the_max_not_the_mean_of_step_differences() {
        let a: Vec<f64> = (1..=20).map(f64::from).collect();
        let mut b = vec![1.0; 19];
        b.push(21.0);
        let d = ks_distance(&a, &b).expect("non-empty finite inputs");
        assert!((d - 0.9).abs() < 1e-9, "expected max diff 0.9, got {d}");
    }

    #[test]
    fn ks_distance_is_none_for_an_empty_slice() {
        assert_eq!(ks_distance(&[], &[1.0]), None);
        assert_eq!(ks_distance(&[1.0], &[]), None);
        assert_eq!(ks_distance(&[], &[]), None);
    }

    #[test]
    fn ks_distance_rejects_non_finite_values() {
        assert_eq!(ks_distance(&[1.0, f64::NAN], &[2.0]), None);
        assert_eq!(ks_distance(&[1.0], &[2.0, f64::INFINITY]), None);
    }

    #[test]
    fn ks_two_sample_p_on_identical_samples_exceeds_0_05() {
        let a = [3.0, 1.0, 4.0, 1.0, 5.0, 9.0, 2.0, 6.0];
        let p = ks_two_sample_p(&a, &a).expect("non-empty finite inputs");
        assert!(p > 0.05, "expected p > 0.05 for identical samples, got {p}");
    }

    /// Hand-derived: a = [1,2,3,4,5], b = [100,101,102,103,104], D = 1.0
    /// (see `ks_distance_of_disjoint_samples_is_exactly_one`).
    /// en = n*m/(n+m) = 25/10 = 2.5, sqrt(en) = 1.581138...
    /// lambda = (1.581138 + 0.12 + 0.11/1.581138) * 1.0 = 1.770709...
    /// Q(lambda): a2 = -2*lambda^2 = -6.270819...
    ///   j=1: term = 2*exp(-6.270819) = 0.00378135...
    ///   j=2: term = -2*exp(-25.083) = ~-2.6e-11, negligible next to term 1
    ///   -> sum converges to ~0.00378135, comfortably under 0.01.
    #[test]
    fn ks_two_sample_p_on_disjoint_samples_is_below_0_01() {
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [100.0, 101.0, 102.0, 103.0, 104.0];
        let p = ks_two_sample_p(&a, &b).expect("non-empty finite inputs");
        assert!(p < 0.01, "expected p < 0.01 for disjoint samples, got {p}");
        assert!((p - 0.003_781_354).abs() < 1e-8, "got {p}");
    }

    #[test]
    fn ks_two_sample_p_is_none_for_an_empty_slice() {
        assert_eq!(ks_two_sample_p(&[], &[1.0]), None);
    }

    #[test]
    fn ks_two_sample_p_rejects_non_finite_values() {
        assert_eq!(ks_two_sample_p(&[1.0, f64::NAN], &[2.0, 3.0]), None);
    }
}
