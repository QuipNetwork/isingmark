// SPDX-License-Identifier: AGPL-3.0-or-later
//! Instance-weighted wall-clock and device-budget success curves.

use std::{collections::BTreeMap, error::Error, fmt, time::Duration};

use crate::campaign::{
    contracts::TimeBasis,
    runner::{TerminalReason, Trial},
    timing::{budget_outcome, BudgetOutcome},
    trace::read_trace,
};

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
}

impl Default for AnalysisDesign {
    fn default() -> Self {
        Self {
            wall_grid_s: vec![1.0, 10.0, 30.0, 60.0, 1_200.0],
            device_grid_s: vec![0.1, 1.0, 10.0, 30.0],
            censoring: Censoring::FixedWindow,
        }
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
            Self::Empty => write!(formatter, "analysis requires trials and nonempty grids"),
            Self::CorruptTrace(detail) => write!(formatter, "corrupt campaign trace: {detail}"),
            Self::MixedBasis => write!(formatter, "FixedWindow requires equal wall-clock limits"),
            Self::InvalidProbability => write!(formatter, "probability must be in (0, 1]"),
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
/// Wall bands equal the point estimate. Device bands retain unresolved completion costs.
///
/// # Errors
/// Returns `Empty` for missing trials or grids, `MixedBasis` for unequal fixed windows,
/// and `CorruptTrace` for unreadable or partial traces, missing wall observations, or
/// device grid times that cannot be represented as a nanosecond budget.
pub fn estimate_curves(
    trials: &[Trial],
    design: &AnalysisDesign,
) -> Result<Vec<Curve>, AnalysisError> {
    if trials.is_empty() || design.wall_grid_s.is_empty() || design.device_grid_s.is_empty() {
        return Err(AnalysisError::Empty);
    }
    let mut instances: BTreeMap<&str, Vec<&Trial>> = BTreeMap::new();
    for trial in trials {
        let path = trial.events.display();
        let trace = read_trace(&trial.events)
            .map_err(|error| AnalysisError::CorruptTrace(format!("{path}: {error}")))?;
        if trace.truncated_tail {
            return Err(AnalysisError::CorruptTrace(format!(
                "{path}: truncated final event"
            )));
        }
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
    let wall_estimate = wall_estimates(&instances, &wall_grid, &design.censoring)?;
    let (device_estimate, device_upper) = device_estimates(&instances, &device_grid)?;
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
            lower: wall_estimate.clone(),
            upper: wall_estimate.clone(),
            estimate: wall_estimate,
            observation_limit_s: wall_limit,
            trials: trial_count,
            instances: instance_count,
            unresolved: 0,
            assumptions: vec![design.censoring.assumption()],
        },
        Curve {
            basis: TimeBasis::QpuDeviceAccess,
            grid_s: device_grid,
            lower: device_estimate.clone(),
            upper: device_upper,
            estimate: device_estimate,
            observation_limit_s: device_limit,
            trials: trial_count,
            instances: instance_count,
            unresolved,
            assumptions: vec![
                design.censoring.assumption(),
                "Device success is a joint condition with each trial's wall-clock cap and policy stops. \
                 Stops before the device budget is spent remain unresolved, never independent censors. \
                 The upper bound includes their instance-weighted share and unresolved timing."
                    .into(),
            ],
        },
    ])
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
    censoring: &Censoring,
) -> Result<Vec<f64>, AnalysisError> {
    let mut estimates = vec![0.0; grid.len()];
    for trials in instances.values() {
        let mut observations = trials
            .iter()
            .map(|trial| wall_observation(trial))
            .collect::<Result<Vec<_>, _>>()?;
        observations.sort_unstable_by_key(|&(time, _)| time);
        let mut next = 0;
        let mut successes = 0;
        let mut survival = 1.0;
        for (&time, estimate) in grid.iter().zip(&mut estimates) {
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
            *estimate += match censoring {
                Censoring::FixedWindow => count_f64(successes) / count_f64(trials.len()),
                Censoring::KaplanMeier { .. } => 1.0 - survival,
            };
        }
    }
    for estimate in &mut estimates {
        *estimate /= count_f64(instances.len());
    }
    Ok(estimates)
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
) -> Result<(Vec<f64>, Vec<f64>), AnalysisError> {
    let mut estimates = Vec::with_capacity(grid.len());
    let mut upper = Vec::with_capacity(grid.len());
    for &time in grid {
        let budget_ns = device_budget_ns(time)?;
        let mut estimate = 0.0;
        let mut possible = 0.0;
        for trials in instances.values() {
            let mut successes = 0;
            let mut unresolved = 0;
            for trial in trials {
                match device_result(trial, budget_ns) {
                    DeviceResult::Success => successes += 1,
                    DeviceResult::Unresolved => unresolved += 1,
                    DeviceResult::Failure => {}
                }
            }
            estimate += count_f64(successes) / count_f64(trials.len());
            possible += count_f64(successes + unresolved) / count_f64(trials.len());
        }
        estimates.push(estimate / count_f64(instances.len()));
        upper.push(possible / count_f64(instances.len()));
    }
    Ok((estimates, upper))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "In-memory trial and instance counts are below f64's exact integer range."
)]
fn count_f64(count: usize) -> f64 {
    count as f64
}
