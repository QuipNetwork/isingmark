//! Integrity checks for the research source manifest `problems/sources.json`.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use tempfile::TempDir;

mod inventory {
    use std::collections::{BTreeSet, HashMap};
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde::Deserialize;

    const MANIFEST: &str = "problems/sources.json";
    const SCHEMA: &str = "isingmark.research-sources/2";
    const KINDS: [&str; 7] = [
        "solver",
        "generator",
        "postprocess",
        "build",
        "target-manifest",
        "topology",
        "script",
    ];
    /// The one copied source that lives outside `problems/`. It is renamed on copy, so the
    /// file-name rule skips it.
    const SPEC_PATH: &str = "fixtures/suite-live-2026-09-21.spec.json";
    const TICKET_PREFIXES: [&str; 9] = ["SC", "CPU", "CU", "MT", "DW", "CO", "VAL", "IM", "EXP"];

    #[derive(Deserialize)]
    struct Manifest {
        schema: String,
        digest: String,
        sources: Vec<Source>,
        duplicates: Vec<Duplicate>,
    }

    #[derive(Deserialize)]
    pub struct SkippedCheck {
        pub check: String,
    }

    #[derive(Deserialize)]
    struct SkippedChecks {
        skipped_checks: Vec<SkippedCheck>,
    }

    /// Returns the `skipped_checks` recorded in `problems/sources.json` under `root`.
    pub fn skipped_checks(root: &Path) -> Result<Vec<SkippedCheck>, String> {
        let raw = fs::read(root.join(MANIFEST)).map_err(|e| e.to_string())?;
        let parsed: SkippedChecks = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
        Ok(parsed.skipped_checks)
    }

    #[derive(Deserialize)]
    struct Source {
        id: String,
        origin: String,
        path: String,
        kind: String,
        bytes: u64,
        blake3: String,
        functions: Vec<Function>,
    }

    #[derive(Deserialize)]
    struct Function {
        ticket: String,
    }

    #[derive(Deserialize)]
    struct Duplicate {
        origin: String,
        blake3: String,
        same_as: String,
    }

    /// True for `^(SC|CPU|CU|MT|DW|CO|VAL|IM|EXP)-[0-9]+$`.
    fn is_ticket(ticket: &str) -> bool {
        ticket.split_once('-').is_some_and(|(prefix, number)| {
            TICKET_PREFIXES.contains(&prefix)
                && !number.is_empty()
                && number.bytes().all(|b| b.is_ascii_digit())
        })
    }

    fn is_digest(digest: &str) -> bool {
        digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }

    fn is_safe_origin(origin: &str) -> bool {
        !origin.starts_with('/')
            && !origin.contains('\\')
            && origin
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
    }

    /// A copy lives under `problems/`, except for the one topology spec under `fixtures/`.
    fn is_valid_path(path: &str) -> bool {
        is_safe_origin(path) && (path.starts_with("problems/") || path == SPEC_PATH)
    }

    fn file_name(path: &str) -> &str {
        path.rsplit('/').next().unwrap_or(path)
    }

    /// Lists files under `dir` as `/`-separated paths relative to `root`.
    fn list_files(root: &Path, dir: &Path, out: &mut BTreeSet<String>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let path: PathBuf = entry?.path();
            if path.is_dir() {
                list_files(root, &path, out)?;
            } else if let Ok(rel) = path.strip_prefix(root) {
                let parts: Vec<_> = rel.iter().map(|p| p.to_string_lossy()).collect();
                out.insert(parts.join("/"));
            }
        }
        Ok(())
    }

    /// Checks `problems/sources.json` under `root` against the copied files and returns every
    /// violation.
    pub fn validate(root: &Path) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let manifest: Manifest = match fs::read(root.join(MANIFEST))
            .map_err(|e| e.to_string())
            .and_then(|raw| serde_json::from_slice(&raw).map_err(|e| e.to_string()))
        {
            Ok(manifest) => manifest,
            Err(e) => return Err(vec![format!("cannot read {MANIFEST}: {e}")]),
        };

        if manifest.schema != SCHEMA {
            errors.push(format!(
                "schema is {:?}, expected {SCHEMA:?}",
                manifest.schema
            ));
        }
        if manifest.digest != "blake3" {
            errors.push(format!(
                "digest is {:?}, expected \"blake3\"",
                manifest.digest
            ));
        }

        let mut ids: HashMap<&str, &Source> = HashMap::new();
        let mut digests = BTreeSet::new();
        let mut origins = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for source in &manifest.sources {
            let origin = &source.origin;
            if ids.insert(&source.id, source).is_some() {
                errors.push(format!("duplicate id {}", source.id));
            }
            if !digests.insert(source.blake3.as_str()) {
                errors.push(format!("duplicate blake3 {} ({origin})", source.blake3));
            }
            if !origins.insert(origin.as_str()) {
                errors.push(format!("duplicate origin {origin}"));
            }
            if !is_digest(&source.blake3) {
                errors.push(format!(
                    "{origin}: blake3 is not 64 lowercase hex characters"
                ));
            }
            for function in &source.functions {
                if !is_ticket(&function.ticket) {
                    errors.push(format!("{origin}: bad ticket {:?}", function.ticket));
                }
            }
            if !KINDS.contains(&source.kind.as_str()) {
                errors.push(format!("{origin}: unknown kind {:?}", source.kind));
            }
            if matches!(source.kind.as_str(), "solver" | "generator" | "postprocess")
                && source.functions.is_empty()
            {
                errors.push(format!("{origin}: {} source has no functions", source.kind));
            }
            let mut usable = true;
            if !is_safe_origin(origin) {
                errors.push(format!("unsafe origin {origin:?}"));
                usable = false;
            }
            if !is_valid_path(&source.path) {
                errors.push(format!("{origin}: bad path {:?}", source.path));
                usable = false;
            } else if source.path != SPEC_PATH && file_name(&source.path) != file_name(origin) {
                errors.push(format!(
                    "{origin}: path {:?} has a different file name",
                    source.path
                ));
            }
            if !paths.insert(source.path.as_str()) {
                errors.push(format!("duplicate path {}", source.path));
            }
            if usable {
                check_file(root, source, &mut errors);
            }
        }

        for duplicate in &manifest.duplicates {
            let origin = &duplicate.origin;
            if !is_safe_origin(origin) {
                errors.push(format!("unsafe origin {origin:?}"));
            }
            if !origins.insert(origin.as_str()) {
                errors.push(format!("duplicate origin {origin}"));
            }
            match ids.get(duplicate.same_as.as_str()) {
                None => errors.push(format!(
                    "{origin}: same_as names unknown source id {}",
                    duplicate.same_as
                )),
                Some(source) if source.blake3 != duplicate.blake3 => errors.push(format!(
                    "{origin}: duplicate blake3 differs from source {}",
                    source.id
                )),
                Some(_) => {}
            }
        }

        check_unlisted(root, &paths, &mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Reports every file under `problems/*/sources/` that no source lists as its `path`.
    fn check_unlisted(root: &Path, paths: &BTreeSet<&str>, errors: &mut Vec<String>) {
        let mut files = BTreeSet::new();
        if let Ok(families) = fs::read_dir(root.join("problems")) {
            for family in families.flatten() {
                let sources = family.path().join("sources");
                if !sources.is_dir() {
                    continue;
                }
                if let Err(e) = list_files(root, &sources, &mut files) {
                    errors.push(format!("cannot list {}: {e}", sources.display()));
                }
            }
        }
        for file in files {
            if !paths.contains(file.as_str()) {
                errors.push(format!("unlisted file {file}"));
            }
        }
    }

    fn check_file(root: &Path, source: &Source, errors: &mut Vec<String>) {
        let origin = &source.origin;
        let content = match fs::read(root.join(&source.path)) {
            Ok(content) => content,
            Err(e) => {
                errors.push(format!("{origin}: missing copied file ({e})"));
                return;
            }
        };
        let len = content.len() as u64;
        if len != source.bytes {
            errors.push(format!(
                "{origin}: size mismatch, manifest {} but file {len}",
                source.bytes
            ));
        }
        if blake3::hash(&content).to_hex().as_str() != source.blake3 {
            errors.push(format!("{origin}: blake3 mismatch"));
        }
    }
}

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Builds an entry whose copy sits at `problems/shared/sources/<origin>`.
fn source(id: &str, origin: &str, content: &[u8]) -> Value {
    json!({
        "id": id,
        "origin": origin,
        "path": format!("problems/shared/sources/{origin}"),
        "kind": "solver",
        "bytes": content.len(),
        "blake3": hash(content),
        "functions": [{ "name": "sample", "ticket": "CPU-2" }],
        "provenance": {
            "rng": null,
            "matching_library": null,
            "topology": null,
            "postprocess": null,
            "license": "unknown",
            "redistribution": "unrecorded"
        },
        "notes": "synthetic"
    })
}

fn manifest(sources: &[Value], duplicates: &[Value]) -> Value {
    json!({
        "schema": "isingmark.research-sources/2",
        "research_root": "/research",
        "digest": "blake3",
        "sources": sources,
        "duplicates": duplicates,
        "retained_evidence": [],
        "skipped_checks": []
    })
}

/// Writes the copy of `origin` where `source` expects it.
fn write_file(dir: &Path, origin: &str, content: &[u8]) -> TestResult {
    let path = dir.join("problems/shared/sources").join(origin);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn write_manifest(dir: &Path, manifest: &Value) -> TestResult {
    fs::create_dir_all(dir.join("problems"))?;
    fs::write(
        dir.join("problems/sources.json"),
        serde_json::to_vec_pretty(manifest)?,
    )?;
    Ok(())
}

/// Writes one valid source file `a/one.rs` and returns its manifest entry.
fn one_source(dir: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    write_file(dir, "a/one.rs", b"fn one() {}")?;
    Ok(source("one", "a/one.rs", b"fn one() {}"))
}

fn expect_error(dir: &Path, needle: &str) -> TestResult {
    let errors = inventory::validate(dir).err().unwrap_or_default();
    if errors.iter().any(|e| e.contains(needle)) {
        Ok(())
    } else {
        Err(format!("expected an error containing {needle:?}, got {errors:?}").into())
    }
}

fn expect_valid(dir: &Path) -> TestResult {
    inventory::validate(dir).map_err(|errors| format!("unexpected errors: {errors:?}").into())
}

#[test]
fn valid_manifest_passes() -> TestResult {
    let tmp = TempDir::new()?;
    let one = one_source(tmp.path())?;
    let dup = json!({ "origin": "b/copy.rs", "blake3": hash(b"fn one() {}"), "same_as": "one" });
    write_manifest(tmp.path(), &manifest(&[one], &[dup]))?;
    expect_valid(tmp.path())?;
    Ok(())
}

#[test]
fn missing_source_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let one = source("one", "a/gone.rs", b"x");
    write_manifest(tmp.path(), &manifest(&[one], &[]))?;
    expect_error(tmp.path(), "a/gone.rs")?;
    Ok(())
}

#[test]
fn duplicate_id_fails() -> TestResult {
    let tmp = TempDir::new()?;
    write_file(tmp.path(), "a.rs", b"a")?;
    write_file(tmp.path(), "b.rs", b"b")?;
    let sources = vec![source("same", "a.rs", b"a"), source("same", "b.rs", b"b")];
    write_manifest(tmp.path(), &manifest(&sources, &[]))?;
    expect_error(tmp.path(), "duplicate id")?;
    Ok(())
}

#[test]
fn duplicate_digest_fails() -> TestResult {
    let tmp = TempDir::new()?;
    write_file(tmp.path(), "a.rs", b"same")?;
    write_file(tmp.path(), "b.rs", b"same")?;
    let sources = vec![source("a", "a.rs", b"same"), source("b", "b.rs", b"same")];
    write_manifest(tmp.path(), &manifest(&sources, &[]))?;
    expect_error(tmp.path(), "duplicate blake3")?;
    Ok(())
}

#[test]
fn changed_hash_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let one = one_source(tmp.path())?;
    write_file(tmp.path(), "a/one.rs", b"fn onE() {}")?;
    write_manifest(tmp.path(), &manifest(&[one], &[]))?;
    expect_error(tmp.path(), "blake3 mismatch")?;
    Ok(())
}

#[test]
fn size_mismatch_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let mut one = one_source(tmp.path())?;
    one["bytes"] = json!(3);
    write_manifest(tmp.path(), &manifest(&[one], &[]))?;
    expect_error(tmp.path(), "size mismatch")?;
    Ok(())
}

#[test]
fn unsafe_origin_fails() -> TestResult {
    for origin in ["../x", "/abs", "a//b", "a/./b"] {
        let tmp = TempDir::new()?;
        write_manifest(tmp.path(), &manifest(&[source("x", origin, b"x")], &[]))?;
        expect_error(tmp.path(), "unsafe origin")?;
    }
    Ok(())
}

#[test]
fn bad_ticket_fails() -> TestResult {
    for ticket in ["QUI-1999", "cpu-2"] {
        let tmp = TempDir::new()?;
        let mut one = one_source(tmp.path())?;
        one["functions"] = json!([{ "name": "sample", "ticket": ticket }]);
        write_manifest(tmp.path(), &manifest(&[one], &[]))?;
        expect_error(tmp.path(), "bad ticket")?;
    }
    Ok(())
}

#[test]
fn solver_without_functions_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let mut one = one_source(tmp.path())?;
    one["functions"] = json!([]);
    write_manifest(tmp.path(), &manifest(&[one], &[]))?;
    expect_error(tmp.path(), "no functions")?;
    Ok(())
}

#[test]
fn dangling_duplicate_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let one = one_source(tmp.path())?;
    let unknown = json!({ "origin": "b.rs", "blake3": hash(b"fn one() {}"), "same_as": "nope" });
    write_manifest(
        tmp.path(),
        &manifest(std::slice::from_ref(&one), &[unknown]),
    )?;
    expect_error(tmp.path(), "unknown source id")?;

    let mismatch = json!({ "origin": "b.rs", "blake3": hash(b"other"), "same_as": "one" });
    write_manifest(tmp.path(), &manifest(&[one], &[mismatch]))?;
    expect_error(tmp.path(), "duplicate blake3 differs")?;
    Ok(())
}

#[test]
fn unlisted_file_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let one = one_source(tmp.path())?;
    write_file(tmp.path(), "a/extra.rs", b"extra")?;
    write_manifest(tmp.path(), &manifest(&[one], &[]))?;
    expect_error(tmp.path(), "a/extra.rs")?;
    Ok(())
}

fn recorded_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

#[test]
fn recorded_manifest_is_valid() -> TestResult {
    expect_valid(&recorded_dir())
}

#[test]
fn recorded_manifest_skips_sc2_and_sc7() -> TestResult {
    let checks = inventory::skipped_checks(&recorded_dir())?;
    for key in ["SC-2", "SC-7"] {
        if !checks.iter().any(|c| c.check.contains(key)) {
            return Err(format!("skipped_checks does not name {key}").into());
        }
    }
    Ok(())
}

#[test]
fn unknown_kind_fails() -> TestResult {
    for kind in ["fixture", "banana"] {
        let tmp = TempDir::new()?;
        let mut one = one_source(tmp.path())?;
        one["kind"] = json!(kind);
        write_manifest(tmp.path(), &manifest(&[one], &[]))?;
        expect_error(tmp.path(), "unknown kind")?;
    }
    Ok(())
}

#[test]
fn wrong_schema_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let mut m = manifest(&[], &[]);
    m["schema"] = json!("other/1");
    write_manifest(tmp.path(), &m)?;
    expect_error(tmp.path(), "schema is")
}

#[test]
fn wrong_digest_header_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let mut m = manifest(&[], &[]);
    m["digest"] = json!("sha256");
    write_manifest(tmp.path(), &m)?;
    expect_error(tmp.path(), "digest is")
}

#[test]
fn malformed_digest_fails() -> TestResult {
    let good = hash(b"fn one() {}");
    for bad in [good.to_uppercase(), good[..63].to_string()] {
        let tmp = TempDir::new()?;
        let mut one = one_source(tmp.path())?;
        one["blake3"] = json!(bad);
        write_manifest(tmp.path(), &manifest(&[one], &[]))?;
        expect_error(tmp.path(), "not 64 lowercase hex")?;
    }
    Ok(())
}

#[test]
fn repeated_origin_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let one = one_source(tmp.path())?;
    let dup = json!({ "origin": "a/one.rs", "blake3": hash(b"fn one() {}"), "same_as": "one" });
    write_manifest(tmp.path(), &manifest(&[one], &[dup]))?;
    expect_error(tmp.path(), "duplicate origin")
}

#[test]
fn bad_path_fails() -> TestResult {
    for path in [
        "elsewhere/one.rs",
        "problems/../one.rs",
        "/problems/one.rs",
        "fixtures/other.spec.json",
    ] {
        let tmp = TempDir::new()?;
        let mut one = one_source(tmp.path())?;
        one["path"] = json!(path);
        write_manifest(tmp.path(), &manifest(&[one], &[]))?;
        expect_error(tmp.path(), "bad path")?;
    }
    Ok(())
}

#[test]
fn path_with_other_file_name_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let mut one = one_source(tmp.path())?;
    one["path"] = json!("problems/shared/sources/a/renamed.rs");
    write_manifest(tmp.path(), &manifest(&[one], &[]))?;
    expect_error(tmp.path(), "different file name")?;
    Ok(())
}

#[test]
fn spec_path_outside_problems_is_allowed() -> TestResult {
    let tmp = TempDir::new()?;
    let spec = "fixtures/suite-live-2026-09-21.spec.json";
    fs::create_dir_all(tmp.path().join("fixtures"))?;
    fs::write(tmp.path().join(spec), b"{}")?;
    let mut entry = source("spec", "suite-live-2026-09-21.spec.json", b"{}");
    entry["path"] = json!(spec);
    entry["kind"] = json!("topology");
    write_manifest(tmp.path(), &manifest(&[entry], &[]))?;
    expect_valid(tmp.path())
}
