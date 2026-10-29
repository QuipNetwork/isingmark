// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact binomial limits and repeatable resampling of complete instances.

use isingmark::campaign::analysis::{
    confidence::{binomial_lower, binomial_upper, cluster_bootstrap, n_required, zero_event_upper},
    survival::AnalysisError,
};

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
}

#[test]
fn zero_success_bound_crosses_one_percent_at_299() {
    assert!(zero_event_upper(298, 0.05).expect("valid bound") > 0.01);
    assert!(zero_event_upper(299, 0.05).expect("valid bound") <= 0.01);
}

#[test]
#[expect(
    clippy::unreadable_literal,
    reason = "Keep the preregistered spec table probabilities verbatim, as required by the brief."
)]
fn n_required_matches_spec_table() {
    for (target, count) in [(0.01, 299), (0.0001004983, 29_808), (0.0000100503, 298_073)] {
        assert_eq!(n_required(target, 0.05), Ok(count));
    }
}

#[test]
fn n_required_guards_roundoff_at_an_integer() {
    // ln(0.05) / ln(1 - 0.95) rounds to 1.0000000000000002.
    assert_eq!(n_required(0.95, 0.05), Ok(1));
    assert_eq!(n_required(0.949_999_999, 0.05), Ok(2));
    assert_eq!(n_required(0.950_000_001, 0.05), Ok(1));
}

#[test]
fn perfect_qpu_successes_bound() {
    assert!(binomial_lower(59, 59, 0.05).expect("valid bound") > 0.95);
    assert!(binomial_lower(58, 58, 0.05).expect("valid bound") < 0.95);
}

#[test]
fn perfect_race_wins_bound() {
    assert!(binomial_lower(29, 29, 0.05).expect("valid bound") > 0.90);
    assert!(binomial_lower(28, 28, 0.05).expect("valid bound") < 0.90);
}

#[test]
fn upper_bound_matches_zero_event() {
    for n in [1, 299, 29_808] {
        for alpha in [0.025, 0.05, 0.5] {
            close(
                binomial_upper(0, n, alpha).expect("valid bound"),
                zero_event_upper(n, alpha).expect("valid zero-event bound"),
            );
        }
    }
    assert!(zero_event_upper(u64::MAX, 0.05).expect("large count") > 0.0);
}

#[test]
fn binomial_endpoints_are_exact() {
    for n in [1, 59, 29_808] {
        assert_eq!(
            binomial_lower(0, n, 0.025)
                .expect("zero successes")
                .to_bits(),
            0.0_f64.to_bits()
        );
        assert_eq!(
            binomial_upper(n, n, 0.025)
                .expect("all successes")
                .to_bits(),
            1.0_f64.to_bits()
        );
    }
}

#[test]
fn interior_bounds_invert_binomial_tails() {
    close(
        binomial_lower(1, 2, 0.05).expect("lower bound"),
        1.0 - 0.95_f64.sqrt(),
    );
    close(
        binomial_upper(1, 2, 0.05).expect("upper bound"),
        0.95_f64.sqrt(),
    );
    close(
        binomial_lower(3, 10, 0.025).expect("lower bound"),
        0.066_739_511_177_734_5,
    );
    close(
        binomial_upper(3, 10, 0.025).expect("upper bound"),
        0.652_452_850_059_997_3,
    );
    // At p = 0.5, P(X <= 1) = 2,001 / 2^2,000 underflows outside log space.
    let upper = binomial_upper(1, 2_000, 0.025).expect("tail bound");
    close(
        (1.0 - upper).powi(2_000) + 2_000.0 * upper * (1.0 - upper).powi(1_999),
        0.025,
    );
}

#[test]
fn rare_lower_tail_stays_accurate_at_the_spec_sample_size() {
    // Closed form for one success: 1 - (1 - alpha)^(1/n).
    close(
        binomial_lower(1, 298_073, 0.025).expect("rare lower tail"),
        8.493_827_655_998_489e-8,
    );
}

#[test]
fn invalid_probabilities_and_counts_are_rejected() {
    for alpha in [
        f64::NAN,
        f64::NEG_INFINITY,
        -0.1,
        0.0,
        1.0,
        1.1,
        f64::INFINITY,
    ] {
        assert_eq!(
            zero_event_upper(1, alpha),
            Err(AnalysisError::InvalidProbability)
        );
        assert_eq!(
            n_required(0.01, alpha),
            Err(AnalysisError::InvalidProbability)
        );
        for (k, n) in [(0, 1), (1, 1)] {
            assert_eq!(
                binomial_lower(k, n, alpha),
                Err(AnalysisError::InvalidProbability)
            );
            assert_eq!(
                binomial_upper(k, n, alpha),
                Err(AnalysisError::InvalidProbability)
            );
        }
        assert_eq!(
            n_required(alpha, 0.05),
            Err(AnalysisError::InvalidProbability)
        );
    }
    assert_eq!(
        zero_event_upper(0, 0.05),
        Err(AnalysisError::InvalidProbability)
    );
    for (k, n) in [(0, 0), (2, 1)] {
        assert_eq!(
            binomial_lower(k, n, 0.05),
            Err(AnalysisError::InvalidProbability)
        );
        assert_eq!(
            binomial_upper(k, n, 0.05),
            Err(AnalysisError::InvalidProbability)
        );
    }
}

fn item_count(sample: &[&[u8]]) -> f64 {
    sample
        .iter()
        .map(|cluster| f64::from(u32::try_from(cluster.len()).expect("tiny fixture")))
        .sum()
}

#[test]
fn bootstrap_resamples_whole_clusters() {
    let clusters = [vec![1, 2, 3], vec![4]];
    let values = cluster_bootstrap(&clusters, 100, 17, |sample| {
        assert_eq!(sample.len(), 2);
        assert!(sample
            .iter()
            .all(|cluster| *cluster == [1, 2, 3] || *cluster == [4]));
        item_count(sample)
    })
    .expect("whole clusters");
    assert_eq!(values.len(), 100);
    assert!(values.iter().all(|value| [2.0, 4.0, 6.0].contains(value)));
    for count in [2.0, 4.0, 6.0] {
        assert!(values.contains(&count));
    }
}

#[test]
fn bootstrap_is_deterministic_for_a_seed() {
    let clusters = [vec![1, 2, 3], vec![4]];
    let first = cluster_bootstrap(&clusters, 100, 17, item_count).expect("seeded bootstrap");
    assert_eq!(
        first,
        cluster_bootstrap(&clusters, 100, 17, item_count).expect("same seed")
    );
    assert_ne!(
        first,
        cluster_bootstrap(&clusters, 100, 18, item_count).expect("different seed")
    );
}

#[test]
fn bootstrap_rejects_missing_clusters_or_resamples() {
    assert_eq!(
        cluster_bootstrap::<u8, _>(&[], 20, 0, item_count),
        Err(AnalysisError::Empty)
    );
    assert_eq!(
        cluster_bootstrap(&[vec![1]], 0, 0, item_count),
        Err(AnalysisError::Empty)
    );
}
