// SPDX-License-Identifier: AGPL-3.0-or-later
//! The explicit, integer-energy `ising-fixed` control family.

use std::collections::BTreeMap;

use super::super::{
    contracts::{
        EncodingOutcome, EvalError, Evaluation, EvaluationContext, FamilyConfig, FamilyEvaluator,
        IsingEncoding, Witness,
    },
    instance::{fixed_encoding, instance_from_value, Instance},
    CampaignError,
};

/// An exact control whose coefficients and threshold are supplied explicitly.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedIsing;

impl FamilyEvaluator for FixedIsing {
    /// `params` holds a complete explicit instance. The seed never redraws it.
    fn generate(&self, config: &FamilyConfig, _seed: u64) -> Result<Instance, EvalError> {
        if config.family != "ising-fixed" {
            return Err(EvalError {
                code: "family_mismatch",
                reason: "expected ising-fixed configuration".to_owned(),
            });
        }
        let instance = instance_from_value(config.params.clone())?;
        fixed_encoding(&instance)?;
        Ok(instance)
    }

    fn encode(&self, instance: &Instance) -> Result<EncodingOutcome, EvalError> {
        Ok(EncodingOutcome::Exact(fixed_encoding(instance)?))
    }

    fn evaluate(
        &self,
        instance: &Instance,
        witness: &Witness,
        _context: &EvaluationContext,
    ) -> Result<Evaluation, EvalError> {
        Self::evaluate_with_energy(instance, witness).map(|(evaluation, _)| evaluation)
    }

    fn canonicalize(&self, instance: &Instance, witness: &Witness) -> Result<Vec<u8>, EvalError> {
        checked_witness(instance, witness)?;
        canonical_bytes(witness)
    }
}

impl FixedIsing {
    /// Keep the exact energy alongside the evaluation's floating-point score.
    #[expect(
        clippy::cast_precision_loss,
        reason = "Integer energy decides acceptance; the score has a separate rounding bound."
    )]
    pub(crate) fn evaluate_with_energy(
        instance: &Instance,
        witness: &Witness,
    ) -> Result<(Evaluation, i64), EvalError> {
        let encoding = checked_witness(instance, witness)?;
        let threshold = instance
            .objective
            .get("max_energy_milli")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| EvalError {
                code: "invalid_objective",
                reason: "max_energy_milli must be an i64 integer".to_owned(),
            })?;
        let mut energy = encoding.offset;
        for (&coefficient, &spin) in encoding.h.iter().zip(&witness.spins) {
            energy = energy
                .checked_add(i64::from(coefficient) * i64::from(spin))
                .ok_or(CampaignError::Overflow)?;
        }
        for (&coefficient, &(left, right)) in encoding.j.iter().zip(&encoding.edges) {
            let left = usize::try_from(left).map_err(|_| CampaignError::Overflow)?;
            let right = usize::try_from(right).map_err(|_| CampaignError::Overflow)?;
            energy = energy
                .checked_add(
                    i64::from(coefficient)
                        * i64::from(witness.spins[left])
                        * i64::from(witness.spins[right]),
                )
                .ok_or(CampaignError::Overflow)?;
        }
        let valid = energy < threshold;
        let score = energy as f64;
        let uncertainty = if energy.unsigned_abs() <= (1_u64 << 53) {
            0.0
        } else {
            score.abs() * f64::EPSILON
        };
        let evaluation = Evaluation {
            valid,
            score: Some(score),
            units: encoding.units,
            canonical_id: Some(
                blake3::hash(&canonical_bytes(witness)?)
                    .to_hex()
                    .to_string(),
            ),
            observables: BTreeMap::from([("energy_milli".to_owned(), score)]),
            uncertainty,
            rejections: if valid {
                Vec::new()
            } else {
                vec!["energy_not_below_threshold".to_owned()]
            },
        };
        Ok((evaluation, energy))
    }
}

fn checked_witness(instance: &Instance, witness: &Witness) -> Result<IsingEncoding, EvalError> {
    witness.validate()?;
    if witness.instance_id != instance.instance_id {
        return Err(EvalError {
            code: "instance_mismatch",
            reason: "witness belongs to another instance".to_owned(),
        });
    }
    let encoding = fixed_encoding(instance)?;
    if witness.spins.len() != encoding.h.len() {
        return Err(CampaignError::LengthMismatch.into());
    }
    if witness.spins.iter().any(|&spin| spin != -1 && spin != 1) {
        return Err(EvalError {
            code: "invalid_spin",
            reason: "spins must be -1 or 1".to_owned(),
        });
    }
    Ok(encoding)
}

fn canonical_bytes(witness: &Witness) -> Result<Vec<u8>, EvalError> {
    // Preserve orientation: a nonzero field can break global reversal symmetry.
    serde_json::to_vec(&(&witness.instance_id, &witness.spins))
        .map_err(|error| CampaignError::Parse(error).into())
}
