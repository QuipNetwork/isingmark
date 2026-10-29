// SPDX-License-Identifier: AGPL-3.0-or-later
//! Instance-weighted wall-clock and device-budget success curves.

use std::{collections::BTreeMap, error::Error, fmt, time::Duration};

use crate::campaign::{
    contracts::TimeBasis,
    runner::{TerminalReason, Trial},
    timing::{budget_outcome, BudgetOutcome},
    trace::read_trace,
};

use super::confidence::{binomial_lower, binomial_upper, cluster_bootstrap};

#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    pub basis: TimeBasis,
    pub grid_s: Vec<f64>,
    /// Success probability, F(t) = 1 - S(t).
    pub estimate: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    pub observation_limit_s: f64,
    pub trials: u64,
    pub instances: u64,
    /// Trial count unresolved at the observation limit, without instance weighting.
    pub unresolved: u64,
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisDesign {
    pub wall_grid_s: Vec<f64>,
    pub device_grid_s: Vec<f64>,
    pub censoring: Censoring,
    pub bootstrap_resamples: u32,
    pub bootstrap_seed: u64,
}

impl Default for AnalysisDesign {
    fn default() -> Self {
        Self {
            wall_grid_s: vec![1.0, 10.0, 30.0, 60.0, 1_200.0],
            device_grid_s: vec![0.1, 1.0, 10.0, 30.0],
            censoring: Censoring::FixedWindow,
            bootstrap_resamples: 2_000,
            bootstrap_seed: 0,
        }
    }
}

impl AnalysisDesign {
    fn band_assumption(&self, instances: u64) -> String {
        format!(
            "Pointwise two-sided 95% instance cluster bootstrap bands, nearest rank percentiles, \
             alpha=0.025 per side, resamples={}, seed={}. Exact Clopper-Pearson endpoint \
             overrides use independent n={instances} instances.",
            self.bootstrap_resamples, self.bootstrap_seed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Censoring {
    FixedWindow,
    KaplanMeier { justification: String },
}

impl Censoring {
    fn assumption(&self) -> String {
        match self {
            Self::FixedWindow => {
                "FixedWindow: equal administrative wall-clock windows justify the empirical success CDF."
                    .into()
            }
            Self::KaplanMeier { justification } => {
                format!("KaplanMeier wall-clock censoring justification: {justification}")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Quantile {
    Identified(f64),
    Above(f64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    Empty,
    CorruptTrace(String),
    MixedBasis,
    InvalidProbability,
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(
                formatter,
                "analysis requires observations, grids, and resamples"
            ),
            Self::CorruptTrace(detail) => write!(formatter, "corrupt campaign trace: {detail}"),
            Self::MixedBasis => write!(formatter, "FixedWindow requires equal wall-clock limits"),
            Self::InvalidProbability => write!(formatter, "invalid probability or binomial count"),
        }
    }
}

impl Error for AnalysisError {}

/// Validate a requested success probability.
///
/// # Errors
/// Returns `AnalysisError::InvalidProbability` for values outside (0, 1], including NaN.
pub fn check_probability(q: f64) -> Result<(), AnalysisError> {
    if q > 0.0 && q <= 1.0 {
        Ok(())
    } else {
        Err(AnalysisError::InvalidProbability)
    }
}

/// Return the first observed grid time reaching `q`, or a bound at the observation limit.
/// Call [`check_probability`] first when the probability comes from user input.
#[must_use]
pub fn quantile(curve: &Curve, q: f64) -> Quantile {
    for (&time, &estimate) in curve.grid_s.iter().zip(&curve.estimate) {
        if estimate >= q {
            return Quantile::Identified(time);
        }
    }
    Quantile::Above(curve.observation_limit_s)
}

/// Estimate wall-clock and QPU device-budget success, giving each instance equal weight.
/// Pointwise 95% cluster bands use exact overrides for zero or complete success.
/// Device upper bands also include unresolved completion costs before exact overrides.
///
/// # Errors
/// Returns `Empty` for missing trials, grids, or resamples, `MixedBasis` for unequal fixed windows,
/// and `CorruptTrace` for unreadable or partial traces, missing wall observations, or
/// device grid times that cannot be represented as a nanosecond budget.
pub fn estimate_curves(
    trials: &[Trial],
    design: &AnalysisDesign,
) -> Result<Vec<Curve>, AnalysisError> {
    if trials.is_empty()
        || design.wall_grid_s.is_empty()
        || design.device_grid_s.is_empty()
        || design.bootstrap_resamples == 0
    {
        return Err(AnalysisError::Empty);
    }
    let mut instances: BTreeMap<&str, Vec<&Trial>> = BTreeMap::new();
    for trial in trials {
        validate_trace(trial)?;
        // Validate before checking window equality so a missing observation stays corrupt.
        wall_observation(trial)?;
        instances.entry(&trial.instance_id).or_default().push(trial);
    }
    let wall_limit = trials
        .iter()
        .filter_map(|trial| trial.wall_limit)
        .max()
        .ok_or_else(|| AnalysisError::CorruptTrace("no wall-clock observation limit".into()))?
        .as_secs_f64();
    if design.censoring == Censoring::FixedWindow
        && trials
            .iter()
            .any(|trial| trial.wall_limit != trials[0].wall_limit)
    {
        return Err(AnalysisError::MixedBasis);
    }
    let device_limit = *design.device_grid_s.last().ok_or(AnalysisError::Empty)?;
    let wall_grid = curve_grid(
        &design.wall_grid_s,
        trials.iter().filter_map(|trial| {
            if trial.terminal == TerminalReason::Accepted {
                trial.accepted_wall.map(|time| time.as_secs_f64())
            } else {
                None
            }
        }),
        wall_limit,
    );
    let device_grid = curve_grid(
        &design.device_grid_s,
        trials.iter().filter_map(|trial| {
            let upper = trial.device_total.upper_ns?;
            if trial.terminal == TerminalReason::Accepted
                && budget_outcome(&trial.device_total, upper) == BudgetOutcome::Within
            {
                Some(Duration::from_nanos(upper).as_secs_f64())
            } else {
                None
            }
        }),
        device_limit,
    );
    let wall_points = wall_estimates(&instances, &wall_grid, design)?;
    let device_points = device_estimates(&instances, &device_grid, design)?;
    let device_limit_ns = device_budget_ns(device_limit)?;
    let unresolved = trials
        .iter()
        .filter(|trial| device_result(trial, device_limit_ns) == DeviceResult::Unresolved)
        .count() as u64;
    let trial_count = trials.len() as u64;
    let instance_count = instances.len() as u64;
    Ok(vec![
        Curve {
            basis: TimeBasis::WallClock,
            grid_s: wall_grid,
            lower: wall_points.iter().map(|point| point.lower).collect(),
            upper: wall_points.iter().map(|point| point.upper).collect(),
            estimate: wall_points.iter().map(|point| point.estimate).collect(),
            observation_limit_s: wall_limit,
            trials: trial_count,
            instances: instance_count,
            unresolved: 0,
            assumptions: vec![
                design.censoring.assumption(),
                design.band_assumption(instance_count),
            ],
        },
        Curve {
            basis: TimeBasis::QpuDeviceAccess,
            grid_s: device_grid,
            lower: device_points.iter().map(|point| point.lower).collect(),
            upper: device_points.iter().map(|point| point.upper).collect(),
            estimate: device_points.iter().map(|point| point.estimate).collect(),
            observation_limit_s: device_limit,
            trials: trial_count,
            instances: instance_count,
            unresolved,
            assumptions: vec![
                design.censoring.assumption(),
                design.band_assumption(instance_count),
                "Device success is a joint condition with each trial's wall-clock cap and policy stops. \
                 Stops before the device budget is spent remain unresolved, never independent censors. \
                 The upper statistic includes their instance-weighted share and unresolved timing. \
                 Exact endpoint overrides replace bootstrap bounds."
                    .into(),
            ],
        },
    ])
}

fn validate_trace(trial: &Trial) -> Result<(), AnalysisError> {
    let path = trial.events.display();
    let trace = read_trace(&trial.events)
        .map_err(|error| AnalysisError::CorruptTrace(format!("{path}: {error}")))?;
    if trace.truncated_tail {
        return Err(AnalysisError::CorruptTrace(format!(
            "{path}: truncated final event"
        )));
    }
    Ok(())
}

fn curve_grid(design: &[f64], successes: impl Iterator<Item = f64>, limit: f64) -> Vec<f64> {
    let mut grid: Vec<_> = design
        .iter()
        .copied()
        .chain(successes)
        .filter(|time| *time <= limit)
        .collect();
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    grid
}

fn wall_observation(trial: &Trial) -> Result<(Duration, bool), AnalysisError> {
    let accepted = trial.terminal == TerminalReason::Accepted;
    let time = if accepted {
        trial.accepted_wall
    } else {
        trial.wall_limit
    };
    time.map(|time| (time, accepted)).ok_or_else(|| {
        AnalysisError::CorruptTrace(format!("trial {} has no wall observation", trial.trial_id))
    })
}

fn wall_estimates(
    instances: &BTreeMap<&str, Vec<&Trial>>,
    grid: &[f64],
    design: &AnalysisDesign,
) -> Result<Vec<CurvePoint>, AnalysisError> {
    let mut clusters = Vec::with_capacity(instances.len());
    for trials in instances.values() {
        let mut observations = trials
            .iter()
            .map(|trial| wall_observation(trial))
            .collect::<Result<Vec<_>, _>>()?;
        observations.sort_unstable_by_key(|&(time, _)| time);
        clusters.push(observations);
    }
    let mut points = Vec::with_capacity(grid.len());
    for &time in grid {
        let statistic = |sample: &[&[(Duration, bool)]]| {
            sample
                .iter()
                .map(|observations| wall_probability(observations, time, &design.censoring))
                .sum::<f64>()
                / count_f64(sample.len())
        };
        // KM can reach one after censoring without every trial having succeeded.
        let all_succeeded = clusters
            .iter()
            .flatten()
            .all(|&(observed, accepted)| accepted && observed.as_secs_f64() <= time);
        points.push(point_bands(
            &clusters,
            design,
            statistic,
            statistic,
            all_succeeded,
        )?);
    }
    Ok(points)
}

fn wall_probability(observations: &[(Duration, bool)], time: f64, censoring: &Censoring) -> f64 {
    let mut next = 0;
    let mut successes = 0;
    let mut survival = 1.0;
    while next < observations.len() && observations[next].0.as_secs_f64() <= time {
        let start = next;
        let event_time = observations[next].0;
        let mut events = 0;
        while next < observations.len() && observations[next].0 == event_time {
            events += usize::from(observations[next].1);
            next += 1;
        }
        successes += events;
        // Censors tied with events leave the risk set after this factor.
        survival *= 1.0 - count_f64(events) / count_f64(observations.len() - start);
    }
    match censoring {
        Censoring::FixedWindow => count_f64(successes) / count_f64(observations.len()),
        Censoring::KaplanMeier { .. } => 1.0 - survival,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceResult {
    Success,
    Failure,
    Unresolved,
}

fn device_result(trial: &Trial, budget_ns: u64) -> DeviceResult {
    match trial.terminal {
        TerminalReason::Accepted => match budget_outcome(&trial.device_total, budget_ns) {
            BudgetOutcome::Within => DeviceResult::Success,
            BudgetOutcome::Exceeded => DeviceResult::Failure,
            BudgetOutcome::Unresolved => DeviceResult::Unresolved,
        },
        TerminalReason::DeviceBudgetExhausted => DeviceResult::Failure,
        TerminalReason::WallTimeout
        | TerminalReason::HostComputeLimit
        | TerminalReason::EvaluationLimit
        | TerminalReason::MinerCrashed
        | TerminalReason::Cancelled
        | TerminalReason::InvalidRun => {
            if trial.device_total.lower_ns >= budget_ns {
                DeviceResult::Failure
            } else {
                DeviceResult::Unresolved
            }
        }
    }
}

fn device_budget_ns(seconds: f64) -> Result<u64, AnalysisError> {
    let invalid = || {
        AnalysisError::CorruptTrace(format!(
            "device grid time {seconds} is outside the budget range"
        ))
    };
    let duration = Duration::try_from_secs_f64(seconds).map_err(|_| invalid())?;
    u64::try_from(duration.as_nanos()).map_err(|_| invalid())
}

fn device_estimates(
    instances: &BTreeMap<&str, Vec<&Trial>>,
    grid: &[f64],
    design: &AnalysisDesign,
) -> Result<Vec<CurvePoint>, AnalysisError> {
    let clusters: Vec<_> = instances.values().cloned().collect();
    let mut points = Vec::with_capacity(grid.len());
    for &time in grid {
        let budget_ns = device_budget_ns(time)?;
        let statistic = |sample: &[&[&Trial]], include_unresolved| {
            sample
                .iter()
                .map(|trials| device_probability(trials, budget_ns, include_unresolved))
                .sum::<f64>()
                / count_f64(sample.len())
        };
        let all_succeeded = clusters
            .iter()
            .flatten()
            .all(|trial| device_result(trial, budget_ns) == DeviceResult::Success);
        points.push(point_bands(
            &clusters,
            design,
            |sample| statistic(sample, false),
            |sample| statistic(sample, true),
            all_succeeded,
        )?);
    }
    Ok(points)
}

fn device_probability(trials: &[&Trial], budget_ns: u64, include_unresolved: bool) -> f64 {
    let count = trials
        .iter()
        .filter(|trial| match device_result(trial, budget_ns) {
            DeviceResult::Success => true,
            DeviceResult::Unresolved => include_unresolved,
            DeviceResult::Failure => false,
        })
        .count();
    count_f64(count) / count_f64(trials.len())
}

struct CurvePoint {
    estimate: f64,
    lower: f64,
    upper: f64,
}

fn point_bands<T>(
    clusters: &[Vec<T>],
    design: &AnalysisDesign,
    success: impl Fn(&[&[T]]) -> f64,
    possible: impl Fn(&[&[T]]) -> f64,
    all_succeeded: bool,
) -> Result<CurvePoint, AnalysisError> {
    let original: Vec<_> = clusters.iter().map(Vec::as_slice).collect();
    let estimate = success(&original).clamp(0.0, 1.0);
    let independent = clusters.len() as u64;
    let (lower, upper) = if estimate <= 0.0 {
        let exact = binomial_upper(0, independent, 0.025)?;
        let upper = if possible(&original) > 0.0 {
            let mut possibilities = cluster_bootstrap(
                clusters,
                design.bootstrap_resamples,
                design.bootstrap_seed,
                possible,
            )?;
            exact.max(nearest_rank(&mut possibilities, 39))
        } else {
            exact
        };
        (0.0, upper.clamp(0.0, 1.0))
    } else if all_succeeded {
        (binomial_lower(independent, independent, 0.025)?, 1.0)
    } else {
        // Restarting the stream pairs draws across grid points and both statistics.
        let mut successes = cluster_bootstrap(
            clusters,
            design.bootstrap_resamples,
            design.bootstrap_seed,
            success,
        )?;
        let mut possibilities = cluster_bootstrap(
            clusters,
            design.bootstrap_resamples,
            design.bootstrap_seed,
            possible,
        )?;
        (
            nearest_rank(&mut successes, 1),
            nearest_rank(&mut possibilities, 39),
        )
    };
    Ok(CurvePoint {
        estimate,
        lower: lower.clamp(0.0, estimate),
        upper: upper.clamp(estimate, 1.0),
    })
}

// Nearest rank: the one-based ceil(m * p)-th order statistic, without interpolation.
// Callers supply nonempty bootstrap output and p = 1/40 or 39/40 (2.5% or 97.5%).
fn nearest_rank(samples: &mut [f64], fortieths: u32) -> f64 {
    samples.sort_unstable_by(f64::total_cmp);
    let rank = (samples.len() as u64 * u64::from(fortieths)).div_ceil(40);
    let index = usize::try_from(rank - 1).expect("percentile rank is within the sample");
    samples[index]
}

#[expect(
    clippy::cast_precision_loss,
    reason = "In-memory trial and instance counts are below f64's exact integer range."
)]
fn count_f64(count: usize) -> f64 {
    count as f64
}

#[cfg(test)]
mod tests {
    use super::nearest_rank;

    #[test]
    fn percentile_uses_one_based_nearest_rank_without_interpolation() {
        let mut forty: Vec<_> = (0..40).rev().map(f64::from).collect();
        assert!(nearest_rank(&mut forty, 1).abs() < f64::EPSILON);
        assert!((nearest_rank(&mut forty, 39) - 38.0).abs() < f64::EPSILON);
        let mut forty_one: Vec<_> = (0..41).rev().map(f64::from).collect();
        assert!((nearest_rank(&mut forty_one, 1) - 1.0).abs() < f64::EPSILON);
        assert!((nearest_rank(&mut forty_one, 39) - 39.0).abs() < f64::EPSILON);
    }
}
