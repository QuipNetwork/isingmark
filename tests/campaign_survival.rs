// SPDX-License-Identifier: AGPL-3.0-or-later
//! Success curves retain censoring, instance weights, and device uncertainty.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::{Duration, SystemTime},
};

use isingmark::campaign::{
    analysis::confidence::{binomial_lower, binomial_upper},
    analysis::survival::{
        check_probability, estimate_curves, quantile, AnalysisDesign, AnalysisError, Censoring,
        Curve, Quantile,
    },
    contracts::{TimeBasis, TimingStatus},
    runner::{Preparation, TerminalReason, Trial},
    trace::{DeviceTime, EventKind, TraceEvent, TraceWriter, TRACE_VERSION},
};

#[expect(
    clippy::duration_suboptimal_units,
    reason = "Keep the specified 1,200-second wall observation window explicit."
)]
fn trial(dir: &Path, id: &str, instance: &str, accepted_s: Option<u64>) -> Trial {
    let events = dir.join(format!("{id}.jsonl"));
    let event = TraceEvent {
        version: TRACE_VERSION,
        run_id: "run".into(),
        trial_id: id.into(),
        session_id: "session".into(),
        seq: 0,
        observer: "coordinator".into(),
        kind: EventKind::TrialTermination,
        wall_ns: accepted_s.unwrap_or(1_200) * 1_000_000_000,
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
        .expect("write complete event");
    Trial {
        trial_id: id.into(),
        instance_id: instance.into(),
        policy_id: "policy".into(),
        started: SystemTime::UNIX_EPOCH,
        wall_limit: Some(Duration::from_secs(1_200)),
        device_budget_ns: Some(30_000_000_000),
        resources: BTreeMap::new(),
        events,
        accepted_wall: accepted_s.map(Duration::from_secs),
        device_total: DeviceTime::from_exact_ns(if accepted_s.is_some() {
            1_000_000_000
        } else {
            30_000_000_000
        }),
        terminal: if accepted_s.is_some() {
            TerminalReason::Accepted
        } else {
            TerminalReason::WallTimeout
        },
        preparation: Preparation::Cold,
    }
}

fn curve(curves: &[Curve], basis: TimeBasis) -> &Curve {
    curves
        .iter()
        .find(|curve| curve.basis == basis)
        .expect("curve for the requested clock")
}

fn point(curve: &Curve, seconds: f64) -> usize {
    curve
        .grid_s
        .iter()
        .position(|time| time.total_cmp(&seconds).is_eq())
        .expect("time is in the grid")
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

fn km_design() -> AnalysisDesign {
    AnalysisDesign {
        censoring: Censoring::KaplanMeier {
            justification: "Observation windows were assigned before trials began.".into(),
        },
        ..AnalysisDesign::default()
    }
}

#[test]
fn complete_cdf_matches_empirical() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let trials: Vec<_> = (1..=10)
        .map(|id| trial(dir.path(), &id.to_string(), "one", (id <= 7).then_some(id)))
        .collect();
    let curves = estimate_curves(&trials, &AnalysisDesign::default()).expect("complete CDF");
    let wall = curve(&curves, TimeBasis::WallClock);
    close(wall.estimate[point(wall, 5.0)], 0.5);
    close(wall.estimate[point(wall, 1_200.0)], 0.7);
    assert_eq!(wall.lower, wall.estimate);
    assert_eq!(wall.upper, wall.estimate);
    assert_eq!((wall.trials, wall.instances, wall.unresolved), (10, 1, 0));
    assert_eq!(quantile(wall, 0.5), Quantile::Identified(5.0));
    assert!(wall
        .assumptions
        .iter()
        .any(|text| text.contains("FixedWindow")));
}

#[test]
fn km_matches_hand_calculation() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut trials: Vec<_> = [2, 4, 6]
        .into_iter()
        .map(|time| trial(dir.path(), &time.to_string(), "one", Some(time)))
        .collect();
    for time in [3, 5] {
        let mut censored = trial(dir.path(), &time.to_string(), "one", None);
        censored.wall_limit = Some(Duration::from_secs(time));
        trials.push(censored);
    }
    let design = km_design();
    let curves = estimate_curves(&trials, &design).expect("Kaplan-Meier estimate");
    let wall = curve(&curves, TimeBasis::WallClock);
    for (time, survival) in [(2.0, 0.8), (4.0, 0.533_333), (6.0, 0.0)] {
        close(1.0 - wall.estimate[point(wall, time)], survival);
    }
    assert!(wall.assumptions.iter().any(|text| {
        text.contains("KaplanMeier")
            && text.contains("Observation windows were assigned before trials began.")
    }));
}

#[test]
fn all_timeouts_have_unresolved_quantiles() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let trials: Vec<_> = (0..5)
        .map(|id| trial(dir.path(), &id.to_string(), "one", None))
        .collect();
    for design in [AnalysisDesign::default(), km_design()] {
        let curves = estimate_curves(&trials, &design).expect("timeout curve");
        let wall = curve(&curves, TimeBasis::WallClock);
        assert_eq!(quantile(wall, 0.95), Quantile::Above(1_200.0));
        assert_eq!(wall.unresolved, 0);
        assert!(wall.estimate.iter().all(|estimate| estimate.abs() < 1e-12));
    }
}

#[test]
fn device_and_wall_curves_have_distinct_bases() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let curves = estimate_curves(
        &[trial(dir.path(), "one", "one", Some(80))],
        &AnalysisDesign::default(),
    )
    .expect("both clocks");
    assert_eq!(curves.len(), 2);
    let wall = curve(&curves, TimeBasis::WallClock);
    let device = curve(&curves, TimeBasis::QpuDeviceAccess);
    close(wall.observation_limit_s, 1_200.0);
    close(device.observation_limit_s, 30.0);
    close(wall.estimate[point(wall, 30.0)], 0.0);
    close(device.estimate[point(device, 30.0)], 1.0);
}

#[test]
fn missing_cost_cannot_pass_device_guide() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut accepted = trial(dir.path(), "one", "one", Some(2));
    accepted.device_total = DeviceTime::missing();
    let curves = estimate_curves(&[accepted], &AnalysisDesign::default()).expect("bounded result");
    let device = curve(&curves, TimeBasis::QpuDeviceAccess);
    assert_eq!(device.unresolved, 1);
    let last = point(device, 30.0);
    close(device.estimate[last], 0.0);
    close(device.lower[last], 0.0);
    close(device.upper[last], 1.0);
}

#[test]
fn instance_weights_ignore_repeat_count() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let trials = [
        trial(dir.path(), "a1", "A", Some(1)),
        trial(dir.path(), "b1", "B", None),
        trial(dir.path(), "b2", "B", None),
        trial(dir.path(), "b3", "B", None),
    ];
    let curves = estimate_curves(&trials, &AnalysisDesign::default()).expect("instance weights");
    for curve in curves {
        close(
            curve.estimate[point(&curve, curve.observation_limit_s)],
            0.5,
        );
        assert_eq!((curve.trials, curve.instances), (4, 2));
    }
}

#[test]
fn corrupted_trace_prevents_a_curve() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let corrupt = trial(dir.path(), "corrupt", "one", Some(2));
    OpenOptions::new()
        .append(true)
        .open(&corrupt.events)
        .expect("append partial line")
        .write_all(b"{\"version\":")
        .expect("write truncated tail without newline");
    let valid = trial(dir.path(), "valid", "one", Some(1));
    assert!(matches!(
        estimate_curves(&[valid, corrupt], &AnalysisDesign::default()),
        Err(AnalysisError::CorruptTrace(_))
    ));
}

#[test]
fn unreadable_or_malformed_trace_prevents_a_curve() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut trial = trial(dir.path(), "one", "one", Some(1));
    let valid_path = trial.events.clone();
    for path in [dir.path().join("absent.jsonl"), dir.path().to_path_buf()] {
        trial.events = path;
        assert!(matches!(
            estimate_curves(&[trial.clone()], &AnalysisDesign::default()),
            Err(AnalysisError::CorruptTrace(_))
        ));
    }
    trial.events = valid_path;
    fs::write(&trial.events, b"malformed complete line\n").expect("malformed trace");
    assert!(matches!(
        estimate_curves(&[trial], &AnalysisDesign::default()),
        Err(AnalysisError::CorruptTrace(_))
    ));
}

#[test]
fn trace_without_termination_prevents_a_curve() {
    let dir = tempfile::tempdir().expect("temporary traces");
    for empty in [true, false] {
        let trial = trial(dir.path(), &format!("empty-{empty}"), "one", Some(1));
        let mut event = isingmark::campaign::trace::read_trace(&trial.events)
            .expect("trace")
            .events
            .remove(0);
        let mut writer = TraceWriter::create(&trial.events).expect("replace trace");
        if !empty {
            event.kind = EventKind::ChallengeRelease;
            writer.append(&event).expect("release only");
        }
        assert!(matches!(
            estimate_curves(&[trial], &AnalysisDesign::default()),
            Err(AnalysisError::CorruptTrace(_))
        ));
    }
}

#[test]
fn empty_trials_or_either_empty_grid_are_rejected() {
    assert!(matches!(
        estimate_curves(&[], &AnalysisDesign::default()),
        Err(AnalysisError::Empty)
    ));
    let dir = tempfile::tempdir().expect("temporary traces");
    let trials = [trial(dir.path(), "one", "one", Some(1))];
    for design in [
        AnalysisDesign {
            wall_grid_s: vec![],
            ..AnalysisDesign::default()
        },
        AnalysisDesign {
            device_grid_s: vec![],
            ..AnalysisDesign::default()
        },
        AnalysisDesign {
            bootstrap_resamples: 0,
            ..AnalysisDesign::default()
        },
    ] {
        assert!(matches!(
            estimate_curves(&trials, &design),
            Err(AnalysisError::Empty)
        ));
    }
}

#[test]
fn fixed_window_rejects_unequal_windows() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut short = trial(dir.path(), "short", "one", None);
    short.wall_limit = Some(Duration::from_secs(3));
    let long = trial(dir.path(), "long", "one", Some(4));
    assert!(matches!(
        estimate_curves(&[short, long], &AnalysisDesign::default()),
        Err(AnalysisError::MixedBasis)
    ));
}

#[test]
fn non_success_requires_a_wall_limit() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut failed = trial(dir.path(), "one", "one", None);
    failed.wall_limit = None;
    for design in [AnalysisDesign::default(), km_design()] {
        assert!(matches!(
            estimate_curves(&[failed.clone()], &design),
            Err(AnalysisError::CorruptTrace(_))
        ));
    }
}

#[test]
fn proof_acceptance_is_independent_of_terminal() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut trials = vec![trial(dir.path(), "success", "one", Some(2))];
    for terminal in [
        TerminalReason::WallTimeout,
        TerminalReason::DeviceBudgetExhausted,
        TerminalReason::HostComputeLimit,
        TerminalReason::EvaluationLimit,
        TerminalReason::MinerCrashed,
        TerminalReason::MinerRejected,
        TerminalReason::Cancelled,
    ] {
        let mut failed = trial(dir.path(), &format!("{terminal:?}"), "one", Some(1));
        failed.terminal = terminal;
        trials.push(failed);
    }
    for design in [AnalysisDesign::default(), km_design()] {
        let curves = estimate_curves(&trials, &design).expect("operational failures");
        let wall = curve(&curves, TimeBasis::WallClock);
        close(wall.estimate[point(wall, 1.0)], 0.875);
        close(wall.estimate[point(wall, 2.0)], 1.0);
        close(wall.estimate[point(wall, 1_200.0)], 1.0);
        let device = curve(&curves, TimeBasis::QpuDeviceAccess);
        close(device.estimate[point(device, 30.0)], 1.0);
    }
}

#[test]
fn km_is_computed_within_each_instance() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut censored = trial(dir.path(), "a2", "A", None);
    censored.wall_limit = Some(Duration::from_secs(3));
    let trials = [
        trial(dir.path(), "a1", "A", Some(2)),
        censored,
        trial(dir.path(), "b1", "B", Some(4)),
    ];
    let curves = estimate_curves(&trials, &km_design()).expect("instance KM estimates");
    let wall = curve(&curves, TimeBasis::WallClock);
    close(wall.estimate[point(wall, 2.0)], 0.25);
    close(wall.estimate[point(wall, 4.0)], 0.75);
}

#[test]
fn km_keeps_tied_censors_at_risk_for_events() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut censored = trial(dir.path(), "censor", "one", None);
    censored.wall_limit = Some(Duration::from_secs(2));
    let trials = [
        trial(dir.path(), "first", "one", Some(2)),
        trial(dir.path(), "second", "one", Some(2)),
        trial(dir.path(), "last", "one", Some(4)),
        censored,
    ];
    let curves = estimate_curves(&trials, &km_design()).expect("tied events and censor");
    let wall = curve(&curves, TimeBasis::WallClock);
    close(wall.estimate[point(wall, 2.0)], 0.5);
    close(wall.estimate[point(wall, 4.0)], 1.0);
}

#[test]
fn success_times_extend_sorted_deduplicated_grids_within_the_limits() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut first = trial(dir.path(), "first", "one", Some(2));
    first.accepted_wall = Some(Duration::from_millis(2_250));
    first.device_total = DeviceTime::from_exact_ns(2_500_000_000);
    let mut tied = trial(dir.path(), "tied", "one", Some(2));
    tied.accepted_wall = first.accepted_wall;
    tied.device_total = DeviceTime {
        lower_ns: 1_000_000_000,
        upper_ns: Some(2_500_000_000),
        status: TimingStatus::ConservativeEstimate,
    };
    let mut late = trial(dir.path(), "late", "one", Some(1_201));
    late.device_total = DeviceTime::from_exact_ns(31_000_000_000);
    let design = AnalysisDesign {
        wall_grid_s: vec![1_300.0, 10.0, 1.0, 10.0],
        device_grid_s: vec![35.0, 10.0, 1.0, 10.0, 30.0],
        censoring: Censoring::FixedWindow,
        ..AnalysisDesign::default()
    };
    let curves = estimate_curves(&[first, tied, late], &design).expect("exact success times");
    let wall = curve(&curves, TimeBasis::WallClock);
    assert_eq!(wall.grid_s, vec![1.0, 2.25, 10.0]);
    assert_eq!(quantile(wall, 0.5), Quantile::Identified(2.25));
    assert_eq!(quantile(wall, 0.95), Quantile::Above(1_200.0));
    let device = curve(&curves, TimeBasis::QpuDeviceAccess);
    assert_eq!(device.grid_s, vec![1.0, 2.5, 10.0, 30.0]);
    assert_eq!(quantile(device, 0.5), Quantile::Identified(2.5));
    close(device.estimate[point(device, 1.0)], 0.0);
    close(device.upper[point(device, 1.0)], 0.975);
}

#[test]
fn accepted_device_costs_use_full_bounds_and_timing_status() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let cases = [
        (
            DeviceTime::from_exact_ns(30_000_000_000),
            1.0,
            0.025,
            1.0,
            0,
        ),
        (
            DeviceTime::from_exact_ns(31_000_000_000),
            0.0,
            0.0,
            0.975,
            0,
        ),
        (DeviceTime::from_truncated_us(30_000_000), 0.0, 0.0, 1.0, 1),
        (
            DeviceTime {
                lower_ns: 29_000_000_000,
                upper_ns: Some(30_000_000_000),
                status: TimingStatus::ConservativeEstimate,
            },
            1.0,
            0.025,
            1.0,
            0,
        ),
        (
            DeviceTime {
                lower_ns: 29_000_000_000,
                upper_ns: Some(31_000_000_000),
                status: TimingStatus::Complete,
            },
            0.0,
            0.0,
            1.0,
            1,
        ),
        (
            DeviceTime {
                lower_ns: 0,
                upper_ns: Some(0),
                status: TimingStatus::Missing,
            },
            0.0,
            0.0,
            1.0,
            1,
        ),
        (
            DeviceTime {
                lower_ns: 31_000_000_000,
                upper_ns: None,
                status: TimingStatus::Missing,
            },
            0.0,
            0.0,
            0.975,
            0,
        ),
    ];
    for (index, (cost, estimate, lower, upper, unresolved)) in cases.into_iter().enumerate() {
        let mut accepted = trial(dir.path(), &index.to_string(), "one", Some(2));
        accepted.device_total = cost;
        let curves =
            estimate_curves(&[accepted], &AnalysisDesign::default()).expect("device bounds");
        let device = curve(&curves, TimeBasis::QpuDeviceAccess);
        let last = point(device, 30.0);
        close(device.estimate[last], estimate);
        close(device.lower[last], lower);
        close(device.upper[last], upper);
        assert_eq!(device.unresolved, unresolved);
    }
}

#[test]
fn halted_device_trials_are_unresolved_until_the_budget_is_spent() {
    let dir = tempfile::tempdir().expect("temporary traces");
    for terminal in [
        TerminalReason::WallTimeout,
        TerminalReason::HostComputeLimit,
        TerminalReason::EvaluationLimit,
    ] {
        let mut halted = trial(dir.path(), &format!("{terminal:?}"), "one", None);
        halted.terminal = terminal;
        halted.device_total = DeviceTime::from_exact_ns(10_000_000_000);
        let curves = estimate_curves(&[halted], &AnalysisDesign::default()).expect("joint bounds");
        let device = curve(&curves, TimeBasis::QpuDeviceAccess);
        close(device.upper[point(device, 10.0)], 0.975);
        close(device.upper[point(device, 30.0)], 1.0);
        close(device.estimate[point(device, 30.0)], 0.0);
        assert_eq!(device.unresolved, 1);
        assert!(device
            .assumptions
            .iter()
            .any(|text| text.contains("joint") && text.contains("wall")));
    }
}

#[test]
fn exhausted_device_budget_is_failure_even_without_timing() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut exhausted = trial(dir.path(), "one", "one", None);
    exhausted.terminal = TerminalReason::DeviceBudgetExhausted;
    exhausted.device_total = DeviceTime::missing();
    let curves =
        estimate_curves(&[exhausted], &AnalysisDesign::default()).expect("exhausted budget");
    let device = curve(&curves, TimeBasis::QpuDeviceAccess);
    close(device.upper[point(device, 30.0)], 0.975);
    assert_eq!(device.unresolved, 0);
}

#[test]
fn operational_failures_do_not_widen_the_unresolved_device_band() {
    let dir = tempfile::tempdir().expect("temporary traces");
    for terminal in [
        TerminalReason::MinerCrashed,
        TerminalReason::MinerRejected,
        TerminalReason::Cancelled,
    ] {
        let mut failed = trial(dir.path(), &format!("{terminal:?}"), "one", None);
        failed.terminal = terminal;
        failed.device_total = DeviceTime::from_exact_ns(10_000_000_000);
        let curves =
            estimate_curves(&[failed], &AnalysisDesign::default()).expect("failure curves");
        let device = curve(&curves, TimeBasis::QpuDeviceAccess);
        close(device.estimate[point(device, 30.0)], 0.0);
        close(device.upper[point(device, 30.0)], 0.975);
        assert_eq!(device.unresolved, 0);
        assert!(device
            .assumptions
            .iter()
            .any(|text| text.contains("operational failure")));
        let wall = curve(&curves, TimeBasis::WallClock);
        close(wall.estimate[point(wall, 1_200.0)], 0.0);
    }
}

#[test]
fn invalid_run_prevents_a_conclusive_report_even_with_a_proof() {
    let dir = tempfile::tempdir().expect("temporary traces");
    for accepted in [None, Some(5)] {
        let mut invalid = trial(dir.path(), &format!("{accepted:?}"), "one", accepted);
        invalid.terminal = TerminalReason::InvalidRun;
        assert!(matches!(
            estimate_curves(&[invalid], &AnalysisDesign::default()),
            Err(AnalysisError::CorruptTrace(_))
        ));
    }
}

#[test]
fn wall_success_survives_an_exceeded_device_budget() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut accepted = trial(dir.path(), "split", "one", Some(5));
    accepted.terminal = TerminalReason::DeviceBudgetExhausted;
    accepted.device_total = DeviceTime::from_exact_ns(31_000_000_000);
    let design = AnalysisDesign {
        wall_grid_s: vec![5.0, 1_200.0],
        ..AnalysisDesign::default()
    };
    let curves = estimate_curves(&[accepted], &design).expect("split curves");
    let wall = curve(&curves, TimeBasis::WallClock);
    close(wall.estimate[point(wall, 5.0)], 1.0);
    let device = curve(&curves, TimeBasis::QpuDeviceAccess);
    close(device.estimate[point(device, 30.0)], 0.0);
    assert_eq!(device.unresolved, 0);
}

#[test]
fn device_success_survives_a_late_wall_acceptance() {
    let dir = tempfile::tempdir().expect("temporary traces");
    for terminal in [TerminalReason::WallTimeout, TerminalReason::Accepted] {
        let mut accepted = trial(dir.path(), &format!("{terminal:?}"), "one", Some(1_300));
        accepted.terminal = terminal;
        accepted.device_total = DeviceTime::from_exact_ns(29_000_000_000);
        for design in [AnalysisDesign::default(), km_design()] {
            let curves = estimate_curves(&[accepted.clone()], &design).expect("split curves");
            let wall = curve(&curves, TimeBasis::WallClock);
            close(wall.estimate[point(wall, 1_200.0)], 0.0);
            assert!(!wall.grid_s.contains(&1_300.0));
            let device = curve(&curves, TimeBasis::QpuDeviceAccess);
            close(device.estimate[point(device, 30.0)], 1.0);
            assert_eq!(device.unresolved, 0);
        }
    }
}

#[test]
fn unresolved_band_uses_instance_weights_but_count_uses_trials() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut trials = vec![trial(dir.path(), "a1", "A", Some(1))];
    for id in 0..3 {
        let mut missing = trial(dir.path(), &format!("b{id}"), "B", Some(2));
        missing.device_total = DeviceTime::missing();
        trials.push(missing);
    }
    let curves =
        estimate_curves(&trials, &AnalysisDesign::default()).expect("weighted uncertainty");
    let device = curve(&curves, TimeBasis::QpuDeviceAccess);
    close(device.estimate[point(device, 30.0)], 0.5);
    close(device.upper[point(device, 30.0)], 1.0);
    assert_eq!(device.unresolved, 3);
}

#[test]
fn probability_must_be_in_the_open_closed_unit_interval() {
    for probability in [f64::NAN, f64::NEG_INFINITY, -0.1, 0.0, 1.1, f64::INFINITY] {
        assert_eq!(
            check_probability(probability),
            Err(AnalysisError::InvalidProbability)
        );
    }
    for probability in [f64::MIN_POSITIVE, 0.5, 1.0] {
        assert_eq!(check_probability(probability), Ok(()));
    }
}

#[test]
fn zero_success_has_positive_exact_upper_at_every_point() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let trials = [
        trial(dir.path(), "a1", "A", None),
        trial(dir.path(), "a2", "A", None),
        trial(dir.path(), "b1", "B", None),
    ];
    let expected = binomial_upper(0, 2, 0.025).expect("two independent instances");
    for curve in estimate_curves(&trials, &AnalysisDesign::default()).expect("zero-success curves")
    {
        for (&lower, &upper) in curve.lower.iter().zip(&curve.upper) {
            close(lower, 0.0);
            assert!(upper > 0.0);
            close(upper, expected);
        }
    }
}

#[test]
fn unresolved_trials_widen_a_zero_success_device_band() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut trials = Vec::with_capacity(30);
    for index in 0..30 {
        let id = format!("instance-{index}");
        let mut accepted = trial(dir.path(), &id, &id, Some(1));
        accepted.device_total = DeviceTime::missing();
        trials.push(accepted);
    }

    let curves = estimate_curves(&trials, &AnalysisDesign::default()).expect("missing timing");
    let device = curve(&curves, TimeBasis::QpuDeviceAccess);
    let at_30_seconds = point(device, 30.0);
    close(device.estimate[at_30_seconds], 0.0);
    assert_eq!(device.unresolved, 30);
    close(device.upper[at_30_seconds], 1.0);
}

#[test]
fn all_success_uses_exact_lower_at_the_limit() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let trials = [
        trial(dir.path(), "a1", "A", Some(1)),
        trial(dir.path(), "a2", "A", Some(2)),
        trial(dir.path(), "b1", "B", Some(3)),
    ];
    let expected = binomial_lower(2, 2, 0.025).expect("two independent instances");
    for curve in estimate_curves(&trials, &AnalysisDesign::default()).expect("all-success curves") {
        let limit = point(&curve, curve.observation_limit_s);
        close(curve.lower[limit], expected);
        close(curve.estimate[limit], 1.0);
        close(curve.upper[limit], 1.0);
    }
}

#[test]
fn unresolved_trials_widen_only_the_device_upper_band() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut stopped = trial(dir.path(), "stopped", "A", None);
    stopped.device_total = DeviceTime::missing();
    let trials = [trial(dir.path(), "accepted", "A", Some(1)), stopped];
    let curves = estimate_curves(&trials, &AnalysisDesign::default()).expect("uncertain timing");
    let wall = curve(&curves, TimeBasis::WallClock);
    let device = curve(&curves, TimeBasis::QpuDeviceAccess);
    let wall_point = point(wall, 30.0);
    let device_point = point(device, 30.0);
    close(device.estimate[device_point], wall.estimate[wall_point]);
    close(device.lower[device_point], wall.lower[wall_point]);
    assert!(device.upper[device_point] >= wall.upper[wall_point]);
    close(wall.upper[wall_point], 0.5);
    close(device.upper[device_point], 1.0);
    assert_eq!(wall.unresolved, 0);
}

#[test]
fn bootstrap_bands_use_equal_instance_weights_and_enclose_estimates() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let trials = [
        trial(dir.path(), "a1", "A", Some(1)),
        trial(dir.path(), "a2", "A", None),
        trial(dir.path(), "a3", "A", None),
        trial(dir.path(), "a4", "A", None),
        trial(dir.path(), "b1", "B", Some(1)),
    ];
    let design = AnalysisDesign::default();
    let curves = estimate_curves(&trials, &design).expect("cluster bands");
    assert_eq!(
        curves,
        estimate_curves(&trials, &design).expect("repeatable bands")
    );
    for curve in curves {
        let limit = point(&curve, curve.observation_limit_s);
        close(curve.estimate[limit], 0.625);
        close(curve.lower[limit], 0.25);
        close(curve.upper[limit], 1.0);
        for ((&lower, &estimate), &upper) in
            curve.lower.iter().zip(&curve.estimate).zip(&curve.upper)
        {
            assert!(0.0 <= lower && lower <= estimate && estimate <= upper && upper <= 1.0);
        }
        assert!(curve.assumptions.iter().any(|text| {
            text.contains("nearest rank")
                && text.contains("0.025")
                && text.contains("2000")
                && text.contains("seed=0")
                && text.contains("n=2")
        }));
    }
}

#[test]
fn km_bootstrap_recomputes_each_drawn_instance() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut censored = trial(dir.path(), "a2", "A", None);
    censored.wall_limit = Some(Duration::from_secs(3));
    let trials = [
        trial(dir.path(), "a1", "A", Some(2)),
        censored,
        trial(dir.path(), "b1", "B", Some(4)),
    ];
    let curves = estimate_curves(&trials, &km_design()).expect("instance KM bands");
    let wall = curve(&curves, TimeBasis::WallClock);
    let at_four = point(wall, 4.0);
    close(wall.estimate[at_four], 0.75);
    close(wall.lower[at_four], 0.5);
    close(wall.upper[at_four], 1.0);
}

#[test]
fn km_at_one_does_not_imply_every_trial_succeeded() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let mut censored = trial(dir.path(), "censor", "A", None);
    censored.wall_limit = Some(Duration::from_secs(1));
    let trials = [censored, trial(dir.path(), "accepted", "A", Some(2))];
    let curves = estimate_curves(&trials, &km_design()).expect("censored KM endpoint");
    let wall = curve(&curves, TimeBasis::WallClock);
    close(wall.lower[point(wall, 2.0)], 1.0);
}

#[test]
fn one_resample_encloses_the_estimate_and_uses_the_configured_seed() {
    let dir = tempfile::tempdir().expect("temporary traces");
    let trials = [
        trial(dir.path(), "a1", "A", Some(1)),
        trial(dir.path(), "a2", "A", None),
        trial(dir.path(), "b1", "B", Some(1)),
    ];
    let mut saw_lower_draw = false;
    let mut saw_upper_draw = false;
    for seed in 0..16 {
        let design = AnalysisDesign {
            bootstrap_resamples: 1,
            bootstrap_seed: seed,
            ..AnalysisDesign::default()
        };
        let curves = estimate_curves(&trials, &design).expect("single bootstrap draw");
        for curve in curves {
            let limit = point(&curve, curve.observation_limit_s);
            close(curve.estimate[limit], 0.75);
            assert!(curve.lower[limit] <= 0.75 && curve.upper[limit] >= 0.75);
            assert!(curve.upper[limit] - curve.lower[limit] <= 0.25);
            saw_lower_draw |= curve.lower[limit] < 0.75;
            saw_upper_draw |= curve.upper[limit] > 0.75;
        }
    }
    assert!(saw_lower_draw && saw_upper_draw);
}
