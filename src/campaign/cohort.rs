// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fresh cohorts of canonical-family instances, written as campaign input files.
//!
//! A cohort is `count` instances of one registered recipe on one base topology. Each nonce
//! comes from a fixed rule over the recipe name and the index, so the same arguments always
//! give the same cohort. The command writes the instances, a manifest built from a policy
//! template, and a cohort record with the family, revision, parameter digest, nonces, and
//! instance digests.

use std::{
    fs,
    path::{Path, PathBuf},
};

use ising_generators::descriptor::params_digest;
use serde::Serialize;
use serde_json::{json, Value};

use super::{
    contracts::FamilyConfig,
    families::{canonical::pinned_digests, FamilyRegistry},
    CampaignError,
};
use crate::{modes::hex_encode, topology::load_topology};

/// The BLAKE3 key-derivation context of the cohort nonce rule.
pub const NONCE_CONTEXT: &str = "isingmark cohort nonce v1";

/// A registered parameter set of a canonical family.
#[derive(Debug, Clone, Copy)]
pub struct CohortRecipe {
    pub family_id: &'static str,
    pub revision: u32,
    pub alias: &'static str,
    pub params: &'static [u8],
}

/// The recipes `isingmark campaign cohort` accepts.
pub const RECIPES: &[CohortRecipe] = &[CohortRecipe {
    family_id: ising_generators::ice::CHARGED_ICE_V1.family_id,
    revision: ising_generators::ice::CHARGED_ICE_V1.revision,
    alias: ising_generators::ice::ICE_CHARGE_RECIPE.alias,
    params: ising_generators::ice::ICE_CHARGE_RECIPE.params,
}];

/// The recipe registered under `alias`.
///
/// # Errors
/// `InvalidRun` for an unknown alias.
pub fn recipe(alias: &str) -> Result<&'static CohortRecipe, CampaignError> {
    RECIPES
        .iter()
        .find(|recipe| recipe.alias == alias)
        .ok_or_else(|| CampaignError::InvalidRun(format!("unknown recipe {alias}")))
}

/// Nonce `index` of the cohort of `alias`: the BLAKE3 key derived under [`NONCE_CONTEXT`]
/// from the alias bytes, a zero byte, and the index as a little-endian `u32`.
#[must_use]
pub fn cohort_nonce(alias: &str, index: u32) -> [u8; 32] {
    let mut material = alias.as_bytes().to_vec();
    material.push(0);
    material.extend_from_slice(&index.to_le_bytes());
    blake3::derive_key(NONCE_CONTEXT, &material)
}

/// What `isingmark campaign cohort` writes.
#[derive(Debug)]
pub struct CohortRequest {
    pub recipe: String,
    pub count: u32,
    pub topology: PathBuf,
    /// The objective threshold of every instance, in natural energy units.
    pub max_energy: f64,
    /// A campaign manifest without `instances`. The cohort adds them.
    pub policies: PathBuf,
    pub out: PathBuf,
}

/// One instance of a cohort record.
#[derive(Debug, Serialize)]
pub struct CohortMember {
    pub instance_id: String,
    pub nonce: String,
    pub instance_digest: String,
    pub file: String,
}

/// The identity of a cohort: enough to regenerate and check every instance.
#[derive(Debug, Serialize)]
pub struct CohortRecord {
    pub family_id: String,
    pub revision: u32,
    pub recipe: String,
    pub params_hex: String,
    pub params_digest: String,
    pub descriptor_digest: String,
    pub topology: String,
    pub topology_hash: String,
    pub nonce_rule: String,
    pub max_energy: f64,
    pub instances: Vec<CohortMember>,
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), CampaignError> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes)?;
    Ok(())
}

/// Generate the cohort into `request.out`, which must not exist.
///
/// # Errors
/// `InvalidRun` when `out` exists, the recipe is unknown, or the policy template already
/// lists instances. Generator, topology, and I/O failures pass through.
pub fn write_cohort(
    registry: &FamilyRegistry,
    request: &CohortRequest,
) -> Result<CohortRecord, CampaignError> {
    if request.out.exists() {
        return Err(CampaignError::InvalidRun(format!(
            "{} exists; a cohort is written only into a new directory",
            request.out.display()
        )));
    }
    let recipe = recipe(&request.recipe)?;
    let mut manifest: Value = serde_json::from_slice(&fs::read(&request.policies)?)?;
    let template = manifest.as_object_mut().ok_or_else(|| {
        CampaignError::InvalidRun("the policy template must be a JSON object".to_owned())
    })?;
    if template.contains_key("instances") {
        return Err(CampaignError::InvalidRun(
            "the policy template must not list instances".to_owned(),
        ));
    }
    let topology = load_topology(&request.topology)
        .map_err(|error| CampaignError::InvalidRun(format!("topology: {error}")))?;
    let evaluator = registry
        .evaluator(recipe.family_id, &recipe.revision.to_string())
        .map_err(CampaignError::Eval)?;
    let params_hex = hex_encode(recipe.params);
    let config = FamilyConfig {
        family: recipe.family_id.to_owned(),
        params: json!({
            "schema_version": 2,
            "instance_id": "",
            "family": recipe.family_id,
            "model_version": recipe.revision.to_string(),
            "challenge_id": format!("cohort-{}", recipe.alias),
            "split": "validation",
            "physical": {
                "descriptor": {
                    "family_id": recipe.family_id,
                    "revision": recipe.revision,
                    "params_hex": params_hex,
                },
                "topology": {"nodes": topology.nodes, "edges": topology.edges},
            },
            "search_space": {},
            "objective": {"max_energy": request.max_energy},
            "constraints": {},
            "hardness": {},
            "source_hashes": {},
        }),
    };
    fs::create_dir_all(&request.out)?;
    let mut members = Vec::new();
    let mut descriptor_digest = String::new();
    for index in 0..request.count {
        let nonce = cohort_nonce(recipe.alias, index);
        let mut instance = evaluator
            .generate(&config, nonce)
            .map_err(CampaignError::Eval)?;
        let nonce_hex = hex_encode(&nonce);
        instance.instance_id = format!("{}-{}", recipe.alias, &nonce_hex[..16]);
        let (descriptor, digest) = pinned_digests(&instance).map_err(CampaignError::Eval)?;
        descriptor_digest = descriptor;
        let file = format!("{}.instance.json", instance.instance_id);
        write_json(&request.out.join(&file), &instance)?;
        members.push(CohortMember {
            instance_id: instance.instance_id,
            nonce: nonce_hex,
            instance_digest: digest,
            file,
        });
    }
    template.insert(
        "instances".to_owned(),
        members.iter().map(|member| json!(member.file)).collect(),
    );
    write_json(&request.out.join("manifest.json"), &manifest)?;
    let record = CohortRecord {
        family_id: recipe.family_id.to_owned(),
        revision: recipe.revision,
        recipe: recipe.alias.to_owned(),
        params_digest: hex_encode(&params_digest(recipe.params)),
        params_hex,
        descriptor_digest,
        topology: request.topology.display().to_string(),
        topology_hash: hex_encode(&topology.topology_hash()),
        nonce_rule: format!(
            "blake3 derive_key(\"{NONCE_CONTEXT}\", alias || 0x00 || u32 index little-endian)"
        ),
        max_energy: request.max_energy,
        instances: members,
    };
    write_json(&request.out.join("cohort.json"), &record)?;
    Ok(record)
}
