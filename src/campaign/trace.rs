// SPDX-License-Identifier: AGPL-3.0-or-later
//! Versioned campaign events with separate wall and device clocks.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{contracts::TimingStatus, CampaignError};

pub const TRACE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTime {
    pub lower_ns: u64,
    pub upper_ns: Option<u64>,
    pub status: TimingStatus,
}

impl DeviceTime {
    /// Retain the uncertainty from a measurement truncated to microseconds.
    /// Unrepresentable nanosecond bounds produce missing timing.
    #[must_use]
    pub fn from_truncated_us(us: u64) -> Self {
        let Some(lower_ns) = us.checked_mul(1_000) else {
            return Self::missing();
        };
        let Some(upper_ns) = lower_ns.checked_add(999) else {
            return Self::missing();
        };
        Self {
            lower_ns,
            upper_ns: Some(upper_ns),
            status: TimingStatus::Complete,
        }
    }

    #[must_use]
    pub fn from_exact_ns(ns: u64) -> Self {
        Self {
            lower_ns: ns,
            upper_ns: Some(ns),
            status: TimingStatus::Complete,
        }
    }

    #[must_use]
    pub fn missing() -> Self {
        Self {
            lower_ns: 0,
            upper_ns: None,
            status: TimingStatus::Missing,
        }
    }

    /// Sum full device costs and retain the least complete timing status.
    ///
    /// # Errors
    /// Returns `CampaignError::Overflow` if either nanosecond sum exceeds `u64`.
    #[expect(
        clippy::should_implement_trait,
        reason = "The campaign API requires fallible inherent addition to report overflow."
    )]
    pub fn add(self, other: Self) -> Result<Self, CampaignError> {
        let lower_ns = self
            .lower_ns
            .checked_add(other.lower_ns)
            .ok_or(CampaignError::Overflow)?;
        let upper_ns = self
            .upper_ns
            .zip(other.upper_ns)
            .map(|(left, right)| left.checked_add(right).ok_or(CampaignError::Overflow))
            .transpose()?;
        let status =
            if self.status == TimingStatus::Missing || other.status == TimingStatus::Missing {
                TimingStatus::Missing
            } else if self.status == TimingStatus::ConservativeEstimate
                || other.status == TimingStatus::ConservativeEstimate
            {
                TimingStatus::ConservativeEstimate
            } else {
                TimingStatus::Complete
            };
        Ok(Self {
            lower_ns,
            upper_ns,
            status,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    ChallengeRelease,
    JobDispatch,
    BatchReceipt,
    WitnessValidation,
    ProofAcceptance,
    Rejection,
    TargetChange,
    Cancellation,
    TrialTermination,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub version: u32,
    pub run_id: String,
    pub trial_id: String,
    pub session_id: String,
    pub seq: u64,
    pub observer: String,
    pub kind: EventKind,
    pub wall_ns: u64,
    pub instance_id: Option<String>,
    pub job_id: Option<String>,
    pub batch_id: Option<String>,
    pub payload: Value,
    pub device: Option<DeviceTime>,
    pub anneal_ns: Option<u64>,
    pub estimate_provenance: Option<String>,
    pub resources: BTreeMap<String, u64>,
}

#[derive(Debug)]
pub struct TraceWriter {
    file: BufWriter<File>,
    last_seq: Option<u64>,
}

impl TraceWriter {
    /// Create a new trace, truncating an existing file at this path.
    ///
    /// # Errors
    /// Returns an I/O error if the trace cannot be created.
    pub fn create(path: &Path) -> Result<Self, CampaignError> {
        Ok(Self {
            file: BufWriter::new(File::create(path)?),
            last_seq: None,
        })
    }

    /// Write and flush one event with a strictly increasing sequence number.
    ///
    /// # Errors
    /// Returns an error for a non-increasing sequence, serialization, or I/O failure.
    pub fn append(&mut self, event: &TraceEvent) -> Result<(), CampaignError> {
        if self.last_seq.is_some_and(|last| event.seq <= last) {
            return Err(CampaignError::InvalidRun(format!(
                "trace sequence {} does not increase",
                event.seq
            )));
        }
        serde_json::to_writer(&mut self.file, event)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        self.last_seq = Some(event.seq);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraceRead {
    pub events: Vec<TraceEvent>,
    pub truncated_tail: bool,
}

/// Read complete events, retaining an explicit indication of a partial final line.
///
/// # Errors
/// Returns an error for I/O failures, unsupported versions, or malformed complete lines.
pub fn read_trace(path: &Path) -> Result<TraceRead, CampaignError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut trace = TraceRead {
        events: Vec::new(),
        truncated_tail: false,
    };
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        let value = match serde_json::from_slice::<Value>(&line) {
            Ok(value) => value,
            Err(_) if !line.ends_with(b"\n") => {
                trace.truncated_tail = true;
                break;
            }
            Err(error) => {
                return Err(CampaignError::InvalidRun(format!(
                    "malformed trace line {}: {error}",
                    trace.events.len() + 1
                )));
            }
        };
        if let Some(version) = value["version"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
        {
            if version != TRACE_VERSION {
                return Err(CampaignError::UnsupportedVersion(version));
            }
        }
        let event: TraceEvent = serde_json::from_value(value).map_err(|error| {
            CampaignError::InvalidRun(format!(
                "malformed trace line {}: {error}",
                trace.events.len() + 1
            ))
        })?;
        if trace
            .events
            .last()
            .is_some_and(|last| event.seq <= last.seq)
        {
            return Err(CampaignError::InvalidRun(format!(
                "trace sequence {} does not increase",
                event.seq
            )));
        }
        trace.events.push(event);
    }
    Ok(trace)
}
