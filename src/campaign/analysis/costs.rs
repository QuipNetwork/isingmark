// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact binomial power planning, cluster inflation, and campaign resource checks.

use super::{
    confidence::{binomial_lower, binomial_upper, n_required},
    survival::AnalysisError,
};

const MAX_TRIALS: u64 = 10_000_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Guides {
    pub qpu_device_lower: f64,
    pub classical_wall_upper: f64,
    pub race_lower: f64,
    pub alpha: f64,
}

impl Default for Guides {
    fn default() -> Self {
        Self {
            qpu_device_lower: 0.95,
            classical_wall_upper: 0.01,
            race_lower: 0.90,
            alpha: 0.05,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alternatives {
    pub classical_p: f64,
    pub qpu_p: f64,
    pub race_p: f64,
    pub power: f64,
}

impl Default for Alternatives {
    fn default() -> Self {
        Self {
            classical_p: 0.001,
            qpu_p: 0.99,
            race_p: 0.95,
            power: 0.80,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PilotSummary {
    pub trials_per_cluster: f64,
    pub intracluster_correlation: f64,
    pub mean_trial_wall_s: f64,
    pub mean_trial_device_s: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResourceLimits {
    pub cpu_hours: f64,
    pub gpu_hours: f64,
    pub qpu_access_s: f64,
    pub elapsed_hours: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceOutcome {
    Affordable,
    InsufficientBudget { short: Vec<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvidencePlan {
    pub classical_trials: u64,
    pub qpu_trials: u64,
    pub race_trials: u64,
    pub design_effect: f64,
    /// Classical occupancy in hours, checked against both CPU and GPU limits.
    pub device_hours: f64,
    pub qpu_access_s: f64,
    pub outcome: EvidenceOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Classical,
    Qpu,
    Race,
}

/// Plan the smallest independent samples reaching the requested exact power,
/// then inflate each sample by the pilot design effect and round upward.
///
/// The elapsed estimate assumes serial execution: classical occupancy plus
/// the full wall time of every QPU and race trial. No overlap is credited.
/// The design effect is a planning approximation, not a cluster power simulation.
///
/// # Errors
/// Returns `InvalidProbability` for invalid probabilities, nonfinite or negative
/// times or limits, cluster sizes below one, ICC outside [0, 1], or count overflow.
/// Returns `NoSizeReachesPower` if no size at or below 10,000,000 reaches
/// the requested power.
pub fn plan_evidence(
    guides: &Guides,
    alternatives: &Alternatives,
    pilot: &PilotSummary,
    limits: &ResourceLimits,
) -> Result<EvidencePlan, AnalysisError> {
    validate_guides(guides)?;
    for p in [
        alternatives.classical_p,
        alternatives.qpu_p,
        alternatives.race_p,
    ] {
        validate_probability(p)?;
    }
    if !(alternatives.power > 0.0
        && alternatives.power <= 1.0
        && pilot.trials_per_cluster.is_finite()
        && pilot.trials_per_cluster >= 1.0
        && (0.0..=1.0).contains(&pilot.intracluster_correlation))
    {
        return Err(AnalysisError::InvalidProbability);
    }
    for value in [
        pilot.mean_trial_wall_s,
        pilot.mean_trial_device_s,
        limits.cpu_hours,
        limits.gpu_hours,
        limits.qpu_access_s,
        limits.elapsed_hours,
    ] {
        if !value.is_finite() || value < 0.0 {
            return Err(AnalysisError::InvalidProbability);
        }
    }
    let design_effect = 1.0 + (pilot.trials_per_cluster - 1.0) * pilot.intracluster_correlation;
    let mut sizes = [0; 3];
    for (index, (gate, p)) in [
        (Gate::Classical, alternatives.classical_p),
        (Gate::Qpu, alternatives.qpu_p),
        (Gate::Race, alternatives.race_p),
    ]
    .into_iter()
    .enumerate()
    {
        let independent = required_trials(gate, p, alternatives.power, guides)?;
        sizes[index] = rounded_trials(count_f64(independent) * design_effect)?;
    }
    let [classical_trials, qpu_trials, race_trials] = sizes;
    let qpu_and_race = qpu_trials
        .checked_add(race_trials)
        .ok_or(AnalysisError::InvalidProbability)?;
    let classical_hours = device_hours(classical_trials, pilot.mean_trial_wall_s);
    let qpu_access_s = count_f64(qpu_and_race) * pilot.mean_trial_device_s;
    let elapsed_hours = classical_hours + device_hours(qpu_and_race, pilot.mean_trial_wall_s);
    if !classical_hours.is_finite() || !qpu_access_s.is_finite() || !elapsed_hours.is_finite() {
        return Err(AnalysisError::InvalidProbability);
    }
    let mut short = Vec::new();
    for (name, required, available) in [
        ("cpu_hours", classical_hours, limits.cpu_hours),
        ("gpu_hours", classical_hours, limits.gpu_hours),
        ("qpu_access_s", qpu_access_s, limits.qpu_access_s),
        ("elapsed_hours", elapsed_hours, limits.elapsed_hours),
    ] {
        if required > available {
            short.push(name.to_owned());
        }
    }
    let outcome = if short.is_empty() {
        EvidenceOutcome::Affordable
    } else {
        EvidenceOutcome::InsufficientBudget { short }
    };
    Ok(EvidencePlan {
        classical_trials,
        qpu_trials,
        race_trials,
        design_effect,
        device_hours: classical_hours,
        qpu_access_s,
        outcome,
    })
}

/// Probability that the one-sided exact gate passes in `n` independent trials.
/// Classical successes must have an upper bound at or below their guide.
/// QPU successes and race wins must have a lower bound at or above their guide.
///
/// # Errors
/// Returns `InvalidProbability` if `n` is zero, `p` is outside [0, 1],
/// or any guide or `alpha` is outside (0, 1).
pub fn gate_power(gate: Gate, n: u64, p: f64, guides: &Guides) -> Result<f64, AnalysisError> {
    validate_guides(guides)?;
    validate_probability(p)?;
    if n == 0 {
        return Err(AnalysisError::InvalidProbability);
    }
    let Some(k) = passing_count(gate, n, guides)? else {
        return Ok(0.0);
    };
    let (event_p, other_p) = match gate {
        Gate::Classical => (p, 1.0 - p),
        Gate::Qpu | Gate::Race => (1.0 - p, p),
    };
    if other_p == 0.0 {
        return Ok(0.0);
    }
    if event_p == 0.0 {
        return Ok(1.0);
    }
    let (log_event, log_other) = match gate {
        Gate::Classical => (p.ln(), (-p).ln_1p()),
        Gate::Qpu | Gate::Race => ((-p).ln_1p(), p.ln()),
    };
    // Sum the smaller tail, reflecting successes and failures when necessary.
    let power = if count_f64(k) < count_f64(n) * event_p {
        log_lower_tail(k, n, log_event, log_other).exp()
    } else {
        -log_lower_tail(n - k - 1, n, log_other, log_event).exp_m1()
    };
    Ok(power.clamp(0.0, 1.0))
}

/// Success probability for independent devices at the same observation horizon.
#[must_use]
pub fn fleet_success(p: f64, devices: u64) -> f64 {
    if devices == 0 {
        return 0.0;
    }
    -(count_f64(devices) * (-p).ln_1p()).exp_m1()
}

/// Full-trial resource occupancy in hours.
#[must_use]
pub fn device_hours(trials: u64, window_s: f64) -> f64 {
    count_f64(trials) * window_s / 3_600.0
}

fn validate_guides(guides: &Guides) -> Result<(), AnalysisError> {
    for p in [
        guides.classical_wall_upper,
        guides.qpu_device_lower,
        guides.race_lower,
        guides.alpha,
    ] {
        if !(p > 0.0 && p < 1.0) {
            return Err(AnalysisError::InvalidProbability);
        }
    }
    Ok(())
}

fn validate_probability(p: f64) -> Result<(), AnalysisError> {
    if (0.0..=1.0).contains(&p) {
        Ok(())
    } else {
        Err(AnalysisError::InvalidProbability)
    }
}

fn required_trials(gate: Gate, p: f64, power: f64, guides: &Guides) -> Result<u64, AnalysisError> {
    let first = match gate {
        Gate::Classical => n_required(guides.classical_wall_upper, guides.alpha)?,
        Gate::Qpu => rounded_trials(guides.alpha.ln() / guides.qpu_device_lower.ln())?,
        Gate::Race => rounded_trials(guides.alpha.ln() / guides.race_lower.ln())?,
    };
    // Exact binomial power has downward jumps. A binary search over n is invalid.
    for n in first..=MAX_TRIALS {
        if gate_power(gate, n, p, guides)? >= power {
            return Ok(n);
        }
    }
    Err(AnalysisError::NoSizeReachesPower)
}

// Count classical successes or, for lower-bound gates, failures. In either case
// the passing set starts at zero. Exponential bracketing keeps rare counts small,
// then bisection finds the last passing count without checking every outcome.
fn passing_count(gate: Gate, n: u64, guides: &Guides) -> Result<Option<u64>, AnalysisError> {
    let passes = |events| match gate {
        Gate::Classical => {
            Ok(binomial_upper(events, n, guides.alpha)? <= guides.classical_wall_upper)
        }
        Gate::Qpu => Ok(binomial_lower(n - events, n, guides.alpha)? >= guides.qpu_device_lower),
        Gate::Race => Ok(binomial_lower(n - events, n, guides.alpha)? >= guides.race_lower),
    };
    if !passes(0)? {
        return Ok(None);
    }
    // All n events fail because guides are strictly between zero and one.
    let (mut low, mut high) = (0, 1);
    while high < n && passes(high)? {
        low = high;
        high = high.saturating_mul(2).min(n);
    }
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if passes(middle)? {
            low = middle;
        } else {
            high = middle;
        }
    }
    Ok(Some(low))
}

// Log P(X <= k), called only on the side below the binomial mean. Starting at k
// prevents underflow at zero and makes the remaining terms decrease in size.
fn log_lower_tail(mut k: u64, n: u64, log_p: f64, log_q: f64) -> f64 {
    let mut term = log_factorial(n) - log_factorial(k) - log_factorial(n - k)
        + count_f64(k) * log_p
        + count_f64(n - k) * log_q;
    let mut sum = term;
    while k > 0 {
        let log_ratio = count_f64(k).ln() - count_f64(n - k + 1).ln() + log_q - log_p;
        term += log_ratio;
        let larger = sum.max(term);
        sum = larger + (-(sum - term).abs()).exp().ln_1p();
        k -= 1;
        // Subsequent ratios only decrease. Bound their geometric remainder
        // before dropping terms below relative machine precision.
        if log_ratio < 0.0 && term - sum - (-log_ratio.exp()).ln_1p() < f64::EPSILON.ln() {
            break;
        }
    }
    sum.min(0.0)
}

// ln(n!) = ln Gamma(n + 1). Sum small factorials directly. Above 32 use
// Stirling's expansion through n^-9, whose next term is below 1e-19 at 32.
fn log_factorial(n: u64) -> f64 {
    if n <= 32 {
        let mut sum = 0.0;
        for i in 2..=n {
            sum += count_f64(i).ln();
        }
        return sum;
    }
    let x = count_f64(n);
    let inverse = x.recip();
    let square = inverse * inverse;
    let correction = inverse
        * (1.0 / 12.0
            + square
                * (-1.0 / 360.0
                    + square * (1.0 / 1_260.0 + square * (-1.0 / 1_680.0 + square / 1_188.0))));
    (x + 0.5) * x.ln() - x + std::f64::consts::TAU.ln() / 2.0 + correction
}

// Use the same four-relative-epsilon integer guard as confidence::n_required.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "The finite, positive rounded count is checked against the u64 range before casting."
)]
fn rounded_trials(value: f64) -> Result<u64, AnalysisError> {
    let nearest = value.round();
    let rounded = if (value - nearest).abs() <= 4.0 * f64::EPSILON * value {
        nearest
    } else {
        value.ceil()
    };
    if !rounded.is_finite() || rounded < 0.0 || rounded >= count_f64(u64::MAX) {
        return Err(AnalysisError::InvalidProbability);
    }
    Ok((rounded as u64).max(1))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Binomial calculations use f64, including rounded counts above 2^53."
)]
fn count_f64(count: u64) -> f64 {
    count as f64
}

#[cfg(test)]
mod tests {
    use super::log_factorial;

    #[test]
    fn log_factorials_match_reference_values() {
        for (n, expected) in [
            (0, 0.0),
            (1, 0.0),
            (10, 15.104_412_573_075_516),
            (32, 81.557_959_456_115_04),
            (33, 85.054_467_017_581_52),
            (100, 363.739_375_555_563_47),
            (1_000, 5_912.128_178_488_163),
        ] {
            let actual = log_factorial(n);
            assert!(
                (actual - expected).abs() < 2e-12,
                "{n}: {actual} != {expected}"
            );
        }
    }
}
