// SPDX-License-Identifier: AGPL-3.0-or-later
//! Suite-fit A/B comparisons beside the campaign probability analysis.
//!
//! Each arm keeps the instance-weighted wall and device curves of [`estimate_curves`]. A paired
//! A−B difference exists only for a matched pair whose arms both hold campaign trials. It resamples
//! complete groups jointly across both arms. A Tier label from [`tier`] is historical context only.

pub mod table;
pub mod tier;

use std::{collections::BTreeMap, time::Duration};

use serde::Serialize;

use crate::campaign::{contracts::TimeBasis, runner::Trial};

use super::{
    confidence::{binomial_lower, binomial_upper, cluster_bootstrap},
    survival::{
        device_budget_ns, device_result, estimate_curves, nearest_rank, validate_trace,
        wall_observation, wall_probability, AnalysisDesign, AnalysisError, Censoring, Curve,
        DeviceResult,
    },
};

/// The physical pairs compare archived A cells with B lattices of other sizes. They stay
/// `unmatched_size` until a matched A cohort clears the size gate.
pub const PHYSICAL_PAIRS: [&str; 5] = ["p-charge", "d-charge", "f-h500", "f-h1000", "f-routing"];

/// Instance id to resampling group: related templates, cost-scaled F requests, shared A runs,
/// or sessions. An instance without an entry is its own group.
pub type Groups = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Side {
    A,
    B,
}

/// Synthetic pairs report the native target and the common A target as separate panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetPanel {
    Native,
    Common,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SizeStatus {
    Matched,
    UnmatchedSize,
}

/// The identity of one arm of a pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArmSpec {
    pub side: Side,
    pub family: String,
    pub family_revision: String,
    pub parameter_digest: String,
    pub policy_id: String,
    pub size_spins: u64,
    pub target_rule: String,
    pub panel: TargetPanel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PairSpec {
    pub pair_id: String,
    pub a: ArmSpec,
    pub b: ArmSpec,
}

impl PairSpec {
    /// Physical pairs are always unmatched. Other pairs match when both arms have the same size.
    #[must_use]
    pub fn size_status(&self) -> SizeStatus {
        if PHYSICAL_PAIRS.contains(&self.pair_id.as_str()) || self.a.size_spins != self.b.size_spins
        {
            SizeStatus::UnmatchedSize
        } else {
            SizeStatus::Matched
        }
    }
}

/// Why a pair has no A−B difference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotComparable {
    UnmatchedSize,
    PanelMismatch,
    /// One arm is an archived reference with no campaign trials.
    ArchivedArm,
    NoCommonGroups,
}

/// A pointwise A−B success difference on common groups.
#[derive(Debug, Clone, PartialEq)]
pub struct Difference {
    pub basis: TimeBasis,
    pub grid_s: Vec<f64>,
    pub estimate: Vec<f64>,
    pub lower: Vec<f64>,
    pub upper: Vec<f64>,
    /// Groups with trials in both arms. Only these enter the difference.
    pub groups: u64,
    pub a_only: u64,
    pub b_only: u64,
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DifferenceOutcome {
    Estimated(Vec<Difference>),
    NotComparable(NotComparable),
}

/// Copies of `trials` whose instance id is their group, so a group counts as one observation.
#[must_use]
pub fn grouped(trials: &[Trial], groups: &Groups) -> Vec<Trial> {
    trials
        .iter()
        .map(|trial| {
            let mut trial = trial.clone();
            if let Some(group) = groups.get(&trial.instance_id) {
                trial.instance_id.clone_from(group);
            }
            trial
        })
        .collect()
}

/// The wall and device curves of one campaign arm, weighted by group.
///
/// # Errors
/// Returns the errors of [`estimate_curves`].
pub fn arm_curves(
    trials: &[Trial],
    groups: &Groups,
    design: &AnalysisDesign,
) -> Result<Vec<Curve>, AnalysisError> {
    estimate_curves(&grouped(trials, groups), design)
}

/// Success and possible success of one arm in one group at one grid point.
#[derive(Debug, Clone, Copy)]
struct ArmPoint {
    success: f64,
    possible: f64,
}

/// One group at one grid point: A, then B.
type GroupPoint = (ArmPoint, ArmPoint);

/// The A−B wall and device success differences over groups present in both arms.
///
/// Wall differences use the design's censoring within each group. Device differences treat an
/// unresolved trial as a failure for the estimate and as either outcome for the band. Bands are
/// pointwise two-sided 95 percent nearest-rank intervals from joint group resampling.
///
/// # Errors
/// Returns `CorruptTrace` for an invalid trial, `MixedBasis` for unequal fixed windows across the
/// arms, `Empty` without grids or resamples, and binomial errors from the exact endpoint bounds.
pub fn paired_difference(
    pair: &PairSpec,
    a: &[Trial],
    b: &[Trial],
    groups: &Groups,
    design: &AnalysisDesign,
) -> Result<DifferenceOutcome, AnalysisError> {
    if pair.size_status() == SizeStatus::UnmatchedSize {
        return Ok(DifferenceOutcome::NotComparable(
            NotComparable::UnmatchedSize,
        ));
    }
    if pair.a.panel != pair.b.panel {
        return Ok(DifferenceOutcome::NotComparable(
            NotComparable::PanelMismatch,
        ));
    }
    if design.wall_grid_s.is_empty()
        || design.device_grid_s.is_empty()
        || design.bootstrap_resamples == 0
    {
        return Err(AnalysisError::Empty);
    }
    for trial in a.iter().chain(b) {
        validate_trace(trial)?;
        wall_observation(trial)?;
    }
    let mut limits = a.iter().chain(b).map(|trial| trial.wall_limit);
    let first = limits.next().flatten();
    if design.censoring == Censoring::FixedWindow && limits.any(|limit| limit != first) {
        return Err(AnalysisError::MixedBasis);
    }
    let (common, a_only, b_only) = split_groups(a, b, groups);
    if common.is_empty() {
        return Ok(DifferenceOutcome::NotComparable(
            NotComparable::NoCommonGroups,
        ));
    }
    let wall_limit = a
        .iter()
        .chain(b)
        .filter_map(|trial| trial.wall_limit)
        .max()
        .map_or(0.0, |limit| limit.as_secs_f64());
    let wall_grid: Vec<f64> = design
        .wall_grid_s
        .iter()
        .copied()
        .filter(|time| *time <= wall_limit)
        .collect();
    let wall = wall_points(&common, &wall_grid, &design.censoring)?;
    let device = device_points(&common, &design.device_grid_s)?;
    let groups_count = common.len() as u64;
    let difference = |basis, grid_s: &[f64], clusters: &[Vec<GroupPoint>], extra: &str| {
        let mut out = Difference {
            basis,
            grid_s: grid_s.to_vec(),
            estimate: Vec::with_capacity(grid_s.len()),
            lower: Vec::with_capacity(grid_s.len()),
            upper: Vec::with_capacity(grid_s.len()),
            groups: groups_count,
            a_only,
            b_only,
            assumptions: vec![
                difference_assumption(design, groups_count),
                extra.to_owned(),
            ],
        };
        for index in 0..grid_s.len() {
            let (estimate, lower, upper) = difference_point(clusters, index, design)?;
            out.estimate.push(estimate);
            out.lower.push(lower);
            out.upper.push(upper);
        }
        Ok::<_, AnalysisError>(out)
    };
    Ok(DifferenceOutcome::Estimated(vec![
        difference(
            TimeBasis::WallClock,
            &wall_grid,
            &wall,
            "Wall success uses the design censoring within each group.",
        )?,
        difference(
            TimeBasis::QpuDeviceAccess,
            &design.device_grid_s,
            &device,
            "An unresolved device trial is a failure in the estimate and either outcome in the band.",
        )?,
    ]))
}

/// Groups with trials in both arms, then the counts of groups in only A and only B.
fn split_groups<'a>(
    a: &'a [Trial],
    b: &'a [Trial],
    groups: &Groups,
) -> (Vec<GroupTrials<'a>>, u64, u64) {
    let mut by_group: BTreeMap<String, GroupTrials<'a>> = BTreeMap::new();
    for trial in a {
        by_group
            .entry(group_of(trial, groups))
            .or_default()
            .0
            .push(trial);
    }
    for trial in b {
        by_group
            .entry(group_of(trial, groups))
            .or_default()
            .1
            .push(trial);
    }
    let a_only = by_group.values().filter(|(_, b)| b.is_empty()).count() as u64;
    let b_only = by_group.values().filter(|(a, _)| a.is_empty()).count() as u64;
    let common = by_group
        .into_values()
        .filter(|(a, b)| !a.is_empty() && !b.is_empty())
        .collect();
    (common, a_only, b_only)
}

fn group_of(trial: &Trial, groups: &Groups) -> String {
    groups
        .get(&trial.instance_id)
        .unwrap_or(&trial.instance_id)
        .clone()
}

fn difference_assumption(design: &AnalysisDesign, groups: u64) -> String {
    format!(
        "Pointwise two-sided 95% bands from {} joint resamples of complete groups across A and B, \
         seed={}, nearest rank percentiles. Only the {groups} groups with trials in both arms enter. \
         An arm with no success or only successes widens the band by its exact Clopper-Pearson bound.",
        design.bootstrap_resamples, design.bootstrap_seed
    )
}

/// The trials of one group: A, then B.
type GroupTrials<'a> = (Vec<&'a Trial>, Vec<&'a Trial>);

type Arms<'a> = [GroupTrials<'a>];

fn wall_points(
    common: &Arms<'_>,
    grid: &[f64],
    censoring: &Censoring,
) -> Result<Vec<Vec<GroupPoint>>, AnalysisError> {
    let observations = |trials: &[&Trial]| {
        let mut observations = trials
            .iter()
            .map(|trial| wall_observation(trial))
            .collect::<Result<Vec<(Duration, bool)>, _>>()?;
        observations.sort_unstable_by_key(|&(time, _)| time);
        Ok::<_, AnalysisError>(observations)
    };
    common
        .iter()
        .map(|(a, b)| {
            let (a, b) = (observations(a)?, observations(b)?);
            Ok(grid
                .iter()
                .map(|&time| {
                    let point = |observations: &[(Duration, bool)]| {
                        let success = wall_probability(observations, time, censoring);
                        ArmPoint {
                            success,
                            possible: success,
                        }
                    };
                    (point(&a), point(&b))
                })
                .collect())
        })
        .collect()
}

fn device_points(common: &Arms<'_>, grid: &[f64]) -> Result<Vec<Vec<GroupPoint>>, AnalysisError> {
    let budgets = grid
        .iter()
        .map(|seconds| device_budget_ns(*seconds))
        .collect::<Result<Vec<_>, _>>()?;
    let point = |trials: &[&Trial], budget_ns: u64| {
        let results: Vec<DeviceResult> = trials
            .iter()
            .map(|trial| device_result(trial, budget_ns))
            .collect();
        let share = |accept: &dyn Fn(DeviceResult) -> bool| {
            count_f64(results.iter().filter(|result| accept(**result)).count())
                / count_f64(results.len())
        };
        ArmPoint {
            success: share(&|result| result == DeviceResult::Success),
            possible: share(&|result| result != DeviceResult::Failure),
        }
    };
    Ok(common
        .iter()
        .map(|(a, b)| {
            budgets
                .iter()
                .map(|&budget_ns| (point(a, budget_ns), point(b, budget_ns)))
                .collect()
        })
        .collect())
}

/// The estimate and band at one grid point. The lower band resamples A success minus B possible,
/// the upper A possible minus B success. An arm at zero or full success widens the band by its
/// exact bound, so a band never claims certainty from a degenerate bootstrap.
fn difference_point(
    clusters: &[Vec<GroupPoint>],
    index: usize,
    design: &AnalysisDesign,
) -> Result<(f64, f64, f64), AnalysisError> {
    let mean = |sample: &[&[GroupPoint]], value: &dyn Fn(&GroupPoint) -> f64| {
        sample.iter().map(|group| value(&group[index])).sum::<f64>() / count_f64(sample.len())
    };
    let all: Vec<&[GroupPoint]> = clusters.iter().map(Vec::as_slice).collect();
    let estimate = mean(&all, &|(a, b)| a.success - b.success);
    let resample = |value: &dyn Fn(&GroupPoint) -> f64| {
        cluster_bootstrap(
            clusters,
            design.bootstrap_resamples,
            design.bootstrap_seed,
            |sample| mean(sample, value),
        )
    };
    let mut lower = nearest_rank(&mut resample(&|(a, b)| a.success - b.possible)?, 1);
    let mut upper = nearest_rank(&mut resample(&|(a, b)| a.possible - b.success)?, 39);
    let groups = clusters.len() as u64;
    let zero = binomial_upper(0, groups, 0.025)?;
    let full = 1.0 - binomial_lower(groups, groups, 0.025)?;
    let a_success = mean(&all, &|(a, _)| a.success);
    let a_possible = mean(&all, &|(a, _)| a.possible);
    let b_success = mean(&all, &|(_, b)| b.success);
    let b_possible = mean(&all, &|(_, b)| b.possible);
    if a_possible <= 0.0 {
        upper += zero;
    }
    if a_success >= 1.0 {
        lower -= full;
    }
    if b_possible <= 0.0 {
        lower -= zero;
    }
    if b_success >= 1.0 {
        upper += full;
    }
    Ok((
        estimate,
        lower.clamp(-1.0, estimate),
        upper.clamp(estimate, 1.0),
    ))
}

#[expect(
    clippy::cast_precision_loss,
    reason = "In-memory trial and group counts are below f64's exact integer range."
)]
fn count_f64(count: usize) -> f64 {
    count as f64
}
