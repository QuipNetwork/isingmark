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
    analysis::suite_fit::table::{ab_csv, ab_rows, load_archived_cell, ArmEvidence, AB_CSV_HEADER},
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
        rung: None,
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
    // 15 at the reference and 15 missing neither reaches nor bounds: the missing states decide.
    let half = [
        run(ClassicalDevice::Cpu, "cpu", 30, 1.0, |index| {
            (index < 15).then_some(0)
        }),
        run(ClassicalDevice::Cuda, "cuda", 30, 1.0, |index| {
            (index < 15).then_some(0)
        }),
    ];
    let label = tier(&qpu, &half).expect("a reference has a tier");
    assert!(label.speed.is_none());
    assert_eq!(label.tier, Tier::NeedsMeasurement);
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

#[test]
fn seeds_of_one_rung_reach_only_when_every_seed_reaches() {
    let qpu = reference(30, 0.25);
    let seed = |name: &str, offset: i64| {
        let mut seed = run(ClassicalDevice::Cuda, name, 30, 1.0, every(offset));
        seed.rung = Some("cluster-s1".into());
        seed
    };
    let runs = [
        run(ClassicalDevice::Cpu, "cpu", 30, 10.0, every(0)),
        seed("cuda-seed-0", 0),
        seed("cuda-seed-1", 1),
    ];
    let label = tier(&qpu, &runs).expect("a reference has a tier");
    assert_eq!(label.speed.expect("a speed pick").name, "cpu");
    assert_eq!(label.tier, Tier::One);
}

#[test]
fn missing_states_never_establish_a_speed_bound() {
    let qpu = reference(30, 0.25);
    let mut runs = vec![
        run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(1)),
        run(ClassicalDevice::Cuda, "cuda", 30, 1.0, every(1)),
    ];
    assert_eq!(tier_of(&qpu, &runs), Tier::Two);
    runs.push(run(ClassicalDevice::Cpu, "cpu-empty", 30, 100.0, |_| None));
    let label = tier(&qpu, &runs).expect("a reference has a tier");
    assert_ne!(label.speed.expect("a bound").name, "cpu-empty");
    assert_eq!(label.tier, Tier::Two);
}

#[test]
fn non_finite_timing_is_missing_timing() {
    let qpu = reference(30, 0.25);
    let cuda = run(ClassicalDevice::Cuda, "cuda", 30, 1.0, every(-1));
    let mut nan_wall = run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1));
    nan_wall.wall_s = Some(f64::NAN);
    assert_eq!(
        tier_of(&qpu, &[nan_wall, cuda.clone()]),
        Tier::NeedsMeasurement
    );
    let mut nan_budget = run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1));
    nan_budget.budget_s = Some(f64::NAN);
    assert_eq!(
        tier_of(&qpu, &[nan_budget, cuda.clone()]),
        Tier::NeedsMeasurement
    );
    let mut negative = run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1));
    negative.device_s = Some(-1.0);
    negative.wall_s = Some(-1.0);
    assert_eq!(
        tier_of(&qpu, &[negative, cuda.clone()]),
        Tier::NeedsMeasurement
    );
    let cpu = run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1));
    assert_eq!(
        tier_of(&reference(30, f64::NAN), &[cpu, cuda]),
        Tier::NeedsMeasurement
    );
}

#[test]
fn a_scaled_beat_run_is_marked_estimated() {
    let qpu = reference(30, 0.25);
    let mut cpu = run(ClassicalDevice::Cpu, "cpu", 30, 1.0, every(-1));
    cpu.wall_basis = WallBasis::Scaled;
    let cuda = run(ClassicalDevice::Cuda, "cuda", 30, 0.5, every(1));
    let label = tier(&qpu, &[cpu, cuda]).expect("a reference has a tier");
    let beat = label.beat_run.as_ref().expect("a beating run");
    assert!(beat.estimated);
    assert!(label.reason.contains("estimated"), "{}", label.reason);
}

#[derive(Clone, Copy)]
enum Outcome {
    Accepted(u64),
    Timeout,
    IncompleteCapture,
    /// Stopped by the host limit before the device budget was spent: unresolved on the device clock.
    Halted,
}

/// A trial whose trace holds one termination event, with a 30 s device budget.
fn trial(dir: &Path, id: &str, instance: &str, window_s: u64, outcome: Outcome) -> Trial {
    let events = dir.join(format!("{id}.jsonl"));
    let accepted = match outcome {
        Outcome::Accepted(seconds) => Some(seconds),
        Outcome::Timeout | Outcome::IncompleteCapture | Outcome::Halted => None,
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
        device_total: DeviceTime::from_exact_ns(
            if accepted.is_some() || matches!(outcome, Outcome::Halted) {
                1_000_000_000
            } else {
                30_000_000_000
            },
        ),
        terminal: match outcome {
            Outcome::Accepted(_) => TerminalReason::Accepted,
            Outcome::Timeout => TerminalReason::WallTimeout,
            Outcome::IncompleteCapture => TerminalReason::IncompleteCapture,
            Outcome::Halted => TerminalReason::HostComputeLimit,
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
        paired_difference(
            &synthetic,
            &a,
            &groups,
            &b,
            &groups,
            &AnalysisDesign::default()
        ),
        Err(AnalysisError::MixedBasis)
    );
    let km = AnalysisDesign {
        censoring: Censoring::KaplanMeier {
            justification: "Windows were assigned before the trials began.".into(),
        },
        ..AnalysisDesign::default()
    };
    let differences = estimated(
        paired_difference(&synthetic, &a, &groups, &b, &groups, &km).expect("KM difference"),
    );
    let wall = basis(&differences, TimeBasis::WallClock);
    close(at(wall, 10.0).0, 1.0);
    // B is unobserved after 600 s, so the difference stops at the shorter window.
    assert_eq!(wall.grid_s.last(), Some(&60.0));
    assert!(wall.grid_s.iter().all(|time| *time <= 600.0));
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
            &Groups::new(),
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
            &Groups::new(),
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
            &Groups::new(),
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
        .all(|curve| (curve.instances, curve.groups, curve.trials) == (3, 1, 3)));
    let differences = estimated(
        paired_difference(
            &pair("x-w62-p1200", 4_575, 4_575),
            &a,
            &groups,
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
        paired_difference(
            &pair("p-charge", 4_575, 4_575),
            &a,
            &groups,
            &b,
            &groups,
            &design
        ),
        Ok(DifferenceOutcome::NotComparable(
            NotComparable::UnmatchedSize
        ))
    );
    let mut mixed = pair("x-w62-p1200", 4_575, 4_575);
    mixed.b.panel = TargetPanel::Common;
    assert_eq!(
        paired_difference(&mixed, &a, &groups, &b, &groups, &design),
        Ok(DifferenceOutcome::NotComparable(
            NotComparable::PanelMismatch
        ))
    );
    let disjoint = arm(dir.path(), "c", &[1], 1_200, Outcome::Accepted(5));
    assert_eq!(
        paired_difference(
            &pair("x-w62-p1200", 4_575, 4_575),
            &a,
            &groups,
            &disjoint,
            &groups,
            &design
        ),
        Ok(DifferenceOutcome::NotComparable(
            NotComparable::NoCommonGroups
        ))
    );
}

const INDEX: &str = "evaluation/qblock/archive-index.json";

#[test]
fn an_archived_cell_loads_its_qpu_reference() {
    let cell = load_archived_cell(Path::new(INDEX), "ice-charge").expect("archived cell");
    assert_eq!(cell.reference.energies_milli.len(), 100);
    assert!((cell.reference.post_s - 0.25).abs() < f64::EPSILON);
    assert_eq!(cell.family, "ice-tiles");
    assert_eq!(
        cell.reference
            .energies_milli
            .get("06fef0b7a59d82ae777cf83a30d9e9d834bca1be8a3e71c58dbee636ca0cc2d0"),
        Some(&-1_523_188)
    );
    assert_eq!(cell.source_path, "new-pow/targets-pp-100.json");
    assert!(load_archived_cell(Path::new(INDEX), "no-such-cell").is_err());
}

#[test]
fn an_archived_arm_has_no_curve_and_no_difference() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let cell = load_archived_cell(Path::new(INDEX), "ice-twisted-w62-dimer1200").expect("cell");
    let b = arm(dir.path(), "b", &[0, 1, 2], 1_200, Outcome::Accepted(5));
    let groups = Groups::new();
    let (rows, outcome) = ab_rows(
        &pair("x-w62-p1200", 4_575, 4_575),
        &ArmEvidence::Archived {
            cell: &cell,
            runs: &[],
        },
        &ArmEvidence::Campaign {
            trials: &b,
            groups: &groups,
        },
        &AnalysisDesign::default(),
    )
    .expect("rows");
    assert_eq!(
        outcome,
        DifferenceOutcome::NotComparable(NotComparable::ArchivedArm)
    );
    let [a_row, b_row] = &rows;
    assert_eq!(
        (a_row.source, a_row.models, a_row.trials),
        ("archived", 100, 0)
    );
    assert!(a_row.wall.iter().all(|(_, band)| band.is_none()));
    assert!(a_row.device_30.is_none() && a_row.state_coverage.is_none());
    let label = a_row.tier.as_ref().expect("a historical label");
    assert_eq!(
        (label.rule_version, label.tier),
        (3, Tier::NeedsMeasurement)
    );
    assert_eq!(
        (b_row.source, b_row.models, b_row.trials),
        ("campaign", 3, 3)
    );
    assert!(b_row.tier.is_none());
    let band = b_row
        .wall
        .iter()
        .find(|(time, _)| time.total_cmp(&10.0).is_eq())
        .and_then(|(_, band)| *band);
    close(band.expect("F(10)").estimate, 1.0);

    let csv = ab_csv(&rows);
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some(AB_CSV_HEADER));
    let a_line: Vec<&str> = lines.next().expect("A row").split(',').collect();
    let header: Vec<&str> = AB_CSV_HEADER.split(',').collect();
    let column = |name: &str| header.iter().position(|field| *field == name).expect(name);
    assert_eq!(a_line[column("comparison")], "archived_arm");
    assert_eq!(a_line[column("wall_f_1200")], "");
    assert_eq!(a_line[column("historical_tier")], "Needs measurement");
    assert_eq!(a_line[column("tier_rule_version")], "3");
    assert_eq!(a_line[column("reference")], "qpu80_pp");
}

#[test]
fn campaign_rows_keep_costs_coverage_and_missing_timing() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut a = arm(dir.path(), "a", &[0, 1, 2, 3], 1_200, Outcome::Accepted(5));
    a.push(trial(
        dir.path(),
        "a-4",
        "i4",
        1_200,
        Outcome::IncompleteCapture,
    ));
    for trial in &mut a {
        trial
            .resources
            .insert("host_compute_ns".into(), 2_000_000_000);
        trial.resources.insert("evaluations".into(), 3);
    }
    let mut b = arm(
        dir.path(),
        "b",
        &[0, 1, 2, 3, 4],
        1_200,
        Outcome::Accepted(5),
    );
    b[0].device_total = DeviceTime::missing();
    let groups = Groups::new();
    let (rows, outcome) = ab_rows(
        &pair("x-w62-p1200", 4_575, 4_575),
        &ArmEvidence::Campaign {
            trials: &a,
            groups: &groups,
        },
        &ArmEvidence::Campaign {
            trials: &b,
            groups: &groups,
        },
        &AnalysisDesign::default(),
    )
    .expect("rows");
    assert!(matches!(outcome, DifferenceOutcome::Estimated(_)));
    let [a_row, b_row] = &rows;
    close(a_row.state_coverage.expect("coverage"), 0.8);
    close(a_row.host_compute_s.expect("host"), 10.0);
    assert_eq!(a_row.evaluations, Some(15));
    // Four 1 s proofs and one 30 s capture failure.
    close(a_row.device_s_upper.expect("device upper"), 34.0);
    assert_eq!(b_row.device_s_upper, None);
    assert_eq!(b_row.host_compute_s, None);
    let csv = ab_csv(&rows);
    let header: Vec<&str> = AB_CSV_HEADER.split(',').collect();
    let b_line: Vec<&str> = csv.lines().nth(2).expect("B row").split(',').collect();
    let column = |name: &str| header.iter().position(|field| *field == name).expect(name);
    assert_eq!(b_line[column("device_timing")], "missing");
    assert_eq!(b_line[column("device_s_upper")], "");
    assert_eq!(b_line[column("comparison")], "estimated");
}

fn synthetic() -> PairSpec {
    pair("x-w62-p1200", 4_575, 4_575)
}

fn wall_band_at_10(
    a: &[Trial],
    a_groups: &Groups,
    b: &[Trial],
    b_groups: &Groups,
) -> (f64, f64, f64) {
    let differences = estimated(
        paired_difference(
            &synthetic(),
            a,
            a_groups,
            b,
            b_groups,
            &AnalysisDesign::default(),
        )
        .expect("difference"),
    );
    at(basis(&differences, TimeBasis::WallClock), 10.0)
}

fn group_map(pairs: &[(&str, &str)]) -> Groups {
    pairs
        .iter()
        .map(|(instance, group)| ((*instance).to_string(), (*group).to_string()))
        .collect()
}

#[test]
fn groups_weigh_their_instances_equally() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut a: Vec<Trial> = (0..9)
        .map(|index| {
            trial(
                dir.path(),
                &format!("a-{index}"),
                "busy",
                1_200,
                Outcome::Accepted(5),
            )
        })
        .collect();
    a.push(trial(
        dir.path(),
        "a-quiet",
        "quiet",
        1_200,
        Outcome::Timeout,
    ));
    let groups = group_map(&[("busy", "g"), ("quiet", "g")]);
    let curves = arm_curves(&a, &groups, &AnalysisDesign::default()).expect("curves");
    let wall = curves
        .iter()
        .find(|curve| curve.basis == TimeBasis::WallClock)
        .expect("wall curve");
    let index = wall
        .grid_s
        .iter()
        .position(|time| time.total_cmp(&10.0).is_eq())
        .expect("10 s point");
    close(wall.estimate[index], 0.5);
    let b: Vec<Trial> = ["busy", "quiet"]
        .iter()
        .map(|instance| {
            trial(
                dir.path(),
                &format!("b-{instance}"),
                instance,
                1_200,
                Outcome::Timeout,
            )
        })
        .collect();
    close(wall_band_at_10(&a, &groups, &b, &groups).0, 0.5);
}

#[test]
fn each_arm_uses_its_own_group_map() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let outcomes = |prefix: &str| {
        vec![
            trial(
                dir.path(),
                &format!("{prefix}-0"),
                "i0",
                1_200,
                Outcome::Accepted(5),
            ),
            trial(
                dir.path(),
                &format!("{prefix}-1"),
                "i1",
                1_200,
                Outcome::Timeout,
            ),
        ]
    };
    let (a, b) = (outcomes("a"), outcomes("b"));
    let a_groups = group_map(&[("i0", "g0"), ("i1", "g1")]);
    let b_groups = group_map(&[("i0", "g1"), ("i1", "g0")]);
    let (rows, outcome) = ab_rows(
        &synthetic(),
        &ArmEvidence::Campaign {
            trials: &a,
            groups: &a_groups,
        },
        &ArmEvidence::Campaign {
            trials: &b,
            groups: &b_groups,
        },
        &AnalysisDesign::default(),
    )
    .expect("rows");
    assert_eq!(rows[1].groups, 2);
    let differences = estimated(outcome);
    // Group differences are +1 and -1, so the joint band must have width.
    let (estimate, lower, upper) = at(basis(&differences, TimeBasis::WallClock), 10.0);
    close(estimate, 0.0);
    assert!(lower < 0.0 && upper > 0.0, "[{lower}, {upper}]");
}

#[test]
fn resampling_keeps_the_arms_paired() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let arm_of = |prefix: &str, first: Outcome, second: Outcome| {
        vec![
            trial(dir.path(), &format!("{prefix}-0"), "i0", 1_200, first),
            trial(dir.path(), &format!("{prefix}-1"), "i1", 1_200, second),
        ]
    };
    let none = Groups::new();
    let a = arm_of("a", Outcome::Accepted(5), Outcome::Timeout);
    let same = arm_of("same", Outcome::Accepted(5), Outcome::Timeout);
    let (estimate, lower, upper) = wall_band_at_10(&a, &none, &same, &none);
    close(estimate, 0.0);
    close(lower, 0.0);
    close(upper, 0.0);
    let opposite = arm_of("opposite", Outcome::Timeout, Outcome::Accepted(5));
    let (estimate, lower, upper) = wall_band_at_10(&a, &none, &opposite, &none);
    close(estimate, 0.0);
    assert!(lower < 0.0 && upper > 0.0, "[{lower}, {upper}]");
}

#[test]
fn unresolved_device_time_keeps_the_zero_success_bound() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut a = Vec::new();
    for group in 0..10 {
        let instance = format!("i{group}");
        for index in 0..9 {
            a.push(trial(
                dir.path(),
                &format!("a-{group}-{index}"),
                &instance,
                1_200,
                Outcome::Timeout,
            ));
        }
        a.push(trial(
            dir.path(),
            &format!("a-{group}-h"),
            &instance,
            1_200,
            Outcome::Halted,
        ));
    }
    let b = arm(
        dir.path(),
        "b",
        &(0..10).collect::<Vec<_>>(),
        1_200,
        Outcome::Timeout,
    );
    let none = Groups::new();
    let differences = estimated(
        paired_difference(
            &synthetic(),
            &a,
            &none,
            &b,
            &none,
            &AnalysisDesign::default(),
        )
        .expect("difference"),
    );
    let exact = binomial_upper(0, 10, 0.025).expect("exact bound");
    let (estimate, _, upper) = at(basis(&differences, TimeBasis::QpuDeviceAccess), 30.0);
    close(estimate, 0.0);
    assert!(upper >= exact, "{upper} < {exact}");
}

#[test]
fn an_early_lost_capture_lowers_state_coverage() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut a = arm(dir.path(), "a", &[0, 1], 1_200, Outcome::Accepted(5));
    a[0].resources.insert("incomplete_captures".into(), 1);
    let groups = Groups::new();
    let (rows, _) = ab_rows(
        &synthetic(),
        &ArmEvidence::Campaign {
            trials: &a,
            groups: &groups,
        },
        &ArmEvidence::Campaign {
            trials: &a,
            groups: &groups,
        },
        &AnalysisDesign::default(),
    )
    .expect("rows");
    close(rows[0].state_coverage.expect("coverage"), 0.5);
}

#[test]
fn the_csv_marks_a_scaled_beat_run() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let cell = load_archived_cell(Path::new(INDEX), "ice-twisted-w62-dimer1200").expect("cell");
    let classical = |device, name: &str, wall_s: f64, offset: i64, wall_basis| ClassicalRun {
        device,
        name: name.into(),
        budget_s: None,
        wall_s: Some(wall_s),
        device_s: Some(wall_s),
        wall_basis,
        rung: None,
        energies_milli: cell
            .reference
            .energies_milli
            .iter()
            .map(|(model, energy)| (model.clone(), Some(energy + offset)))
            .collect(),
    };
    let runs = [
        classical(ClassicalDevice::Cpu, "cpu", 1.0, -1, WallBasis::Scaled),
        classical(ClassicalDevice::Cuda, "cuda", 0.5, 0, WallBasis::Measured),
    ];
    let b = arm(dir.path(), "b", &[0], 1_200, Outcome::Accepted(5));
    let groups = Groups::new();
    let (rows, _) = ab_rows(
        &synthetic(),
        &ArmEvidence::Archived {
            cell: &cell,
            runs: &runs,
        },
        &ArmEvidence::Campaign {
            trials: &b,
            groups: &groups,
        },
        &AnalysisDesign::default(),
    )
    .expect("rows");
    let csv = ab_csv(&rows);
    let header: Vec<&str> = AB_CSV_HEADER.split(',').collect();
    let a_line: Vec<&str> = csv.lines().nth(1).expect("A row").split(',').collect();
    let column = |name: &str| header.iter().position(|field| *field == name).expect(name);
    assert_eq!(a_line[column("tier_beat_wall")], "scaled");
    assert_eq!(a_line[column("tier_speed_wall")], "measured");
}

#[test]
fn the_speed_text_rounds_walls_to_milliseconds() {
    let qpu = reference(30, 0.25);
    let cpu = run(
        ClassicalDevice::Cpu,
        "cpu",
        30,
        23.274_044_127_499_998,
        every(-1),
    );
    let cuda = run(ClassicalDevice::Cuda, "cuda", 30, 1.0, every(1));
    let label = tier(&qpu, &[cpu, cuda]).expect("a reference has a tier");
    assert!(
        label.reason.contains("at 23.274 s per model"),
        "{}",
        label.reason
    );
    assert!(!label.reason.contains("23.27404"), "{}", label.reason);
}
