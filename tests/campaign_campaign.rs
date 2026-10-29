// SPDX-License-Identifier: AGPL-3.0-or-later
//! Full batch accounting and independent trial clocks.

use std::time::Duration;

use isingmark::campaign::{
    contracts::TimingStatus,
    timing::{wall_outcome, BudgetOutcome, DeviceLedger, TrialBudget, WallOutcome},
    trace::DeviceTime,
    CampaignError,
};

#[test]
fn invalid_run_display_preserves_the_runtime_reason() {
    assert_eq!(
        CampaignError::InvalidRun("miner shutdown failed".into()).to_string(),
        "invalid run: miner shutdown failed"
    );
}

#[test]
fn full_batch_cost_prevents_fractional_success() {
    let mut ledger = DeviceLedger::new();
    // Read 0 supplies the witness, but the complete batch costs 31 seconds.
    ledger
        .charge("batch-1", DeviceTime::from_exact_ns(31_000_000_000))
        .expect("charge the full batch");

    assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Exceeded);
    assert_eq!(ledger.total().lower_ns, 31_000_000_000);
}

#[test]
fn slow_wall_arrival_keeps_device_result_separate() {
    let mut ledger = DeviceLedger::new();
    ledger
        .charge("batch-1", DeviceTime::from_exact_ns(29_000_000_000))
        .expect("charge device time independently of arrival");

    assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Within);
    assert_eq!(
        wall_outcome(Some(Duration::from_secs(80)), Duration::from_secs(30)),
        WallOutcome::Timeout
    );
}

#[test]
fn cancelled_missing_timing_is_unresolved() {
    let mut ledger = DeviceLedger::new();
    ledger
        .charge("batch-1", DeviceTime::from_exact_ns(10_000_000_000))
        .expect("charge completed submission");
    ledger
        .launched_uncharged("cancelled-2")
        .expect("retain cancelled submission without timing");

    assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Unresolved);
    assert_eq!(ledger.total(), DeviceTime::from_exact_ns(10_000_000_000));
}

#[test]
fn rounding_bound_at_budget_is_unresolved() {
    let mut ledger = DeviceLedger::new();
    for submission in 0..30 {
        ledger
            .charge(
                &format!("batch-{submission}"),
                DeviceTime::from_truncated_us(1_000_000),
            )
            .expect("charge the complete truncated measurement");
    }

    assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Unresolved);
    assert_eq!(ledger.total().lower_ns, 30_000_000_000);
    assert_eq!(ledger.total().upper_ns, Some(30_000_029_970));
}

#[test]
#[expect(
    clippy::duration_suboptimal_units,
    reason = "Keep the specification's 1,200-second observation window explicit."
)]
fn unfinished_classical_trial_is_timeout() {
    assert_eq!(
        wall_outcome(None, Duration::from_secs(1_200)),
        WallOutcome::Timeout
    );
}

#[test]
fn trial_budget_requires_at_least_one_clock() {
    assert!(matches!(
        TrialBudget::new(None, None),
        Err(CampaignError::MissingField("budget"))
    ));
}

#[test]
#[expect(
    clippy::duration_suboptimal_units,
    reason = "Keep the specification's 1,200-second observation window explicit."
)]
fn trial_budget_preserves_each_clock_including_zero_limits() {
    for (wall_limit, device_budget_ns) in [
        (Some(Duration::from_secs(1_200)), None),
        (None, Some(30_000_000_000)),
        (Some(Duration::from_secs(1_200)), Some(30_000_000_000)),
        (Some(Duration::ZERO), None),
        (None, Some(0)),
    ] {
        let budget = TrialBudget::new(wall_limit, device_budget_ns).expect("valid trial budget");
        assert_eq!(budget.wall_limit, wall_limit);
        assert_eq!(budget.device_budget_ns, device_budget_ns);
    }
}

#[test]
fn empty_and_exact_boundary_totals_are_within_budget() {
    for mut ledger in [DeviceLedger::new(), DeviceLedger::default()] {
        assert_eq!(ledger.total(), DeviceTime::from_exact_ns(0));
        assert_eq!(ledger.outcome(0), BudgetOutcome::Within);
        ledger
            .charge("batch-1", DeviceTime::from_exact_ns(30_000_000_000))
            .expect("charge exact boundary cost");
        assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Within);
        assert_eq!(ledger.outcome(29_999_999_999), BudgetOutcome::Exceeded);
    }
}

#[test]
fn budget_outcome_uses_bounds_and_timing_status() {
    for (status, upper_ns, expected) in [
        (TimingStatus::Complete, Some(20), BudgetOutcome::Within),
        (TimingStatus::Complete, Some(21), BudgetOutcome::Unresolved),
        (TimingStatus::Complete, None, BudgetOutcome::Unresolved),
        (
            TimingStatus::ConservativeEstimate,
            Some(20),
            BudgetOutcome::Within,
        ),
        (
            TimingStatus::ConservativeEstimate,
            None,
            BudgetOutcome::Unresolved,
        ),
        (TimingStatus::Missing, Some(20), BudgetOutcome::Unresolved),
        (TimingStatus::Missing, None, BudgetOutcome::Unresolved),
    ] {
        let mut ledger = DeviceLedger::new();
        ledger
            .charge(
                "batch-1",
                DeviceTime {
                    lower_ns: 10,
                    upper_ns,
                    status,
                },
            )
            .expect("charge timing bounds");

        assert_eq!(ledger.outcome(20), expected, "status {status:?}");
        assert_eq!(ledger.outcome(9), BudgetOutcome::Exceeded);
    }
}

#[test]
fn known_excess_overrides_an_uncharged_launch() {
    let mut ledger = DeviceLedger::new();
    ledger
        .launched_uncharged("pending-1")
        .expect("record pending submission");
    ledger
        .charge("batch-2", DeviceTime::from_exact_ns(31_000_000_000))
        .expect("charge known excess");

    assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Exceeded);
}

#[test]
fn late_charges_resolve_only_their_matching_launches() {
    let mut ledger = DeviceLedger::new();
    for submission in ["cancelled-1", "speculative-2"] {
        ledger
            .launched_uncharged(submission)
            .expect("record launched submission");
    }
    ledger
        .charge("cancelled-1", DeviceTime::from_exact_ns(10_000_000_000))
        .expect("resolve late timing after cancellation");
    assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Unresolved);
    ledger
        .charge("speculative-2", DeviceTime::from_exact_ns(19_000_000_000))
        .expect("resolve remaining timing");

    assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Within);
    assert_eq!(ledger.total(), DeviceTime::from_exact_ns(29_000_000_000));
}

#[test]
fn duplicate_charges_are_rejected_without_changing_the_total() {
    let mut ledger = DeviceLedger::new();
    ledger
        .charge("batch-1", DeviceTime::from_exact_ns(10))
        .expect("charge submission once");

    assert!(matches!(
        ledger.charge("batch-1", DeviceTime::from_exact_ns(u64::MAX)),
        Err(CampaignError::InvalidRun(_))
    ));
    assert_eq!(ledger.total(), DeviceTime::from_exact_ns(10));
    assert_eq!(ledger.outcome(10), BudgetOutcome::Within);
}

#[test]
fn charged_ids_cannot_be_registered_as_uncharged() {
    let mut ledger = DeviceLedger::new();
    ledger
        .charge("batch-1", DeviceTime::from_exact_ns(10))
        .expect("charge submission");

    assert!(matches!(
        ledger.launched_uncharged("batch-1"),
        Err(CampaignError::InvalidRun(_))
    ));
    assert_eq!(ledger.total(), DeviceTime::from_exact_ns(10));
    assert_eq!(ledger.outcome(10), BudgetOutcome::Within);
}

#[test]
fn repeated_pending_registration_is_idempotent() {
    let mut ledger = DeviceLedger::new();
    for _ in 0..2 {
        ledger
            .launched_uncharged("pending-1")
            .expect("register the same pending submission");
    }
    assert_eq!(ledger.total(), DeviceTime::from_exact_ns(0));
    assert_eq!(ledger.outcome(30_000_000_000), BudgetOutcome::Unresolved);
    ledger
        .charge("pending-1", DeviceTime::from_exact_ns(10))
        .expect("one charge resolves repeated registration");

    assert_eq!(ledger.outcome(10), BudgetOutcome::Within);
}

#[test]
fn charged_missing_timing_never_becomes_zero_cost() {
    let mut ledger = DeviceLedger::new();
    ledger
        .charge("missing-1", DeviceTime::missing())
        .expect("retain missing timing");
    ledger
        .charge("known-2", DeviceTime::from_exact_ns(10))
        .expect("charge known timing");

    assert_eq!(ledger.total().lower_ns, 10);
    assert_eq!(ledger.total().upper_ns, None);
    assert_eq!(ledger.total().status, TimingStatus::Missing);
    assert_eq!(ledger.outcome(u64::MAX), BudgetOutcome::Unresolved);
    assert_eq!(ledger.outcome(9), BudgetOutcome::Exceeded);
}

#[test]
fn lower_bound_overflow_stays_exceeded_and_the_ledger_remains_usable() {
    let mut ledger = DeviceLedger::new();
    ledger
        .charge("batch-1", DeviceTime::from_exact_ns(u64::MAX - 1))
        .expect("charge representable cost");
    assert!(matches!(
        ledger.charge("overflow-2", DeviceTime::from_exact_ns(2)),
        Err(CampaignError::Overflow)
    ));
    assert_eq!(ledger.total(), DeviceTime::from_exact_ns(u64::MAX - 1));
    assert_eq!(ledger.outcome(u64::MAX), BudgetOutcome::Exceeded);
    assert!(matches!(
        ledger.charge("overflow-2", DeviceTime::from_exact_ns(2)),
        Err(CampaignError::InvalidRun(_))
    ));
    ledger
        .charge("batch-3", DeviceTime::from_exact_ns(1))
        .expect("continue charging after overflow");
    ledger
        .launched_uncharged("pending-4")
        .expect("continue recording launches after overflow");
    assert_eq!(ledger.total(), DeviceTime::from_exact_ns(u64::MAX));
    for budget in [0, 30_000_000_000, u64::MAX] {
        assert_eq!(ledger.outcome(budget), BudgetOutcome::Exceeded);
    }
}

#[test]
fn upper_bound_overflow_keeps_the_fitting_lower_bound_unresolved() {
    let mut ledger = DeviceLedger::new();
    let initial = DeviceTime {
        lower_ns: 0,
        upper_ns: Some(u64::MAX),
        status: TimingStatus::ConservativeEstimate,
    };
    ledger.charge("batch-1", initial).expect("charge bounds");
    assert_eq!(ledger.outcome(u64::MAX), BudgetOutcome::Within);
    ledger
        .launched_uncharged("overflow-2")
        .expect("record launched submission");

    assert!(matches!(
        ledger.charge("overflow-2", DeviceTime::from_exact_ns(1)),
        Err(CampaignError::Overflow)
    ));
    assert_eq!(
        ledger.total(),
        DeviceTime {
            lower_ns: 1,
            upper_ns: None,
            status: TimingStatus::ConservativeEstimate,
        }
    );
    assert_eq!(ledger.outcome(0), BudgetOutcome::Exceeded);
    for budget in [1, 30_000_000_000, u64::MAX] {
        assert_eq!(ledger.outcome(budget), BudgetOutcome::Unresolved);
    }
    assert!(matches!(
        ledger.charge("overflow-2", DeviceTime::from_exact_ns(1)),
        Err(CampaignError::InvalidRun(_))
    ));
    assert!(matches!(
        ledger.launched_uncharged("overflow-2"),
        Err(CampaignError::InvalidRun(_))
    ));

    ledger
        .charge("batch-3", DeviceTime::from_exact_ns(1))
        .expect("continue charging after upper-only overflow");
    assert_eq!(ledger.total().lower_ns, 2);
    assert_eq!(ledger.total().upper_ns, None);
    assert_eq!(ledger.outcome(1), BudgetOutcome::Exceeded);
    assert_eq!(ledger.outcome(2), BudgetOutcome::Unresolved);
}

#[test]
fn upper_bound_overflow_keeps_the_worst_timing_status() {
    use TimingStatus::{Complete, ConservativeEstimate, Missing};

    for (initial_status, cost_status, expected) in [
        (Complete, Complete, Complete),
        (ConservativeEstimate, Complete, ConservativeEstimate),
        (Complete, ConservativeEstimate, ConservativeEstimate),
        (Missing, ConservativeEstimate, Missing),
        (ConservativeEstimate, Missing, Missing),
    ] {
        let mut ledger = DeviceLedger::new();
        ledger
            .charge(
                "batch-1",
                DeviceTime {
                    lower_ns: 0,
                    upper_ns: Some(u64::MAX),
                    status: initial_status,
                },
            )
            .expect("charge initial timing status");

        assert!(matches!(
            ledger.charge(
                "overflow-2",
                DeviceTime {
                    lower_ns: 1,
                    upper_ns: Some(1),
                    status: cost_status,
                }
            ),
            Err(CampaignError::Overflow)
        ));
        assert_eq!(ledger.total().status, expected);
        assert_eq!(ledger.outcome(u64::MAX), BudgetOutcome::Unresolved);
    }
}

#[test]
#[expect(
    clippy::duration_suboptimal_units,
    reason = "Keep the specification's 1,200-second observation window explicit."
)]
fn wall_acceptance_includes_the_limit_and_rejects_one_nanosecond_late() {
    let limit = Duration::from_secs(1_200);
    for accepted_at in [Duration::ZERO, Duration::from_secs(80), limit] {
        assert_eq!(wall_outcome(Some(accepted_at), limit), WallOutcome::Success);
    }
    assert_eq!(
        wall_outcome(Some(limit + Duration::from_nanos(1)), limit),
        WallOutcome::Timeout
    );
    assert_eq!(wall_outcome(None, Duration::ZERO), WallOutcome::Timeout);
    assert_eq!(
        wall_outcome(Some(Duration::ZERO), Duration::ZERO),
        WallOutcome::Success
    );
}
