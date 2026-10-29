// SPDX-License-Identifier: AGPL-3.0-or-later
//! Proof replay through the pinned chain verifier and a distinct-witness rule.

use std::{collections::BTreeSet, fs::File, path::Path};

use quip_protocol::{
    diversity::diversity_milli,
    target::{meets_target, validate_proof_set, Target, TargetMiss},
};
use serde::{Deserialize, Serialize};

use super::{
    contracts::{EvalError, Witness},
    families::fixed::FixedIsing,
    instance::{fixed_encoding, Instance},
    CampaignError,
};

/// Update this pin whenever the `quip-protocol` dependency version changes.
pub const PINNED_VERIFIER_VERSION: &str = "0.0.2";

/// Evidence and rule labels for the compiled verifier's replay profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolProfile {
    pub profile_id: String,
    pub verifier_crate: String,
    pub verifier_version: String,
    pub verifier_rev_evidence: String,
    pub threshold: String,
    pub count: String,
    pub diversity: String,
    pub subset: String,
    pub ranking: String,
    pub decay: String,
    pub inclusion: String,
    pub tie_rule: String,
}

impl ProtocolProfile {
    fn check_pin(&self) -> Result<(), CampaignError> {
        if self.verifier_version != PINNED_VERIFIER_VERSION {
            return Err(CampaignError::InvalidRun(format!(
                "profile verifier version {} differs from pinned quip-protocol {PINNED_VERIFIER_VERSION}",
                self.verifier_version
            )));
        }
        Ok(())
    }
}

/// Load a profile that names the compiled verifier version.
///
/// # Errors
/// Returns an error for unreadable JSON, missing fields, or a different version.
pub fn load_protocol_profile(path: &Path) -> Result<ProtocolProfile, CampaignError> {
    let profile: ProtocolProfile = serde_json::from_reader(File::open(path)?)?;
    profile.check_pin()?;
    Ok(profile)
}

/// The acceptance result for one candidate proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProofDecision {
    Accepted {
        best_energy_milli: i64,
        diversity_milli: u32,
        valid_solution_count: u32,
    },
    Rejected {
        reason: String,
    },
}

/// The current chain rule or an independent count of valid canonical witnesses.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "The RuleSet interface requires Current to carry ProtocolProfile by value."
)]
pub enum RuleSet {
    Current(ProtocolProfile),
    ProposedUnique { min_distinct: u32 },
}

impl RuleSet {
    /// Check a candidate proof against this rule set.
    ///
    /// # Errors
    /// Returns an error for unsupported families, invalid instances, or profiles.
    pub fn validate_set(
        &self,
        instance: &Instance,
        witnesses: &[Witness],
    ) -> Result<ProofDecision, CampaignError> {
        match self {
            Self::Current(profile) => validate_set(instance, witnesses, profile),
            Self::ProposedUnique { min_distinct } => {
                validate_unique(instance, witnesses, (*min_distinct).max(1))
            }
        }
    }
}

/// Replay a complete candidate proof through the pinned chain verifier.
///
/// # Errors
/// Returns an error for unsupported families, invalid instance data, or profiles.
pub fn validate_set(
    instance: &Instance,
    witnesses: &[Witness],
    profile: &ProtocolProfile,
) -> Result<ProofDecision, CampaignError> {
    if instance.family != "ising-fixed" {
        return Err(unsupported_family(instance));
    }
    if witnesses
        .iter()
        .any(|witness| witness.instance_id != instance.instance_id)
    {
        return Ok(rejected("CrossInstanceWitness"));
    }
    profile.check_pin()?;
    let target = target_from_objective(instance)?;
    let mut set: Vec<_> = witnesses
        .iter()
        .map(|witness| (witness.spins.as_slice(), target.max_energy_milli))
        .collect();
    // Equality energies cannot pass. Forward only structural failures here so
    // the pinned size/shape checks retain their order before family scoring.
    if let Err(miss @ (TargetMiss::TooManySolutions | TargetMiss::MalformedSolutions)) =
        validate_proof_set(&set, &target)
    {
        return Ok(rejected(&format!("{miss:?}")));
    }
    fixed_encoding(instance)?;
    for ((_, energy), witness) in set.iter_mut().zip(witnesses) {
        match FixedIsing::evaluate_with_energy(instance, witness) {
            Ok((_, exact_energy)) => *energy = exact_energy,
            Err(error) => return evaluation_failure(error),
        }
    }
    Ok(match validate_proof_set(&set, &target) {
        Ok(stats) => ProofDecision::Accepted {
            best_energy_milli: stats.best_energy_milli,
            diversity_milli: stats.diversity_milli,
            valid_solution_count: stats.valid_solution_count,
        },
        Err(miss) => rejected(&format!("{miss:?}")),
    })
}

/// Select proof indices with the pinned miner-side selection procedure.
///
/// # Errors
/// Returns `InvalidRun` containing the pinned `TargetMiss` variant on failure.
pub fn select_proof(reads: &[(&[i8], i64)], target: &Target) -> Result<Vec<usize>, CampaignError> {
    meets_target(reads, target)
        .map(|proof| proof.indices)
        .map_err(|miss| CampaignError::InvalidRun(format!("{miss:?}")))
}

pub(crate) fn target_from_objective(instance: &Instance) -> Result<Target, CampaignError> {
    let energy = instance
        .objective
        .get("max_energy_milli")
        .ok_or(CampaignError::MissingField("max_energy_milli"))?;
    let count = |field, default| {
        instance.objective.get(field).map_or(Ok(default), |value| {
            serde_json::from_value(value.clone()).map_err(CampaignError::from)
        })
    };
    Ok(Target {
        max_energy_milli: serde_json::from_value(energy.clone())?,
        min_solutions: count("min_solutions", 1)?,
        min_diversity_milli: count("min_diversity_milli", 0)?,
        max_proof_solutions: count("max_proof_solutions", u32::MAX)?,
    })
}

fn validate_unique(
    instance: &Instance,
    witnesses: &[Witness],
    min_distinct: u32,
) -> Result<ProofDecision, CampaignError> {
    if instance.family != "ising-fixed" {
        return Err(unsupported_family(instance));
    }
    if witnesses
        .iter()
        .any(|witness| witness.instance_id != instance.instance_id)
    {
        return Ok(rejected("CrossInstanceWitness"));
    }
    fixed_encoding(instance)?;
    let mut canonical_ids = BTreeSet::new();
    let mut distinct_spins = Vec::new();
    let mut best_energy: Option<i64> = None;
    for witness in witnesses {
        let (evaluation, energy) = match FixedIsing::evaluate_with_energy(instance, witness) {
            Ok(result) => result,
            Err(error) => return evaluation_failure(error),
        };
        if !evaluation.valid {
            continue;
        }
        let canonical_id = evaluation
            .canonical_id
            .ok_or(CampaignError::MissingField("canonical_id"))?;
        if canonical_ids.insert(canonical_id) {
            distinct_spins.push(witness.spins.as_slice());
            best_energy = Some(best_energy.map_or(energy, |best| best.min(energy)));
        }
    }
    let distinct_count = u32::try_from(canonical_ids.len()).map_err(|_| CampaignError::Overflow)?;
    let Some(best_energy_milli) = best_energy.filter(|_| distinct_count >= min_distinct) else {
        return Ok(rejected("InsufficientDistinct"));
    };
    Ok(ProofDecision::Accepted {
        best_energy_milli,
        diversity_milli: diversity_milli(&distinct_spins),
        valid_solution_count: distinct_count,
    })
}

fn unsupported_family(instance: &Instance) -> CampaignError {
    CampaignError::InvalidRun(format!(
        "unsupported family for current-rule replay: {}",
        instance.family
    ))
}

fn evaluation_failure(error: EvalError) -> Result<ProofDecision, CampaignError> {
    if error.code == "length_mismatch" || error.code == "invalid_spin" {
        return Ok(rejected("MalformedSolutions"));
    }
    Err(error.into())
}

fn rejected(reason: &str) -> ProofDecision {
    ProofDecision::Rejected {
        reason: reason.to_owned(),
    }
}
