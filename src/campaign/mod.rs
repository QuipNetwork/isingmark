// SPDX-License-Identifier: AGPL-3.0-or-later
//! Explicit campaign inputs, evaluators, and analysis contracts.

use std::{error::Error, fmt, io};

use contracts::EvalError;

pub mod analysis;
pub mod contracts;
pub mod families;
pub mod instance;
pub mod trace;

/// A campaign input, arithmetic, or evaluation failure.
#[derive(Debug)]
pub enum CampaignError {
    Io(io::Error),
    Parse(serde_json::Error),
    UnsupportedVersion(u32),
    LengthMismatch,
    TooLarge { what: &'static str, limit: u64 },
    MissingField(&'static str),
    Overflow,
    InvalidRun(String),
    Eval(EvalError),
}

impl fmt::Display for CampaignError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "campaign I/O: {error}"),
            Self::Parse(error) => write!(formatter, "campaign JSON: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported instance schema version {version}")
            }
            Self::LengthMismatch => write!(formatter, "inconsistent encoding or witness lengths"),
            Self::TooLarge { what, limit } => {
                write!(formatter, "{what} exceeds its limit of {limit}")
            }
            Self::MissingField(field) => write!(formatter, "missing required field {field}"),
            Self::Overflow => write!(formatter, "coefficient or energy exceeds its integer range"),
            Self::InvalidRun(reason) => write!(formatter, "invalid campaign input: {reason}"),
            Self::Eval(error) => error.fmt(formatter),
        }
    }
}

impl Error for CampaignError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::Eval(error) => Some(error),
            Self::UnsupportedVersion(_)
            | Self::LengthMismatch
            | Self::TooLarge { .. }
            | Self::MissingField(_)
            | Self::Overflow
            | Self::InvalidRun(_) => None,
        }
    }
}

impl From<io::Error> for CampaignError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for CampaignError {
    fn from(error: serde_json::Error) -> Self {
        Self::Parse(error)
    }
}

impl From<EvalError> for CampaignError {
    fn from(error: EvalError) -> Self {
        Self::Eval(error)
    }
}

impl From<CampaignError> for EvalError {
    fn from(error: CampaignError) -> Self {
        let code = match &error {
            CampaignError::Io(_) => "io",
            CampaignError::Parse(_) => "parse",
            CampaignError::UnsupportedVersion(_) => "unsupported_version",
            CampaignError::LengthMismatch => "length_mismatch",
            CampaignError::TooLarge { .. } => "too_large",
            CampaignError::MissingField(_) => "missing_field",
            CampaignError::Overflow => "overflow",
            CampaignError::InvalidRun(_) => "invalid_run",
            CampaignError::Eval(error) => return error.clone(),
        };
        Self {
            code,
            reason: error.to_string(),
        }
    }
}
