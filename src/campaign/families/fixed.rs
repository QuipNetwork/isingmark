// SPDX-License-Identifier: AGPL-3.0-or-later
//! The explicit, integer-energy `ising-fixed` control family.

use super::super::{
    contracts::{
        EncodingOutcome, EvalError, Evaluation, EvaluationContext, FamilyConfig, FamilyEvaluator,
        Witness,
    },
    instance::{fixed_encoding, instance_from_value, Instance},
};
use super::{canonical_bytes, check_witness, exact_evaluation, FIXED_FAMILY};

/// An exact control whose coefficients and threshold are supplied explicitly.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixedIsing;

impl FamilyEvaluator for FixedIsing {
    /// `params` holds a complete explicit instance. The nonce never redraws it.
    fn generate(&self, config: &FamilyConfig, _nonce: [u8; 32]) -> Result<Instance, EvalError> {
        if config.family != FIXED_FAMILY {
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
        let encoding = fixed_encoding(instance)?;
        check_witness(instance, &encoding, witness)?;
        exact_evaluation(instance, &encoding, witness, Vec::new())
    }

    fn canonicalize(&self, instance: &Instance, witness: &Witness) -> Result<Vec<u8>, EvalError> {
        check_witness(instance, &fixed_encoding(instance)?, witness)?;
        canonical_bytes(witness)
    }
}
