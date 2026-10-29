// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact binomial bounds and deterministic resampling of complete instance clusters.

use super::survival::AnalysisError;

/// One-sided exact upper bound after zero successes in `n` independent observations.
///
/// # Errors
/// Returns `InvalidProbability` if `n` is zero or `alpha` is outside (0, 1).
pub fn zero_event_upper(n: u64, alpha: f64) -> Result<f64, AnalysisError> {
    validate_binomial(0, n, alpha)?;
    // exp_m1 retains a positive bound when alpha^(1/n) would round to one.
    Ok(-(alpha.ln() / count_f64(n)).exp_m1())
}

/// Independent zero-success observations needed for an upper bound of `p_target`.
///
/// Ratios within four relative machine epsilons of an integer round to that integer
/// before taking the ceiling. This prevents an extra observation from roundoff in
/// the logarithms, including ln(0.05) / ln(1 - 0.95) = 1.0000000000000002.
///
/// # Errors
/// Returns `InvalidProbability` for inputs outside (0, 1), or a count exceeding `u64`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "The finite, positive rounded count is checked against the u64 range before casting."
)]
pub fn n_required(p_target: f64, alpha: f64) -> Result<u64, AnalysisError> {
    check_open_probability(p_target)?;
    check_open_probability(alpha)?;
    let ratio = alpha.ln() / (-p_target).ln_1p();
    let nearest = ratio.round();
    let rounded = if (ratio - nearest).abs() <= 4.0 * f64::EPSILON * ratio {
        nearest
    } else {
        ratio.ceil()
    };
    // u64::MAX rounds upward to 2^64 as f64, so equality is out of range too.
    if !rounded.is_finite() || rounded >= count_f64(u64::MAX) {
        return Err(AnalysisError::InvalidProbability);
    }
    Ok((rounded as u64).max(1))
}

/// One-sided Clopper–Pearson upper bound for `k` successes among `n` observations.
/// Interior bounds bisect the exact log binomial tail to better than 1e-12 in p.
///
/// # Errors
/// Returns `InvalidProbability` if `k > n`, `n == 0`, or `alpha` is outside (0, 1).
pub fn binomial_upper(k: u64, n: u64, alpha: f64) -> Result<f64, AnalysisError> {
    validate_binomial(k, n, alpha)?;
    if k == n {
        return Ok(1.0);
    }
    if k == 0 {
        return zero_event_upper(n, alpha);
    }
    let log_alpha = alpha.ln();
    let (mut low, mut high) = (0.0, 1.0);
    while high - low > 1e-14 {
        let middle = f64::midpoint(low, high);
        // P(X <= k) decreases as the candidate success probability increases.
        if log_binomial_cdf(k, n, middle) > log_alpha {
            low = middle;
        } else {
            high = middle;
        }
    }
    Ok(high)
}

/// One-sided Clopper–Pearson lower bound for `k` successes among `n` observations.
/// Uses the complementary failure count for interior bounds.
///
/// # Errors
/// Returns `InvalidProbability` if `k > n`, `n == 0`, or `alpha` is outside (0, 1).
pub fn binomial_lower(k: u64, n: u64, alpha: f64) -> Result<f64, AnalysisError> {
    validate_binomial(k, n, alpha)?;
    if k == 0 {
        return Ok(0.0);
    }
    if k == n {
        return Ok((alpha.ln() / count_f64(n)).exp());
    }
    Ok(1.0 - binomial_upper(n - k, n, alpha)?)
}

fn check_open_probability(probability: f64) -> Result<(), AnalysisError> {
    if probability > 0.0 && probability < 1.0 {
        Ok(())
    } else {
        Err(AnalysisError::InvalidProbability)
    }
}

fn validate_binomial(k: u64, n: u64, alpha: f64) -> Result<(), AnalysisError> {
    check_open_probability(alpha)?;
    if n == 0 || k > n {
        return Err(AnalysisError::InvalidProbability);
    }
    Ok(())
}

fn log_binomial_cdf(k: u64, n: u64, probability: f64) -> f64 {
    let log_failure = (-probability).ln_1p();
    let log_odds = probability.ln() - log_failure;
    let mut log_term = count_f64(n) * log_failure;
    let mut log_sum = log_term;
    for successes in 1..=k {
        // Adjacent binomial masses differ by (n - i + 1) / i * p / (1 - p).
        log_term += count_f64(n - successes + 1).ln() - count_f64(successes).ln() + log_odds;
        let larger = log_sum.max(log_term);
        log_sum = larger + (-(log_sum - log_term).abs()).exp().ln_1p();
    }
    log_sum
}

/// Draw `clusters.len()` whole clusters with replacement for each statistic.
///
/// The blake3 derived-key XOF consumes little-endian seed and u64 words. Rejection
/// sampling discards the short remainder of the u64 range to avoid modulo bias.
/// The same seed and cluster ordering give the same draws, including across statistics.
///
/// # Errors
/// Returns `Empty` if the cluster collection is empty or `resamples` is zero.
pub fn cluster_bootstrap<T, F>(
    clusters: &[Vec<T>],
    resamples: u32,
    seed: u64,
    stat: F,
) -> Result<Vec<f64>, AnalysisError>
where
    F: Fn(&[&[T]]) -> f64,
{
    if clusters.is_empty() || resamples == 0 {
        return Err(AnalysisError::Empty);
    }
    let mut hasher = blake3::Hasher::new_derive_key("isingmark campaign cluster bootstrap v1");
    hasher.update(&seed.to_le_bytes());
    let mut stream = hasher.finalize_xof();
    let count = clusters.len() as u64;
    let rejected = count.wrapping_neg() % count;
    let mut sample = Vec::with_capacity(clusters.len());
    let mut statistics = Vec::with_capacity(resamples as usize);
    for _ in 0..resamples {
        sample.clear();
        while sample.len() < clusters.len() {
            let mut bytes = [0; 8];
            stream.fill(&mut bytes);
            let word = u64::from_le_bytes(bytes);
            if word >= rejected {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "The remainder is below clusters.len(), which fits usize."
                )]
                let index = (word % count) as usize;
                sample.push(clusters[index].as_slice());
            }
        }
        statistics.push(stat(&sample));
    }
    Ok(statistics)
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Binomial probabilities use f64 arithmetic, including rounded counts above 2^53."
)]
fn count_f64(count: u64) -> f64 {
    count as f64
}
