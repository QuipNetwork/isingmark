// SPDX-License-Identifier: AGPL-3.0-or-later
//! A/B comparison rows, the archived A reference, and their CSV form.
//!
//! A campaign arm reports its wall and device curves at fixed points, its costs, and its state
//! coverage. An archived arm reports the `qpu80_pp` reference and a historical Tier label, never a
//! curve. A missing value stays an empty cell and never becomes a zero.

use std::{fs, path::Path};

use serde::Deserialize;

use crate::campaign::{
    analysis::survival::{AnalysisDesign, AnalysisError, Curve},
    contracts::{TimeBasis, TimingStatus},
    runner::{TerminalReason, Trial},
    CampaignError,
};

use super::{
    arm_curves, paired_difference,
    tier::{tier, ClassicalRun, QpuReference, TierLabel},
    ArmSpec, DifferenceOutcome, Groups, NotComparable, PairSpec, SizeStatus,
};

/// Wall-clock points of the `wall_f_*` columns, in seconds.
pub const WALL_REPORT_S: [f64; 5] = [1.0, 10.0, 30.0, 60.0, 1_200.0];
/// Device point of the `device_g_30` columns, in seconds.
pub const DEVICE_REPORT_S: f64 = 30.0;

/// The archived A side of one cell in `archive-index.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct ArchivedCell {
    pub cell: String,
    pub family: String,
    pub method: String,
    /// Path of the research source, relative to the research root.
    pub source_path: String,
    pub source_blake3: String,
    /// `qpu80_pp` per nonce, with the family postprocess seconds per call.
    pub reference: QpuReference,
}

#[derive(Deserialize)]
struct Index {
    source: IndexSource,
    cells: Vec<IndexCell>,
}

#[derive(Deserialize)]
struct IndexSource {
    path: String,
    blake3: String,
}

#[derive(Deserialize)]
struct IndexCell {
    cell: String,
    family: String,
    method: String,
    budget_s: f64,
    rows: Vec<IndexRow>,
}

#[derive(Deserialize)]
struct IndexRow {
    nonce: String,
    qpu80_pp: String,
}

/// Load one cell of the archive index.
///
/// # Errors
/// Returns `Io` or `Parse` for an unreadable index, and `InvalidArchive` for a missing cell, a
/// repeated nonce, or a `qpu80_pp` that is not an integer.
pub fn load_archived_cell(index: &Path, cell: &str) -> Result<ArchivedCell, CampaignError> {
    let index: Index = serde_json::from_slice(&fs::read(index)?)?;
    let found = index
        .cells
        .into_iter()
        .find(|entry| entry.cell == cell)
        .ok_or_else(|| CampaignError::InvalidArchive(format!("no cell {cell}")))?;
    let mut energies_milli = std::collections::BTreeMap::new();
    for row in found.rows {
        let energy = row.qpu80_pp.parse::<i64>().map_err(|_| {
            CampaignError::InvalidArchive(format!(
                "{cell} {}: qpu80_pp is not an integer",
                row.nonce
            ))
        })?;
        if energies_milli.insert(row.nonce.clone(), energy).is_some() {
            return Err(CampaignError::InvalidArchive(format!(
                "{cell} repeats nonce {}",
                row.nonce
            )));
        }
    }
    Ok(ArchivedCell {
        cell: found.cell,
        family: found.family,
        method: found.method,
        source_path: index.source.path,
        source_blake3: index.source.blake3,
        reference: QpuReference {
            energies_milli,
            post_s: found.budget_s,
        },
    })
}

/// The evidence behind one arm.
#[derive(Debug, Clone, Copy)]
pub enum ArmEvidence<'a> {
    Campaign {
        trials: &'a [Trial],
        groups: &'a Groups,
    },
    /// The archived reference, with the archived classical runs for its historical Tier label.
    Archived {
        cell: &'a ArchivedCell,
        runs: &'a [ClassicalRun],
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Band {
    pub estimate: f64,
    pub lower: f64,
    pub upper: f64,
}

/// One arm of one pair.
#[derive(Debug, Clone, PartialEq)]
pub struct AbRow {
    pub pair_id: String,
    pub arm: ArmSpec,
    /// `campaign` or `archived`.
    pub source: &'static str,
    pub size_status: SizeStatus,
    /// `estimated`, or the reason the pair has no difference.
    pub comparison: &'static str,
    /// The energy reference: `qpu80_pp` for an archived arm, the target rule for a campaign arm.
    pub reference: String,
    /// Instances of a campaign arm, reference models of an archived arm.
    pub models: u64,
    /// Independent resampling groups. An archived arm counts each reference model.
    pub groups: u64,
    pub trials: u64,
    pub unresolved: u64,
    /// Share of trials with every state captured in every stage.
    pub state_coverage: Option<f64>,
    /// Wall success F at each of [`WALL_REPORT_S`], `None` beyond the observation window.
    pub wall: Vec<(f64, Option<Band>)>,
    /// QPU device success G at [`DEVICE_REPORT_S`].
    pub device_30: Option<Band>,
    /// The worst device timing status over the trials.
    pub device_timing: Option<TimingStatus>,
    pub device_s_lower: Option<f64>,
    /// `None` when any trial lacks an upper device bound or the total overflows.
    pub device_s_upper: Option<f64>,
    /// `None` when any trial lacks the counter or the total overflows.
    pub host_compute_s: Option<f64>,
    pub evaluations: Option<u64>,
    /// Historical context only: never a probability guide or a validity claim.
    pub tier: Option<TierLabel>,
}

/// Both rows of a pair and its difference outcome.
///
/// # Errors
/// Returns the errors of [`arm_curves`] and [`paired_difference`].
pub fn ab_rows(
    pair: &PairSpec,
    a: &ArmEvidence<'_>,
    b: &ArmEvidence<'_>,
    design: &AnalysisDesign,
) -> Result<([AbRow; 2], DifferenceOutcome), AnalysisError> {
    let outcome = match (a, b) {
        (
            ArmEvidence::Campaign {
                trials: a_trials,
                groups: a_groups,
            },
            ArmEvidence::Campaign {
                trials: b_trials,
                groups: b_groups,
            },
        ) => paired_difference(pair, a_trials, a_groups, b_trials, b_groups, design)?,
        _ if pair.size_status() == SizeStatus::UnmatchedSize => {
            DifferenceOutcome::NotComparable(NotComparable::UnmatchedSize)
        }
        _ => DifferenceOutcome::NotComparable(NotComparable::ArchivedArm),
    };
    let comparison = match &outcome {
        DifferenceOutcome::Estimated(_) => "estimated",
        DifferenceOutcome::NotComparable(reason) => reason.label(),
    };
    let row = |arm: &ArmSpec, evidence: &ArmEvidence<'_>| match evidence {
        ArmEvidence::Campaign { trials, groups } => {
            campaign_row(pair, arm, comparison, trials, groups, design)
        }
        ArmEvidence::Archived { cell, runs } => Ok(archived_row(pair, arm, comparison, cell, runs)),
    };
    Ok(([row(&pair.a, a)?, row(&pair.b, b)?], outcome))
}

impl NotComparable {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::UnmatchedSize => "unmatched_size",
            Self::PanelMismatch => "panel_mismatch",
            Self::ArchivedArm => "archived_arm",
            Self::NoCommonGroups => "no_common_groups",
        }
    }
}

fn campaign_row(
    pair: &PairSpec,
    arm: &ArmSpec,
    comparison: &'static str,
    trials: &[Trial],
    groups: &Groups,
    design: &AnalysisDesign,
) -> Result<AbRow, AnalysisError> {
    let curves = arm_curves(trials, groups, design)?;
    let curve = |basis| curves.iter().find(|curve| curve.basis == basis);
    let wall = curve(TimeBasis::WallClock);
    let device = curve(TimeBasis::QpuDeviceAccess);
    let total = |key: &str| {
        trials
            .iter()
            .map(|trial| trial.resources.get(key).copied())
            .try_fold(0_u64, |sum, value| sum.checked_add(value?))
    };
    // A portfolio can lose states in an early stage and still end accepted.
    let complete = trials
        .iter()
        .filter(|trial| {
            trial.terminal != TerminalReason::IncompleteCapture
                && trial
                    .resources
                    .get("incomplete_captures")
                    .copied()
                    .unwrap_or(0)
                    == 0
        })
        .count();
    Ok(AbRow {
        pair_id: pair.pair_id.clone(),
        arm: arm.clone(),
        source: "campaign",
        size_status: pair.size_status(),
        comparison,
        reference: arm.target_rule.clone(),
        models: wall.map_or(0, |curve| curve.instances),
        groups: wall.map_or(0, |curve| curve.groups),
        trials: trials.len() as u64,
        unresolved: device.map_or(0, |curve| curve.unresolved),
        state_coverage: (!trials.is_empty())
            .then(|| count_f64(complete as u64) / count_f64(trials.len() as u64)),
        wall: WALL_REPORT_S
            .iter()
            .map(|time| (*time, wall.and_then(|curve| band_at(curve, *time))))
            .collect(),
        device_30: device.and_then(|curve| band_at(curve, DEVICE_REPORT_S)),
        device_timing: trials
            .iter()
            .map(|trial| trial.device_total.status)
            .reduce(TimingStatus::worst),
        device_s_lower: trials
            .iter()
            .try_fold(0_u64, |sum, trial| {
                sum.checked_add(trial.device_total.lower_ns)
            })
            .map(nanos_s),
        device_s_upper: trials
            .iter()
            .try_fold(0_u64, |sum, trial| {
                sum.checked_add(trial.device_total.upper_ns?)
            })
            .map(nanos_s),
        host_compute_s: total("host_compute_ns").map(nanos_s),
        evaluations: total("evaluations"),
        tier: None,
    })
}

fn archived_row(
    pair: &PairSpec,
    arm: &ArmSpec,
    comparison: &'static str,
    cell: &ArchivedCell,
    runs: &[ClassicalRun],
) -> AbRow {
    AbRow {
        pair_id: pair.pair_id.clone(),
        arm: arm.clone(),
        source: "archived",
        size_status: pair.size_status(),
        comparison,
        reference: "qpu80_pp".into(),
        models: cell.reference.energies_milli.len() as u64,
        groups: cell.reference.energies_milli.len() as u64,
        trials: 0,
        unresolved: 0,
        state_coverage: None,
        wall: WALL_REPORT_S.iter().map(|time| (*time, None)).collect(),
        device_30: None,
        device_timing: None,
        device_s_lower: None,
        device_s_upper: None,
        host_compute_s: None,
        evaluations: None,
        tier: tier(&cell.reference, runs),
    }
}

fn band_at(curve: &Curve, time: f64) -> Option<Band> {
    let index = curve
        .grid_s
        .iter()
        .position(|grid| grid.total_cmp(&time).is_eq())?;
    Some(Band {
        estimate: curve.estimate[index],
        lower: curve.lower[index],
        upper: curve.upper[index],
    })
}

/// The CSV header of [`ab_csv`].
pub const AB_CSV_HEADER: &str = "pair_id,side,source,family,family_revision,parameter_digest,\
policy_id,size_spins,target_rule,panel,size_status,comparison,reference,models,groups,trials,unresolved,\
state_coverage,wall_f_1,wall_f_1_lower,wall_f_1_upper,wall_f_10,wall_f_10_lower,wall_f_10_upper,\
wall_f_30,wall_f_30_lower,wall_f_30_upper,wall_f_60,wall_f_60_lower,wall_f_60_upper,wall_f_1200,\
wall_f_1200_lower,wall_f_1200_upper,device_g_30,device_g_30_lower,device_g_30_upper,device_timing,\
device_s_lower,device_s_upper,host_compute_s,evaluations,tier_rule_version,historical_tier,\
tier_beat_wall,tier_speed_wall";

/// The rows as CSV under [`AB_CSV_HEADER`]. A missing value is an empty field.
#[must_use]
pub fn ab_csv(rows: &[AbRow]) -> String {
    let mut out = String::from(AB_CSV_HEADER);
    out.push('\n');
    for row in rows {
        let mut fields = vec![
            row.pair_id.clone(),
            format!("{:?}", row.arm.side),
            row.source.into(),
            row.arm.family.clone(),
            row.arm.family_revision.clone(),
            row.arm.parameter_digest.clone(),
            row.arm.policy_id.clone(),
            row.arm.size_spins.to_string(),
            row.arm.target_rule.clone(),
            snake(&row.arm.panel),
            snake(&row.size_status),
            row.comparison.into(),
            row.reference.clone(),
            row.models.to_string(),
            row.groups.to_string(),
            row.trials.to_string(),
            row.unresolved.to_string(),
            optional(row.state_coverage),
        ];
        for (_, band) in &row.wall {
            fields.extend(band_fields(*band));
        }
        fields.extend(band_fields(row.device_30));
        fields.push(row.device_timing.as_ref().map(snake).unwrap_or_default());
        fields.push(optional(row.device_s_lower));
        fields.push(optional(row.device_s_upper));
        fields.push(optional(row.host_compute_s));
        fields.push(
            row.evaluations
                .map(|count| count.to_string())
                .unwrap_or_default(),
        );
        fields.push(
            row.tier
                .as_ref()
                .map(|label| label.rule_version.to_string())
                .unwrap_or_default(),
        );
        fields.push(
            row.tier
                .as_ref()
                .map(|label| label.tier.label().to_owned())
                .unwrap_or_default(),
        );
        let basis = |estimated: bool| if estimated { "scaled" } else { "measured" }.to_owned();
        let label = row.tier.as_ref();
        fields.push(
            label
                .and_then(|label| label.beat_run.as_ref())
                .map(|run| basis(run.estimated))
                .unwrap_or_default(),
        );
        fields.push(
            label
                .and_then(|label| label.speed.as_ref())
                .map(|speed| basis(speed.estimated))
                .unwrap_or_default(),
        );
        let quoted: Vec<String> = fields.iter().map(|field| quote(field)).collect();
        out.push_str(&quoted.join(","));
        out.push('\n');
    }
    out
}

fn band_fields(band: Option<Band>) -> [String; 3] {
    [
        optional(band.map(|band| band.estimate)),
        optional(band.map(|band| band.lower)),
        optional(band.map(|band| band.upper)),
    ]
}

fn optional(value: Option<f64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

/// The `snake_case` serde name of a unit enum value.
fn snake(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

fn quote(field: &str) -> String {
    if field.contains([',', '"', '\n']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Nanosecond totals and counts are reported as approximate seconds and shares."
)]
fn count_f64(count: u64) -> f64 {
    count as f64
}

fn nanos_s(nanos: u64) -> f64 {
    count_f64(nanos) / 1e9
}
