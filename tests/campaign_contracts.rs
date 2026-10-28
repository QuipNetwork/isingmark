// SPDX-License-Identifier: AGPL-3.0-or-later
//! Explicit campaign contracts and the fixed Ising control.

use std::path::{Path, PathBuf};

use isingmark::campaign::{
    contracts::{
        EncodingOutcome, EvaluationContext, FamilyConfig, FamilyEvaluator, IsingEncoding, Witness,
    },
    families::fixed::FixedIsing,
    instance::{load_instance, validate_source_manifest, Instance, Split},
    CampaignError,
};
use serde_json::{json, Value};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("research/qblock_eval/fixtures")
        .join(name)
}

fn two_spin() -> Instance {
    load_instance(&fixture("two_spin.instance.json")).expect("load two-spin control")
}

fn witness(spins: Vec<i8>) -> Witness {
    Witness {
        instance_id: "two-spin".to_owned(),
        spins,
        design: None,
        route: None,
        controls: None,
    }
}

fn context() -> EvaluationContext {
    EvaluationContext {
        tolerances: json!({}),
    }
}

fn load_value(value: &Value) -> Result<Instance, CampaignError> {
    let directory = tempfile::tempdir().expect("create instance directory");
    let path = directory.path().join("instance.json");
    std::fs::write(
        &path,
        serde_json::to_vec(value).expect("serialize instance"),
    )
    .expect("write instance");
    load_instance(&path)
}

#[test]
fn two_spin_instance_loads() {
    let instance = two_spin();
    assert_eq!(instance.family, "ising-fixed");
    assert_eq!(instance.split, Split::Training);
    let encoding: IsingEncoding =
        serde_json::from_value(instance.physical).expect("decode physical encoding");
    assert_eq!(encoding.variables, ["s0", "s1"]);
    assert_eq!(encoding.h, [0, 0]);
    assert_eq!(encoding.j, [-1000]);
    assert_eq!(encoding.edges, [(0, 1)]);
}

#[test]
fn fixed_family_scores_strictly_below() {
    let instance = two_spin();
    let equal = FixedIsing
        .evaluate(&instance, &witness(vec![1, -1]), &context())
        .expect("evaluate threshold equality");
    assert!(!equal.valid);
    assert_eq!(equal.score, Some(1000.0));
    assert_eq!(equal.units, "milli");
    assert!(!equal.rejections.is_empty());

    let below = FixedIsing
        .evaluate(&instance, &witness(vec![1, 1]), &context())
        .expect("evaluate below threshold");
    assert!(below.valid);
    assert_eq!(below.score, Some(-1000.0));
    assert!(below.rejections.is_empty());
    assert!(below.canonical_id.is_some());
}

#[test]
fn overflow_risk_instance_is_rejected() {
    assert!(matches!(
        load_instance(&fixture("overflow_risk.instance.json")),
        Err(CampaignError::Overflow)
    ));
}

#[test]
fn unsupported_version_is_rejected() {
    assert!(matches!(
        load_instance(&fixture("bad_version.instance.json")),
        Err(CampaignError::UnsupportedVersion(2))
    ));
}

#[test]
fn inconsistent_lengths_are_rejected() {
    assert!(matches!(
        load_instance(&fixture("bad_lengths.instance.json")),
        Err(CampaignError::LengthMismatch)
    ));
}

#[test]
fn missing_required_field_is_rejected() {
    assert!(matches!(
        load_instance(&fixture("missing_field.instance.json")),
        Err(CampaignError::MissingField("nonce"))
    ));
}

#[test]
fn oversized_witness_is_rejected() {
    assert!(matches!(
        witness(vec![1; 1024 * 1024 + 1]).validate(),
        Err(CampaignError::TooLarge {
            what: "witness",
            limit: 1_048_576,
        })
    ));
}

#[test]
fn source_manifest_requires_hashes() {
    assert!(validate_source_manifest(&fixture("manifest_unhashed.json")).is_err());
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("research/qblock_eval/sources/manifest.json");
    let manifest = validate_source_manifest(&path).expect("accept Task 1 manifest unchanged");
    assert_eq!(manifest.sources.len(), 4);
    assert!(manifest.sources.iter().all(|source| !source.reproduced));
}

#[test]
fn fixed_encoding_is_exact_and_generation_keeps_the_explicit_instance() {
    let instance = two_spin();
    let encoded = FixedIsing.encode(&instance).expect("encode fixed control");
    assert!(matches!(encoded, EncodingOutcome::Exact(encoding)
        if encoding.j == [-1000] && encoding.exact));
    let config = FamilyConfig {
        family: "ising-fixed".to_owned(),
        params: serde_json::to_value(&instance).expect("serialize fixed configuration"),
    };
    let generated = FixedIsing.generate(&config, 42).expect("load fixed config");
    assert_eq!(
        serde_json::to_value(generated).expect("serialize generated instance"),
        serde_json::to_value(instance).expect("serialize original instance")
    );
}

#[test]
fn every_instance_field_is_required_and_version_precedes_content_checks() {
    let instance = serde_json::to_value(two_spin()).expect("serialize instance");
    for field in [
        "schema_version",
        "instance_id",
        "family",
        "model_version",
        "challenge_id",
        "nonce",
        "split",
        "physical",
        "search_space",
        "objective",
        "constraints",
        "hardness",
        "source_hashes",
    ] {
        let mut missing = instance.clone();
        missing
            .as_object_mut()
            .expect("instance object")
            .remove(field);
        let error = load_value(&missing).expect_err("reject each absent field");
        assert!(matches!(error, CampaignError::MissingField(name) if name == field));
    }
    let mut unsupported = json!({"schema_version": 2});
    unsupported["physical"] = json!({"h": [3_000_000_000_i64]});
    assert!(matches!(
        load_value(&unsupported),
        Err(CampaignError::UnsupportedVersion(2))
    ));
}

#[test]
fn oversized_instance_is_rejected_before_json_parsing() {
    let directory = tempfile::tempdir().expect("create instance directory");
    let path = directory.path().join("oversized.json");
    let file = std::fs::File::create(&path).expect("create sparse instance file");
    file.set_len(16 * 1024 * 1024 + 1).expect("set file size");
    assert!(matches!(
        load_instance(&path),
        Err(CampaignError::TooLarge {
            what: "instance",
            limit: 16_777_216,
        })
    ));
}

#[test]
fn spin_limit_and_encoding_indices_are_checked() {
    let mut instance = serde_json::to_value(two_spin()).expect("serialize instance");
    instance["physical"]["variables"] = json!(vec!["s"; 65_537]);
    instance["physical"]["h"] = json!(vec![0; 65_537]);
    assert!(matches!(
        load_value(&instance),
        Err(CampaignError::TooLarge {
            what: "spins",
            limit: 65_536
        })
    ));
    let mut instance = serde_json::to_value(two_spin()).expect("serialize instance");
    instance["physical"]["edges"] = json!([[0, 2]]);
    assert!(load_value(&instance).is_err());
    instance["physical"]["edges"] = json!([[0, 1]]);
    instance["physical"]["h"] = json!([0]);
    assert!(matches!(
        load_value(&instance),
        Err(CampaignError::LengthMismatch)
    ));
}

#[test]
fn wire_boundaries_offset_and_integer_threshold_are_preserved() {
    let mut instance = serde_json::to_value(two_spin()).expect("serialize instance");
    instance["physical"]["h"] = json!([i32::MIN, i32::MAX]);
    instance["physical"]["j"] = json!([0]);
    let instance = load_value(&instance).expect("accept signed wire endpoints");
    let result = FixedIsing
        .evaluate(&instance, &witness(vec![1, 1]), &context())
        .expect("compute endpoint energy without i32 overflow");
    assert_eq!(result.score, Some(-1.0));

    let mut instance = serde_json::to_value(two_spin()).expect("serialize instance");
    instance["physical"]["h"] = json!([0, 0]);
    instance["physical"]["j"] = json!([0]);
    instance["physical"]["offset"] = json!(9_007_199_254_740_992_i64);
    instance["objective"]["max_energy_milli"] = json!(9_007_199_254_740_993_i64);
    let instance = load_value(&instance).expect("load large integer objective");
    assert!(
        FixedIsing
            .evaluate(&instance, &witness(vec![1, 1]), &context())
            .expect("compare integers before score conversion")
            .valid
    );
}

#[test]
fn fixed_family_rejects_wrong_identity_spin_length_and_spin_values() {
    let instance = two_spin();
    let mut wrong_identity = witness(vec![1, 1]);
    wrong_identity.instance_id = "another-instance".to_owned();
    for invalid in [wrong_identity, witness(vec![1]), witness(vec![1, 0])] {
        assert!(FixedIsing
            .evaluate(&instance, &invalid, &context())
            .is_err());
        assert!(FixedIsing.canonicalize(&instance, &invalid).is_err());
    }
}

#[test]
fn canonicalization_keeps_spin_orientation_and_ignores_unused_fields() {
    let instance = two_spin();
    let original = FixedIsing
        .canonicalize(&instance, &witness(vec![1, -1]))
        .expect("canonicalize witness");
    let reversed = FixedIsing
        .canonicalize(&instance, &witness(vec![-1, 1]))
        .expect("canonicalize reversal");
    assert_ne!(original, reversed);
    let mut decorated = witness(vec![1, -1]);
    decorated.design = Some(json!({"unused": 1}));
    assert_eq!(
        original,
        FixedIsing
            .canonicalize(&instance, &decorated)
            .expect("ignore unused fixed-family fields")
    );
}

#[test]
fn witness_size_includes_serialized_optional_fields() {
    let mut oversized = witness(vec![1, 1]);
    oversized.controls = Some(json!("x".repeat(1024 * 1024)));
    assert!(matches!(
        oversized.validate(),
        Err(CampaignError::TooLarge {
            what: "witness",
            ..
        })
    ));
    witness(vec![1, 1])
        .validate()
        .expect("accept small witness");
}

#[test]
fn other_family_physical_metadata_stays_opaque() {
    let mut instance = serde_json::to_value(two_spin()).expect("serialize instance");
    instance["family"] = json!("future-physical-family");
    instance["physical"] = json!({"geometry": [1, 2, 3]});
    let instance = load_value(&instance).expect("leave physical metadata to its family");
    assert!(FixedIsing.encode(&instance).is_err());
}
