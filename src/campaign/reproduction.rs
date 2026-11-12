// SPDX-License-Identifier: AGPL-3.0-or-later
//! The RP-0 reproduction comparison: campaign outputs against a frozen archived row.
//!
//! The frozen file (for example `evaluation/qblock/configs/reproduction/ice-charge.json`) holds
//! the pass rule, the archived values, the policy ids of the CPU and CUDA arms, and the digests
//! of the inputs. `policies.file` is relative to the frozen file. `control.index` is relative to
//! the working directory, so run the command from the repository root. The command reads `campaign.json` and each trial's `events.jsonl` from every output
//! directory, keeps the lowest recomputed energy and the wall of the first batch receipt per
//! trial, and compares two cohorts:
//!
//! * the control arm, the archived models, against their archived `qpu80_pp`: gap, win counts,
//!   Tier rule v3, time to target, and the pass-rule verdict;
//! * the fresh cohort, which has no QPU energies until RP-6: energies and walls only.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    analysis::suite_fit::tier::{
        gap_stat, tier, ClassicalDevice, ClassicalRun, GapStat, QpuReference, TierLabel, WallBasis,
    },
    runner::{TerminalReason, Trial},
    trace::{read_trace, EventKind},
    CampaignError,
};

fn invalid(reason: impl Into<String>) -> CampaignError {
    CampaignError::InvalidRun(reason.into())
}

#[derive(Debug, Deserialize)]
struct Frozen {
    cell: String,
    pass_rule: PassRule,
    archived: Archived,
    policies: Policies,
    cohort: FrozenCohort,
    control: FrozenControl,
}

#[derive(Debug, Deserialize)]
struct PassRule {
    gap_sd_multiple: f64,
    time_factor: f64,
}

#[derive(Debug, Deserialize)]
struct Archived {
    tier: String,
    gap: ArchivedGap,
    time_to_target_s: BTreeMap<String, f64>,
    best_device: String,
    qpu_post_s: f64,
}

#[derive(Debug, Deserialize)]
struct ArchivedGap {
    mean_units: f64,
    sd_units: f64,
}

#[derive(Debug, Deserialize)]
struct Policies {
    file: String,
    blake3: String,
    cpu: PolicyRef,
    cuda: PolicyRef,
}

#[derive(Debug, Deserialize)]
struct PolicyRef {
    policy_id: String,
}

#[derive(Debug, Deserialize)]
struct FrozenCohort {
    count: usize,
    verdict_until_rp6: String,
}

#[derive(Debug, Deserialize)]
struct FrozenControl {
    index: String,
    index_blake3: String,
    rows: usize,
}

#[derive(Deserialize)]
struct CampaignFile {
    trials: Vec<Trial>,
}

/// One trial reduced to what the comparison uses.
#[derive(Debug, Clone, Serialize)]
pub struct TrialResult {
    pub instance_id: String,
    pub nonce: String,
    pub policy_id: String,
    pub terminal: TerminalReason,
    /// The lowest recomputed energy over every returned state, in milli.
    pub best_milli: Option<i64>,
    pub states: usize,
    /// Seconds from trial start to the first batch receipt.
    pub wall_s: Option<f64>,
}

/// Summary of one arm on one cohort.
#[derive(Debug, Clone, Serialize)]
pub struct ArmSummary {
    pub device: ClassicalDevice,
    pub policy_id: String,
    pub trials: usize,
    pub with_state: usize,
    pub terminals: BTreeMap<String, usize>,
    pub energy_units: Option<Spread>,
    pub median_wall_s: Option<f64>,
}

/// Median, mean, and range of a sample, in the unit of the field that holds it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Spread {
    pub median: f64,
    pub mean: f64,
    pub min: f64,
    pub max: f64,
}

/// One device on the control arm against the archived `qpu80_pp`.
#[derive(Debug, Clone, Serialize)]
pub struct ControlArm {
    pub summary: ArmSummary,
    pub gap: Option<GapStat>,
    pub qpu_wins: usize,
    pub ties: usize,
    pub classical_wins: usize,
    /// Models without a recomputed energy, counted as no win for either side.
    pub missing: usize,
    /// Models at or below `qpu80_pp`, the archive's reach count (ties count).
    pub at_or_below: usize,
    /// More than half of the common models are at or below the target.
    pub reached: bool,
    /// The median wall per model when the arm reached, the archive's time-to-target basis.
    pub time_to_target_s: Option<f64>,
}

/// One pass-rule check: `None` when it cannot be computed.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub archived: String,
    pub reproduced: String,
    pub pass: Option<bool>,
}

/// The whole comparison.
#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub cell: String,
    pub control: BTreeMap<String, ControlArm>,
    pub tier: Option<TierLabel>,
    pub checks: Vec<Check>,
    pub control_verdict: String,
    pub fresh: BTreeMap<String, ArmSummary>,
    pub fresh_verdict: String,
    pub trials: Vec<TrialResult>,
}

fn file_blake3(path: &Path) -> Result<String, CampaignError> {
    Ok(blake3::hash(&fs::read(path)?).to_hex().to_string())
}

fn check_digest(path: &Path, expected: &str) -> Result<(), CampaignError> {
    let found = file_blake3(path)?;
    if found == expected {
        Ok(())
    } else {
        Err(invalid(format!(
            "{} has BLAKE3 {found}, the frozen file pins {expected}",
            path.display()
        )))
    }
}

#[expect(clippy::cast_precision_loss, reason = "nanoseconds well below 2^52")]
fn seconds(nanos: u64) -> f64 {
    nanos as f64 / 1e9
}

/// Read every trial of one campaign output directory.
///
/// # Errors
/// I/O and parse failures of `campaign.json` or a trace.
pub fn read_campaign(dir: &Path) -> Result<Vec<TrialResult>, CampaignError> {
    let file: CampaignFile = serde_json::from_slice(&fs::read(dir.join("campaign.json"))?)?;
    file.trials
        .into_iter()
        .map(|trial| {
            let trace = read_trace(&dir.join(&trial.trial_id).join("events.jsonl"))?;
            let mut best: Option<i64> = None;
            let mut states = 0;
            let mut wall_s = None;
            for event in trace
                .events
                .iter()
                .filter(|event| event.kind == EventKind::BatchReceipt)
            {
                wall_s.get_or_insert(seconds(event.wall_ns));
                for solution in event.payload["solutions"].as_array().into_iter().flatten() {
                    states += 1;
                    if let Some(energy) = solution["recomputed_energy_milli"].as_i64() {
                        best = Some(best.map_or(energy, |low| low.min(energy)));
                    }
                }
            }
            Ok(TrialResult {
                instance_id: trial.instance_id,
                nonce: trial.identity.nonce,
                policy_id: trial.policy_id,
                terminal: trial.terminal,
                best_milli: best,
                states,
                wall_s,
            })
        })
        .collect()
}

#[expect(clippy::cast_precision_loss, reason = "sample sizes are small")]
fn spread(values: &[f64]) -> Option<Spread> {
    let mut sorted: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let median = if n % 2 == 1 {
        sorted[n / 2]
    } else {
        f64::midpoint(sorted[n / 2 - 1], sorted[n / 2])
    };
    Some(Spread {
        median,
        mean: sorted.iter().sum::<f64>() / n as f64,
        min: sorted[0],
        max: sorted[n - 1],
    })
}

#[expect(clippy::cast_precision_loss, reason = "energies far below 2^52 milli")]
fn units(milli: i64) -> f64 {
    milli as f64 / 1000.0
}

fn summarize(device: ClassicalDevice, policy_id: &str, trials: &[&TrialResult]) -> ArmSummary {
    let mut terminals = BTreeMap::new();
    for trial in trials {
        let name = serde_json::to_value(trial.terminal)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("{:?}", trial.terminal));
        *terminals.entry(name).or_insert(0) += 1;
    }
    let energies: Vec<f64> = trials
        .iter()
        .filter_map(|t| t.best_milli)
        .map(units)
        .collect();
    let walls: Vec<f64> = trials.iter().filter_map(|t| t.wall_s).collect();
    ArmSummary {
        device,
        policy_id: policy_id.to_owned(),
        trials: trials.len(),
        with_state: energies.len(),
        terminals,
        energy_units: spread(&energies),
        median_wall_s: spread(&walls).map(|s| s.median),
    }
}

fn reference(frozen: &Frozen) -> Result<QpuReference, CampaignError> {
    let index_path = PathBuf::from(&frozen.control.index);
    check_digest(&index_path, &frozen.control.index_blake3)?;
    let index: Value = serde_json::from_slice(&fs::read(&index_path)?)?;
    let cell = index["cells"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|cell| cell["cell"] == frozen.cell.as_str())
        .ok_or_else(|| {
            invalid(format!(
                "{} has no cell {}",
                index_path.display(),
                frozen.cell
            ))
        })?;
    let mut energies_milli = BTreeMap::new();
    for row in cell["rows"].as_array().into_iter().flatten() {
        let nonce = row["nonce"]
            .as_str()
            .ok_or_else(|| invalid("row without nonce"))?;
        let pp = row["qpu80_pp"]
            .as_str()
            .and_then(|text| text.parse::<i64>().ok())
            .ok_or_else(|| invalid(format!("row {nonce} without an integer qpu80_pp")))?;
        energies_milli.insert(nonce.to_owned(), pp);
    }
    if energies_milli.len() != frozen.control.rows {
        return Err(invalid(format!(
            "{} rows in the index, the frozen file expects {}",
            energies_milli.len(),
            frozen.control.rows
        )));
    }
    Ok(QpuReference {
        energies_milli,
        post_s: frozen.archived.qpu_post_s,
    })
}

fn control_arm(
    reference: &QpuReference,
    device: ClassicalDevice,
    policy_id: &str,
    trials: &[&TrialResult],
) -> (ControlArm, ClassicalRun) {
    let summary = summarize(device, policy_id, trials);
    let run = ClassicalRun {
        device,
        name: policy_id.to_owned(),
        budget_s: None,
        wall_s: summary.median_wall_s,
        device_s: summary.median_wall_s,
        wall_basis: WallBasis::Measured,
        rung: Some(policy_id.to_owned()),
        energies_milli: trials
            .iter()
            .map(|trial| (trial.nonce.clone(), trial.best_milli))
            .collect(),
    };
    let (mut qpu_wins, mut ties, mut classical_wins, mut missing, mut common) = (0, 0, 0, 0, 0);
    for trial in trials {
        let Some(target) = reference.energies_milli.get(&trial.nonce) else {
            continue;
        };
        common += 1;
        match trial.best_milli {
            Some(energy) if energy < *target => classical_wins += 1,
            Some(energy) if energy == *target => ties += 1,
            Some(_) => qpu_wins += 1,
            None => missing += 1,
        }
    }
    let at_or_below = classical_wins + ties;
    let reached = common > 0 && 2 * at_or_below > common;
    let arm = ControlArm {
        gap: gap_stat(reference, &run),
        qpu_wins,
        ties,
        classical_wins,
        missing,
        at_or_below,
        reached,
        time_to_target_s: reached.then_some(summary.median_wall_s).flatten(),
        summary,
    };
    (arm, run)
}

fn device_key(device: ClassicalDevice) -> &'static str {
    match device {
        ClassicalDevice::Cpu => "cpu",
        ClassicalDevice::Cuda => "cuda",
    }
}

fn checks(
    frozen: &Frozen,
    label: Option<&TierLabel>,
    arms: &BTreeMap<String, ControlArm>,
) -> Vec<Check> {
    let archived = &frozen.archived;
    let tier_check = Check {
        name: "tier",
        archived: archived.tier.clone(),
        reproduced: label.map_or_else(|| "none".to_owned(), |l| l.tier.label().to_owned()),
        pass: label.map(|l| l.tier.label() == archived.tier),
    };
    let gap = label.and_then(|l| l.beat_run.as_ref()).map(|run| run.stat);
    let limit = frozen.pass_rule.gap_sd_multiple * archived.gap.sd_units;
    let gap_check = Check {
        name: "gap",
        archived: format!(
            "{:+.3} (sd {:.3}), pass within {limit:.3}",
            archived.gap.mean_units, archived.gap.sd_units
        ),
        reproduced: gap.map_or_else(
            || "no deciding run".to_owned(),
            |g| {
                format!(
                    "{:+.3} (sd {:.3}, {} models)",
                    g.mean_units, g.sd_units, g.models
                )
            },
        ),
        pass: gap.map(|g| (g.mean_units - archived.gap.mean_units).abs() <= limit),
    };
    let best = &archived.best_device;
    let archived_time = archived.time_to_target_s.get(best).copied();
    let arm = arms.get(best);
    let factor = frozen.pass_rule.time_factor;
    let time_check = Check {
        name: "time_to_target",
        archived: archived_time.map_or_else(|| "none".to_owned(), |t| format!("{best} {t:.4} s")),
        reproduced: match arm {
            None => format!("no {best} arm"),
            Some(arm) => arm.time_to_target_s.map_or_else(
                || {
                    format!(
                        "{best} did not reach ({} of {} at or below)",
                        arm.at_or_below, arm.summary.trials
                    )
                },
                |t| format!("{best} {t:.4} s"),
            ),
        },
        pass: match (archived_time, arm) {
            (Some(target), Some(arm)) if arm.summary.trials > 0 => Some(
                arm.time_to_target_s
                    .is_some_and(|t| t <= target * factor && t >= target / factor),
            ),
            _ => None,
        },
    };
    vec![tier_check, gap_check, time_check]
}

fn verdict(checks: &[Check]) -> String {
    let failed: Vec<&str> = checks
        .iter()
        .filter(|c| c.pass == Some(false))
        .map(|c| c.name)
        .collect();
    let missing: Vec<&str> = checks
        .iter()
        .filter(|c| c.pass.is_none())
        .map(|c| c.name)
        .collect();
    if !failed.is_empty() {
        format!("not reproduced: {} failed", failed.join(", "))
    } else if !missing.is_empty() {
        format!("incomplete: {} not computable", missing.join(", "))
    } else {
        "reproduced".to_owned()
    }
}

/// One frozen arm and its trials.
type Arm<'a> = (ClassicalDevice, &'a str, Vec<&'a TrialResult>);

/// Split the trials of one cohort into the frozen arms. No trials gives no arms. Otherwise every
/// trial belongs to a frozen policy, both arms have trials, and no arm repeats a model.
fn split_arms<'a>(
    trials: &'a [TrialResult],
    arms: &[(ClassicalDevice, &'a str)],
    cohort: &str,
) -> Result<Vec<Arm<'a>>, CampaignError> {
    if trials.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(stray) = trials
        .iter()
        .find(|t| arms.iter().all(|(_, id)| t.policy_id != *id))
    {
        return Err(invalid(format!(
            "{cohort} trial {}: policy {} is not in the frozen file",
            stray.instance_id, stray.policy_id
        )));
    }
    let mut split = Vec::new();
    for &(device, policy_id) in arms {
        let arm: Vec<&TrialResult> = trials.iter().filter(|t| t.policy_id == policy_id).collect();
        if arm.is_empty() {
            return Err(invalid(format!("no {cohort} trials for {policy_id}")));
        }
        let mut seen = std::collections::BTreeSet::new();
        if let Some(repeat) = arm.iter().find(|t| !seen.insert(t.nonce.as_str())) {
            return Err(invalid(format!(
                "{policy_id} has more than one trial of model {}",
                repeat.nonce
            )));
        }
        split.push((device, policy_id, arm));
    }
    Ok(split)
}

/// Compare the control and fresh campaign outputs with the frozen archived row.
///
/// # Errors
/// A digest mismatch with the frozen file, a missing arm, a trial of another policy, a repeated
/// or missing model, a wrong cohort size, or a read failure.
pub fn compare(
    frozen_path: &Path,
    control_dirs: &[PathBuf],
    fresh_dirs: &[PathBuf],
) -> Result<Comparison, CampaignError> {
    let frozen: Frozen = serde_json::from_slice(&fs::read(frozen_path)?)?;
    let policies_path = frozen_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&frozen.policies.file);
    check_digest(&policies_path, &frozen.policies.blake3)?;
    let reference = reference(&frozen)?;
    let arms = [
        (ClassicalDevice::Cpu, frozen.policies.cpu.policy_id.as_str()),
        (
            ClassicalDevice::Cuda,
            frozen.policies.cuda.policy_id.as_str(),
        ),
    ];
    let read_all = |dirs: &[PathBuf]| -> Result<Vec<TrialResult>, CampaignError> {
        let mut all = Vec::new();
        for dir in dirs {
            all.extend(read_campaign(dir)?);
        }
        Ok(all)
    };
    let control_trials = read_all(control_dirs)?;
    let fresh_trials = read_all(fresh_dirs)?;

    let mut control = BTreeMap::new();
    let mut runs = Vec::new();
    for (device, policy_id, trials) in split_arms(&control_trials, &arms, "control")? {
        if let Some(stray) = trials
            .iter()
            .find(|t| !reference.energies_milli.contains_key(&t.nonce))
        {
            return Err(invalid(format!(
                "control trial {} is not an archived model",
                stray.instance_id
            )));
        }
        if trials.len() != reference.energies_milli.len() {
            return Err(invalid(format!(
                "{policy_id} covers {} of {} archived models",
                trials.len(),
                reference.energies_milli.len()
            )));
        }
        let (arm, run) = control_arm(&reference, device, policy_id, &trials);
        control.insert(device_key(device).to_owned(), arm);
        runs.push(run);
    }
    let label = tier(&reference, &runs);
    let checks = checks(&frozen, label.as_ref(), &control);
    let control_verdict = if control.is_empty() {
        "incomplete: no control trials".to_owned()
    } else {
        verdict(&checks)
    };

    let mut fresh = BTreeMap::new();
    for (device, policy_id, trials) in split_arms(&fresh_trials, &arms, "fresh")? {
        if trials.len() != frozen.cohort.count {
            return Err(invalid(format!(
                "{policy_id} has {} fresh trials, the frozen cohort has {}",
                trials.len(),
                frozen.cohort.count
            )));
        }
        if let Some(shared) = trials
            .iter()
            .find(|t| reference.energies_milli.contains_key(&t.nonce))
        {
            return Err(invalid(format!(
                "fresh trial {} uses an archived nonce",
                shared.instance_id
            )));
        }
        fresh.insert(
            device_key(device).to_owned(),
            summarize(device, policy_id, &trials),
        );
    }
    let mut trials = control_trials;
    trials.extend(fresh_trials);
    Ok(Comparison {
        cell: frozen.cell,
        control,
        tier: label,
        checks,
        control_verdict,
        fresh,
        fresh_verdict: frozen.cohort.verdict_until_rp6,
        trials,
    })
}

fn opt(value: Option<f64>, digits: usize) -> String {
    value.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.digits$}"))
}

/// The comparison as Markdown tables, for the report.
#[must_use]
pub fn markdown(comparison: &Comparison) -> String {
    let mut out = format!(
        "# Reproduction of `{}`\n\n## Control arm: archived models\n\n",
        comparison.cell
    );
    out.push_str("| Device | Policy | Models | QPU / tie / classical | No state | Gap mean (sd) | Median energy | Median wall s | Time to target s |\n");
    out.push_str("|---|---|---:|---|---:|---|---:|---:|---:|\n");
    for arm in comparison.control.values() {
        let s = &arm.summary;
        let _ = writeln!(
            out,
            "| {} | `{}` | {} | {} / {} / {} | {} | {} | {} | {} | {} |",
            s.device.label(),
            s.policy_id,
            s.trials,
            arm.qpu_wins,
            arm.ties,
            arm.classical_wins,
            arm.missing,
            arm.gap.map_or_else(
                || "n/a".to_owned(),
                |g| format!("{:+.3} ({:.3})", g.mean_units, g.sd_units)
            ),
            opt(s.energy_units.map(|e| e.median), 3),
            opt(s.median_wall_s, 3),
            opt(arm.time_to_target_s, 3),
        );
    }
    let _ = write!(
        out,
        "\nTier rule v3: {}\n\n| Check | Archived | Reproduced | Pass |\n|---|---|---|---|\n",
        comparison.tier.as_ref().map_or_else(
            || "none".to_owned(),
            |t| format!("{} ({})", t.tier.label(), t.reason)
        )
    );
    for check in &comparison.checks {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} |",
            check.name,
            check.archived,
            check.reproduced,
            check.pass.map_or("n/a", |p| if p { "yes" } else { "no" })
        );
    }
    let _ = write!(out,
        "\nControl verdict: {}\n\n## Fresh cohort\n\n| Device | Policy | Models | With state | Median energy | Mean energy | Range | Median wall s |\n|---|---|---:|---:|---:|---:|---|---:|\n",
        comparison.control_verdict
    );
    for arm in comparison.fresh.values() {
        let _ = writeln!(
            out,
            "| {} | `{}` | {} | {} | {} | {} | {} | {} |",
            arm.device.label(),
            arm.policy_id,
            arm.trials,
            arm.with_state,
            opt(arm.energy_units.map(|e| e.median), 3),
            opt(arm.energy_units.map(|e| e.mean), 3),
            arm.energy_units.map_or_else(
                || "n/a".to_owned(),
                |e| format!("{:.3} to {:.3}", e.min, e.max)
            ),
            opt(arm.median_wall_s, 3),
        );
    }
    let _ = write!(out, "\nFresh verdict: {}\n", comparison.fresh_verdict);
    out
}
