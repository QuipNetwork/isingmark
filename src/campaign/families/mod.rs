// SPDX-License-Identifier: AGPL-3.0-or-later
//! Campaign objective families and the registry that resolves them.

use std::collections::BTreeMap;

use super::{
    contracts::{EvalError, Evaluation, FamilyEvaluator, IsingEncoding, Witness},
    instance::Instance,
    CampaignError,
};

pub mod canonical;
pub mod check;
pub mod fixed;
pub mod numeric;

use canonical::CanonicalFamily;
use fixed::FixedIsing;

/// The family id of the explicit control.
pub const FIXED_FAMILY: &str = "ising-fixed";
/// The only revision of the explicit control.
pub const FIXED_REVISION: &str = "1";

/// The canonical families this build supports. SC-8 and later add entries.
const PRODUCTION: &[CanonicalFamily] = &[];

/// Resolve a family id and revision to its evaluator.
#[derive(Debug, Clone, Copy)]
pub struct FamilyRegistry {
    canonical: &'static [CanonicalFamily],
}

impl Default for FamilyRegistry {
    fn default() -> Self {
        Self::production()
    }
}

impl FamilyRegistry {
    /// The explicit control plus every canonical family this build supports.
    #[must_use]
    pub const fn production() -> Self {
        Self {
            canonical: PRODUCTION,
        }
    }

    /// The explicit control plus `families`, for callers that bring their own contracts.
    #[must_use]
    pub const fn with_canonical(families: &'static [CanonicalFamily]) -> Self {
        Self {
            canonical: families,
        }
    }

    /// The evaluator for `family` at `revision`, a decimal string.
    ///
    /// # Errors
    /// `family_unsupported` for an unknown id, `revision_unsupported` for a known id at
    /// another revision.
    pub fn evaluator(
        &self,
        family: &str,
        revision: &str,
    ) -> Result<&'static dyn FamilyEvaluator, EvalError> {
        let unsupported = |code| EvalError {
            code,
            reason: format!("{family} revision {revision}"),
        };
        if family == FIXED_FAMILY {
            return if revision == FIXED_REVISION {
                Ok(&FixedIsing)
            } else {
                Err(unsupported("revision_unsupported"))
            };
        }
        let mut known = false;
        for entry in self.canonical {
            if entry.contract.family_id == family {
                known = true;
                if entry.contract.revision.to_string() == revision {
                    return Ok(entry);
                }
            }
        }
        Err(unsupported(if known {
            "revision_unsupported"
        } else {
            "family_unsupported"
        }))
    }

    /// Whether any revision of `family` is registered.
    #[must_use]
    pub fn has_family(&self, family: &str) -> bool {
        family == FIXED_FAMILY
            || self
                .canonical
                .iter()
                .any(|entry| entry.contract.family_id == family)
    }

    /// The evaluator named by the instance's `family` and `model_version`.
    ///
    /// # Errors
    /// Same as [`FamilyRegistry::evaluator`].
    pub fn for_instance(
        &self,
        instance: &Instance,
    ) -> Result<&'static dyn FamilyEvaluator, EvalError> {
        self.evaluator(&instance.family, &instance.model_version)
    }
}

/// Check the witness against its instance and encoding before scoring it.
pub(crate) fn check_witness(
    instance: &Instance,
    encoding: &IsingEncoding,
    witness: &Witness,
) -> Result<(), EvalError> {
    witness.validate()?;
    if witness.instance_id != instance.instance_id {
        return Err(EvalError {
            code: "instance_mismatch",
            reason: "witness belongs to another instance".to_owned(),
        });
    }
    if witness.spins.len() != encoding.h.len() {
        return Err(CampaignError::LengthMismatch.into());
    }
    if witness.spins.iter().any(|&spin| spin != -1 && spin != 1) {
        return Err(EvalError {
            code: "invalid_spin",
            reason: "spins must be -1 or 1".to_owned(),
        });
    }
    Ok(())
}

/// The exact energy of checked spins in the encoding's integer units.
pub(crate) fn exact_energy(encoding: &IsingEncoding, spins: &[i8]) -> Result<i64, EvalError> {
    let mut energy = encoding.offset;
    for (&coefficient, &spin) in encoding.h.iter().zip(spins) {
        energy = energy
            .checked_add(i64::from(coefficient) * i64::from(spin))
            .ok_or(CampaignError::Overflow)?;
    }
    for (&coefficient, &(left, right)) in encoding.j.iter().zip(&encoding.edges) {
        let left = usize::try_from(left).map_err(|_| CampaignError::Overflow)?;
        let right = usize::try_from(right).map_err(|_| CampaignError::Overflow)?;
        let (Some(&left), Some(&right)) = (spins.get(left), spins.get(right)) else {
            return Err(CampaignError::LengthMismatch.into());
        };
        energy = energy
            .checked_add(i64::from(coefficient) * i64::from(left) * i64::from(right))
            .ok_or(CampaignError::Overflow)?;
    }
    Ok(energy)
}

/// The instance's strict energy threshold, `max_energy_milli`.
pub(crate) fn energy_threshold(instance: &Instance) -> Result<i64, EvalError> {
    instance
        .objective
        .get("max_energy_milli")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| EvalError {
            code: "invalid_objective",
            reason: "max_energy_milli must be an i64 integer".to_owned(),
        })
}

/// Orientation-preserving identity bytes: a nonzero field breaks global reversal symmetry.
pub(crate) fn canonical_bytes(witness: &Witness) -> Result<Vec<u8>, EvalError> {
    serde_json::to_vec(&(&witness.instance_id, &witness.spins))
        .map_err(|error| CampaignError::Parse(error).into())
}

/// Score `witness`: valid when its energy is strictly below the threshold and every
/// family constraint in `violations` is absent.
#[expect(
    clippy::cast_precision_loss,
    reason = "Integer energy decides acceptance; the score has a separate rounding bound."
)]
pub(crate) fn exact_evaluation(
    instance: &Instance,
    encoding: &IsingEncoding,
    witness: &Witness,
    violations: Vec<String>,
) -> Result<Evaluation, EvalError> {
    let threshold = energy_threshold(instance)?;
    let energy = exact_energy(encoding, &witness.spins)?;
    let mut rejections = violations;
    if energy >= threshold {
        rejections.push("energy_not_below_threshold".to_owned());
    }
    let score = energy as f64;
    let uncertainty = if energy.unsigned_abs() <= (1_u64 << 53) {
        0.0
    } else {
        score.abs() * f64::EPSILON
    };
    Ok(Evaluation {
        valid: rejections.is_empty(),
        score: Some(score),
        units: encoding.units.clone(),
        canonical_id: Some(
            blake3::hash(&canonical_bytes(witness)?)
                .to_hex()
                .to_string(),
        ),
        observables: BTreeMap::from([("energy_milli".to_owned(), score)]),
        uncertainty,
        rejections,
        energy_milli: Some(energy),
    })
}
