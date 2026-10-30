//! Format and rejection checks for archived research fixtures under
//! `problems/<family>/fixtures/<cell>.json`.

use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};
use tempfile::TempDir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

mod fixtures {
    use std::collections::{BTreeSet, HashMap};
    use std::fs;
    use std::path::Path;

    use serde::Deserialize;
    use serde_json::Value;

    #[derive(Deserialize)]
    struct SourceManifest {
        sources: Vec<ManifestSource>,
    }

    #[derive(Deserialize)]
    struct ManifestSource {
        id: String,
        kind: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fixture {
        schema_revision: u32,
        provenance: String,
        about: Vec<String>,
        sources: Vec<FileRef>,
        archived_instances: Vec<Instance>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FileRef {
        path: String,
        blake3: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Instance {
        name: String,
        cell: String,
        nonce: String,
        archive_origin: String,
        archive_blake3: String,
        source_recipe: Recipe,
        node_order: String,
        node_ids: Vec<i64>,
        edges: Vec<[i64; 2]>,
        h: Vec<i64>,
        j: Vec<i64>,
        scale: i64,
        offset_milli: String,
        qubo_offset_milli: Option<String>,
        family_id: Option<String>,
        descriptor_digest: Option<String>,
        params_hex: Option<String>,
        postprocess_policy: Policy,
        witnesses: Vec<Witness>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Recipe {
        source: String,
        name: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Policy {
        family: String,
        method: String,
        budget_s: f64,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Witness {
        name: String,
        spins: Option<Vec<i64>>,
        claimed_energy_milli: Option<String>,
        missing: Option<String>,
    }

    /// Names each i64 field written as a JSON number. The typed parse rejects these too, but
    /// its message does not name the field.
    fn number_fields(file: &str, raw: &[u8]) -> Vec<String> {
        let Ok(value) = serde_json::from_slice::<Value>(raw) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let instances = value["archived_instances"].as_array().into_iter().flatten();
        for inst in instances {
            let at = format!("{file}: instance {}", inst["name"].as_str().unwrap_or("?"));
            for field in ["offset_milli", "qubo_offset_milli"] {
                if inst[field].is_number() {
                    found.push(format!(
                        "{at}: {field} is a JSON number, expected a decimal string"
                    ));
                }
            }
            for witness in inst["witnesses"].as_array().into_iter().flatten() {
                if witness["claimed_energy_milli"].is_number() {
                    found.push(format!(
                        "{at}: claimed_energy_milli is a JSON number, expected a decimal string"
                    ));
                }
            }
        }
        found
    }

    fn is_lower_hex(text: &str) -> bool {
        text.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    fn is_digest(text: &str) -> bool {
        text.len() == 64 && is_lower_hex(text)
    }

    fn is_safe_path(path: &str) -> bool {
        !path.starts_with('/')
            && !path.contains('\\')
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
    }

    fn check_sources(file: &str, fixture: &Fixture, root: &Path, errors: &mut Vec<String>) {
        for source in &fixture.sources {
            let path = &source.path;
            if !is_safe_path(path) {
                errors.push(format!("{file}: unsafe source path {path:?}"));
                continue;
            }
            if !is_digest(&source.blake3) {
                errors.push(format!(
                    "{file}: source {path}: blake3 is not 64 lowercase hex characters"
                ));
            }
            match fs::read(root.join(path)) {
                Err(e) => errors.push(format!("{file}: cannot read source {path}: {e}")),
                Ok(bytes) => {
                    let actual = blake3::hash(&bytes).to_hex().to_string();
                    if actual != source.blake3 {
                        errors.push(format!(
                            "{file}: source {path}: blake3 mismatch, file hashes to {actual}"
                        ));
                    }
                }
            }
        }
    }

    fn check_graph(at: &str, inst: &Instance, errors: &mut Vec<String>) {
        let nodes = inst.node_ids.len();
        let unique: BTreeSet<_> = inst.node_ids.iter().collect();
        if unique.len() != nodes {
            errors.push(format!("{at}: node_ids has a duplicate"));
        }
        match inst.node_order.as_str() {
            "physical" => {}
            "ascending" => {
                if !inst.node_ids.windows(2).all(|w| w[0] < w[1]) {
                    errors.push(format!(
                        "{at}: node_order is ascending but node_ids is not strictly increasing"
                    ));
                }
            }
            other => errors.push(format!("{at}: unknown node_order {other:?}")),
        }
        for &[a, b] in &inst.edges {
            if !(0 <= a && a < b && usize::try_from(b).is_ok_and(|b| b < nodes)) {
                errors.push(format!(
                    "{at}: edge [{a}, {b}] is out of range or not ordered a < b"
                ));
            }
        }
        if !inst.edges.windows(2).all(|w| w[0] < w[1]) {
            errors.push(format!(
                "{at}: edges are not strictly increasing (unsorted or duplicate)"
            ));
        }
        if inst.h.len() != nodes {
            errors.push(format!(
                "{at}: h has {} entries for {nodes} nodes",
                inst.h.len()
            ));
        }
        if inst.j.len() != inst.edges.len() {
            errors.push(format!(
                "{at}: j has {} entries for {} edges",
                inst.j.len(),
                inst.edges.len()
            ));
        }
        if inst.scale != 1000 {
            errors.push(format!("{at}: scale is {}, expected 1000", inst.scale));
        }
    }

    fn check_optional(at: &str, inst: &Instance, errors: &mut Vec<String>) {
        if inst.family_id.as_deref().is_some_and(str::is_empty) {
            errors.push(format!("{at}: family_id is empty"));
        }
        if inst
            .descriptor_digest
            .as_deref()
            .is_some_and(|d| !is_digest(d))
        {
            errors.push(format!(
                "{at}: descriptor_digest is not 64 lowercase hex characters"
            ));
        }
        if inst
            .params_hex
            .as_deref()
            .is_some_and(|p| p.len() % 2 != 0 || !is_lower_hex(p))
        {
            errors.push(format!("{at}: params_hex is not even-length lowercase hex"));
        }
        let policy = &inst.postprocess_policy;
        if policy.family.is_empty() || policy.method.is_empty() {
            errors.push(format!(
                "{at}: postprocess_policy needs a non-empty family and method"
            ));
        }
        if policy.budget_s < 0.0 {
            errors.push(format!("{at}: postprocess_policy budget_s is negative"));
        }
    }

    fn parse_i64(at: &str, field: &str, text: &str, errors: &mut Vec<String>) -> Option<i64> {
        let parsed = text.parse::<i64>().ok();
        if parsed.is_none() {
            errors.push(format!("{at}: {field} {text:?} is not a decimal i64"));
        }
        parsed
    }

    fn energy(inst: &Instance, offset: i64, spins: &[i64]) -> i128 {
        let mut total = i128::from(offset);
        for (h, s) in inst.h.iter().zip(spins) {
            total += i128::from(*h) * i128::from(*s);
        }
        for (j, [a, b]) in inst.j.iter().zip(&inst.edges) {
            let spin = |i: i64| usize::try_from(i).ok().and_then(|i| spins.get(i));
            if let (Some(sa), Some(sb)) = (spin(*a), spin(*b)) {
                total += i128::from(*j) * i128::from(*sa) * i128::from(*sb);
            }
        }
        total
    }

    fn check_witnesses(at: &str, inst: &Instance, errors: &mut Vec<String>) {
        let offset = parse_i64(at, "offset_milli", &inst.offset_milli, errors);
        if let Some(text) = &inst.qubo_offset_milli {
            parse_i64(at, "qubo_offset_milli", text, errors);
        }
        let mut names = BTreeSet::new();
        for witness in &inst.witnesses {
            let at = format!("{at}: witness {}", witness.name);
            if !names.insert(witness.name.as_str()) {
                errors.push(format!("{at}: duplicate witness name"));
            }
            match (
                &witness.spins,
                &witness.claimed_energy_milli,
                &witness.missing,
            ) {
                (Some(spins), Some(claimed), None) => {
                    let claimed = parse_i64(&at, "claimed_energy_milli", claimed, errors);
                    if spins.len() != inst.node_ids.len() {
                        errors.push(format!(
                            "{at}: spins has {} entries for {} nodes",
                            spins.len(),
                            inst.node_ids.len()
                        ));
                    } else if spins.iter().any(|s| *s != 1 && *s != -1) {
                        errors.push(format!("{at}: a spin is not 1 or -1"));
                    } else if let (Some(offset), Some(claimed)) = (offset, claimed) {
                        let actual = energy(inst, offset, spins);
                        if actual != i128::from(claimed) {
                            errors.push(format!(
                                "{at}: claimed energy {claimed} but the energy is {actual}"
                            ));
                        }
                    }
                }
                (None, None, Some(reason)) => {
                    if reason.is_empty() {
                        errors.push(format!("{at}: missing reason is empty"));
                    }
                }
                _ => errors.push(format!(
                    "{at}: needs either spins with claimed_energy_milli, or missing"
                )),
            }
        }
    }

    fn check_instance(
        file: &str,
        inst: &Instance,
        generators: &HashMap<&str, bool>,
        errors: &mut Vec<String>,
    ) {
        let at = format!("{file}: instance {}", inst.name);
        if !is_digest(&inst.nonce) {
            errors.push(format!("{at}: nonce is not 64 lowercase hex characters"));
        }
        if !is_digest(&inst.archive_blake3) {
            errors.push(format!(
                "{at}: archive_blake3 is not 64 lowercase hex characters"
            ));
        }
        if !is_safe_path(&inst.archive_origin) {
            errors.push(format!(
                "{at}: unsafe archive_origin {:?}",
                inst.archive_origin
            ));
        }
        if inst.cell.is_empty() {
            errors.push(format!("{at}: cell is empty"));
        }
        match generators.get(inst.source_recipe.source.as_str()) {
            Some(true) => {}
            Some(false) => errors.push(format!(
                "{at}: source_recipe source {} is not a generator",
                inst.source_recipe.source
            )),
            None => errors.push(format!(
                "{at}: source_recipe source {} is not in the source manifest",
                inst.source_recipe.source
            )),
        }
        if inst.source_recipe.name.is_empty() {
            errors.push(format!("{at}: source_recipe name is empty"));
        }
        check_graph(&at, inst, errors);
        check_optional(&at, inst, errors);
        check_witnesses(&at, inst, errors);
    }

    /// Checks one fixture file and returns every violation. Source paths resolve under
    /// `repo_root`. Recipe sources resolve against `sources_manifest`.
    pub fn validate(
        file: &Path,
        repo_root: &Path,
        sources_manifest: &Path,
    ) -> Result<(), Vec<String>> {
        let name = file.display().to_string();
        let raw = fs::read(file).map_err(|e| vec![format!("{name}: cannot read: {e}")])?;
        let fixture: Fixture = serde_json::from_slice(&raw).map_err(|e| {
            let mut errors = number_fields(&name, &raw);
            if errors.is_empty() {
                errors.push(format!("{name}: cannot parse: {e}"));
            }
            errors
        })?;
        let manifest: SourceManifest = fs::read(sources_manifest)
            .map_err(|e| e.to_string())
            .and_then(|raw| serde_json::from_slice(&raw).map_err(|e| e.to_string()))
            .map_err(|e| vec![format!("{}: cannot read: {e}", sources_manifest.display())])?;
        let generators: HashMap<&str, bool> = manifest
            .sources
            .iter()
            .map(|s| (s.id.as_str(), s.kind == "generator"))
            .collect();

        let mut errors = Vec::new();
        if fixture.schema_revision != 1 {
            errors.push(format!(
                "{name}: schema_revision is {}, expected 1",
                fixture.schema_revision
            ));
        }
        if fixture.provenance != "archived-research" {
            errors.push(format!(
                "{name}: provenance is {:?}, expected \"archived-research\"",
                fixture.provenance
            ));
        }
        if fixture.about.is_empty() {
            errors.push(format!("{name}: about is empty"));
        }
        check_sources(&name, &fixture, repo_root, &mut errors);
        let mut names = BTreeSet::new();
        for inst in &fixture.archived_instances {
            if !names.insert(inst.name.as_str()) {
                errors.push(format!("{name}: duplicate instance name {}", inst.name));
            }
            check_instance(&name, inst, &generators, &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

const SOURCE_BODY: &str = "fn generate() {}\n";
const SOURCE_PATH: &str = "problems/demo/sources/gen.rs";
const FIXTURE: &str = "problems/demo/fixtures/cell.json";

fn hex(ch: char) -> String {
    ch.to_string().repeat(64)
}

fn baseline() -> Value {
    json!({
        "schema_revision": 1,
        "provenance": "archived-research",
        "about": ["synthetic test fixture"],
        "sources": [
            { "path": SOURCE_PATH, "blake3": blake3::hash(SOURCE_BODY.as_bytes()).to_hex().to_string() }
        ],
        "archived_instances": [{
            "name": "inst-a",
            "cell": "demo-cell",
            "nonce": hex('a'),
            "archive_origin": "demo/archive/a.json",
            "archive_blake3": hex('b'),
            "source_recipe": { "source": "gen-src", "name": "demo" },
            "node_order": "physical",
            "node_ids": [907, 41, 12],
            "edges": [[0, 1], [0, 2]],
            "h": [0, -500, 31],
            "j": [-1000, 62],
            "scale": 1000,
            "offset_milli": "0",
            "qubo_offset_milli": null,
            "family_id": null,
            "descriptor_digest": null,
            "params_hex": null,
            "postprocess_policy": { "family": "ice-tiles", "method": "anneal:b0=2", "budget_s": 0.25 },
            "witnesses": [
                { "name": "qpu-best", "spins": [1, -1, 1], "claimed_energy_milli": "1593" },
                { "name": "pp-state", "missing": "state not stored in the archive" }
            ]
        }]
    })
}

/// Builds the synthetic repository and writes `fixture` into it.
fn repo(fixture: &Value) -> Result<TempDir, Box<dyn std::error::Error>> {
    let dir = TempDir::new()?;
    let root = dir.path();
    fs::create_dir_all(root.join("problems/demo/sources"))?;
    fs::create_dir_all(root.join("problems/demo/fixtures"))?;
    fs::write(root.join(SOURCE_PATH), SOURCE_BODY)?;
    let manifest = json!({
        "sources": [
            { "id": "gen-src", "kind": "generator" },
            { "id": "solver-src", "kind": "solver" }
        ]
    });
    fs::write(
        root.join("problems/sources.json"),
        serde_json::to_vec(&manifest)?,
    )?;
    fs::write(root.join(FIXTURE), serde_json::to_vec_pretty(fixture)?)?;
    Ok(dir)
}

fn run(dir: &TempDir) -> Result<(), Vec<String>> {
    let root = dir.path();
    fixtures::validate(
        &root.join(FIXTURE),
        root,
        &root.join("problems/sources.json"),
    )
}

/// Applies `edit` to the baseline and returns the validation outcome.
fn mutated(
    edit: impl FnOnce(&mut Value),
) -> Result<Result<(), Vec<String>>, Box<dyn std::error::Error>> {
    let mut fixture = baseline();
    edit(&mut fixture);
    let dir = repo(&fixture)?;
    Ok(run(&dir))
}

fn expect_error(edit: impl FnOnce(&mut Value), needle: &str) -> TestResult {
    match mutated(edit)? {
        Ok(()) => Err(format!("expected an error containing {needle:?}, got success").into()),
        Err(errors) if errors.iter().any(|e| e.contains(needle)) => Ok(()),
        Err(errors) => Err(format!("no error contains {needle:?}: {errors:#?}").into()),
    }
}

fn expect_ok(edit: impl FnOnce(&mut Value)) -> TestResult {
    mutated(edit)?.map_err(|errors| format!("unexpected errors: {errors:#?}").into())
}

fn inst(fixture: &mut Value) -> &mut Value {
    &mut fixture["archived_instances"][0]
}

#[test]
fn valid_fixture_passes() -> TestResult {
    expect_ok(|_| {})
}

#[test]
fn wrong_header_fails() -> TestResult {
    expect_error(|f| f["schema_revision"] = json!(2), "schema_revision")?;
    expect_error(|f| f["provenance"] = json!("specified"), "provenance")?;
    expect_error(|f| f["about"] = json!([]), "about is empty")
}

#[test]
fn unknown_key_fails() -> TestResult {
    expect_error(|f| f["extra"] = json!(1), "unknown field `extra`")?;
    expect_error(|f| inst(f)["extra"] = json!(1), "unknown field `extra`")?;
    expect_error(
        |f| inst(f)["witnesses"][0]["extra"] = json!(1),
        "unknown field `extra`",
    )
}

#[test]
fn source_hash_mismatch_fails() -> TestResult {
    expect_error(
        |f| f["sources"][0]["blake3"] = json!(hex('0')),
        "blake3 mismatch",
    )
}

#[test]
fn unsafe_source_path_fails() -> TestResult {
    for path in ["/etc/passwd", "../x", "a/./b", "a//b", "a\\b"] {
        expect_error(
            |f| f["sources"][0]["path"] = json!(path),
            "unsafe source path",
        )?;
    }
    expect_error(
        |f| f["sources"][0]["path"] = json!("problems/demo/none.rs"),
        "cannot read source",
    )
}

#[test]
fn duplicate_instance_name_fails() -> TestResult {
    expect_error(
        |f| {
            let copy = inst(f).clone();
            if let Some(list) = f["archived_instances"].as_array_mut() {
                list.push(copy);
            }
        },
        "duplicate instance name inst-a",
    )
}

#[test]
fn bad_archive_blake3_fails() -> TestResult {
    expect_error(
        |f| {
            if let Some(map) = inst(f).as_object_mut() {
                map.remove("archive_blake3");
            }
        },
        "archive_blake3",
    )?;
    expect_error(
        |f| inst(f)["archive_blake3"] = json!("A".repeat(64)),
        "archive_blake3 is not 64 lowercase hex",
    )?;
    expect_error(
        |f| inst(f)["archive_blake3"] = json!("b".repeat(63)),
        "archive_blake3 is not 64 lowercase hex",
    )
}

#[test]
fn unknown_source_recipe_fails() -> TestResult {
    expect_error(
        |f| inst(f)["source_recipe"]["source"] = json!("nope"),
        "not in the source manifest",
    )?;
    expect_error(
        |f| inst(f)["source_recipe"]["source"] = json!("solver-src"),
        "is not a generator",
    )
}

#[test]
fn node_order_mismatch_fails() -> TestResult {
    expect_error(
        |f| inst(f)["node_order"] = json!("ascending"),
        "not strictly increasing",
    )?;
    expect_error(
        |f| inst(f)["node_order"] = json!("sideways"),
        "unknown node_order",
    )
}

#[test]
fn duplicate_node_id_fails() -> TestResult {
    expect_error(
        |f| inst(f)["node_ids"] = json!([907, 41, 41]),
        "node_ids has a duplicate",
    )
}

#[test]
fn unsorted_edge_fails() -> TestResult {
    expect_error(
        |f| inst(f)["edges"] = json!([[0, 2], [0, 1]]),
        "not strictly increasing",
    )
}

#[test]
fn duplicate_edge_fails() -> TestResult {
    expect_error(
        |f| inst(f)["edges"] = json!([[0, 1], [0, 1]]),
        "not strictly increasing",
    )
}

#[test]
fn self_loop_fails() -> TestResult {
    expect_error(
        |f| inst(f)["edges"] = json!([[1, 1], [1, 2]]),
        "edge [1, 1]",
    )
}

#[test]
fn edge_out_of_range_fails() -> TestResult {
    expect_error(
        |f| inst(f)["edges"] = json!([[0, 1], [0, 3]]),
        "edge [0, 3]",
    )
}

#[test]
fn length_mismatch_fails() -> TestResult {
    expect_error(|f| inst(f)["h"] = json!([0, -500]), "h has 2 entries")?;
    expect_error(|f| inst(f)["j"] = json!([-1000]), "j has 1 entries")
}

#[test]
fn i64_as_number_fails() -> TestResult {
    expect_error(|f| inst(f)["offset_milli"] = json!(0), "offset_milli")?;
    expect_error(
        |f| inst(f)["witnesses"][0]["claimed_energy_milli"] = json!(-1031),
        "claimed_energy_milli",
    )
}

#[test]
fn bad_family_fields_fails() -> TestResult {
    expect_error(
        |f| inst(f)["descriptor_digest"] = json!("0123456789"),
        "descriptor_digest",
    )?;
    expect_error(|f| inst(f)["params_hex"] = json!("abc"), "params_hex")?;
    expect_error(|f| inst(f)["family_id"] = json!(""), "family_id")?;
    expect_ok(|f| {
        inst(f)["descriptor_digest"] = json!(hex('c'));
        inst(f)["params_hex"] = json!("00ff");
        inst(f)["family_id"] = json!("ice");
    })
}

#[test]
fn missing_postprocess_policy_fails() -> TestResult {
    expect_error(
        |f| {
            if let Some(map) = inst(f).as_object_mut() {
                map.remove("postprocess_policy");
            }
        },
        "postprocess_policy",
    )?;
    expect_error(
        |f| inst(f)["postprocess_policy"]["method"] = json!(""),
        "postprocess_policy needs",
    )?;
    expect_error(
        |f| inst(f)["postprocess_policy"]["budget_s"] = json!(-1.0),
        "budget_s",
    )
}

#[test]
fn witness_shape_fails() -> TestResult {
    expect_error(
        |f| inst(f)["witnesses"][0]["missing"] = json!("also missing"),
        "needs either spins",
    )?;
    expect_error(
        |f| {
            if let Some(map) = inst(f)["witnesses"][1].as_object_mut() {
                map.remove("missing");
            }
        },
        "needs either spins",
    )?;
    expect_error(
        |f| inst(f)["witnesses"][1]["name"] = json!("qpu-best"),
        "duplicate witness name",
    )
}

#[test]
fn bad_spin_fails() -> TestResult {
    expect_error(
        |f| inst(f)["witnesses"][0]["spins"] = json!([1, 0, 1]),
        "not 1 or -1",
    )?;
    expect_error(
        |f| inst(f)["witnesses"][0]["spins"] = json!([1, -1]),
        "spins has 2 entries",
    )
}

#[test]
fn witness_energy_mismatch_fails() -> TestResult {
    expect_error(
        |f| inst(f)["witnesses"][0]["claimed_energy_milli"] = json!("1592"),
        "claimed energy 1592 but the energy is 1593",
    )
}

#[test]
fn offset_enters_energy() -> TestResult {
    expect_ok(|f| {
        inst(f)["offset_milli"] = json!("-5");
        inst(f)["witnesses"][0]["claimed_energy_milli"] = json!("1588");
    })?;
    expect_ok(|f| inst(f)["qubo_offset_milli"] = json!("-5"))
}

#[test]
fn reports_every_violation() -> TestResult {
    let errors = match mutated(|f| {
        inst(f)["scale"] = json!(1);
        inst(f)["nonce"] = json!("xyz");
    })? {
        Ok(()) => return Err("expected errors".into()),
        Err(errors) => errors,
    };
    for needle in ["scale is 1", "nonce is not"] {
        if !errors.iter().any(|e| e.contains(needle)) {
            return Err(format!("missing {needle:?} in {errors:#?}").into());
        }
    }
    if !errors.iter().all(|e| e.contains("cell.json")) {
        return Err(format!("an error does not name the file: {errors:#?}").into());
    }
    Ok(())
}

#[test]
fn recorded_fixtures_are_valid() -> TestResult {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = root.join("problems/sources.json");
    let mut failures = Vec::new();
    for family in fs::read_dir(root.join("problems"))? {
        let dir: PathBuf = family?.path().join("fixtures");
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "json") {
                if let Err(errors) = fixtures::validate(&path, &root, &manifest) {
                    failures.extend(errors);
                }
            }
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("{failures:#?}").into())
    }
}
