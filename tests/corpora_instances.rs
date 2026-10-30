// SPDX-License-Identifier: AGPL-3.0-or-later
//! Every structured example in `corpora/` loads against the fixture its header names.

use std::fs;
use std::path::Path;

use isingmark::instances::InstanceSource;
use isingmark::topology::load_fixture;
use serde_json::Value;

/// Chain corpora hold nonces, not explicit instances.
const NONCE_CORPORA: [&str; 2] = ["chain-h0.jsonl", "chain-ternary.jsonl"];

/// The two-instance smoke file is not a frozen-model example.
const SMOKE_CORPUS: &str = "smoke-ice-glass.jsonl";

#[test]
fn every_instance_corpus_loads_against_its_fixture() -> Result<(), String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpora");
    let mut checked = 0;
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let is_jsonl = path.extension().is_some_and(|ext| ext == "jsonl");
        if !is_jsonl || NONCE_CORPORA.contains(&name) {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))?;
        let first = text.lines().next().ok_or(format!("{name}: empty file"))?;
        let header: Value = serde_json::from_str(first).map_err(|e| format!("{name}: {e}"))?;
        let fixture = header["topology_fixture"]
            .as_str()
            .ok_or(format!("{name}: header names no topology_fixture"))?;
        let spec = load_fixture(fixture).map_err(|e| format!("{name}: {e}"))?;
        let source =
            InstanceSource::parse(name, &text, &spec).map_err(|e| format!("{name}: {e}"))?;
        let lines = text
            .lines()
            .skip(1)
            .filter(|l| !l.trim().is_empty())
            .count();
        if lines == 0 || source.len() != lines {
            return Err(format!(
                "{name}: {lines} instance lines, {} loaded",
                source.len()
            ));
        }
        if name != SMOKE_CORPUS {
            if lines != 1 {
                return Err(format!("{name}: {lines} instances, expected 1"));
            }
            if header["generator"] != "frozen-model" {
                return Err(format!("{name}: generator is {}", header["generator"]));
            }
        }
        checked += 1;
    }
    if checked == 0 {
        return Err(format!("no instance corpora found under {}", dir.display()));
    }
    Ok(())
}
