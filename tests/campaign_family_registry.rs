// SPDX-License-Identifier: AGPL-3.0-or-later
//! Family dispatch: the registry, acceptance profiles, canonical instances, and units.

use std::{fs, io::Cursor, path::Path, path::PathBuf};

use ising_generators::{
    descriptor::{
        Contract, FieldSpec, Params, SizeBounds, TopologyStats, Value as ParamValue, ValueType,
    },
    family::{BaseTopology, Construction, InstanceBody, Streams},
    instance::{Instance as Generated, InstanceLimits},
    rng::uniform,
    Code,
};
use isingmark::{
    backend::BackendSpec,
    campaign::{
        contracts::{
            EncodingOutcome, EvaluationContext, FamilyConfig, FamilyEvaluator, IsingEncoding,
            Witness,
        },
        families::{canonical::CanonicalFamily, check::family_check, FamilyRegistry},
        instance::{load_instance, Instance},
        protocol::{load_protocol_profile, ProofDecision, RuleSet},
        runner::{
            run_campaign_with_backend, serve_campaign, CampaignConfig, PolicyManifest, Preparation,
            TrialRunner,
        },
        timing::TrialBudget,
        CampaignError,
    },
    params::BackendKind,
};
use serde_json::{json, Value};

/// Format 0x01, then tag 1, length 2, and the coupling magnitude 700 milli (0x02bc LE).
const PARAMS_HEX: &str = "01010002000000bc02";

// ---- the test contract: a chain with random coupling signs and a pinned first spin ----

const SCHEMA: &[FieldSpec] = &[FieldSpec {
    tag: 1,
    kind: ValueType::U16,
    min: 1,
    max: 1000,
}];

#[expect(
    clippy::unnecessary_wraps,
    reason = "SizeBoundsFn requires a Result even when no input can overflow."
)]
fn chain_bounds(_params: &Params<'_>, stats: &TopologyStats) -> Result<SizeBounds, Code> {
    Ok(SizeBounds {
        nodes: stats.n_nodes,
        edges: stats.n_edges,
        degree: stats.max_degree,
    })
}

fn any_topology(_params: &Params<'_>, _topology: &BaseTopology) -> bool {
    true
}

fn no_metadata(_params: &Params<'_>, instance: &Generated) -> bool {
    instance.metadata() == [0, 0]
}

fn chain_generate(
    params: &Params<'_>,
    topology: &BaseTopology,
    streams: &mut Streams<'_>,
) -> Result<InstanceBody, Code> {
    let Some(ParamValue::Unsigned(magnitude)) = params.get(1) else {
        return Err(Code::ParamsMalformed);
    };
    let magnitude = i32::try_from(magnitude).map_err(|_| Code::ParamsMalformed)?;
    let mut signs = streams.open("signs").ok_or(Code::NoInstance)?;
    let two = std::num::NonZeroU32::new(2).ok_or(Code::NoInstance)?;
    let mut edges: Vec<(u32, u32)> = topology
        .edges()
        .iter()
        .map(|&(a, b)| (a.min(b), a.max(b)))
        .collect();
    edges.sort_unstable();
    let j = edges
        .iter()
        .map(|_| {
            if uniform(&mut signs, two) == 0 {
                -magnitude
            } else {
                magnitude
            }
        })
        .collect();
    Ok(InstanceBody {
        node_ids: topology.node_ids().to_vec(),
        h: vec![0; topology.node_ids().len()],
        edges,
        j,
        offset_milli: 0,
        metadata: vec![0, 0],
    })
}

static CHAIN: Construction = Construction {
    labels: &["signs"],
    check_topology: any_topology,
    generate: chain_generate,
    check_metadata: no_metadata,
};

static CONTRACT: Contract = Contract {
    family_id: "test/pinned-chain",
    revision: 1,
    schema: SCHEMA,
    metadata_kinds: &[],
    limits: InstanceLimits::FAM_37,
    size_bounds: chain_bounds,
    construction: Some(&CHAIN),
};

fn first_spin_up(_instance: &Generated, spins: &[i8]) -> Result<(), String> {
    if spins.first() == Some(&1) {
        Ok(())
    } else {
        Err("spin 0 must be +1".to_owned())
    }
}

static FAMILIES: [CanonicalFamily; 1] = [CanonicalFamily {
    contract: &CONTRACT,
    check_witness: first_spin_up,
}];

fn registry() -> FamilyRegistry {
    FamilyRegistry::with_canonical(&FAMILIES)
}

// ---- helpers ----

fn qblock(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evaluation/qblock")
        .join(path)
}

fn two_spin() -> Instance {
    load_instance(&qblock("fixtures/two_spin.instance.json")).expect("fixed instance")
}

fn template(nodes: &[u32], edges: &[(u32, u32)]) -> Value {
    json!({
        "schema_version": 2,
        "instance_id": "chain",
        "family": "test/pinned-chain",
        "model_version": "1",
        "challenge_id": "family-dispatch",
        "split": "training",
        "physical": {
            "descriptor": {"family_id": "test/pinned-chain", "revision": 1, "params_hex": PARAMS_HEX},
            "topology": {"nodes": nodes, "edges": edges}
        },
        "search_space": {},
        "objective": {"max_energy": -2},
        "constraints": {},
        "hardness": {},
        "source_hashes": {}
    })
}

fn generate(nonce: [u8; 32]) -> Instance {
    let config = FamilyConfig {
        family: "test/pinned-chain".into(),
        params: template(&[10, 11, 12, 13], &[(10, 11), (11, 12), (12, 13)]),
    };
    FAMILIES[0]
        .generate(&config, nonce)
        .expect("canonical instance")
}

fn exact(instance: &Instance) -> IsingEncoding {
    let outcome = registry()
        .for_instance(instance)
        .expect("registered family")
        .encode(instance)
        .expect("encoding");
    match outcome {
        EncodingOutcome::Exact(encoding) => Some(encoding),
        EncodingOutcome::Surrogate(_) | EncodingOutcome::Unsupported { .. } => None,
    }
    .expect("an exact encoding")
}

/// The chain's ground state with spin 0 up: each edge follows the sign of its coupling.
fn ground_state(encoding: &IsingEncoding) -> Vec<i8> {
    let mut spins = vec![1_i8; encoding.h.len()];
    for (&(a, b), &j) in encoding.edges.iter().zip(&encoding.j) {
        spins[b as usize] = if j < 0 {
            spins[a as usize]
        } else {
            -spins[a as usize]
        };
    }
    spins
}

fn witness(instance: &Instance, spins: Vec<i8>) -> Witness {
    Witness {
        instance_id: instance.instance_id.clone(),
        spins,
        design: None,
        route: None,
        controls: None,
    }
}

fn context() -> EvaluationContext {
    EvaluationContext {
        tolerances: Value::Null,
    }
}

fn policy() -> PolicyManifest {
    PolicyManifest {
        policy_id: "test-policy".into(),
        backend: "fake".into(),
        num_reads: 1,
        num_sweeps: 10,
        anneal_time_us: 20,
        host_compute_limit_s: 60.0,
        evaluation_limit: 100_000,
        preparation: Preparation::Cold,
    }
}

fn fake_miner(bin: PathBuf) -> BackendSpec {
    BackendSpec {
        name: "fake".into(),
        bin,
        args: Vec::new(),
        kind: BackendKind::Sweeping,
        node_capacity_flag: None,
    }
}

/// Write the instances and a v2 manifest with `acceptance` into `dir`.
fn manifest(dir: &Path, instances: &[Value], acceptance: &Value) -> CampaignConfig {
    let mut paths = Vec::new();
    for (index, instance) in instances.iter().enumerate() {
        let path = dir.join(format!("instance-{index}.json"));
        fs::write(&path, serde_json::to_vec(instance).expect("JSON")).expect("instance");
        paths.push(path);
    }
    let path = dir.join("manifest.json");
    fs::write(
        &path,
        serde_json::to_vec(&json!({
            "schema_version": 2,
            "acceptance": acceptance,
            "instances": paths,
            "policies": [policy()],
            "trials_per_pair": 1,
            "budget": {"wall_limit_s": 5.0}
        }))
        .expect("JSON"),
    )
    .expect("manifest");
    CampaignConfig {
        manifest: path,
        backend: "fake".into(),
        output: dir.join("out"),
        dry_run: false,
        registry: registry(),
    }
}

fn pinned() -> Value {
    json!({"profile": "pinned_current", "protocol_profile": qblock("configs/protocol.json")})
}

fn code(error: CampaignError) -> &'static str {
    match error {
        CampaignError::Eval(error) => Some(error.code),
        _ => None,
    }
    .expect("a family error")
}

// ---- IM-1a: registry, dispatch, and acceptance profiles ----

#[tokio::test]
async fn unknown_family_fails_before_a_miner_starts() {
    let dir = tempfile::tempdir().expect("directory");
    let mut unknown = serde_json::to_value(two_spin()).expect("JSON");
    unknown["family"] = json!("no-such-family");
    let config = manifest(dir.path(), &[unknown], &pinned());
    // A spawn attempt would fail with an I/O error, not a family error.
    let backend = fake_miner(dir.path().join("missing-miner"));
    let error = run_campaign_with_backend(&config, backend)
        .await
        .expect_err("unknown family");
    assert_eq!(code(error), "family_unsupported");
}

#[test]
fn wrong_revision_is_rejected() {
    let error = FamilyRegistry::production()
        .evaluator("ising-fixed", "2")
        .err()
        .expect("revision 2 is unknown");
    assert_eq!(error.code, "revision_unsupported");
    let error = registry()
        .evaluator("test/pinned-chain", "01")
        .err()
        .expect("revisions are canonical decimal");
    assert_eq!(error.code, "revision_unsupported");
}

#[tokio::test]
async fn manifest_v1_is_rejected() {
    let dir = tempfile::tempdir().expect("directory");
    let config = manifest(
        dir.path(),
        &[serde_json::to_value(two_spin()).expect("JSON")],
        &pinned(),
    );
    let mut value: Value =
        serde_json::from_slice(&fs::read(&config.manifest).expect("read")).expect("JSON");
    value["schema_version"] = json!(1);
    fs::write(&config.manifest, serde_json::to_vec(&value).expect("JSON")).expect("write");
    let error = run_campaign_with_backend(&config, fake_miner(PathBuf::from("unused")))
        .await
        .expect_err("version 1");
    assert!(matches!(error, CampaignError::UnsupportedVersion(1)));
}

#[tokio::test]
async fn mixed_manifest_fails_before_any_trial() {
    let dir = tempfile::tempdir().expect("directory");
    let known = serde_json::to_value(two_spin()).expect("JSON");
    let mut unknown = known.clone();
    unknown["instance_id"] = json!("unknown");
    unknown["family"] = json!("no-such-family");
    let config = manifest(dir.path(), &[known, unknown], &pinned());
    let error = run_campaign_with_backend(&config, fake_miner(PathBuf::from("unused")))
        .await
        .expect_err("one unknown family");
    assert_eq!(code(error), "family_unsupported");
    assert!(
        !config.output.exists(),
        "no trial output before load succeeds"
    );
}

#[test]
fn family_single_accepts_one_valid_witness() {
    let instance = two_spin();
    let registry = FamilyRegistry::production();
    assert_eq!(
        RuleSet::FamilySingle
            .validate_set(&registry, &instance, &[witness(&instance, vec![1, 1])])
            .expect("decision"),
        ProofDecision::Accepted {
            best_energy_milli: -1000,
            diversity_milli: 0,
            valid_solution_count: 1,
        }
    );
    assert!(matches!(
        RuleSet::FamilySingle
            .validate_set(&registry, &instance, &[witness(&instance, vec![-1, 1])])
            .expect("decision"),
        ProofDecision::Rejected { .. }
    ));
}

#[tokio::test]
async fn serve_verify_set_uses_family_single() {
    let dir = tempfile::tempdir().expect("directory");
    let config = manifest(
        dir.path(),
        &[serde_json::to_value(two_spin()).expect("JSON")],
        &json!({"profile": "family_single"}),
    );
    let input = [
        json!({"cmd": "verify_set", "instance_id": "two-spin",
            "witnesses": [witness(&two_spin(), vec![1, 1])]}),
        json!({"cmd": "finish"}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n");
    let mut output = Vec::new();
    serve_campaign(&config, Cursor::new(input), &mut output)
        .await
        .expect("serve");
    let reply: Value = serde_json::from_slice(
        output
            .split(|&byte| byte == b'\n')
            .next()
            .expect("first reply"),
    )
    .expect("reply JSON");
    assert_eq!(reply["event"], "verified");
    assert_eq!(
        reply["decision"],
        json!({"Accepted": {"best_energy_milli": -1000, "diversity_milli": 0, "valid_solution_count": 1}})
    );
}

// ---- IM-1b: full nonce, canonical adapter, and identity records ----

#[test]
fn high_nonce_bytes_change_the_generated_instance() {
    let low = generate([0; 32]);
    let mut high_nonce = [0_u8; 32];
    high_nonce[0] = 1;
    let high = generate(high_nonce);
    assert_ne!(
        low.physical["instance_digest"],
        high.physical["instance_digest"]
    );
    assert_eq!(&high.nonce[..2], "01");
    // Coupling signs differ for some nonce among the first few high bytes.
    let differs = (1..=8_u8).any(|byte| {
        let mut nonce = [0_u8; 32];
        nonce[0] = byte;
        exact(&generate(nonce)).j != exact(&low).j
    });
    assert!(differs, "the high nonce byte must reach the coupling draw");
}

#[test]
fn instance_digest_pin_mismatch_is_rejected() {
    let mut instance = generate([3; 32]);
    let pin = instance.physical["instance_digest"]
        .as_str()
        .expect("pin")
        .to_owned();
    let flipped = if pin.starts_with('0') { "1" } else { "0" };
    instance.physical["instance_digest"] = json!(format!("{flipped}{}", &pin[1..]));
    let error = registry()
        .for_instance(&instance)
        .expect("registered")
        .encode(&instance)
        .expect_err("pin mismatch");
    assert_eq!(error.code, "instance_digest_mismatch");
}

#[test]
fn infeasible_low_energy_witness_fails_true_acceptance() {
    let instance = generate([5; 32]);
    let ground = ground_state(&exact(&instance));
    let flipped: Vec<i8> = ground.iter().map(|spin| -spin).collect();
    let evaluator = registry().for_instance(&instance).expect("registered");
    let good = evaluator
        .evaluate(&instance, &witness(&instance, ground.clone()), &context())
        .expect("evaluation");
    let bad = evaluator
        .evaluate(&instance, &witness(&instance, flipped.clone()), &context())
        .expect("evaluation");
    assert_eq!(good.energy_milli, Some(-2100));
    assert_eq!(
        bad.energy_milli,
        Some(-2100),
        "the flip has the same energy"
    );
    assert!(good.valid);
    assert!(!bad.valid);
    assert!(bad
        .rejections
        .iter()
        .any(|reason| reason.starts_with("constraint_violated")));
    let profile = load_protocol_profile(&qblock("configs/protocol.json")).expect("profile");
    for rules in [RuleSet::FamilySingle, RuleSet::Current(profile)] {
        assert!(matches!(
            rules
                .validate_set(
                    &registry(),
                    &instance,
                    &[witness(&instance, flipped.clone())]
                )
                .expect("decision"),
            ProofDecision::Rejected { .. }
        ));
        assert!(matches!(
            rules
                .validate_set(
                    &registry(),
                    &instance,
                    &[witness(&instance, ground.clone())]
                )
                .expect("decision"),
            ProofDecision::Accepted {
                best_energy_milli: -2100,
                ..
            }
        ));
    }
}

#[test]
fn revision_fields_must_agree() {
    let mut instance = generate([7; 32]);
    instance.physical["descriptor"]["revision"] = json!(2);
    let error = FAMILIES[0]
        .encode(&instance)
        .expect_err("descriptor revision 2");
    assert_eq!(error.code, "family_mismatch");
}

#[test]
fn nonce_must_be_64_lowercase_hex() {
    let dir = tempfile::tempdir().expect("directory");
    let zeros = "0".repeat(64);
    for nonce in [
        "A".repeat(64),
        "0".repeat(62),
        format!("0x{}", "0".repeat(62)),
        format!("{}g", "0".repeat(63)),
    ] {
        let mut value = serde_json::to_value(two_spin()).expect("JSON");
        value["nonce"] = json!(nonce);
        let path = dir.path().join("instance.json");
        fs::write(&path, serde_json::to_vec(&value).expect("JSON")).expect("write");
        assert!(
            load_instance(&path).is_err(),
            "nonce {nonce:?} must be rejected"
        );
    }
    assert_eq!(two_spin().nonce, zeros);
}

#[test]
fn generated_instance_over_spin_limit_is_rejected() {
    let nodes: Vec<u32> = (0..65_537).collect();
    let edges: Vec<(u32, u32)> = nodes.windows(2).map(|pair| (pair[0], pair[1])).collect();
    let config = FamilyConfig {
        family: "test/pinned-chain".into(),
        params: template(&nodes, &edges),
    };
    let error = FAMILIES[0]
        .generate(&config, [0; 32])
        .expect_err("65537 spins");
    assert_eq!(error.code, "too_large");
}

#[tokio::test]
async fn trial_records_identity() {
    let dir = tempfile::tempdir().expect("directory");
    let instance = generate([9; 32]);
    let mut runner = TrialRunner::new(
        fake_miner(PathBuf::from(env!("CARGO_BIN_EXE_isingmark-fake-miner"))),
        dir.path().to_owned(),
        RuleSet::FamilySingle,
    )
    .with_registry(registry());
    let budget = TrialBudget::new(Some(std::time::Duration::from_secs(5)), None).expect("budget");
    let trial = runner
        .run_trial(&instance, &policy(), &budget)
        .await
        .expect("trial");
    runner.finish().await.expect("shutdown");
    assert_eq!(trial.identity.family, "test/pinned-chain");
    assert_eq!(trial.identity.revision, "1");
    assert_eq!(trial.identity.nonce, "09".repeat(32));
    assert_eq!(
        trial.identity.instance_digest.as_deref(),
        instance.physical["instance_digest"].as_str()
    );
    assert_eq!(
        trial.identity.descriptor_digest.as_ref().map(String::len),
        Some(64)
    );
}

// ---- natural units in instance files ----

fn evaluate(instance: &Instance, spins: Vec<i8>) -> isingmark::campaign::contracts::Evaluation {
    FamilyRegistry::production()
        .for_instance(instance)
        .expect("fixed family")
        .evaluate(instance, &witness(instance, spins), &context())
        .expect("evaluation")
}

#[test]
fn exact_threshold_and_offset() {
    let mut instance = two_spin();
    instance.physical["offset"] = json!(0.007);
    for (threshold, valid) in [(-0.993, false), (-0.992, true)] {
        instance.objective["max_energy"] = json!(threshold);
        let evaluation = evaluate(&instance, vec![1, 1]);
        assert_eq!(evaluation.energy_milli, Some(-993));
        assert_eq!(evaluation.valid, valid, "threshold {threshold}");
    }
}

#[test]
fn natural_decimals_load_as_exact_milli() {
    let mut instance = two_spin();
    instance.physical["h"] = json!([0.5, -0.001]);
    instance.physical["j"] = json!([-1.25]);
    instance.physical["offset"] = json!(2);
    let encoding = exact(&instance);
    assert_eq!(
        (encoding.h, encoding.j, encoding.offset),
        (vec![500, -1], vec![-1250], 2000)
    );
    for (threshold, valid) in [(json!(1.249), false), (json!(1.25), true)] {
        instance.objective["max_energy"] = threshold;
        let evaluation = evaluate(&instance, vec![1, 1]);
        assert_eq!(evaluation.energy_milli, Some(1249));
        assert_eq!(evaluation.valid, valid);
    }
}

#[test]
fn more_than_three_decimals_is_rejected() {
    let mut instance = two_spin();
    instance.physical["h"] = json!([0.0005, 0]);
    let error = FamilyRegistry::production()
        .for_instance(&instance)
        .expect("fixed family")
        .encode(&instance)
        .expect_err("half a milli");
    assert_eq!(error.code, "unsupported_precision");
    let mut instance = two_spin();
    instance.objective["max_energy"] = json!(1.0001);
    let error = FamilyRegistry::production()
        .for_instance(&instance)
        .expect("fixed family")
        .evaluate(&instance, &witness(&instance, vec![1, 1]), &context())
        .expect_err("a tenth of a milli");
    assert_eq!(error.code, "unsupported_precision");
}

#[test]
fn natural_coefficients_reach_the_wire_endpoints() {
    let mut instance = two_spin();
    instance.physical["h"] = json!([-2_147_483.648, 2_147_483.647]);
    instance.physical["j"] = json!([0]);
    assert_eq!(exact(&instance).h, vec![i32::MIN, i32::MAX]);
    instance.physical["h"] = json!([2_147_483.648, 0]);
    let error = FamilyRegistry::production()
        .for_instance(&instance)
        .expect("fixed family")
        .encode(&instance)
        .expect_err("one milli past i32::MAX");
    assert_eq!(error.code, "overflow");
}

#[test]
fn milli_field_names_are_gone_from_instance_files() {
    let mut instance = two_spin();
    instance.objective = json!({"max_energy_milli": 1000});
    let error = FamilyRegistry::production()
        .for_instance(&instance)
        .expect("fixed family")
        .evaluate(&instance, &witness(&instance, vec![1, 1]), &context())
        .expect_err("old field name");
    assert_eq!(error.code, "invalid_objective");
    let mut value = serde_json::to_value(two_spin()).expect("instance");
    value["schema_version"] = json!(1);
    let path = tempfile::NamedTempFile::new().expect("file");
    fs::write(path.path(), value.to_string()).expect("write");
    assert!(matches!(
        load_instance(path.path()),
        Err(CampaignError::UnsupportedVersion(1))
    ));
}

#[test]
fn oversized_or_cross_instance_witness_fails() {
    let instance = two_spin();
    let evaluator = FamilyRegistry::production()
        .for_instance(&instance)
        .expect("fixed family");
    let mut other = witness(&instance, vec![1, 1]);
    other.instance_id = "another".into();
    let error = evaluator
        .evaluate(&instance, &other, &context())
        .expect_err("cross instance");
    assert_eq!(error.code, "instance_mismatch");
    let error = evaluator
        .evaluate(&instance, &witness(&instance, vec![1; 65_537]), &context())
        .expect_err("too many spins");
    assert_eq!(error.code, "too_large");
}

// ---- IM-1d: family-check ----

fn oracle_dir() -> PathBuf {
    qblock("fixtures/family-check/ising-fixed")
}

#[test]
fn family_check_passes_on_the_fixed_oracle() {
    let report =
        family_check(&FamilyRegistry::production(), "ising-fixed", &oracle_dir()).expect("report");
    assert_eq!((report.fixtures, report.witnesses), (1, 2));
    assert_eq!(report.revision, "1");
    assert!(report.failures.is_empty(), "{:?}", report.failures);
}

fn wrong_oracle() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("directory");
    fs::copy(
        oracle_dir().join("two_spin.instance.json"),
        dir.path().join("two_spin.instance.json"),
    )
    .expect("copy");
    fs::write(
        dir.path().join("two_spin.oracle.json"),
        json!({"witnesses": [
            {"spins": [1, 1], "valid": true, "energy": -1},
            {"spins": [-1, 1], "valid": true, "energy": 1.0}
        ]})
        .to_string(),
    )
    .expect("oracle");
    dir
}

#[test]
fn family_check_reports_a_wrong_oracle() {
    let dir = wrong_oracle();
    let report =
        family_check(&FamilyRegistry::production(), "ising-fixed", dir.path()).expect("report");
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].fixture, "two_spin");
    assert_eq!(report.failures[0].witness, Some(1));
    assert_eq!(
        report.failures[0].reason,
        "expected valid true at energy 1, got valid false at energy 1"
    );
}

#[test]
fn family_check_revision_comes_from_the_checked_family() {
    let dir = tempfile::tempdir().expect("directory");
    let source = oracle_dir();
    for name in ["two_spin.instance.json", "two_spin.oracle.json"] {
        fs::copy(source.join(name), dir.path().join(name)).expect("copy");
    }
    let mut foreign: serde_json::Value =
        serde_json::from_slice(&fs::read(source.join("two_spin.instance.json")).expect("instance"))
            .expect("json");
    foreign["family"] = json!("other");
    foreign["model_version"] = json!("9");
    // Sorts before two_spin, so it is loaded first.
    fs::write(
        dir.path().join("a_other.instance.json"),
        foreign.to_string(),
    )
    .expect("foreign");
    let report =
        family_check(&FamilyRegistry::production(), "ising-fixed", dir.path()).expect("report");
    assert_eq!(report.revision, "1");
    assert_eq!((report.fixtures, report.witnesses), (2, 2));
    assert_eq!(report.failures.len(), 1, "{:?}", report.failures);
    assert_eq!(report.failures[0].fixture, "a_other");
}

#[test]
fn family_check_refuses_an_unknown_family() {
    let error = family_check(
        &FamilyRegistry::production(),
        "no-such-family",
        Path::new("/nonexistent/fixtures"),
    )
    .expect_err("unknown family");
    assert_eq!(code(error), "family_unsupported");
}

#[test]
fn family_check_bounds_the_directory() {
    let dir = tempfile::tempdir().expect("directory");
    for index in 0..=4096 {
        fs::write(dir.path().join(format!("f{index}.instance.json")), "{}").expect("fixture");
    }
    let error = family_check(&FamilyRegistry::production(), "ising-fixed", dir.path())
        .expect_err("4097 fixtures");
    assert!(matches!(
        error,
        CampaignError::TooLarge {
            what: "fixtures",
            ..
        }
    ));
}

#[test]
fn family_check_cli_prints_one_json_object() {
    let run = |fixtures: &Path| {
        std::process::Command::new(env!("CARGO_BIN_EXE_isingmark"))
            .args([
                "campaign",
                "family-check",
                "--family",
                "ising-fixed",
                "--fixtures",
            ])
            .arg(fixtures)
            .output()
            .expect("run isingmark")
    };
    let output = run(&oracle_dir());
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("one JSON object");
    assert_eq!(report["failures"], json!([]));
    let dir = wrong_oracle();
    let output = run(dir.path());
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("one JSON object");
    assert_eq!(report["failures"].as_array().map(Vec::len), Some(1));
}
