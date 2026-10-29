// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bounded instance loading and source provenance validation.

use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    contracts::{IsingEncoding, Witness},
    CampaignError,
};

pub const MAX_INSTANCE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_SPINS: u64 = 65_536;
pub const MAX_WITNESS_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Split {
    Training,
    Validation,
    Test,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub schema_version: u32,
    pub instance_id: String,
    pub family: String,
    pub model_version: String,
    pub challenge_id: String,
    pub nonce: String,
    pub split: Split,
    pub physical: Value,
    pub search_space: Value,
    pub objective: Value,
    pub constraints: Value,
    pub hardness: Value,
    pub source_hashes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceManifest {
    pub sources: Vec<SourceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEntry {
    pub id: String,
    pub url: String,
    pub revision_utc: String,
    pub export_sha256: String,
    pub retrieved_at: String,
    pub reproduced: bool,
    pub input_hashes: Vec<String>,
}

/// Load a bounded explicit instance without deriving coefficients from a nonce.
///
/// # Errors
/// Rejects unreadable, oversized, unsupported, incomplete, or invalid instances.
pub fn load_instance(path: &Path) -> Result<Instance, CampaignError> {
    let bytes = read_bounded(path, "instance")?;
    instance_from_value(serde_json::from_slice(&bytes)?)
}

pub(crate) fn instance_from_value(value: Value) -> Result<Instance, CampaignError> {
    let version = value
        .get("schema_version")
        .ok_or(CampaignError::MissingField("schema_version"))?;
    let version = serde_json::from_value(version.clone())?;
    if version != 1 {
        return Err(CampaignError::UnsupportedVersion(version));
    }
    for field in [
        "instance_id",
        "family",
        "model_version",
        "challenge_id",
        "nonce",
        "split",
        "physical",
        "search_space",
        "objective",
        "constraints",
        "hardness",
        "source_hashes",
    ] {
        if value.get(field).is_none() {
            return Err(CampaignError::MissingField(field));
        }
    }
    let instance: Instance = serde_json::from_value(value)?;
    if instance.family == "ising-fixed" {
        fixed_encoding(&instance)?;
    }
    Ok(instance)
}

/// Validate source claims while permitting explicitly unreproduced sources.
///
/// # Errors
/// Rejects unreadable or malformed manifests and reproduced sources without hashes.
pub fn validate_source_manifest(path: &Path) -> Result<SourceManifest, CampaignError> {
    let manifest: SourceManifest = serde_json::from_slice(&read_bounded(path, "source manifest")?)?;
    for source in &manifest.sources {
        if source.reproduced
            && (source.input_hashes.is_empty()
                || source
                    .input_hashes
                    .iter()
                    .any(|hash| hash.trim().is_empty()))
        {
            return Err(CampaignError::InvalidRun(format!(
                "reproduced source {} requires input hashes",
                source.id
            )));
        }
    }
    Ok(manifest)
}

fn read_bounded(path: &Path, what: &'static str) -> Result<Vec<u8>, CampaignError> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_INSTANCE_BYTES {
        return Err(CampaignError::TooLarge {
            what,
            limit: MAX_INSTANCE_BYTES,
        });
    }
    let mut bytes = Vec::new();
    file.take(MAX_INSTANCE_BYTES + 1).read_to_end(&mut bytes)?;
    // Bound the read too, since a file can grow after the metadata check.
    if u64::try_from(bytes.len()).map_err(|_| CampaignError::Overflow)? > MAX_INSTANCE_BYTES {
        return Err(CampaignError::TooLarge {
            what,
            limit: MAX_INSTANCE_BYTES,
        });
    }
    Ok(bytes)
}

impl Witness {
    /// Enforce the spin count and the size of the complete JSON witness.
    ///
    /// # Errors
    /// Returns `TooLarge` when the witness or spin count exceeds its limit.
    /// Returns `Parse` if JSON serialization fails.
    pub fn validate(&self) -> Result<(), CampaignError> {
        let spin_count = u64::try_from(self.spins.len()).map_err(|_| CampaignError::Overflow)?;
        if spin_count > MAX_WITNESS_BYTES {
            return Err(CampaignError::TooLarge {
                what: "witness",
                limit: MAX_WITNESS_BYTES,
            });
        }
        let size = serde_json::to_vec(self)?.len();
        if u64::try_from(size).map_err(|_| CampaignError::Overflow)? > MAX_WITNESS_BYTES {
            return Err(CampaignError::TooLarge {
                what: "witness",
                limit: MAX_WITNESS_BYTES,
            });
        }
        if spin_count > MAX_SPINS {
            return Err(CampaignError::TooLarge {
                what: "spins",
                limit: MAX_SPINS,
            });
        }
        Ok(())
    }
}

// Coefficients must reach validation as i64, before narrowing to the wire type.
#[derive(Deserialize)]
struct IntegerEncoding {
    variables: Vec<String>,
    h: Vec<i64>,
    j: Vec<i64>,
    edges: Vec<(u32, u32)>,
    units: String,
    offset: i64,
    auxiliaries: Vec<u32>,
    precision_bound: f64,
    exact: bool,
}

pub(crate) fn fixed_encoding(instance: &Instance) -> Result<IsingEncoding, CampaignError> {
    if instance.schema_version != 1 {
        return Err(CampaignError::UnsupportedVersion(instance.schema_version));
    }
    if instance.family != "ising-fixed" {
        return Err(CampaignError::InvalidRun(format!(
            "expected ising-fixed, got {}",
            instance.family
        )));
    }
    let encoding: IntegerEncoding = serde_json::from_value(instance.physical.clone())?;
    let spins = u64::try_from(encoding.variables.len()).map_err(|_| CampaignError::Overflow)?;
    if spins > MAX_SPINS {
        return Err(CampaignError::TooLarge {
            what: "spins",
            limit: MAX_SPINS,
        });
    }
    if encoding.h.len() != encoding.variables.len()
        || encoding.j.len() != encoding.edges.len()
        || encoding
            .edges
            .iter()
            .any(|&(left, right)| u64::from(left) >= spins || u64::from(right) >= spins)
        || encoding
            .auxiliaries
            .iter()
            .any(|&index| u64::from(index) >= spins)
    {
        return Err(CampaignError::LengthMismatch);
    }
    let h = encoding
        .h
        .into_iter()
        .map(i32::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| CampaignError::Overflow)?;
    let j = encoding
        .j
        .into_iter()
        .map(i32::try_from)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| CampaignError::Overflow)?;
    let mut bound = 0_i64;
    for &coefficient in h.iter().chain(&j) {
        bound = bound
            .checked_add(i64::from(coefficient).abs())
            .ok_or(CampaignError::Overflow)?;
    }
    // Include the offset when bounding every possible signed coefficient sum.
    encoding
        .offset
        .checked_add(bound)
        .ok_or(CampaignError::Overflow)?;
    encoding
        .offset
        .checked_sub(bound)
        .ok_or(CampaignError::Overflow)?;
    if encoding.units != "milli" || !encoding.exact {
        return Err(CampaignError::InvalidRun(
            "ising-fixed requires an exact milli encoding".to_owned(),
        ));
    }
    Ok(IsingEncoding {
        variables: encoding.variables,
        h,
        j,
        edges: encoding.edges,
        units: encoding.units,
        offset: encoding.offset,
        auxiliaries: encoding.auxiliaries,
        precision_bound: encoding.precision_bound,
        exact: encoding.exact,
    })
}
