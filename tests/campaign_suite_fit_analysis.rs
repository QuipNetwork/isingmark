// SPDX-License-Identifier: AGPL-3.0-or-later
//! Suite-fit A/B comparisons and the historical Tier rule version 3.

use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, SystemTime},
};

use isingmark::campaign::analysis::suite_fit::tier::{
    qpu_wall, tier, ClassicalDevice, ClassicalRun, QpuReference, Tier, WallBasis, TIER_RULE_VERSION,
};
use isingmark::campaign::{
    analysis::confidence::binomial_upper,
    analysis::suite_fit::{
        arm_curves, paired_difference, ArmSpec, Difference, DifferenceOutcome, Groups,
        NotComparable, PairSpec, Side, SizeStatus, TargetPanel,
    },
    analysis::survival::{AnalysisDesign, AnalysisError, Censoring},
    contracts::TimeBasis,
    runner::{InstanceIdentity, Preparation, TerminalReason, Trial},
    trace::{DeviceTime, EventKind, TraceEvent, TraceWriter, TRACE_VERSION},
};

/// A reference of `models` models, each at -1000 units.
fn reference(models: usize, post_s: f64) -> QpuReference {
    QpuReference {
        energies_milli: (0..models)
            .map(|index| (format!("m{index:03}"), -1_000_000))
            .collect(),
        post_s,
    }
}

/// A run whose energy on model `i` is the reference energy plus `offsets(i)` milli, or a missing
/// state when `offsets(i)` is `None`.
fn run(
    device: ClassicalDevice,
    name: &str,
    models: usize,
    wall_s: f64,
    offsets: impl Fn(usize) -> Option<i64>,
) -> ClassicalRun {
    ClassicalRun {
        device,
        name: name.into(),
        budget_s: None,
        wall_s: Some(wall_s),
        device_s: Some(wall_s),
        wall_basis: WallBasis::Measured,
        energies_milli: (0..models)
            .map(|index| {
                (
                    format!("m{index:03}"),
                    offsets(index).map(|offset| -1_000_000 + offset),
                )
            })
            .collect(),
    }
}

/// Every model at the same `offset` milli from the reference.
fn every(offset: i64) -> impl Fn(usize) -> Option<i64> {
    move |_| Some(offset)
}

fn tier_of(reference: &QpuReference, runs: &[ClassicalRun]) -> Tier {
    tier(reference, runs).expect("a reference has a tier").tier
}

#[test]
fn twenty_nine_common_models_receive_no_tier_label() {
    let qpu = reference(100, 0.25);
    let short = [
        run(ClassicalDevice::Cpu, "cpu", 29, 1.0, every(-1)),
        run(ClassicalDevice::Cuda, "cuda", 29, 1.0, every(-1)),
    ];
    let label = tier(&qpu, &short).expect("a reference has a tier");
    assert_eq!(label.tier, Tier::NeedsMeasurement);
    assert!(label.reason.contains("no CPU run"), "{}", label.reason);
    assert!(label.reason.contains("no CUDA run"), "{}", label.reason);
    let floor = [
        run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1)),
        run(ClassicalDevice::Cuda, "cuda", 30, 1.0, every(-1)),
    ];
    assert_eq!(tier_of(&qpu, &floor), Tier::Four);
}

#[test]
fn a_tie_reaches_the_reference_but_never_beats_it() {
    let qpu = reference(30, 0.25);
    let fast = [
        run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(0)),
        run(ClassicalDevice::Cuda, "cuda", 30, 1.0, every(0)),
    ];
    // 1 s over the 0.35 s QPU wall is below the speed ratio of 5.
    assert_eq!(tier_of(&qpu, &fast), Tier::Two);
    let slow = [
        run(ClassicalDevice::Cpu, "cpu", 30, 10.0, every(0)),
        run(ClassicalDevice::Cuda, "cuda", 30, 10.0, every(0)),
    ];
    assert_eq!(tier_of(&qpu, &slow), Tier::One);
}

#[test]
fn a_beat_needs_sixty_percent_strictly_below() {
    let qpu = reference(30, 0.25);
    let share = |wins: usize| {
        [
            run(ClassicalDevice::Cpu, "cpu", 30, 10.0, move |index| {
                Some(if index < wins { -1 } else { 1 })
            }),
            run(ClassicalDevice::Cuda, "cuda", 30, 10.0, every(0)),
        ]
    };
    // CUDA ties every model, so it reaches; at 10 s the QPU is ahead on speed.
    assert_eq!(tier_of(&qpu, &share(18)), Tier::Three);
    assert_eq!(tier_of(&qpu, &share(17)), Tier::One);
}

#[test]
fn the_beat_budget_admits_120_seconds_and_no_more() {
    let qpu = reference(30, 0.25);
    let budgeted = |budget_s: f64| {
        let mut cpu = run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1));
        cpu.budget_s = Some(budget_s);
        [cpu, run(ClassicalDevice::Cuda, "cuda", 30, 1.0, every(-1))]
    };
    assert_eq!(tier_of(&qpu, &budgeted(120.0)), Tier::Four);
    let label = tier(&qpu, &budgeted(120.5)).expect("a reference has a tier");
    assert_eq!(label.tier, Tier::NeedsMeasurement);
    assert!(label.reason.contains("no CPU run"), "{}", label.reason);
}

#[test]
fn the_qpu_is_ahead_at_exactly_five_times_its_wall() {
    // QPU wall: 0.1 s production plus 0.4 s postprocess.
    let qpu = reference(30, 0.4);
    assert!((qpu_wall(0.4) - 0.5).abs() < f64::EPSILON);
    let runs = |wall_s: f64| {
        [
            run(ClassicalDevice::Cpu, "cpu", 30, wall_s, every(0)),
            run(ClassicalDevice::Cuda, "cuda", 30, wall_s, every(0)),
        ]
    };
    let label = tier(&qpu, &runs(2.5)).expect("a reference has a tier");
    let speed = label.speed.expect("a speed pick");
    assert!((speed.ratio() - 5.0).abs() < f64::EPSILON);
    assert_eq!(label.tier, Tier::One);
    assert_eq!(tier_of(&qpu, &runs(2.4)), Tier::Two);
}

#[test]
fn missing_states_count_against_the_classical_run() {
    let qpu = reference(30, 0.25);
    // 17 wins and 13 missing states: 17 of 17 measured, but 17 of 30 attempted.
    let partial = [
        run(ClassicalDevice::Cpu, "cpu", 30, 10.0, |index| {
            (index < 17).then_some(-1)
        }),
        run(ClassicalDevice::Cuda, "cuda", 30, 10.0, every(1)),
    ];
    let label = tier(&qpu, &partial).expect("a reference has a tier");
    let beat = label.beat_run.expect("a deciding run");
    assert_eq!(
        (
            beat.name.as_str(),
            beat.classical_wins,
            beat.missing,
            beat.common
        ),
        ("cpu", 17, 13, 30)
    );
    assert!(!beat.beats());
    // 17 of 30 at or below still reaches, and 10 s is slow, so the QPU holds both tests.
    assert_eq!(label.tier, Tier::One);
    // 15 at the reference and 15 missing does not reach on a majority of 30.
    let half = [
        run(ClassicalDevice::Cpu, "cpu", 30, 1.0, |index| {
            (index < 15).then_some(0)
        }),
        run(ClassicalDevice::Cuda, "cuda", 30, 1.0, |index| {
            (index < 15).then_some(0)
        }),
    ];
    let speed = tier(&qpu, &half)
        .expect("a reference has a tier")
        .speed
        .expect("a bound");
    assert!(!speed.reached);
}

#[test]
fn missing_or_modeled_timing_cannot_decide_a_tier() {
    let qpu = reference(30, 0.25);
    let mut untimed = run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1));
    untimed.wall_s = None;
    let cuda = run(ClassicalDevice::Cuda, "cuda", 30, 1.0, every(-1));
    let label = tier(&qpu, &[untimed, cuda.clone()]).expect("a reference has a tier");
    assert_eq!(label.tier, Tier::NeedsMeasurement);
    assert!(label.reason.contains("no CPU run"), "{}", label.reason);

    let mut modeled = run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1));
    modeled.wall_basis = WallBasis::Modeled;
    assert_eq!(
        tier_of(&qpu, &[modeled, cuda.clone()]),
        Tier::NeedsMeasurement
    );

    let mut scaled = run(ClassicalDevice::Cpu, "cpu", 30, 0.5, every(-1));
    scaled.wall_basis = WallBasis::Scaled;
    let label = tier(&qpu, &[scaled, cuda]).expect("a reference has a tier");
    assert_eq!(label.tier, Tier::Four);
    let speed = label.speed.expect("a speed pick");
    assert_eq!(speed.device, ClassicalDevice::Cpu);
    assert!(speed.estimated);
}

#[test]
fn a_reached_run_wins_over_any_bound() {
    let qpu = reference(30, 0.25);
    let runs = [
        run(ClassicalDevice::Cpu, "cpu-slow", 30, 50.0, every(0)),
        run(ClassicalDevice::Cuda, "cuda-short", 30, 0.5, every(1)),
        run(ClassicalDevice::Cuda, "cuda-long", 30, 2.0, every(1)),
    ];
    let speed = tier(&qpu, &runs)
        .expect("a reference has a tier")
        .speed
        .expect("a speed pick");
    assert_eq!((speed.name.as_str(), speed.reached), ("cpu-slow", true));
    // Without a reached run, the deepest run gives the lower bound.
    let speed = tier(&qpu, &runs[1..])
        .expect("a reference has a tier")
        .speed
        .expect("a bound");
    assert_eq!((speed.name.as_str(), speed.reached), ("cuda-long", false));
}

#[test]
fn a_label_records_rule_version_three_and_no_capture_has_no_tier() {
    let qpu = reference(30, 0.25);
    let runs = [
        run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(0)),
        run(ClassicalDevice::Cuda, "cuda", 30, 1.0, every(0)),
    ];
    let label = tier(&qpu, &runs).expect("a reference has a tier");
    assert_eq!((TIER_RULE_VERSION, label.rule_version), (3, 3));
    let empty = QpuReference {
        energies_milli: BTreeMap::new(),
        post_s: 0.25,
    };
    assert!(tier(&empty, &runs).is_none());
}

#[derive(Clone, Copy)]
enum Outcome {
    Accepted(u64),
    Timeout,
    IncompleteCapture,
}

/// A trial whose trace holds one termination event, with a 30 s device budget.
fn trial(dir: &Path, id: &str, instance: &str, window_s: u64, outcome: Outcome) -> Trial {
    let events = dir.join(format!("{id}.jsonl"));
    let accepted = match outcome {
        Outcome::Accepted(seconds) => Some(seconds),
        Outcome::Timeout | Outcome::IncompleteCapture => None,
    };
    let event = TraceEvent {
        version: TRACE_VERSION,
        run_id: "run".into(),
        trial_id: id.into(),
        session_id: "session".into(),
        seq: 0,
        observer: "coordinator".into(),
        kind: EventKind::TrialTermination,
        wall_ns: accepted.unwrap_or(window_s) * 1_000_000_000,
        instance_id: Some(instance.into()),
        job_id: None,
        batch_id: None,
        payload: serde_json::Value::Null,
        device: None,
        anneal_ns: None,
        estimate_provenance: None,
        resources: BTreeMap::new(),
    };
    TraceWriter::create(&events)
        .expect("create trace")
        .append(&event)
        .expect("write termination");
    Trial {
        trial_id: id.into(),
        instance_id: instance.into(),
        policy_id: "policy".into(),
        started: SystemTime::UNIX_EPOCH,
        wall_limit: Some(Duration::from_secs(window_s)),
        device_budget_ns: Some(30_000_000_000),
        resources: BTreeMap::new(),
        events,
        accepted_wall: accepted.map(Duration::from_secs),
        device_total: DeviceTime::from_exact_ns(if accepted.is_some() {
            1_000_000_000
        } else {
            30_000_000_000
        }),
        terminal: match outcome {
            Outcome::Accepted(_) => TerminalReason::Accepted,
            Outcome::Timeout => TerminalReason::WallTimeout,
            Outcome::IncompleteCapture => TerminalReason::IncompleteCapture,
        },
        preparation: Preparation::Cold,
        identity: InstanceIdentity {
            family: "ising-fixed".into(),
            revision: "1".into(),
            nonce: "00".repeat(32),
            descriptor_digest: None,
            instance_digest: None,
            offset_milli: 0,
            source_hashes: BTreeMap::new(),
        },
    }
}

/// One trial per instance `prefix0..prefix{count}` with the same outcome.
fn arm(
    dir: &Path,
    prefix: &str,
    instances: &[usize],
    window_s: u64,
    outcome: Outcome,
) -> Vec<Trial> {
    instances
        .iter()
        .map(|index| {
            trial(
                dir,
                &format!("{prefix}-{index}"),
                &format!("i{index}"),
                window_s,
                outcome,
            )
        })
        .collect()
}

fn spec(side: Side, size_spins: u64, panel: TargetPanel) -> ArmSpec {
    ArmSpec {
        side,
        family: "ice-pairs".into(),
        family_revision: "1".into(),
        parameter_digest: "digest".into(),
        policy_id: "policy".into(),
        size_spins,
        target_rule: "qpu80_pp".into(),
        panel,
    }
}

fn pair(pair_id: &str, a_size: u64, b_size: u64) -> PairSpec {
    PairSpec {
        pair_id: pair_id.into(),
        a: spec(Side::A, a_size, TargetPanel::Native),
        b: spec(Side::B, b_size, TargetPanel::Native),
    }
}

fn estimated(outcome: DifferenceOutcome) -> Vec<Difference> {
    match outcome {
        DifferenceOutcome::Estimated(differences) => differences,
        DifferenceOutcome::NotComparable(reason) => {
            assert_eq!(Some(reason), None, "the pair must be comparable");
            Vec::new()
        }
    }
}

fn basis(differences: &[Difference], basis: TimeBasis) -> &Difference {
    differences
        .iter()
        .find(|difference| difference.basis == basis)
        .expect("difference for the requested clock")
}

fn at(difference: &Difference, seconds: f64) -> (f64, f64, f64) {
    let index = difference
        .grid_s
        .iter()
        .position(|time| time.total_cmp(&seconds).is_eq())
        .expect("time is in the grid");
    (
        difference.estimate[index],
        difference.lower[index],
        difference.upper[index],
    )
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
}

#[test]
fn fixed_windows_must_match_across_arms_but_km_may_differ() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let ids: Vec<usize> = (0..4).collect();
    let a = arm(dir.path(), "a", &ids, 1_200, Outcome::Accepted(5));
    let b = arm(dir.path(), "b", &ids, 600, Outcome::Timeout);
    let synthetic = pair("x-w62-p1200", 4_575, 4_575);
    let groups = Groups::new();
    assert_eq!(
        paired_difference(&synthetic, &a, &b, &groups, &AnalysisDesign::default()),
        Err(AnalysisError::MixedBasis)
    );
    let km = AnalysisDesign {
        censoring: Censoring::KaplanMeier {
            justification: "Windows were assigned before the trials began.".into(),
        },
        ..AnalysisDesign::default()
    };
    let differences =
        estimated(paired_difference(&synthetic, &a, &b, &groups, &km).expect("KM difference"));
    let wall = basis(&differences, TimeBasis::WallClock);
    close(at(wall, 10.0).0, 1.0);
    // The grid stops at the longer window, and the censored B arm stays at zero.
    assert_eq!(wall.grid_s.last(), Some(&1_200.0));
}

#[test]
fn a_missing_state_is_a_failure_in_the_difference() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let ids: Vec<usize> = (0..10).collect();
    let a = arm(dir.path(), "a", &ids, 1_200, Outcome::Accepted(5));
    let b = arm(dir.path(), "b", &ids, 1_200, Outcome::IncompleteCapture);
    let differences = estimated(
        paired_difference(
            &pair("x-w62-p1200", 4_575, 4_575),
            &a,
            &b,
            &Groups::new(),
            &AnalysisDesign::default(),
        )
        .expect("difference"),
    );
    close(
        at(basis(&differences, TimeBasis::WallClock), 1_200.0).0,
        1.0,
    );
    close(
        at(basis(&differences, TimeBasis::QpuDeviceAccess), 30.0).0,
        1.0,
    );
    let curves = arm_curves(&b, &Groups::new(), &AnalysisDesign::default()).expect("B curves");
    assert!(curves
        .iter()
        .all(|curve| curve.estimate.iter().all(|value| *value == 0.0)));
}

#[test]
fn all_timeouts_keep_an_exact_band_on_both_sides() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let ids: Vec<usize> = (0..10).collect();
    let a = arm(dir.path(), "a", &ids, 1_200, Outcome::Timeout);
    let b = arm(dir.path(), "b", &ids, 1_200, Outcome::Timeout);
    let differences = estimated(
        paired_difference(
            &pair("x-w62-p1200", 4_575, 4_575),
            &a,
            &b,
            &Groups::new(),
            &AnalysisDesign::default(),
        )
        .expect("difference"),
    );
    let exact = binomial_upper(0, 10, 0.025).expect("exact bound");
    for difference in &differences {
        for index in 0..difference.grid_s.len() {
            close(difference.estimate[index], 0.0);
            close(difference.lower[index], -exact);
            close(difference.upper[index], exact);
        }
        assert_eq!(difference.groups, 10);
    }
}

#[test]
fn unequal_coverage_compares_only_the_common_groups() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut a = arm(dir.path(), "a", &[0, 1], 1_200, Outcome::Timeout);
    a.extend(arm(dir.path(), "a", &[2, 3], 1_200, Outcome::Accepted(5)));
    let b = arm(dir.path(), "b", &[2, 3, 4, 5], 1_200, Outcome::Timeout);
    let differences = estimated(
        paired_difference(
            &pair("x-w62-p1200", 4_575, 4_575),
            &a,
            &b,
            &Groups::new(),
            &AnalysisDesign::default(),
        )
        .expect("difference"),
    );
    let wall = basis(&differences, TimeBasis::WallClock);
    assert_eq!((wall.groups, wall.a_only, wall.b_only), (2, 2, 2));
    // On the common groups A always succeeds and B never does.
    close(at(wall, 10.0).0, 1.0);
}

#[test]
fn grouped_instances_count_as_one_observation() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let ids: Vec<usize> = (0..3).collect();
    let a = arm(dir.path(), "a", &ids, 1_200, Outcome::Accepted(5));
    let b = arm(dir.path(), "b", &ids, 1_200, Outcome::Timeout);
    let groups: Groups = ids
        .iter()
        .map(|index| (format!("i{index}"), "template".to_string()))
        .collect();
    let curves = arm_curves(&a, &groups, &AnalysisDesign::default()).expect("A curves");
    assert!(curves
        .iter()
        .all(|curve| curve.instances == 1 && curve.trials == 3));
    let differences = estimated(
        paired_difference(
            &pair("x-w62-p1200", 4_575, 4_575),
            &a,
            &b,
            &groups,
            &AnalysisDesign::default(),
        )
        .expect("difference"),
    );
    assert!(differences.iter().all(|difference| difference.groups == 1));
}

#[test]
fn physical_pairs_stay_unmatched_and_panels_never_mix() {
    for pair_id in ["p-charge", "d-charge", "f-h500", "f-h1000", "f-routing"] {
        assert_eq!(
            pair(pair_id, 4_575, 4_575).size_status(),
            SizeStatus::UnmatchedSize
        );
    }
    assert_eq!(
        pair("x-w62-p1200", 4_575, 4_575).size_status(),
        SizeStatus::Matched
    );
    assert_eq!(
        pair("x-w62-p1200", 4_575, 512).size_status(),
        SizeStatus::UnmatchedSize
    );

    let dir = tempfile::tempdir().expect("temporary traces");
    let a = arm(dir.path(), "a", &[0], 1_200, Outcome::Accepted(5));
    let b = arm(dir.path(), "b", &[0], 1_200, Outcome::Accepted(5));
    let design = AnalysisDesign::default();
    let groups = Groups::new();
    assert_eq!(
        paired_difference(&pair("p-charge", 4_575, 4_575), &a, &b, &groups, &design),
        Ok(DifferenceOutcome::NotComparable(
            NotComparable::UnmatchedSize
        ))
    );
    let mut mixed = pair("x-w62-p1200", 4_575, 4_575);
    mixed.b.panel = TargetPanel::Common;
    assert_eq!(
        paired_difference(&mixed, &a, &b, &groups, &design),
        Ok(DifferenceOutcome::NotComparable(
            NotComparable::PanelMismatch
        ))
    );
    let disjoint = arm(dir.path(), "c", &[1], 1_200, Outcome::Accepted(5));
    assert_eq!(
        paired_difference(
            &pair("x-w62-p1200", 4_575, 4_575),
            &a,
            &disjoint,
            &groups,
            &design
        ),
        Ok(DifferenceOutcome::NotComparable(
            NotComparable::NoCommonGroups
        ))
    );
}
