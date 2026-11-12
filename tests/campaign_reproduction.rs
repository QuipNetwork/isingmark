// SPDX-License-Identifier: AGPL-3.0-or-later
//! `isingmark campaign reproduce-report`: the RP-0 comparison on real fake-miner campaigns.

use std::{
    fs,
    path::{Path, PathBuf},
};

use isingmark::{
    backend::BackendSpec,
    campaign::{
        families::FamilyRegistry,
        reproduction::{compare, markdown},
        runner::{run_campaign_with_backend, CampaignConfig},
    },
    params::BackendKind,
};
use serde_json::{json, Value};

const FROZEN_DIGEST: &str = "17d59fe92cea8070c68037bbd2ecb88b54f065981e23a177c940949f5317fa2c";

fn nonce(byte: u8) -> String {
    format!("{byte:02x}").repeat(32)
}

fn write(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write");
}

fn digest(path: &Path) -> String {
    blake3::hash(&fs::read(path).expect("read"))
        .to_hex()
        .to_string()
}

fn fake_miner() -> BackendSpec {
    BackendSpec {
        name: "fake".into(),
        bin: PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner")),
        args: Vec::new(),
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    }
}

/// A two-spin instance whose fake-miner state (all +1) has energy -1000 milli.
fn instance(dir: &Path, name: &str, nonce: &str, max_energy: f64) -> String {
    let file = format!("{name}.instance.json");
    write(
        &dir.join(&file),
        &json!({
            "schema_version": 2,
            "instance_id": name,
            "family": "ising-fixed",
            "model_version": "1",
            "challenge_id": "reproduction-test",
            "nonce": nonce,
            "split": "validation",
            "physical": {
                "variables": ["s0", "s1"], "h": [0, 0], "j": [-1], "edges": [[0, 1]],
                "offset": 0, "auxiliaries": [], "precision_bound": 0.0, "exact": true
            },
            "search_space": {},
            "objective": {"max_energy": max_energy},
            "constraints": {},
            "hardness": {},
            "source_hashes": {}
        }),
    );
    file
}

fn policies() -> Value {
    let policy = |id: &str| {
        json!({
            "policy_id": id, "backend": "fake", "num_reads": 1, "num_sweeps": 10,
            "anneal_time_us": 0, "host_compute_limit_s": 60.0, "evaluation_limit": 1,
            "preparation": "cold"
        })
    };
    json!({
        "schema_version": 2,
        "acceptance": {"profile": "family_single"},
        "policies": [policy("p-cpu"), policy("p-cuda")],
        "trials_per_pair": 1,
        "budget": {"wall_limit_s": 60.0}
    })
}

async fn campaign(dir: &Path, name: &str, members: &[(&str, String, f64)]) -> PathBuf {
    let inputs = dir.join(format!("{name}-inputs"));
    fs::create_dir_all(&inputs).expect("inputs");
    let mut manifest = policies();
    manifest["instances"] = members
        .iter()
        .map(|(id, nonce, max)| json!(instance(&inputs, id, nonce, *max)))
        .collect();
    write(&inputs.join("manifest.json"), &manifest);
    let output = dir.join(name);
    let config = CampaignConfig {
        manifest: inputs.join("manifest.json"),
        backend: "fake".into(),
        output: output.clone(),
        dry_run: false,
        registry: FamilyRegistry::production(),
    };
    run_campaign_with_backend(&config, fake_miner())
        .await
        .expect("campaign");
    output
}

struct Setup {
    _dir: tempfile::TempDir,
    frozen: PathBuf,
    control: PathBuf,
    fresh: PathBuf,
}

/// Two archived models: the fake state ties `qpu80_pp` on the first and beats it on the second.
async fn setup(fresh_count: usize) -> Setup {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    let index = root.join("archive-index.json");
    write(
        &index,
        &json!({"cells": [{"cell": "ice-charge", "rows": [
            {"nonce": nonce(1), "qpu80_pp": "-1000"},
            {"nonce": nonce(2), "qpu80_pp": "-500"}
        ]}]}),
    );
    write(&root.join("policies.json"), &policies());
    let frozen = root.join("frozen.json");
    write(
        &frozen,
        &json!({
            "cell": "ice-charge",
            "pass_rule": {"gap_sd_multiple": 2.0, "time_factor": 2.0},
            "archived": {
                "tier": "3",
                "gap": {"mean_units": 0.25, "sd_units": 0.1},
                "time_to_target_s": {"cpu": 1.9},
                "best_device": "cpu",
                "qpu_post_s": 0.25
            },
            "policies": {
                "file": "policies.json",
                "blake3": digest(&root.join("policies.json")),
                "cpu": {"policy_id": "p-cpu"},
                "cuda": {"policy_id": "p-cuda"}
            },
            "cohort": {"count": fresh_count, "verdict_until_rp6": "incomplete: waiting on RP-6"},
            "control": {"index": index, "index_blake3": digest(&index), "rows": 2}
        }),
    );
    let control = campaign(
        root,
        "control",
        &[("a1", nonce(1), -0.999), ("a2", nonce(2), -0.499)],
    )
    .await;
    let fresh = campaign(
        root,
        "fresh",
        &[("f1", nonce(0xa1), -1000.0), ("f2", nonce(0xa2), -1000.0)],
    )
    .await;
    Setup {
        _dir: dir,
        frozen,
        control,
        fresh,
    }
}

#[tokio::test]
async fn the_control_arm_is_scored_against_qpu80_pp() {
    let s = setup(2).await;
    let comparison = compare(
        &s.frozen,
        std::slice::from_ref(&s.control),
        std::slice::from_ref(&s.fresh),
    )
    .expect("compare");
    for device in ["cpu", "cuda"] {
        let arm = &comparison.control[device];
        assert_eq!(arm.summary.trials, 2, "{device}");
        assert_eq!(
            (arm.qpu_wins, arm.ties, arm.classical_wins),
            (0, 1, 1),
            "{device}"
        );
        assert!(arm.reached, "{device}");
        assert!(arm.time_to_target_s.is_some(), "{device}");
        let gap = arm.gap.expect("gap");
        assert!((gap.mean_units - 0.25).abs() < 1e-12, "{gap:?}");
        assert_eq!(gap.models, 2);
    }
    // Two models are below the 30-model floor of Tier rule v3.
    let tier = comparison.tier.as_ref().expect("a label");
    assert_eq!(tier.tier.label(), "Needs measurement");
    let names: Vec<&str> = comparison.checks.iter().map(|c| c.name).collect();
    assert_eq!(names, ["tier", "gap", "time_to_target"]);
    assert_eq!(comparison.checks[0].pass, Some(false));
    assert!(
        comparison
            .control_verdict
            .starts_with("not reproduced: tier"),
        "{}",
        comparison.control_verdict
    );

    for device in ["cpu", "cuda"] {
        let arm = &comparison.fresh[device];
        assert_eq!((arm.trials, arm.with_state), (2, 2), "{device}");
        assert!((arm.energy_units.expect("energies").median + 1.0).abs() < 1e-12);
    }
    assert_eq!(comparison.fresh_verdict, "incomplete: waiting on RP-6");
    let text = markdown(&comparison);
    assert!(
        text.contains("| CPU | `p-cpu` | 2 | 0 / 1 / 1 | 0 |"),
        "{text}"
    );
    assert!(
        text.contains("Fresh verdict: incomplete: waiting on RP-6"),
        "{text}"
    );
}

#[tokio::test]
async fn a_fresh_cohort_of_the_wrong_size_is_refused() {
    let s = setup(30).await;
    let error = compare(
        &s.frozen,
        std::slice::from_ref(&s.control),
        std::slice::from_ref(&s.fresh),
    )
    .expect_err("size");
    assert!(
        error.to_string().contains("frozen cohort has 30"),
        "{error}"
    );
}

#[tokio::test]
async fn an_archived_nonce_in_the_fresh_cohort_is_refused() {
    let s = setup(2).await;
    // The control outputs hold archived nonces, so reading them as fresh must fail.
    let error =
        compare(&s.frozen, &[], std::slice::from_ref(&s.control)).expect_err("archived nonce");
    assert!(
        error.to_string().contains("uses an archived nonce"),
        "{error}"
    );
}

#[tokio::test]
async fn an_edited_policy_file_is_refused() {
    let s = setup(2).await;
    let policies = s.frozen.with_file_name("policies.json");
    fs::write(&policies, b"{}").expect("edit");
    let error = compare(&s.frozen, std::slice::from_ref(&s.control), &[]).expect_err("digest");
    assert!(
        error.to_string().contains("the frozen file pins"),
        "{error}"
    );
}

#[test]
fn the_ice_charge_frozen_file_is_pinned() {
    // A later edit of the frozen comparison must be visible in review.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evaluation/qblock/configs/reproduction/ice-charge.json");
    assert_eq!(digest(&path), FROZEN_DIGEST);
}

/// Rewrite the trials of one campaign output, keeping the traces.
fn edit_trials(dir: &Path, edit: impl FnOnce(&mut Vec<Value>)) {
    let path = dir.join("campaign.json");
    let mut file: Value = serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
    let trials = file["trials"].as_array_mut().expect("trials");
    edit(trials);
    write(&path, &file);
}

#[tokio::test]
async fn a_control_arm_without_trials_is_refused() {
    let s = setup(2).await;
    edit_trials(&s.control, |trials| {
        trials.retain(|t| t["policy_id"] != "p-cuda");
    });
    let error = compare(&s.frozen, std::slice::from_ref(&s.control), &[]).expect_err("arm");
    assert!(
        error.to_string().contains("no control trials for p-cuda"),
        "{error}"
    );
}

#[tokio::test]
async fn a_trial_of_another_policy_is_refused() {
    let s = setup(2).await;
    edit_trials(&s.control, |trials| {
        for trial in trials.iter_mut().filter(|t| t["policy_id"] == "p-cuda") {
            trial["policy_id"] = json!("p-old");
        }
    });
    let error = compare(&s.frozen, std::slice::from_ref(&s.control), &[]).expect_err("policy");
    assert!(
        error
            .to_string()
            .contains("policy p-old is not in the frozen file"),
        "{error}"
    );
}

#[tokio::test]
async fn a_control_arm_that_misses_an_archived_model_is_refused() {
    let s = setup(2).await;
    edit_trials(&s.control, |trials| {
        let first = trials
            .iter()
            .position(|t| t["policy_id"] == "p-cpu")
            .expect("a cpu trial");
        trials.remove(first);
    });
    let error = compare(&s.frozen, std::slice::from_ref(&s.control), &[]).expect_err("coverage");
    assert!(
        error
            .to_string()
            .contains("p-cpu covers 1 of 2 archived models"),
        "{error}"
    );
}

#[tokio::test]
async fn a_control_directory_given_twice_is_refused() {
    let s = setup(2).await;
    let twice = [s.control.clone(), s.control.clone()];
    let error = compare(&s.frozen, &twice, &[]).expect_err("duplicate");
    assert!(error.to_string().contains("more than one trial"), "{error}");
}

#[tokio::test]
async fn a_fresh_cohort_with_a_repeated_model_is_refused() {
    let s = setup(4).await;
    let twice = [s.fresh.clone(), s.fresh.clone()];
    let error = compare(&s.frozen, std::slice::from_ref(&s.control), &twice).expect_err("repeat");
    assert!(error.to_string().contains("more than one trial"), "{error}");
}
