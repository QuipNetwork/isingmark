// SPDX-License-Identifier: AGPL-3.0-or-later
//! Canonical families: an `ising-generators` contract plus a direct witness check.
//!
//! The instance names its descriptor, base topology, and nonce. Encoding regenerates
//! the canonical instance from them and requires its digest to equal the pinned
//! `instance_digest`, so a reordered, truncated, or edited instance fails.

use ising_generators::{
    descriptor::{admit, params_digest, AdvertisedLimits, Contract, Descriptor, IdContext},
    family::{admit_topology, generate_instance, BaseTopology, SaltFault},
    instance::Instance as Generated,
    Code,
};
use serde::Deserialize;
use serde_json::Value;

use super::super::{
    contracts::{
        EncodingOutcome, EvalError, Evaluation, EvaluationContext, FamilyConfig, FamilyEvaluator,
        IsingEncoding, Witness,
    },
    instance::{decode_hex, instance_from_value, Instance, MAX_SPINS},
    CampaignError,
};
use super::{canonical_bytes, check_witness, exact_evaluation};
use crate::modes::hex_encode;

/// Checks the family constraints of a witness's spins against the generated instance.
pub type WitnessCheck = fn(&Generated, &[i8]) -> Result<(), String>;

/// One canonical family revision: its generator contract and its direct constraint check.
#[derive(Debug)]
pub struct CanonicalFamily {
    pub contract: &'static Contract,
    pub check_witness: WitnessCheck,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Physical {
    descriptor: DescriptorFields,
    topology: TopologyFields,
    instance_digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorFields {
    family_id: String,
    revision: u32,
    params_hex: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TopologyFields {
    nodes: Vec<u32>,
    edges: Vec<(u32, u32)>,
}

fn eval_error(code: &'static str, reason: impl Into<String>) -> EvalError {
    EvalError {
        code,
        reason: reason.into(),
    }
}

fn code_error(code: Code) -> EvalError {
    let name = match code {
        Code::DescriptorMissing => "descriptor_missing",
        Code::DescriptorUnexpected => "descriptor_unexpected",
        Code::FamilyIdInvalid => "family_id_invalid",
        Code::RevisionZero => "revision_zero",
        Code::FamilyUnsupported => "family_unsupported",
        Code::RevisionUnsupported => "revision_unsupported",
        Code::ParamsMalformed => "params_malformed",
        Code::ParamsDigestMismatch => "params_digest_mismatch",
        Code::ParamsOutOfBounds => "params_out_of_bounds",
        Code::ParamsRestricted => "params_restricted",
        Code::TopologyOrderMismatch => "topology_order_mismatch",
        Code::TopologyIncompatible => "topology_incompatible",
        Code::BoundsExceeded => "bounds_exceeded",
        Code::NoInstance => "no_instance",
        Code::InstanceDigestMismatch => "instance_digest_mismatch",
        Code::MetadataInvalid => "metadata_invalid",
        _ => "generator_code",
    };
    eval_error(name, format!("generator code {}", code as u8))
}

fn physical(value: &Value) -> Result<Physical, EvalError> {
    serde_json::from_value(value.clone()).map_err(|error| CampaignError::Parse(error).into())
}

/// The lowercase hex descriptor and instance digests of a canonical instance.
///
/// # Errors
/// Returns `parse` for a `physical` field without the canonical shape.
pub(crate) fn pinned_digests(instance: &Instance) -> Result<(String, String), EvalError> {
    let physical = physical(&instance.physical)?;
    let params = decode_hex(&physical.descriptor.params_hex)?;
    let descriptor = Descriptor::new(
        physical.descriptor.family_id.as_bytes(),
        physical.descriptor.revision,
        &params,
        IdContext::Lease,
    )
    .map_err(code_error)?;
    Ok((hex_encode(&descriptor.digest()), physical.instance_digest))
}

impl CanonicalFamily {
    fn check_identity(&self, instance: &Instance, physical: &Physical) -> Result<(), EvalError> {
        let revision = self.contract.revision;
        if instance.family != self.contract.family_id
            || physical.descriptor.family_id != self.contract.family_id
            || instance.model_version != revision.to_string()
            || physical.descriptor.revision != revision
        {
            return Err(eval_error(
                "family_mismatch",
                format!(
                    "family, model_version, and descriptor must all name {} revision {revision}",
                    self.contract.family_id
                ),
            ));
        }
        Ok(())
    }

    fn build(&self, physical: &Physical, nonce: [u8; 32]) -> Result<Generated, EvalError> {
        let spins =
            u64::try_from(physical.topology.nodes.len()).map_err(|_| CampaignError::Overflow)?;
        if spins > MAX_SPINS {
            return Err(CampaignError::TooLarge {
                what: "spins",
                limit: MAX_SPINS,
            }
            .into());
        }
        let params = decode_hex(&physical.descriptor.params_hex)?;
        let admitted = admit(
            std::slice::from_ref(self.contract),
            physical.descriptor.family_id.as_bytes(),
            physical.descriptor.revision,
            &params,
            &params_digest(&params),
        )
        .map_err(code_error)?;
        let topology = BaseTopology::new(physical.topology.nodes.clone(), &physical.topology.edges)
            .map_err(|fault| eval_error("topology_invalid", format!("{fault:?}")))?;
        let advertised = AdvertisedLimits {
            max_nodes: MAX_SPINS,
            ..AdvertisedLimits::default()
        };
        let bounds = admit_topology(&admitted, &topology, topology.order_digest(), &advertised)
            .map_err(code_error)?;
        generate_instance(&admitted, &topology, &bounds, nonce).map_err(|fault| match fault {
            SaltFault::Skip(code) => code_error(code),
            SaltFault::Defect(defect) => eval_error("generator_defect", format!("{defect:?}")),
        })
    }

    /// Regenerate the instance and require its digest to equal the pin.
    fn generated(&self, instance: &Instance) -> Result<Generated, EvalError> {
        let physical = physical(&instance.physical)?;
        self.check_identity(instance, &physical)?;
        let generated = self.build(&physical, instance.nonce_bytes()?)?;
        if hex_encode(&generated.digest()) != physical.instance_digest {
            return Err(code_error(Code::InstanceDigestMismatch));
        }
        Ok(generated)
    }
}

fn encoding(generated: &Generated) -> IsingEncoding {
    IsingEncoding {
        variables: generated
            .node_ids()
            .iter()
            .map(|id| format!("n{id}"))
            .collect(),
        h: generated.h().to_vec(),
        j: generated.j().to_vec(),
        edges: generated.edges().to_vec(),
        units: "milli".to_owned(),
        offset: generated.offset_milli(),
        auxiliaries: Vec::new(),
        precision_bound: 0.0,
        exact: true,
    }
}

impl FamilyEvaluator for CanonicalFamily {
    /// `params` is the instance without `nonce` and without `physical.instance_digest`.
    /// Generation sets both, so the nonce alone selects the instance.
    fn generate(&self, config: &FamilyConfig, nonce: [u8; 32]) -> Result<Instance, EvalError> {
        if config.family != self.contract.family_id {
            return Err(eval_error(
                "family_mismatch",
                "configuration names another family",
            ));
        }
        let mut value = config.params.clone();
        let template = value
            .as_object_mut()
            .ok_or_else(|| eval_error("invalid_config", "params must be an instance object"))?;
        template.insert("nonce".to_owned(), Value::String(hex_encode(&nonce)));
        let physical = template
            .get_mut("physical")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| eval_error("invalid_config", "params.physical must be an object"))?;
        // A placeholder pin parses; it is replaced before the instance is returned.
        physical.insert("instance_digest".to_owned(), Value::String(String::new()));
        let mut instance = instance_from_value(value)?;
        let parsed = self::physical(&instance.physical)?;
        self.check_identity(&instance, &parsed)?;
        let digest = hex_encode(&self.build(&parsed, nonce)?.digest());
        instance.physical["instance_digest"] = Value::String(digest);
        Ok(instance)
    }

    fn encode(&self, instance: &Instance) -> Result<EncodingOutcome, EvalError> {
        Ok(EncodingOutcome::Exact(encoding(&self.generated(instance)?)))
    }

    fn evaluate(
        &self,
        instance: &Instance,
        witness: &Witness,
        _context: &EvaluationContext,
    ) -> Result<Evaluation, EvalError> {
        let generated = self.generated(instance)?;
        let encoding = encoding(&generated);
        check_witness(instance, &encoding, witness)?;
        let violations = match (self.check_witness)(&generated, &witness.spins) {
            Ok(()) => Vec::new(),
            Err(reason) => vec![format!("constraint_violated: {reason}")],
        };
        exact_evaluation(instance, &encoding, witness, violations)
    }

    fn canonicalize(&self, instance: &Instance, witness: &Witness) -> Result<Vec<u8>, EvalError> {
        check_witness(instance, &encoding(&self.generated(instance)?), witness)?;
        canonical_bytes(witness)
    }
}
