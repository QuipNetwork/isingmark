// SPDX-License-Identifier: AGPL-3.0-or-later
//! `isingmark campaign cohort`: fresh canonical cohorts and their records.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use isingmark::campaign::{
    cohort::{cohort_nonce, recipe, write_cohort, CohortRequest, NONCE_CONTEXT},
    contracts::EncodingOutcome,
    families::FamilyRegistry,
    instance::load_instance,
};
use serde_json::Value;

fn repo(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn hex(bytes: &[u8]) -> String {
    isingmark::modes::hex_encode(bytes)
}

fn request(out: PathBuf, count: u32) -> CohortRequest {
    CohortRequest {
        recipe: "ice-charge".to_owned(),
        count,
        topology: repo("fixtures/suite-live-2026-09-21.spec.json"),
        max_energy: -1_000_000.0,
        policies: repo("evaluation/qblock/configs/reproduction/ice-charge.policies.json"),
        out,
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read")).expect("json")
}

#[test]
fn the_nonce_rule_is_the_documented_key_derivation() {
    let mut material = b"ice-charge\0".to_vec();
    material.extend_from_slice(&7_u32.to_le_bytes());
    assert_eq!(
        cohort_nonce("ice-charge", 7),
        blake3::derive_key(NONCE_CONTEXT, &material)
    );
    assert_ne!(cohort_nonce("ice-charge", 0), cohort_nonce("ice-charge", 1));
}

#[test]
fn fresh_nonces_share_nothing_with_the_archived_cell() {
    let index = read_json(&repo("evaluation/qblock/archive-index.json"));
    let archived: BTreeSet<String> = index["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .filter(|cell| cell["cell"] == "ice-charge")
        .flat_map(|cell| cell["rows"].as_array().expect("rows").iter())
        .map(|row| row["nonce"].as_str().expect("nonce").to_owned())
        .collect();
    assert_eq!(archived.len(), 100);
    for i in 0..1000 {
        assert!(!archived.contains(&hex(&cohort_nonce("ice-charge", i))));
    }
}

#[test]
fn a_cohort_regenerates_and_records_its_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("cohort");
    let record =
        write_cohort(&FamilyRegistry::production(), &request(out.clone(), 2)).expect("cohort");
    let recipe = recipe("ice-charge").expect("recipe");
    assert_eq!(record.family_id, "quip/charged-ice");
    assert_eq!(record.revision, 1);
    assert_eq!(record.params_hex, hex(recipe.params));
    assert_eq!(
        record.params_digest,
        hex(&ising_generators::descriptor::params_digest(recipe.params))
    );
    assert_eq!(record.instances.len(), 2);

    let manifest = read_json(&out.join("manifest.json"));
    let listed: Vec<&str> = manifest["instances"]
        .as_array()
        .expect("instances")
        .iter()
        .map(|v| v.as_str().expect("path"))
        .collect();
    assert_eq!(
        listed,
        record
            .instances
            .iter()
            .map(|m| m.file.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(manifest["policies"].as_array().expect("policies").len(), 2);
    assert_eq!(read_json(&out.join("cohort.json"))["recipe"], "ice-charge");

    let registry = FamilyRegistry::production();
    for (i, member) in (0_u32..).zip(&record.instances) {
        assert_eq!(member.nonce, hex(&cohort_nonce("ice-charge", i)));
        let instance = load_instance(&out.join(&member.file)).expect("instance");
        assert_eq!(
            instance.physical["instance_digest"],
            member.instance_digest.as_str()
        );
        let evaluator = registry.for_instance(&instance).expect("registered family");
        let outcome = evaluator.encode(&instance).expect("encodes");
        let sizes = match &outcome {
            EncodingOutcome::Exact(encoding) => {
                Some((encoding.variables.len(), encoding.edges.len()))
            }
            _ => None,
        };
        assert_eq!(sizes, Some((4575, 41_474)), "charged ice encodes exactly");
    }
}

#[test]
fn an_edited_nonce_fails_the_pinned_digest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("cohort");
    let record =
        write_cohort(&FamilyRegistry::production(), &request(out.clone(), 1)).expect("cohort");
    let mut instance = load_instance(&out.join(&record.instances[0].file)).expect("instance");
    instance.nonce = hex(&cohort_nonce("ice-charge", 99));
    let registry = FamilyRegistry::production();
    let error = registry
        .for_instance(&instance)
        .expect("registered family")
        .encode(&instance)
        .expect_err("a different nonce gives a different instance");
    assert_eq!(error.code, "instance_digest_mismatch");
}

#[test]
fn a_cohort_never_writes_into_an_existing_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let error = write_cohort(
        &FamilyRegistry::production(),
        &request(dir.path().to_path_buf(), 1),
    )
    .expect_err("existing directory");
    assert!(error.to_string().contains("exists"), "{error}");
}
