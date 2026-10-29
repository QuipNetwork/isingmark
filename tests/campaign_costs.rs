// SPDX-License-Identifier: AGPL-3.0-or-later
//! Exact power sizing, cluster inflation, and resource limits for campaign evidence.

use isingmark::campaign::analysis::{
    confidence::{binomial_lower, binomial_upper},
    costs::{
        device_hours, fleet_success, gate_power, plan_evidence, Alternatives, EvidenceOutcome,
        EvidencePlan, Gate, Guides, PilotSummary, ResourceLimits,
    },
    survival::AnalysisError,
};

fn pilot() -> PilotSummary {
    PilotSummary {
        trials_per_cluster: 1.0,
        intracluster_correlation: 0.0,
        mean_trial_wall_s: 1_200.0,
        mean_trial_device_s: 30.0,
    }
}

fn limits() -> ResourceLimits {
    ResourceLimits {
        cpu_hours: 1_000_000.0,
        gpu_hours: 1_000_000.0,
        qpu_access_s: 1_000_000.0,
        elapsed_hours: 1_000_000.0,
    }
}

fn default_plan(pilot: &PilotSummary, limits: &ResourceLimits) -> EvidencePlan {
    plan_evidence(&Guides::default(), &Alternatives::default(), pilot, limits)
        .expect("valid evidence plan")
}

fn close(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() < tolerance,
        "{actual} != {expected} within {tolerance}"
    );
}

#[test]
fn device_hours_match_spec_table() {
    for (trials, hours) in [(299, 99.67), (29_808, 9_936.00), (298_073, 99_357.67)] {
        close(device_hours(trials, 1_200.0), hours, 0.005);
    }
}

#[test]
fn fleet_success_matches_spec() {
    close(fleet_success(0.01, 100), 0.6340, 0.00005);
    close(fleet_success(0.01, 1_000), 0.999_957, 1e-6);
    // ln_1p and exp_m1 must retain probabilities below machine epsilon.
    close(fleet_success(1e-20, 100), 1e-18, 1e-32);
    close(fleet_success(0.0, 100), 0.0, f64::EPSILON);
    close(fleet_success(1.0, 100), 1.0, f64::EPSILON);
    close(fleet_success(1.0, 0), 0.0, f64::EPSILON);
}

#[test]
fn power_size_is_minimal() {
    let plan = default_plan(&pilot(), &limits());
    let n = plan.classical_trials;
    assert!(n >= 299);
    assert!(gate_power(Gate::Classical, n, 0.001, &Guides::default()).expect("power") >= 0.80);
    assert!(gate_power(Gate::Classical, n - 1, 0.001, &Guides::default()).expect("power") < 0.80);
    // Zero successes pass at n=299, but that outcome has only about 74% power.
    assert!(gate_power(Gate::Classical, 299, 0.001, &Guides::default()).expect("power") < 0.80);
}

#[test]
fn default_evidence_sizes_match_exact_reference_values() {
    let plan = default_plan(&pilot(), &limits());
    assert_eq!(plan.classical_trials, 473);
    assert_eq!(plan.qpu_trials, 124);
    assert_eq!(plan.race_trials, 179);
}

#[test]
fn qpu_power_size_is_minimal() {
    let n = default_plan(&pilot(), &limits()).qpu_trials;
    assert!(gate_power(Gate::Qpu, n, 0.99, &Guides::default()).expect("power") >= 0.80);
    assert!(gate_power(Gate::Qpu, n - 1, 0.99, &Guides::default()).expect("power") < 0.80);
}

#[test]
fn race_power_size_is_minimal() {
    let n = default_plan(&pilot(), &limits()).race_trials;
    assert!(gate_power(Gate::Race, n, 0.95, &Guides::default()).expect("power") >= 0.80);
    assert!(gate_power(Gate::Race, n - 1, 0.95, &Guides::default()).expect("power") < 0.80);
}

#[test]
fn design_effect_inflates_trials() {
    let independent = default_plan(&pilot(), &limits());
    let clustered = default_plan(
        &PilotSummary {
            trials_per_cluster: 3.0,
            intracluster_correlation: 0.1,
            ..pilot()
        },
        &limits(),
    );
    close(clustered.design_effect, 1.2, 1e-15);
    for (base, inflated) in [
        (independent.classical_trials, clustered.classical_trials),
        (independent.qpu_trials, clustered.qpu_trials),
        (independent.race_trials, clustered.race_trials),
    ] {
        // The integer form independently computes ceil(base * 1.2).
        assert_eq!(inflated, (base * 6).div_ceil(5));
    }
}

#[test]
fn design_effect_roundoff_does_not_add_a_trial() {
    let guides = Guides {
        classical_wall_upper: 0.5,
        qpu_device_lower: 0.5,
        race_lower: 0.5,
        alpha: 0.05,
    };
    let alternatives = Alternatives {
        classical_p: 0.0,
        qpu_p: 1.0,
        race_p: 1.0,
        power: 0.8,
    };
    let plan = plan_evidence(
        &guides,
        &alternatives,
        &PilotSummary {
            trials_per_cluster: 3.000_000_000_000_000_4,
            intracluster_correlation: 0.1,
            ..pilot()
        },
        &limits(),
    )
    .expect("rounded cluster inflation");
    // Each independent size is five. Floating multiplication exceeds six by one ulp.
    assert_eq!(plan.classical_trials, 6);
    assert_eq!(plan.qpu_trials, 6);
    assert_eq!(plan.race_trials, 6);
}

#[test]
fn small_budget_is_insufficient_evidence() {
    let plan = default_plan(
        &pilot(),
        &ResourceLimits {
            qpu_access_s: 1.0,
            ..limits()
        },
    );
    assert_eq!(
        plan.outcome,
        EvidenceOutcome::InsufficientBudget {
            short: vec!["qpu_access_s".to_owned()]
        }
    );
}

#[test]
fn budget_shortfalls_keep_the_required_order() {
    let plan = default_plan(
        &pilot(),
        &ResourceLimits {
            cpu_hours: 0.0,
            gpu_hours: 0.0,
            qpu_access_s: 0.0,
            elapsed_hours: 0.0,
        },
    );
    assert_eq!(
        plan.outcome,
        EvidenceOutcome::InsufficientBudget {
            short: ["cpu_hours", "gpu_hours", "qpu_access_s", "elapsed_hours"]
                .map(str::to_owned)
                .to_vec()
        }
    );
}

#[test]
fn occupancy_access_and_serial_elapsed_use_their_own_time_bases() {
    let plan = default_plan(&pilot(), &limits());
    let classical = f64::from(u32::try_from(plan.classical_trials).expect("small sample"));
    let qpu_and_race =
        f64::from(u32::try_from(plan.qpu_trials + plan.race_trials).expect("small samples"));
    close(plan.device_hours, classical / 3.0, 1e-12);
    close(plan.qpu_access_s, qpu_and_race * 30.0, 1e-12);
    assert_eq!(plan.outcome, EvidenceOutcome::Affordable);

    let exact_limits = ResourceLimits {
        cpu_hours: plan.device_hours,
        gpu_hours: plan.device_hours,
        qpu_access_s: plan.qpu_access_s,
        elapsed_hours: plan.device_hours + qpu_and_race * 1_200.0 / 3_600.0,
    };
    assert_eq!(
        default_plan(&pilot(), &exact_limits).outcome,
        EvidenceOutcome::Affordable
    );
    let serial_limited = default_plan(
        &pilot(),
        &ResourceLimits {
            elapsed_hours: exact_limits.elapsed_hours - 0.1,
            ..exact_limits
        },
    );
    assert_eq!(
        serial_limited.outcome,
        EvidenceOutcome::InsufficientBudget {
            short: vec!["elapsed_hours".to_owned()]
        }
    );
}

// A small direct sum is an independent oracle for the optimized log-space tail.
fn enumerated_power(gate: Gate, n: u32, p: f64, guides: &Guides) -> f64 {
    let mut mass = (1.0 - p).powi(i32::try_from(n).expect("small n"));
    let mut power = 0.0;
    for k in 0..=n {
        let passes = match gate {
            Gate::Classical => {
                binomial_upper(u64::from(k), u64::from(n), guides.alpha).expect("upper bound")
                    <= guides.classical_wall_upper
            }
            Gate::Qpu => {
                binomial_lower(u64::from(k), u64::from(n), guides.alpha).expect("lower bound")
                    >= guides.qpu_device_lower
            }
            Gate::Race => {
                binomial_lower(u64::from(k), u64::from(n), guides.alpha).expect("lower bound")
                    >= guides.race_lower
            }
        };
        if passes {
            power += mass;
        }
        mass *= f64::from(n - k) / f64::from(k + 1) * p / (1.0 - p);
    }
    power
}

#[test]
fn gate_power_matches_enumeration_of_exact_passing_outcomes() {
    let guides = Guides {
        classical_wall_upper: 0.4,
        qpu_device_lower: 0.6,
        race_lower: 0.7,
        alpha: 0.1,
    };
    for gate in [Gate::Classical, Gate::Qpu, Gate::Race] {
        for n in [1, 4, 12, 30] {
            for p in [0.1, 0.4, 0.7, 0.95] {
                close(
                    gate_power(gate, u64::from(n), p, &guides).expect("exact power"),
                    enumerated_power(gate, n, p, &guides),
                    1e-12,
                );
            }
        }
    }
}

#[test]
fn log_space_power_handles_large_central_and_underflowing_tails() {
    let guides = Guides {
        classical_wall_upper: 0.5,
        qpu_device_lower: 0.5,
        race_lower: 0.5,
        alpha: 0.5,
    };
    // P(Binomial(n, .5) < n/2) = (1 - C(n, n/2) / 2^n) / 2.
    // At n=2000 the endpoint mass underflows, but the central tail stays near .5.
    for gate in [Gate::Classical, Gate::Qpu, Gate::Race] {
        for (n, expected) in [
            (1_000, 0.487_387_490_910_819_6),
            (2_000, 0.491_080_494_427_072_86),
        ] {
            close(
                gate_power(gate, n, 0.5, &guides).expect("central tail"),
                expected,
                1e-11,
            );
        }
    }
    // Extreme alternatives also keep power inside [0, 1].
    close(
        gate_power(Gate::Classical, 2_000, 0.5, &Guides::default())
            .expect("underflowing lower tail"),
        0.0,
        1e-15,
    );
    close(
        gate_power(Gate::Qpu, 2_000, 0.999, &Guides::default())
            .expect("underflowing endpoint with high power"),
        1.0,
        1e-12,
    );
}

#[test]
fn gate_equality_and_probability_endpoints_are_inclusive() {
    let guides = Guides {
        classical_wall_upper: binomial_upper(1, 6, 0.05).expect("gate equality"),
        ..Guides::default()
    };
    close(
        gate_power(Gate::Classical, 6, 0.2, &guides).expect("inclusive guide"),
        0.655_36,
        1e-12,
    );
    for (gate, good_p, bad_p, first_n) in [
        (Gate::Classical, 0.0, 1.0, 299),
        (Gate::Qpu, 1.0, 0.0, 59),
        (Gate::Race, 1.0, 0.0, 29),
    ] {
        close(
            gate_power(gate, first_n, good_p, &Guides::default()).expect("certain pass"),
            1.0,
            f64::EPSILON,
        );
        close(
            gate_power(gate, first_n, bad_p, &Guides::default()).expect("certain failure"),
            0.0,
            f64::EPSILON,
        );
        close(
            gate_power(gate, first_n - 1, good_p, &Guides::default()).expect("no passing outcome"),
            0.0,
            f64::EPSILON,
        );
    }
}

#[test]
fn invalid_cluster_inputs_are_rejected() {
    for invalid in [
        PilotSummary {
            trials_per_cluster: 0.5,
            ..pilot()
        },
        PilotSummary {
            trials_per_cluster: f64::INFINITY,
            ..pilot()
        },
        PilotSummary {
            intracluster_correlation: -0.1,
            ..pilot()
        },
        PilotSummary {
            intracluster_correlation: 1.1,
            ..pilot()
        },
        PilotSummary {
            intracluster_correlation: f64::NAN,
            ..pilot()
        },
        PilotSummary {
            trials_per_cluster: 1e30,
            intracluster_correlation: 1.0,
            ..pilot()
        },
    ] {
        assert_eq!(
            plan_evidence(
                &Guides::default(),
                &Alternatives::default(),
                &invalid,
                &limits()
            ),
            Err(AnalysisError::InvalidProbability)
        );
    }
}

#[test]
fn invalid_probabilities_timings_and_limits_are_rejected() {
    for p in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
        assert_eq!(
            gate_power(Gate::Classical, 300, p, &Guides::default()),
            Err(AnalysisError::InvalidProbability)
        );
        assert_eq!(
            plan_evidence(
                &Guides::default(),
                &Alternatives {
                    qpu_p: p,
                    ..Alternatives::default()
                },
                &pilot(),
                &limits()
            ),
            Err(AnalysisError::InvalidProbability)
        );
    }
    assert_eq!(
        gate_power(Gate::Classical, 0, 0.001, &Guides::default()),
        Err(AnalysisError::InvalidProbability)
    );
    for value in [0.0, 1.0, f64::NAN] {
        assert_eq!(
            plan_evidence(
                &Guides {
                    alpha: value,
                    ..Guides::default()
                },
                &Alternatives::default(),
                &pilot(),
                &limits()
            ),
            Err(AnalysisError::InvalidProbability)
        );
    }
    assert_eq!(
        plan_evidence(
            &Guides::default(),
            &Alternatives::default(),
            &PilotSummary {
                mean_trial_device_s: f64::NAN,
                ..pilot()
            },
            &limits()
        ),
        Err(AnalysisError::InvalidProbability)
    );
    assert_eq!(
        plan_evidence(
            &Guides::default(),
            &Alternatives::default(),
            &pilot(),
            &ResourceLimits {
                cpu_hours: -1.0,
                ..limits()
            }
        ),
        Err(AnalysisError::InvalidProbability)
    );
}

#[test]
fn sample_search_rejects_a_guide_beyond_the_scan_cap() {
    let error = plan_evidence(
        &Guides {
            classical_wall_upper: 1e-10,
            ..Guides::default()
        },
        &Alternatives {
            classical_p: 0.0,
            ..Alternatives::default()
        },
        &pilot(),
        &limits(),
    )
    .expect_err("scan exhausted");
    assert_eq!(error, AnalysisError::NoSizeReachesPower);
    assert_eq!(
        error.to_string(),
        "no sample size within the scan limit reaches the requested power"
    );
}
