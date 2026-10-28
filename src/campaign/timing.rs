// SPDX-License-Identifier: AGPL-3.0-or-later
//! Full submission costs and independent device and wall-clock limits.

use std::{collections::HashSet, time::Duration};

use super::{contracts::TimingStatus, trace::DeviceTime, CampaignError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrialBudget {
    pub wall_limit: Option<Duration>,
    pub device_budget_ns: Option<u64>,
}

impl TrialBudget {
    /// Set independent limits for either clock or both clocks.
    ///
    /// # Errors
    /// Returns `CampaignError::MissingField("budget")` when neither limit is set.
    pub fn new(
        wall_limit: Option<Duration>,
        device_budget_ns: Option<u64>,
    ) -> Result<Self, CampaignError> {
        if wall_limit.is_none() && device_budget_ns.is_none() {
            return Err(CampaignError::MissingField("budget"));
        }
        Ok(Self {
            wall_limit,
            device_budget_ns,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetOutcome {
    Within,
    Exceeded,
    Unresolved,
}

#[derive(Debug)]
pub struct DeviceLedger {
    total: DeviceTime,
    charged: HashSet<String>,
    uncharged: HashSet<String>,
    overflowed: bool,
}

impl DeviceLedger {
    #[must_use]
    pub fn new() -> Self {
        Self {
            total: DeviceTime::from_exact_ns(0),
            charged: HashSet::new(),
            uncharged: HashSet::new(),
            overflowed: false,
        }
    }

    /// Charge a complete submission and resolve its pending launch, if recorded.
    /// An overflowing charge still records the ID and permanently exceeds all budgets.
    ///
    /// # Errors
    /// Returns `CampaignError::InvalidRun` for an already charged ID, without mutation.
    /// Returns `CampaignError::Overflow` when the sum cannot fit in `DeviceTime`.
    pub fn charge(&mut self, submission_id: &str, cost: DeviceTime) -> Result<(), CampaignError> {
        if self.charged.contains(submission_id) {
            return Err(CampaignError::InvalidRun(format!(
                "submission {submission_id:?} is already charged"
            )));
        }
        self.charged.insert(submission_id.to_owned());
        self.uncharged.remove(submission_id);
        match self.total.add(cost) {
            Ok(total) => {
                self.total = total;
                Ok(())
            }
            Err(error) => {
                self.overflowed = true;
                Err(error)
            }
        }
    }

    /// Record a launched submission whose cost is not yet known.
    /// Repeated registration of the same pending ID has no additional effect.
    ///
    /// # Errors
    /// Returns `CampaignError::InvalidRun` if the submission is already charged.
    pub fn launched_uncharged(&mut self, submission_id: &str) -> Result<(), CampaignError> {
        if self.charged.contains(submission_id) {
            return Err(CampaignError::InvalidRun(format!(
                "submission {submission_id:?} is already charged"
            )));
        }
        self.uncharged.insert(submission_id.to_owned());
        Ok(())
    }

    /// Return the sum of charged costs, excluding pending launches.
    /// After overflow, this retains only additions that fit. Use `outcome` for budgets.
    #[must_use]
    pub fn total(&self) -> DeviceTime {
        self.total
    }

    /// Resolve a device budget using full costs, timing bounds, and pending launches.
    #[must_use]
    pub fn outcome(&self, budget_ns: u64) -> BudgetOutcome {
        if self.overflowed || self.total.lower_ns > budget_ns {
            BudgetOutcome::Exceeded
        } else if self.uncharged.is_empty()
            && self.total.status != TimingStatus::Missing
            && self.total.upper_ns.is_some_and(|upper| upper <= budget_ns)
        {
            BudgetOutcome::Within
        } else {
            BudgetOutcome::Unresolved
        }
    }
}

impl Default for DeviceLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallOutcome {
    Success,
    Timeout,
}

/// Accept only a recorded proof acceptance at or before the wall-clock limit.
#[must_use]
pub fn wall_outcome(accepted_at: Option<Duration>, limit: Duration) -> WallOutcome {
    if accepted_at.is_some_and(|accepted| accepted <= limit) {
        WallOutcome::Success
    } else {
        WallOutcome::Timeout
    }
}
