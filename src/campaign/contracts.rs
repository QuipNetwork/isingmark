// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared family, encoding, witness, and timing types.

use std::{collections::BTreeMap, error::Error, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{families::numeric::IntegerScale, instance::Instance};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FamilyConfig {
    pub family: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationContext {
    pub tolerances: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvalError {
    pub code: &'static str,
    pub reason: String,
}

impl fmt::Display for EvalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.reason)
    }
}

impl Error for EvalError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IsingEncoding {
    pub variables: Vec<String>,
    pub h: Vec<i32>,
    pub j: Vec<i32>,
    pub edges: Vec<(u32, u32)>,
    pub units: String,
    pub offset: i64,
    pub auxiliaries: Vec<u32>,
    pub precision_bound: f64,
    pub exact: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Witness {
    pub instance_id: String,
    pub spins: Vec<i8>,
    pub design: Option<Value>,
    pub route: Option<Vec<u32>>,
    pub controls: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evaluation {
    pub valid: bool,
    pub score: Option<f64>,
    pub units: String,
    pub canonical_id: Option<String>,
    pub observables: BTreeMap<String, f64>,
    pub uncertainty: f64,
    pub rejections: Vec<String>,
    /// The exact energy in milli. Acceptance compares this value, never `score`.
    pub energy_milli: Option<i64>,
}

pub trait FamilyEvaluator: Sync {
    /// Produce an explicit instance from the family's configuration and the full nonce.
    ///
    /// # Errors
    /// Returns a stable family error for invalid or unsupported configuration.
    fn generate(&self, config: &FamilyConfig, nonce: [u8; 32]) -> Result<Instance, EvalError>;

    /// Map the instance to an exact encoding, surrogate, or unsupported outcome.
    ///
    /// # Errors
    /// Returns a stable family error for malformed instance data.
    fn encode(&self, instance: &Instance) -> Result<EncodingOutcome, EvalError>;

    /// Compute the true objective and independently check acceptance.
    ///
    /// # Errors
    /// Returns a stable family error for malformed instances or witnesses.
    fn evaluate(
        &self,
        instance: &Instance,
        witness: &Witness,
        context: &EvaluationContext,
    ) -> Result<Evaluation, EvalError>;

    /// Return the family's deterministic witness identity bytes.
    ///
    /// # Errors
    /// Returns a stable family error for malformed instances or witnesses.
    fn canonicalize(&self, instance: &Instance, witness: &Witness) -> Result<Vec<u8>, EvalError>;

    /// The coefficient scale of this family's encodings.
    fn scale(&self) -> IntegerScale {
        IntegerScale::MILLI
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncodingOutcome {
    Exact(IsingEncoding),
    Surrogate(IsingEncoding),
    Unsupported { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeBasis {
    WallClock,
    QpuDeviceAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimingStatus {
    Complete,
    ConservativeEstimate,
    Missing,
}

impl TimingStatus {
    pub(crate) fn worst(self, other: Self) -> Self {
        if self == Self::Missing || other == Self::Missing {
            Self::Missing
        } else if self == Self::ConservativeEstimate || other == Self::ConservativeEstimate {
            Self::ConservativeEstimate
        } else {
            Self::Complete
        }
    }
}
