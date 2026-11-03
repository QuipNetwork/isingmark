// SPDX-License-Identifier: AGPL-3.0-or-later
//! Portfolio catalogs, slot resolution, and shared-budget portfolio trials.

use std::{fs, path::Path};

use isingmark::campaign::runner::portfolio::{
    load_catalog, resolve_panel, Allocation, Applicability, Catalog,
};
use serde_json::{json, Value};

fn solver(id: &str, backend: &str, device: &str, methods: &[&str], capture: &str) -> Value {
    json!({
        "solver_id": id, "backend": backend, "device": device, "methods": methods,
        "applies_to": "any", "state_capture": capture,
        "num_reads": 4, "num_sweeps": 10, "anneal_time_us": 0
    })
}

fn catalog_value(solvers: &[Value]) -> Value {
    json!({
        "schema_version": 1,
        "host_compute_limit_s": 1200.0,
        "evaluation_limit": 100_000,
        "solvers": solvers,
        "panels": [{"panel_id": "toy", "slots": [
            {"slot_id": "cpu-1", "device": "cpu", "method": "generic-annealing"},
            {"slot_id": "cpu-2", "device": "cpu", "method": "loop-tempering"},
            {"slot_id": "cpu-3", "device": "cpu", "method": "toy-flow", "fallback": "generic-annealing"},
            {"slot_id": "cpu-4", "device": "cpu", "schedule": [
                {"slot": "cpu-3", "allocation": {"seconds": 10.0}},
                {"slot": "cpu-2", "allocation": {"seconds": 190.0}},
                {"slot": "cpu-1", "allocation": "remainder"}
            ]},
            {"slot_id": "gpu-1", "device": "gpu", "method": "generic-annealing"},
            {"slot_id": "gpu-2", "device": "gpu", "method": "loop-updates"}
        ]}]
    })
}

fn write_catalog(dir: &Path, value: &Value) -> Catalog {
    let path = dir.join("policies.json");
    fs::write(&path, serde_json::to_vec(value).expect("JSON")).expect("catalog");
    load_catalog(&path).expect("catalog loads")
}

fn full_solvers() -> Vec<Value> {
    vec![
        solver(
            "cpu-sa",
            "cpu-sa",
            "cpu",
            &["generic-annealing"],
            "all_reads",
        ),
        solver(
            "cpu-loops",
            "cpu-loops",
            "cpu",
            &["loop-tempering"],
            "all_reads",
        ),
        solver("cuda", "cuda", "gpu", &["generic-annealing"], "all_reads"),
        solver(
            "cuda-loops",
            "cuda-loops",
            "gpu",
            &["loop-updates"],
            "all_reads",
        ),
    ]
}

#[test]
fn catalog_resolves_native_and_fallback() {
    let dir = tempfile::tempdir().expect("directory");
    let catalog = write_catalog(dir.path(), &catalog_value(&full_solvers()));
    let resolution = resolve_panel(&catalog, "toy", "any/family", &|_| true).expect("resolves");
    assert!(resolution.complete, "{resolution:?}");
    assert_eq!(resolution.solver_starts, 0);
    let cpu3 = &resolution.slots[2];
    let stage = &cpu3.policy.as_ref().expect("cpu-3 policy").stages[0];
    assert_eq!(stage.solver_id, "cpu-sa");
    assert_eq!(stage.method, "generic-annealing");
    assert_eq!(stage.applicability, Applicability::Fallback);
    let cpu1 = &resolution.slots[0]
        .policy
        .as_ref()
        .expect("cpu-1 policy")
        .stages[0];
    assert_eq!(cpu1.applicability, Applicability::Native);
    assert_eq!(cpu1.allocation, Allocation::Remainder);
}

#[test]
fn cpu4_schedule_is_frozen() {
    let dir = tempfile::tempdir().expect("directory");
    let catalog = write_catalog(dir.path(), &catalog_value(&full_solvers()));
    let resolution = resolve_panel(&catalog, "toy", "any/family", &|_| true).expect("resolves");
    let policy = resolution.slots[3].policy.as_ref().expect("cpu-4 policy");
    assert_eq!(policy.policy_id, "toy/cpu-4");
    let plan: Vec<_> = policy
        .stages
        .iter()
        .map(|stage| (stage.stage_id.as_str(), stage.allocation))
        .collect();
    assert_eq!(
        plan,
        [
            ("cpu-3", Allocation::Seconds(10.0)),
            ("cpu-2", Allocation::Seconds(190.0)),
            ("cpu-1", Allocation::Remainder),
        ]
    );
}

#[test]
fn best_only_solver_fails_admission() {
    let dir = tempfile::tempdir().expect("directory");
    let mut solvers = full_solvers();
    solvers[1] = solver(
        "cpu-loops",
        "cpu-loops",
        "cpu",
        &["loop-tempering"],
        "best_only",
    );
    let catalog = write_catalog(dir.path(), &catalog_value(&solvers));
    let resolution = resolve_panel(&catalog, "toy", "any/family", &|_| true).expect("resolves");
    let cpu2 = &resolution.slots[1];
    assert!(cpu2.policy.is_none());
    assert_eq!(cpu2.rejected[0].solver_id, "cpu-loops");
    // The schedule that includes cpu-2 cannot be labeled complete either.
    assert!(resolution.slots[3].policy.is_none());
    assert_eq!(resolution.missing_slots, ["cpu-2", "cpu-4"]);
    assert!(!resolution.complete);
}

#[test]
fn dry_run_reports_missing_slots_and_zero_starts() {
    let dir = tempfile::tempdir().expect("directory");
    let catalog = write_catalog(dir.path(), &catalog_value(&full_solvers()));
    let resolution = resolve_panel(&catalog, "toy", "any/family", &|backend| {
        backend != "cuda-loops"
    })
    .expect("resolves");
    assert_eq!(resolution.missing_slots, ["gpu-2"]);
    assert_eq!(resolution.solver_starts, 0);
    assert!(!resolution.complete);
    let gpu2 = &resolution.slots[5];
    assert!(
        gpu2.missing[0].contains("loop-updates"),
        "{:?}",
        gpu2.missing
    );
    assert!(gpu2.rejected[0].reason.contains("not available"));
}

#[test]
fn invalid_schedules_are_rejected() {
    let dir = tempfile::tempdir().expect("directory");
    let cases = [
        // The remainder must come last.
        json!([{"slot": "cpu-1", "allocation": "remainder"}, {"slot": "cpu-2", "allocation": {"seconds": 10.0}}]),
        // Only one remainder.
        json!([{"slot": "cpu-1", "allocation": "remainder"}, {"slot": "cpu-2", "allocation": "remainder"}]),
        // Positive seconds only.
        json!([{"slot": "cpu-1", "allocation": {"seconds": 0.0}}, {"slot": "cpu-2", "allocation": "remainder"}]),
        // A schedule cannot cross devices.
        json!([{"slot": "gpu-1", "allocation": {"seconds": 5.0}}, {"slot": "cpu-2", "allocation": "remainder"}]),
        json!([]),
    ];
    for schedule in cases {
        let mut value = catalog_value(&full_solvers());
        value["panels"][0]["slots"][3]["schedule"] = schedule.clone();
        let path = dir.path().join("policies.json");
        fs::write(&path, serde_json::to_vec(&value).expect("JSON")).expect("catalog");
        assert!(load_catalog(&path).is_err(), "accepted {schedule}");
    }
}

#[test]
fn committed_catalog_declares_four_cpu_and_two_gpu_slots_per_panel() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("evaluation/qblock/configs/suite_fit/policies.json");
    let catalog = load_catalog(&path).expect("committed catalog");
    assert_eq!(catalog.panels.len(), 4);
    for panel in &catalog.panels {
        let ids: Vec<_> = panel
            .slots
            .iter()
            .map(|slot| slot.slot_id.as_str())
            .collect();
        assert_eq!(ids, ["cpu-1", "cpu-2", "cpu-3", "cpu-4", "gpu-1", "gpu-2"]);
        let cpu4 = panel.slots[3].schedule.as_ref().expect("cpu-4 schedule");
        let plan: Vec<_> = cpu4
            .iter()
            .map(|stage| (stage.slot.as_str(), stage.allocation))
            .collect();
        assert_eq!(
            plan,
            [
                ("cpu-3", Allocation::Seconds(10.0)),
                ("cpu-2", Allocation::Seconds(190.0)),
                ("cpu-1", Allocation::Remainder),
            ]
        );
        assert!(
            panel.slots[2].fallback.is_some(),
            "{} cpu-3 needs a fallback",
            panel.panel_id
        );
    }
}
