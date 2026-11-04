// SPDX-License-Identifier: AGPL-3.0-or-later
//! Suite-fit A/B comparisons and the historical Tier rule version 3.

use std::collections::BTreeMap;

use isingmark::campaign::analysis::suite_fit::tier::{
    qpu_wall, tier, ClassicalDevice, ClassicalRun, QpuReference, Tier, WallBasis, TIER_RULE_VERSION,
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
