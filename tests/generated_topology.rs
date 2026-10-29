// SPDX-License-Identifier: AGPL-3.0-or-later
use clap::Parser;
use isingmark::cli::{Cli, Command};
use isingmark::topology::{load_topology, TopologySpec};

fn generated_spec(description: &str, couplings: Option<&str>) -> Result<TopologySpec, String> {
    let mut flags = vec![
        "isingmark",
        "throughput",
        "--backend",
        "cpu-sa",
        "--topology-generate",
        description,
        "--duration",
        "1s",
    ];
    if let Some(couplings) = couplings {
        flags.extend(["--couplings", couplings]);
    }
    let cli = Cli::try_parse_from(flags).map_err(|e| e.to_string())?;
    let Command::Throughput(args) = cli.command else {
        return Err("expected throughput".to_string());
    };
    args.into_config()?
        .topology
        .load()
        .map_err(|e| e.to_string())
}

#[test]
fn generated_complete_matches_file_hash() {
    let generated = generated_spec("complete:4", None).expect("generate complete graph");
    assert_eq!(generated.nodes, vec![0, 1, 2, 3]);
    assert_eq!(
        generated.edges,
        vec![(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)]
    );
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("complete.spec.json");
    let json = serde_json::json!({
        "nodes": generated.nodes,
        "edges": generated.edges,
        "allowed_h_milli": generated.allowed_h_milli,
        "allowed_j_milli": generated.allowed_j_milli,
        "allowed_spin_milli": generated.allowed_spin_milli,
    });
    std::fs::write(&path, serde_json::to_vec(&json).expect("serialize spec")).expect("write spec");
    let loaded = load_topology(&path).expect("load generated spec from file");
    assert_eq!(generated.topology_hash(), loaded.topology_hash());
    assert_eq!(generated, loaded);
}

#[test]
fn generated_circulant_degree() {
    for (description, nodes, degree, edges) in [
        ("circulant:10:4", 10, 4, 20),
        ("circulant:61:60", 61, 60, 1830),
        ("circulant:8:0", 8, 0, 0),
    ] {
        let spec = generated_spec(description, None).expect("generate circulant graph");
        assert_eq!(spec.nodes.len(), nodes);
        assert_eq!(spec.edges.len(), edges);
        let mut degrees = vec![0; nodes];
        let mut unique = std::collections::HashSet::new();
        for (u, v) in spec.edges {
            assert!(u < v, "canonical endpoints without self-loops");
            assert!(unique.insert((u, v)), "no duplicate edges");
            degrees[u as usize] += 1;
            degrees[v as usize] += 1;
        }
        assert_eq!(degrees, vec![degree; nodes]);
    }
}

#[test]
fn couplings_sets() {
    let binary = generated_spec("complete:3", None).expect("default binary couplings");
    assert_eq!(binary.allowed_j_milli, vec![-1000, 1000]);
    assert_eq!(binary.allowed_h_milli, vec![-1000, 0, 1000]);
    assert_eq!(binary.allowed_spin_milli, vec![-1000, 1000]);
    assert_eq!(
        binary,
        generated_spec("complete:3", Some("binary")).expect("explicit binary")
    );

    let int8 = generated_spec("complete:3", Some("int8")).expect("int8 couplings");
    assert_eq!(
        int8.allowed_h_milli,
        (-992..=992).step_by(16).collect::<Vec<_>>()
    );
    assert_eq!(
        int8.allowed_j_milli,
        (-992..=992)
            .step_by(16)
            .filter(|&v| v != 0)
            .collect::<Vec<_>>()
    );
    assert_eq!(int8.allowed_h_milli.len(), 125);
    assert_eq!(int8.allowed_j_milli.len(), 124);

    let int32 = generated_spec("complete:3", Some("int32")).expect("int32 couplings");
    assert_eq!(int32.allowed_h_milli, (-1000..=1000).collect::<Vec<_>>());
    assert_eq!(
        int32.allowed_j_milli,
        (-1000..=1000).filter(|&v| v != 0).collect::<Vec<_>>()
    );
    assert_ne!(binary.topology_hash(), int8.topology_hash());
    assert_ne!(int8.topology_hash(), int32.topology_hash());
}

#[test]
fn generated_topology_rejects_invalid_descriptions() {
    for description in [
        "",
        "complete",
        "complete:0",
        "complete:-1",
        "complete:4294967296",
        "complete:4:2",
        "circulant:8",
        "circulant:0:0",
        "circulant:8:8",
        "circulant:8:3",
        "circulant:8:-2",
        "circulant:8:2:4",
        "ring:8",
    ] {
        assert!(
            generated_spec(description, None).is_err(),
            "accepted {description:?}"
        );
    }
}

#[test]
fn generated_topology_flags_enforce_source_and_coupling_constraints() {
    for source in ["--topology-preset", "--topology-file"] {
        assert!(Cli::try_parse_from([
            "isingmark",
            "throughput",
            "--backend",
            "cpu-sa",
            "--duration",
            "1s",
            "--topology-generate",
            "complete:4",
            source,
            "smoke",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "isingmark",
            "throughput",
            "--backend",
            "cpu-sa",
            "--duration",
            "1s",
            source,
            "smoke",
            "--couplings",
            "int8",
        ])
        .is_err());
    }
    assert!(generated_spec("complete:4", Some("float32")).is_err());
}

#[test]
fn generated_topology_overrides_config_file_source() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        "backend: cpu-sa\ntopology_preset: smoke\nduration: 1s\n",
    )
    .expect("write config");
    let cli = Cli::try_parse_from([
        "isingmark",
        "throughput",
        "--config",
        path.to_str().expect("UTF-8 path"),
        "--topology-generate",
        "complete:4",
        "--couplings",
        "int8",
    ])
    .expect("parse flags");
    let Command::Throughput(args) = cli.command else {
        unreachable!()
    };
    let cfg = args.into_config().expect("resolve flags over config");
    assert_eq!(cfg.topology.label(), "complete:4");
    let spec = cfg.topology.load().expect("load generated topology");
    assert_eq!(spec.nodes.len(), 4);
    assert_eq!(spec.allowed_h_milli.len(), 125);
}
