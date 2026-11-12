// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tier rule version 3 of the research tracking table, kept only as a historical comparison label.
//!
//! This ports `rules.tier`, `rules.tier_runs`, `rules.gap_stat`, `rules.qpu_wall`, `rules.SpeedRun`,
//! and the speed pick of `build_pages.classical_gap` with `rules.best_classical`. A Tier label is
//! never a probability guide or a proof of physical validity. Campaign curves answer those questions.
//!
//! Two inputs are stricter than the research table. A model whose classical state is missing stays
//! in the common set and never counts as a classical win, a tie, or as at or below the reference. A
//! run without a measured or scaled wall per model never takes part in either test.

use std::collections::BTreeMap;

use serde::Serialize;

/// The rule version of `tier-rules.md` that this module ports.
pub const TIER_RULE_VERSION: u32 = 3;
/// A run decides a tier only with at least this many models in common with the reference.
pub const MIN_MODELS: usize = 30;
/// The beat test admits runs with a budget per model of at most this many seconds.
pub const BEAT_BUDGET_S: f64 = 120.0;
/// A run beats the QPU when it is strictly below the reference on this share of its common models.
pub const BEAT_SHARE: f64 = 0.6;
/// The QPU is ahead on speed when classical needs at least this multiple of the QPU wall per model.
pub const SPEED_RATIO: f64 = 5.0;
/// QPU production wall per model for a pipelined miner.
pub const PRODUCTION_WALL_S: f64 = 0.1;

/// Classical devices in the order the research builder breaks speed-pick ties: CUDA first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassicalDevice {
    Cpu,
    Cuda,
}

impl ClassicalDevice {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Cuda => "CUDA",
        }
    }
}

/// How a wall per model was obtained. Only measured and scaled walls take part in a Tier test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WallBasis {
    /// Measured on a quiet host.
    Measured,
    /// A contended measurement scaled by the quiet-host source run, marked estimated.
    Scaled,
    /// Computed from a timing model, with no measurement behind it.
    Modeled,
}

/// The QPU reference: per-model `qpu80_pp` energies in milli and the family postprocess seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct QpuReference {
    pub energies_milli: BTreeMap<String, i64>,
    pub post_s: f64,
}

/// One classical campaign of one device, with every model it attempted.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassicalRun {
    pub device: ClassicalDevice,
    pub name: String,
    /// The declared wall budget per model, if the run had one.
    pub budget_s: Option<f64>,
    /// Median wall per model on CPU, card-seconds per model on CUDA.
    pub wall_s: Option<f64>,
    /// Core-seconds per model on CPU, card-seconds per model on CUDA.
    pub device_s: Option<f64>,
    pub wall_basis: WallBasis,
    /// Runs of one device that share a rung, such as the seeds of one sweep count, form one speed
    /// candidate. It reaches only when every run reaches, with the timing of the first run.
    pub rung: Option<String>,
    /// Energy in milli per attempted model, `None` when the state is missing.
    pub energies_milli: BTreeMap<String, Option<i64>>,
}

impl ClassicalRun {
    /// The wall per model when its basis may decide a tier.
    fn tier_wall(&self) -> Option<f64> {
        match self.wall_basis {
            WallBasis::Measured | WallBasis::Scaled => seconds(self.wall_s),
            WallBasis::Modeled => None,
        }
    }

    fn estimated(&self) -> bool {
        self.wall_basis == WallBasis::Scaled
    }

    fn common<'a>(&'a self, reference: &'a QpuReference) -> Vec<(i64, Option<i64>)> {
        self.energies_milli
            .iter()
            .filter_map(|(model, energy)| {
                reference
                    .energies_milli
                    .get(model)
                    .map(|target| (*target, *energy))
            })
            .collect()
    }
}

/// Per-model gaps `reference - classical` in energy units over the models with a state.
/// Positive means the classical energy is deeper. One model gives a standard deviation of 0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct GapStat {
    pub mean_units: f64,
    pub sd_units: f64,
    pub models: usize,
}

/// Mean and sample standard deviation of the per-model gaps, or `None` without a measured common model.
#[must_use]
pub fn gap_stat(reference: &QpuReference, run: &ClassicalRun) -> Option<GapStat> {
    let units: Vec<f64> = run
        .common(reference)
        .into_iter()
        .filter_map(|(target, energy)| energy.map(|energy| milli_units(target - energy)))
        .collect();
    let mean = units.iter().sum::<f64>() / count_f64(units.len());
    let sd = if units.len() > 1 {
        let squares = units.iter().map(|gap| (gap - mean).powi(2)).sum::<f64>();
        (squares / count_f64(units.len() - 1)).sqrt()
    } else {
        0.0
    };
    (!units.is_empty()).then_some(GapStat {
        mean_units: mean,
        sd_units: sd,
        models: units.len(),
    })
}

/// A beat-test candidate restricted to its common models.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TierRun {
    pub device: ClassicalDevice,
    pub name: String,
    pub wall_s: f64,
    pub budget_s: f64,
    pub estimated: bool,
    pub stat: GapStat,
    /// Attempted models shared with the reference, including those with a missing state.
    pub common: usize,
    pub missing: usize,
    pub qpu_wins: usize,
    pub ties: usize,
    pub classical_wins: usize,
    /// Mean classical energy in milli over the common models with a state.
    pub classical_mean_milli: f64,
}

impl TierRun {
    /// The share of common models with a classical energy strictly below the reference.
    #[must_use]
    pub fn share(&self) -> f64 {
        count_f64(self.classical_wins) / count_f64(self.common)
    }

    #[must_use]
    pub fn beats(&self) -> bool {
        count_f64(self.classical_wins) >= BEAT_SHARE * count_f64(self.common)
    }
}

/// The beat-test candidates: runs with a timed wall, a budget per model within [`BEAT_BUDGET_S`]
/// (declared, else the measured wall), and at least [`MIN_MODELS`] common models.
#[must_use]
pub fn tier_runs(reference: &QpuReference, runs: &[ClassicalRun]) -> Vec<TierRun> {
    let mut out = Vec::new();
    for run in runs {
        let Some(wall_s) = run.tier_wall() else {
            continue;
        };
        let budget_s = match run.budget_s {
            None => wall_s,
            Some(budget_s) => match seconds(Some(budget_s)) {
                Some(budget_s) => budget_s,
                None => continue,
            },
        };
        if budget_s > BEAT_BUDGET_S {
            continue;
        }
        let common = run.common(reference);
        if common.len() < MIN_MODELS {
            continue;
        }
        let Some(stat) = gap_stat(reference, run) else {
            continue;
        };
        let measured: Vec<(i64, i64)> = common
            .iter()
            .filter_map(|&(target, energy)| energy.map(|energy| (target, energy)))
            .collect();
        let gaps = || measured.iter().map(|(target, energy)| target - energy);
        out.push(TierRun {
            device: run.device,
            name: run.name.clone(),
            wall_s,
            budget_s,
            estimated: run.estimated(),
            stat,
            common: common.len(),
            missing: common.len() - measured.len(),
            qpu_wins: gaps().filter(|gap| *gap < 0).count(),
            ties: gaps().filter(|gap| *gap == 0).count(),
            classical_wins: gaps().filter(|gap| *gap > 0).count(),
            classical_mean_milli: measured
                .iter()
                .map(|(_, energy)| milli_f64(*energy))
                .sum::<f64>()
                / count_f64(measured.len()),
        });
    }
    out
}

/// The QPU wall per model for the speed test: the production call plus the family postprocess.
#[must_use]
pub fn qpu_wall(post_s: f64) -> f64 {
    PRODUCTION_WALL_S + post_s
}

/// The speed-test run: the cheapest run by device-seconds that reaches the reference on a majority
/// of its common models, or the deepest run as a lower bound when none does.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpeedRun {
    pub device: ClassicalDevice,
    pub name: String,
    pub wall_s: f64,
    pub device_s: f64,
    pub qpu_wall_s: f64,
    pub reached: bool,
    pub models: usize,
    pub estimated: bool,
}

impl SpeedRun {
    /// Classical wall per model over QPU wall per model. A bound when `reached` is false.
    #[must_use]
    pub fn ratio(&self) -> f64 {
        self.wall_s / self.qpu_wall_s
    }

    #[must_use]
    pub fn qpu_ahead(&self) -> bool {
        self.ratio() >= SPEED_RATIO
    }
}

/// The speed pick over both devices among runs with a timed wall, device-seconds, and at least
/// [`MIN_MODELS`] common models. Runs sharing a rung form one candidate. A reached candidate wins
/// over any bound. Among reached candidates the fewest device-seconds win, among bounds the most.
/// Ties go to CUDA, then to the earlier candidate. A candidate whose reach depends on missing
/// states is unresolved and takes no part. Without a finite postprocess time there is no pick.
#[must_use]
pub fn speed_run(reference: &QpuReference, runs: &[ClassicalRun]) -> Option<SpeedRun> {
    let post_s = seconds(Some(reference.post_s))?;
    let qpu_wall_s = qpu_wall(post_s);
    let picks: Vec<SpeedRun> = [ClassicalDevice::Cuda, ClassicalDevice::Cpu]
        .into_iter()
        .filter_map(|device| {
            let mut by_rung: Vec<Vec<&ClassicalRun>> = Vec::new();
            for run in runs.iter().filter(|run| run.device == device) {
                let shared = run.rung.as_ref().and_then(|rung| {
                    by_rung
                        .iter_mut()
                        .find(|members| members[0].rung.as_ref() == Some(rung))
                });
                match shared {
                    Some(members) => members.push(run),
                    None => by_rung.push(vec![run]),
                }
            }
            let candidates: Vec<SpeedRun> = by_rung
                .iter()
                .filter_map(|members| speed_candidate(reference, members, qpu_wall_s))
                .collect();
            pick(candidates)
        })
        .collect();
    pick(picks)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    Reached,
    NotReached,
    /// The missing states decide whether the run reaches.
    Unresolved,
}

/// Whether a run is at or below the reference on more than half of its common models.
fn reach(reference: &QpuReference, run: &ClassicalRun) -> Reach {
    let common = run.common(reference);
    let at_or_below = common
        .iter()
        .filter(|(target, energy)| energy.is_some_and(|energy| energy <= *target))
        .count();
    let missing = common.iter().filter(|(_, energy)| energy.is_none()).count();
    if at_or_below * 2 > common.len() {
        Reach::Reached
    } else if (at_or_below + missing) * 2 > common.len() {
        Reach::Unresolved
    } else {
        Reach::NotReached
    }
}

fn speed_candidate(
    reference: &QpuReference,
    members: &[&ClassicalRun],
    qpu_wall_s: f64,
) -> Option<SpeedRun> {
    let first = members.first()?;
    let wall_s = first.tier_wall()?;
    let device_s = seconds(first.device_s)?;
    let models = first.common(reference).len();
    if models < MIN_MODELS {
        return None;
    }
    let outcomes: Vec<Reach> = members.iter().map(|run| reach(reference, run)).collect();
    let reached = if outcomes.contains(&Reach::NotReached) {
        false
    } else if outcomes.contains(&Reach::Unresolved) {
        return None;
    } else {
        true
    };
    Some(SpeedRun {
        device: first.device,
        name: first.name.clone(),
        wall_s,
        device_s,
        qpu_wall_s,
        reached,
        models,
        estimated: first.estimated(),
    })
}

/// The cheapest reached candidate, else the most expensive bound. The first candidate wins a tie.
fn pick(candidates: Vec<SpeedRun>) -> Option<SpeedRun> {
    let reached = candidates.iter().any(|candidate| candidate.reached);
    candidates
        .into_iter()
        .filter(|candidate| candidate.reached == reached)
        .reduce(|best, next| {
            let better = if reached {
                next.device_s < best.device_s
            } else {
                next.device_s > best.device_s
            };
            if better {
                next
            } else {
                best
            }
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// The QPU wins energy and speed.
    One,
    /// The QPU holds energy, and classical matches it fast enough.
    Two,
    /// Classical digs deeper within 120 s and needs 5 times the QPU wall to match it.
    Three,
    /// Classical wins energy and speed.
    Four,
    /// The evidence is below the model floor.
    NeedsMeasurement,
}

impl Tier {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::One => "1",
            Self::Two => "2",
            Self::Three => "3",
            Self::Four => "4",
            Self::NeedsMeasurement => "Needs measurement",
        }
    }
}

/// A historical Tier label with its deciding evidence.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TierLabel {
    pub rule_version: u32,
    pub tier: Tier,
    pub reason: String,
    /// The beating run with the largest share, else the best run within the budget.
    pub beat_run: Option<TierRun>,
    pub speed: Option<SpeedRun>,
}

/// The Tier of `tier-rules.md` version 3, or `None` without a QPU reference.
#[must_use]
pub fn tier(reference: &QpuReference, runs: &[ClassicalRun]) -> Option<TierLabel> {
    if reference.energies_milli.is_empty() {
        return None;
    }
    let compared = tier_runs(reference, runs);
    let speed = speed_run(reference, runs);
    let missing: Vec<&str> = [ClassicalDevice::Cpu, ClassicalDevice::Cuda]
        .into_iter()
        .filter(|device| compared.iter().all(|run| run.device != *device))
        .map(ClassicalDevice::label)
        .collect();
    let beating = compared.iter().any(TierRun::beats);
    let beat_run = deciding_run(&compared, beating);
    let label = |tier, reason| TierLabel {
        rule_version: TIER_RULE_VERSION,
        tier,
        reason,
        beat_run: beat_run.clone(),
        speed: speed.clone(),
    };
    let floor = speed.as_ref().filter(|speed| speed.models >= MIN_MODELS);
    let (Some(run), Some(speed), true) = (&beat_run, floor, missing.is_empty()) else {
        let mut parts: Vec<String> = missing
            .iter()
            .map(|device| {
                format!(
                    "no {device} run within {BEAT_BUDGET_S} s covers {MIN_MODELS} reference models"
                )
            })
            .collect();
        if floor.is_none() {
            parts.push(format!(
                "no run reaches or bounds the reference on {MIN_MODELS} models"
            ));
        }
        return Some(label(Tier::NeedsMeasurement, parts.join("; ")));
    };
    let verb = if beating { "beats" } else { "does not beat" };
    let estimated = |estimated: bool| if estimated { " (estimated wall)" } else { "" };
    let reason = format!(
        "Beat test: {} `{}`{} {verb} the QPU, below the reference on {} of {} models. \
         Speed test: {} `{}` {} at {:.3} s per model{}, {:.1} times the QPU wall of {:.3} s.",
        run.device.label(),
        run.name,
        estimated(run.estimated),
        run.classical_wins,
        run.common,
        speed.device.label(),
        speed.name,
        if speed.reached {
            "reaches the reference"
        } else {
            "never reaches the reference, a lower bound"
        },
        speed.wall_s,
        estimated(speed.estimated),
        speed.ratio(),
        speed.qpu_wall_s,
    );
    let tier = match (beating, speed.qpu_ahead()) {
        (false, true) => Tier::One,
        (false, false) => Tier::Two,
        (true, true) => Tier::Three,
        (true, false) => Tier::Four,
    };
    Some(label(tier, reason))
}

/// The beating run with the largest share and then the shorter wall, else the run with the most
/// common models, then the lowest mean classical energy, device, and name.
fn deciding_run(compared: &[TierRun], beating: bool) -> Option<TierRun> {
    if beating {
        compared
            .iter()
            .filter(|run| run.beats())
            .reduce(|best, next| {
                let key = |run: &TierRun| (run.share(), -run.wall_s);
                if key(next).partial_cmp(&key(best)) == Some(std::cmp::Ordering::Greater) {
                    next
                } else {
                    best
                }
            })
            .cloned()
    } else {
        compared
            .iter()
            .min_by(|left, right| {
                right
                    .common
                    .cmp(&left.common)
                    .then(
                        left.classical_mean_milli
                            .total_cmp(&right.classical_mean_milli),
                    )
                    .then(left.device.label().cmp(right.device.label()))
                    .then(left.name.cmp(&right.name))
            })
            .cloned()
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Model counts are far below f64's exact integer range."
)]
fn count_f64(count: usize) -> f64 {
    count as f64
}

#[expect(
    clippy::cast_precision_loss,
    reason = "Milli energies of hardware-sized models stay far below 2^53."
)]
fn milli_f64(milli: i64) -> f64 {
    milli as f64
}

/// A finite, nonnegative number of seconds, else missing timing.
fn seconds(value: Option<f64>) -> Option<f64> {
    value.filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
}

fn milli_units(milli: i64) -> f64 {
    milli_f64(milli) / 1_000.0
}
