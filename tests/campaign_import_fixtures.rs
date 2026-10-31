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
        n_nodes: i64,
        n_edges: i64,
        topology_order_digest: String,
        model_digest: String,
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
        sources: Vec<FileRef>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Witness {
        name: String,
        spins_packed: Option<String>,
        claimed_energy_milli: Option<String>,
        missing: Option<String>,
        origin: Option<Origin>,
    }

    /// The archive file a spins witness came from. `array` and `index` are required keys that
    /// may hold null.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Origin {
        path: String,
        blake3: String,
        #[serde(deserialize_with = "required")]
        array: Option<String>,
        #[serde(deserialize_with = "required")]
        index: Option<i64>,
    }

    fn required<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
    where
        D: serde::Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::deserialize(deserializer)
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

    fn check_shape(at: &str, inst: &Instance, errors: &mut Vec<String>) {
        if !matches!(inst.node_order.as_str(), "physical" | "ascending") {
            errors.push(format!("{at}: unknown node_order {:?}", inst.node_order));
        }
        if inst.n_nodes <= 0 {
            errors.push(format!("{at}: n_nodes {} is not positive", inst.n_nodes));
        }
        if inst.n_edges <= 0 {
            errors.push(format!("{at}: n_edges {} is not positive", inst.n_edges));
        }
        for (field, value) in [
            ("topology_order_digest", &inst.topology_order_digest),
            ("model_digest", &inst.model_digest),
        ] {
            if !is_digest(value) {
                errors.push(format!("{at}: {field} is not 64 lowercase hex characters"));
            }
        }
    }

    fn check_policy_sources(at: &str, sources: &[FileRef], errors: &mut Vec<String>) {
        if sources.is_empty() {
            errors.push(format!("{at}: postprocess_policy sources is empty"));
        }
        for source in sources {
            if !is_safe_path(&source.path) {
                errors.push(format!(
                    "{at}: postprocess_policy sources has an unsafe path {:?}",
                    source.path
                ));
            }
            if !is_digest(&source.blake3) {
                errors.push(format!(
                    "{at}: postprocess_policy source {}: blake3 is not 64 lowercase hex characters",
                    source.path
                ));
            }
        }
        if !sources.windows(2).all(|w| w[0].path <= w[1].path) {
            errors.push(format!(
                "{at}: postprocess_policy sources are not sorted by path"
            ));
        }
        let unique: BTreeSet<_> = sources.iter().map(|s| s.path.as_str()).collect();
        if unique.len() != sources.len() {
            errors.push(format!(
                "{at}: postprocess_policy sources has a duplicate path"
            ));
        }
    }

    fn check_origin(at: &str, origin: &Origin, errors: &mut Vec<String>) {
        if !is_safe_path(&origin.path) {
            errors.push(format!("{at}: origin has an unsafe path {:?}", origin.path));
        }
        if !is_digest(&origin.blake3) {
            errors.push(format!(
                "{at}: origin blake3 is not 64 lowercase hex characters"
            ));
        }
        if origin.array.as_deref().is_some_and(str::is_empty) {
            errors.push(format!("{at}: origin array is empty"));
        }
        if origin.index.is_some_and(|i| i < 0) {
            errors.push(format!("{at}: origin index is negative"));
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
        check_policy_sources(at, &policy.sources, errors);
    }

    fn parse_i64(at: &str, field: &str, text: &str, errors: &mut Vec<String>) -> Option<i64> {
        let parsed = text.parse::<i64>().ok();
        if parsed.is_none() {
            errors.push(format!("{at}: {field} {text:?} is not a decimal i64"));
        }
        parsed
    }

    /// Checks the packed spin hex: exactly `ceil(n_nodes / 8)` bytes with zero padding bits.
    fn check_packed(at: &str, packed: &str, n_nodes: i64, errors: &mut Vec<String>) {
        let Ok(nodes) = usize::try_from(n_nodes) else {
            return;
        };
        let bytes = nodes.div_ceil(8);
        if !is_lower_hex(packed) || packed.len() != bytes * 2 {
            errors.push(format!(
                "{at}: spins_packed is not lowercase hex of {bytes} bytes for {nodes} nodes"
            ));
            return;
        }
        let tail = nodes % 8;
        if tail != 0 {
            let last = packed
                .get(packed.len() - 2..)
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            if last.is_some_and(|b| b >> tail != 0) {
                errors.push(format!("{at}: spins_packed has a padding bit set"));
            }
        }
    }

    fn check_witnesses(at: &str, inst: &Instance, errors: &mut Vec<String>) {
        parse_i64(at, "offset_milli", &inst.offset_milli, errors);
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
                &witness.spins_packed,
                &witness.claimed_energy_milli,
                &witness.missing,
            ) {
                (Some(packed), Some(claimed), None) => {
                    parse_i64(&at, "claimed_energy_milli", claimed, errors);
                    check_packed(&at, packed, inst.n_nodes, errors);
                    match &witness.origin {
                        Some(origin) => check_origin(&at, origin, errors),
                        None => errors.push(format!("{at}: a spins witness needs an origin")),
                    }
                }
                (None, None, Some(reason)) => {
                    if reason.is_empty() {
                        errors.push(format!("{at}: missing reason is empty"));
                    }
                    if witness.origin.is_some() {
                        errors.push(format!("{at}: a missing witness must not have an origin"));
                    }
                }
                _ => errors.push(format!(
                    "{at}: needs either spins_packed with claimed_energy_milli, or missing"
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
        check_shape(&at, inst, errors);
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
            "n_nodes": 12,
            "n_edges": 2,
            "topology_order_digest": hex('d'),
            "model_digest": hex('e'),
            "offset_milli": "0",
            "qubo_offset_milli": null,
            "family_id": null,
            "descriptor_digest": null,
            "params_hex": null,
            "postprocess_policy": {
                "family": "ice-tiles",
                "method": "anneal:b0=2",
                "budget_s": 0.25,
                "sources": [
                    { "path": "opt/a.py", "blake3": hex('f') },
                    { "path": "opt/b.py", "blake3": hex('1') }
                ]
            },
            "witnesses": [
                {
                    "name": "qpu-best",
                    "spins_packed": "0f05",
                    "claimed_energy_milli": "1593",
                    "origin": { "path": "demo/qpu/a.npz", "blake3": hex('2'), "array": "spins", "index": 3 }
                },
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

fn origin(fixture: &mut Value) -> &mut Value {
    &mut inst(fixture)["witnesses"][0]["origin"]
}

fn sources(fixture: &mut Value) -> &mut Value {
    &mut inst(fixture)["postprocess_policy"]["sources"]
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
fn i64_as_number_fails() -> TestResult {
    expect_error(
        |f| inst(f)["offset_milli"] = json!(0),
        "offset_milli is a JSON number",
    )?;
    expect_error(
        |f| inst(f)["qubo_offset_milli"] = json!(0),
        "qubo_offset_milli is a JSON number",
    )?;
    expect_error(
        |f| inst(f)["witnesses"][0]["claimed_energy_milli"] = json!(1593),
        "claimed_energy_milli is a JSON number",
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
fn reports_every_violation() -> TestResult {
    let errors = match mutated(|f| {
        inst(f)["n_nodes"] = json!(0);
        inst(f)["nonce"] = json!("xyz");
    })? {
        Ok(()) => return Err("expected errors".into()),
        Err(errors) => errors,
    };
    for needle in ["n_nodes 0", "nonce is not"] {
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

#[test]
fn bad_node_order_fails() -> TestResult {
    expect_error(
        |f| inst(f)["node_order"] = json!("sideways"),
        "unknown node_order",
    )?;
    expect_ok(|f| inst(f)["node_order"] = json!("ascending"))
}

#[test]
fn bad_counts_fail() -> TestResult {
    expect_error(
        |f| inst(f)["n_nodes"] = json!(0),
        "n_nodes 0 is not positive",
    )?;
    expect_error(
        |f| inst(f)["n_edges"] = json!(-3),
        "n_edges -3 is not positive",
    )?;
    expect_error(|f| inst(f)["n_nodes"] = json!("12"), "cannot parse")
}

#[test]
fn bad_digests_fail() -> TestResult {
    for field in ["topology_order_digest", "model_digest"] {
        expect_error(
            |f| inst(f)[field] = json!("A".repeat(64)),
            &format!("{field} is not 64 lowercase hex"),
        )?;
        expect_error(
            |f| inst(f)[field] = json!("a".repeat(63)),
            &format!("{field} is not 64 lowercase hex"),
        )?;
        expect_error(
            |f| {
                if let Some(map) = inst(f).as_object_mut() {
                    map.remove(field);
                }
            },
            field,
        )?;
    }
    Ok(())
}

#[test]
fn bad_packed_spins_fail() -> TestResult {
    for packed in ["0f", "0f0500", "0F05", "0f0g", "0f15"] {
        expect_error(
            |f| inst(f)["witnesses"][0]["spins_packed"] = json!(packed),
            "spins_packed",
        )?;
    }
    expect_error(
        |f| inst(f)["witnesses"][0]["spins_packed"] = json!("0f15"),
        "padding bit",
    )?;
    expect_ok(|f| inst(f)["witnesses"][0]["spins_packed"] = json!("ff0f"))
}

#[test]
fn origin_rules_fail() -> TestResult {
    expect_error(
        |f| {
            if let Some(map) = inst(f)["witnesses"][0].as_object_mut() {
                map.remove("origin");
            }
        },
        "needs an origin",
    )?;
    expect_error(
        |f| {
            inst(f)["witnesses"][1]["origin"] =
                json!({ "path": "a/b", "blake3": hex('2'), "array": null, "index": null });
        },
        "must not have an origin",
    )?;
    for path in ["/abs", "../x", "a//b"] {
        expect_error(
            |f| origin(f)["path"] = json!(path),
            "origin has an unsafe path",
        )?;
    }
    expect_error(
        |f| origin(f)["blake3"] = json!("B".repeat(64)),
        "origin blake3 is not 64 lowercase hex",
    )?;
    expect_error(|f| origin(f)["array"] = json!(""), "origin array is empty")?;
    expect_error(
        |f| origin(f)["index"] = json!(-1),
        "origin index is negative",
    )?;
    for key in ["array", "index"] {
        expect_error(
            |f| {
                if let Some(map) = origin(f).as_object_mut() {
                    map.remove(key);
                }
            },
            key,
        )?;
    }
    expect_ok(|f| {
        origin(f)["array"] = json!(null);
        origin(f)["index"] = json!(null);
    })
}

#[test]
fn policy_sources_rules_fail() -> TestResult {
    expect_error(|f| *sources(f) = json!([]), "sources is empty")?;
    expect_error(
        |f| sources(f)[0]["path"] = json!("../x"),
        "sources has an unsafe path",
    )?;
    expect_error(
        |f| sources(f)[1]["blake3"] = json!("a".repeat(63)),
        "source opt/b.py: blake3 is not 64 lowercase hex",
    )?;
    expect_error(
        |f| *sources(f) = json!([{ "path": "opt/b.py", "blake3": hex('1') }, { "path": "opt/a.py", "blake3": hex('f') }]),
        "not sorted by path",
    )?;
    expect_error(
        |f| *sources(f) = json!([{ "path": "opt/a.py", "blake3": hex('1') }, { "path": "opt/a.py", "blake3": hex('f') }]),
        "duplicate path",
    )?;
    expect_error(
        |f| {
            if let Some(map) = inst(f)["postprocess_policy"].as_object_mut() {
                map.remove("sources");
            }
        },
        "sources",
    )
}

#[test]
fn removed_array_fields_fail() -> TestResult {
    expect_error(
        |f| inst(f)["node_ids"] = json!([1]),
        "unknown field `node_ids`",
    )?;
    expect_error(
        |f| inst(f)["witnesses"][0]["spins"] = json!([1]),
        "unknown field `spins`",
    )
}

#[test]
fn offsets_are_decimal_strings() -> TestResult {
    expect_ok(|f| {
        inst(f)["offset_milli"] = json!("-5");
        inst(f)["qubo_offset_milli"] = json!("-5");
    })?;
    expect_error(
        |f| inst(f)["offset_milli"] = json!("5.0"),
        "offset_milli \"5.0\" is not a decimal i64",
    )
}

mod pinning {
    use ising_generators::descriptor::topology_order_digest;
    use ising_generators::instance::{Instance, InstanceLimits, InstanceParts};

    use super::TestResult;

    const TOPOLOGY_ORDER_DIGEST: &str =
        "c30b0cb2b13ee310c3b6c896c68fd62641ae0221f70353af055b38a0f2e13a78";
    const MODEL_DIGEST: &str = "c421554f0a6d5d9e1df450809473ab23542b9c3b0ca1188803f05d641e9c720f";
    const INSTANCE_DIGEST: &str =
        "f20d5158084abd1651b738cfc304e4be7c54239553d178964feac0b6874956c5";
    /// Format byte, descriptor digest, topology order digest, and nonce.
    const HEAD_LEN: usize = 97;

    fn parts() -> InstanceParts {
        InstanceParts {
            descriptor_digest: [0x22; 32],
            topology_order_digest: [0; 32],
            nonce: Some([0x11; 32]),
            node_ids: vec![907, 41, 12],
            edges: vec![(0, 1), (0, 2)],
            h: vec![0, -500, 31],
            j: vec![-1000, 62],
            offset_milli: -5,
            metadata: vec![0, 0],
        }
    }

    fn hex(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push(char::from(DIGITS[usize::from(b >> 4)]));
            out.push(char::from(DIGITS[usize::from(b & 15)]));
        }
        out
    }

    fn same(label: &str, got: &str, want: &str) -> TestResult {
        if got == want {
            Ok(())
        } else {
            Err(format!("{label}: got {got}, want {want}").into())
        }
    }

    fn derive(context: &str, body: &[u8]) -> String {
        let mut hasher = blake3::Hasher::new_derive_key(context);
        hasher.update(body);
        hasher.finalize().to_hex().to_string()
    }

    /// The expected body: counts, ids, edges, h, j, offset, then the empty metadata region.
    fn expected_body() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend(3u32.to_le_bytes());
        for id in [907u32, 41, 12] {
            body.extend(id.to_le_bytes());
        }
        body.extend(2u32.to_le_bytes());
        for (a, b) in [(0u32, 1u32), (0, 2)] {
            body.extend(a.to_le_bytes());
            body.extend(b.to_le_bytes());
        }
        for h in [0i32, -500, 31] {
            body.extend(h.to_le_bytes());
        }
        for j in [-1000i32, 62] {
            body.extend(j.to_le_bytes());
        }
        body.extend((-5i64).to_le_bytes());
        body.extend([0u8, 0]);
        body
    }

    #[test]
    fn digests_match_the_pinned_values() -> TestResult {
        let mut parts = parts();
        // FAM-19 hashes native node ids, so map the dense edge positions through the ids.
        let native: Vec<(u32, u32)> = parts
            .edges
            .iter()
            .map(|&(a, b)| (parts.node_ids[a as usize], parts.node_ids[b as usize]))
            .collect();
        let topology = topology_order_digest(&parts.node_ids, &native)
            .ok_or("topology order digest overflow")?;
        parts.topology_order_digest = topology;
        let instance = Instance::new(parts, &InstanceLimits::FAM_37, &[], None)
            .map_err(|code| format!("instance rejected: {code:?}"))?;
        let bytes = instance.canonical_bytes()?;
        let body = bytes
            .get(HEAD_LEN..)
            .ok_or("canonical bytes shorter than the head")?;
        same("body", &hex(body), &hex(&expected_body()))?;
        same(
            "topology_order_digest",
            &hex(&topology),
            TOPOLOGY_ORDER_DIGEST,
        )?;
        same(
            "model_digest",
            &derive("isingmark archived model v1", body),
            MODEL_DIGEST,
        )?;
        same("instance digest", &hex(&instance.digest()), INSTANCE_DIGEST)?;
        Ok(())
    }
}
