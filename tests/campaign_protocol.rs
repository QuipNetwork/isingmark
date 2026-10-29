// SPDX-License-Identifier: AGPL-3.0-or-later
//! Proof replay against the pinned verifier and the distinct-witness proposal.

use std::path::{Path, PathBuf};

use isingmark::campaign::{
    contracts::Witness,
    instance::Instance,
    protocol::{
        load_protocol_profile, select_proof, validate_set, ProofDecision, ProtocolProfile, RuleSet,
    },
};
use quip_protocol::{
    diversity::diversity_milli,
    target::{meets_target, Target},
};
use serde_json::json;

fn fixed_instance(width: usize) -> Instance {
    serde_json::from_value(json!({
        "schema_version": 1,
        "instance_id": format!("fixed-{width}"),
        "family": "ising-fixed",
        "model_version": "1",
        "challenge_id": "proof-replay",
        "nonce": "0000000000000000",
        "split": "training",
        "physical": {
            "variables": (0..width).map(|index| format!("s{index}")).collect::<Vec<_>>(),
            "h": vec![0; width],
            "j": [-1000],
            "edges": [[0, 1]],
            "units": "milli",
            "offset": 0,
            "auxiliaries": [],
            "precision_bound": 0.0,
            "exact": true
        },
        "search_space": {},
        "objective": {"max_energy_milli": 1000},
        "constraints": {},
        "hardness": {},
        "source_hashes": {}
    }))
    .expect("build explicit fixed instance")
}

fn witness(instance: &Instance, spins: &[i8]) -> Witness {
    Witness {
        instance_id: instance.instance_id.clone(),
        spins: spins.to_vec(),
        design: None,
        route: None,
        controls: None,
    }
}

fn profile_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("evaluation/qblock/configs/protocol.json")
}

fn profile() -> ProtocolProfile {
    load_protocol_profile(&profile_path()).expect("load pinned Task 1 profile")
}

fn rejected(reason: &str) -> ProofDecision {
    ProofDecision::Rejected {
        reason: reason.to_owned(),
    }
}

#[test]
fn threshold_equality_fails() {
    let instance = fixed_instance(2);
    assert_eq!(
        validate_set(&instance, &[witness(&instance, &[1, -1])], &profile())
            .expect("replay equality"),
        rejected("InsufficientEnergy")
    );
}

#[test]
fn duplicate_reads_count_under_current_rule() {
    let mut instance = fixed_instance(2);
    instance.objective["min_solutions"] = json!(3);
    instance.objective["min_diversity_milli"] = json!(0);
    let reads = vec![witness(&instance, &[1, 1]); 3];
    assert_eq!(
        RuleSet::Current(profile())
            .validate_set(&instance, &reads)
            .expect("count identical valid reads"),
        ProofDecision::Accepted {
            best_energy_milli: -1000,
            diversity_milli: 0,
            valid_solution_count: 3
        }
    );
}

#[test]
fn global_reversal_adds_no_diversity() {
    let mut instance = fixed_instance(2);
    instance.objective["min_solutions"] = json!(2);
    let reads = [witness(&instance, &[1, 1]), witness(&instance, &[-1, -1])];
    assert_eq!(
        validate_set(&instance, &reads, &profile()).expect("score a reversal pair"),
        ProofDecision::Accepted {
            best_energy_milli: -1000,
            diversity_milli: 0,
            valid_solution_count: 2
        }
    );
}

#[test]
fn proof_cap_rejects_extra() {
    let mut instance = fixed_instance(2);
    instance.objective["max_proof_solutions"] = json!(2);
    let reads = vec![witness(&instance, &[1, 1]); 3];
    assert_eq!(
        validate_set(&instance, &reads, &profile()).expect("check proof cap"),
        rejected("TooManySolutions")
    );
}

#[test]
fn greedy_subset_matches_pinned() {
    let spins = [
        [1; 8],
        [1, 1, 1, 1, 1, 1, 1, -1],
        [1, 1, 1, 1, 1, 1, -1, -1],
        [1, 1, 1, 1, 1, -1, -1, -1],
        [-1; 8],
        [1, 1, 1, -1, -1, 1, 1, 1],
    ];
    let reads = [
        (spins[0].as_slice(), -200),
        (spins[1].as_slice(), -300),
        (spins[2].as_slice(), -300),
        (spins[3].as_slice(), -250),
        (spins[4].as_slice(), -100),
        (spins[5].as_slice(), -300),
    ];
    let target = Target {
        max_energy_milli: -100,
        min_solutions: 2,
        min_diversity_milli: 125,
        max_proof_solutions: 3,
    };
    let pinned = meets_target(&reads, &target).expect("pinned six-read tie case");
    assert_eq!(pinned.indices, [1, 2, 5]);
    assert_eq!(
        select_proof(&reads, &target).expect("select pinned proof"),
        pinned.indices
    );
}

#[test]
fn milli_rounding_matches_pinned() {
    let mut instance = fixed_instance(8);
    instance.objective["min_solutions"] = json!(3);
    let spins: [&[i8]; 3] = [
        &[1; 8],
        &[1, 1, 1, 1, 1, 1, 1, -1],
        &[1, 1, 1, 1, 1, 1, -1, -1],
    ];
    let reads: Vec<_> = spins
        .iter()
        .map(|spins| witness(&instance, spins))
        .collect();
    assert_eq!(diversity_milli(&spins), 167);
    assert_eq!(
        validate_set(&instance, &reads, &profile()).expect("use integer milli rounding"),
        ProofDecision::Accepted {
            best_energy_milli: -1000,
            diversity_milli: diversity_milli(&spins),
            valid_solution_count: 3
        }
    );
}

#[test]
fn cross_instance_witnesses_cannot_combine() {
    let instance = fixed_instance(2);
    let mut foreign = witness(&instance, &[1, 1]);
    foreign.instance_id = "another-instance".to_owned();
    // A local malformed read must not hide a foreign witness later in the set.
    let reads = [witness(&instance, &[0]), foreign];
    assert_eq!(
        validate_set(&instance, &reads, &profile()).expect("check all identities before scoring"),
        rejected("CrossInstanceWitness")
    );
    assert_eq!(
        RuleSet::ProposedUnique { min_distinct: 1 }
            .validate_set(&instance, &reads)
            .expect("check proposed-rule identities"),
        rejected("CrossInstanceWitness")
    );
}

#[test]
fn proposed_unique_rule_rejects_duplicates() {
    let mut instance = fixed_instance(2);
    instance.objective["min_solutions"] = json!(3);
    let reads = vec![witness(&instance, &[1, 1]); 3];
    assert_eq!(
        RuleSet::ProposedUnique { min_distinct: 2 }
            .validate_set(&instance, &reads)
            .expect("count canonical IDs independently"),
        rejected("InsufficientDistinct")
    );
}

#[test]
fn proposed_unique_zero_minimum_still_requires_one_distinct_witness() {
    let instance = fixed_instance(2);
    let rule = RuleSet::ProposedUnique { min_distinct: 0 };
    assert_eq!(
        rule.validate_set(&instance, &[]).expect("empty proof"),
        rejected("InsufficientDistinct")
    );
    assert_eq!(
        rule.validate_set(&instance, &[witness(&instance, &[1, -1])])
            .expect("invalid proof"),
        rejected("InsufficientDistinct")
    );
    assert!(matches!(
        rule.validate_set(&instance, &[witness(&instance, &[1, 1])])
            .expect("valid proof"),
        ProofDecision::Accepted {
            valid_solution_count: 1,
            ..
        }
    ));
}

#[test]
fn profile_must_match_compiled_verifier() {
    let mut value = serde_json::to_value(profile()).expect("serialize profile");
    value["verifier_version"] = json!("9.9.9");
    let directory = tempfile::tempdir().expect("create profile directory");
    let path = directory.path().join("protocol.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&value).expect("serialize mismatch"),
    )
    .expect("write mismatched profile");
    assert!(load_protocol_profile(&path).is_err());
}

#[test]
fn malformed_witnesses_reject_with_the_verifier_reason() {
    let instance = fixed_instance(2);
    for spins in [&[][..], &[1][..], &[1, 0][..], &[1, 2][..]] {
        let reads = [witness(&instance, spins)];
        assert_eq!(
            validate_set(&instance, &reads, &profile()).expect("reject malformed fixed witness"),
            rejected("MalformedSolutions")
        );
    }
}

#[test]
fn missing_energy_threshold_is_an_input_error() {
    let mut instance = fixed_instance(2);
    instance.objective = json!({});
    let error = validate_set(&instance, &[], &profile()).expect_err("require an energy threshold");
    assert_eq!(error.to_string(), "missing required field max_energy_milli");
}

#[test]
fn unsupported_family_is_an_input_error() {
    let mut instance = fixed_instance(2);
    instance.family = "other-family".to_owned();
    let error = validate_set(&instance, &[], &profile()).expect_err("reject unsupported family");
    assert_eq!(
        error.to_string(),
        "invalid run: unsupported family for current-rule replay: other-family"
    );
}

#[test]
fn objective_defaults_have_no_proof_cap() {
    let instance = fixed_instance(2);
    let reads = vec![witness(&instance, &[1, 1]); 65];
    assert_eq!(
        validate_set(&instance, &reads, &profile()).expect("use uncapped default target"),
        ProofDecision::Accepted {
            best_energy_milli: -1000,
            diversity_milli: 0,
            valid_solution_count: 65
        }
    );
}

#[test]
fn pinned_count_and_diversity_failures_keep_their_variant_names() {
    let mut instance = fixed_instance(2);
    instance.objective["min_solutions"] = json!(2);
    let reads = [witness(&instance, &[1, 1]), witness(&instance, &[1, -1])];
    assert_eq!(
        validate_set(&instance, &reads, &profile())
            .expect("filter threshold equality before count"),
        rejected("InsufficientSolutions")
    );
    instance.objective["min_diversity_milli"] = json!(1);
    let reads = [witness(&instance, &[1, 1]), witness(&instance, &[-1, -1])];
    assert_eq!(
        validate_set(&instance, &reads, &profile()).expect("check nonzero diversity gate"),
        rejected("InsufficientDiversity")
    );
}

#[test]
fn proposed_unique_ignores_current_counts_caps_and_diversity() {
    let mut instance = fixed_instance(2);
    instance.objective["min_solutions"] = json!(99);
    instance.objective["min_diversity_milli"] = json!(1001);
    instance.objective["max_proof_solutions"] = json!(1);
    let reads = [
        witness(&instance, &[1, 1]),
        witness(&instance, &[1, 1]),
        witness(&instance, &[-1, -1]),
        witness(&instance, &[1, -1]),
    ];
    assert_eq!(
        RuleSet::ProposedUnique { min_distinct: 2 }
            .validate_set(&instance, &reads)
            .expect("count valid canonical IDs without current target gates"),
        ProofDecision::Accepted {
            best_energy_milli: -1000,
            diversity_milli: 0,
            valid_solution_count: 2
        }
    );
}

#[test]
fn replay_preserves_integer_energy_beyond_float_precision() {
    for energy in [9_007_199_254_740_993_i64, -9_007_199_254_740_993_i64] {
        let mut instance = fixed_instance(2);
        instance.physical["j"] = json!([0]);
        instance.physical["offset"] = json!(energy);
        instance.objective["max_energy_milli"] = json!(energy);
        let reads = [witness(&instance, &[1, 1])];
        assert_eq!(
            validate_set(&instance, &reads, &profile()).expect("preserve exact threshold equality"),
            rejected("InsufficientEnergy")
        );
        instance.objective["max_energy_milli"] = json!(energy + 1);
        assert_eq!(
            validate_set(&instance, &reads, &profile())
                .expect("preserve exact energy below target"),
            ProofDecision::Accepted {
                best_energy_milli: energy,
                diversity_milli: 0,
                valid_solution_count: 1
            }
        );
    }
}

#[test]
fn pinned_size_check_precedes_malformed_solutions() {
    let mut instance = fixed_instance(2);
    instance.objective["max_proof_solutions"] = json!(1);
    let reads = [witness(&instance, &[1, 0]), witness(&instance, &[1, 1])];
    assert_eq!(
        validate_set(&instance, &reads, &profile()).expect("preserve pinned gate order"),
        rejected("TooManySolutions")
    );
}
