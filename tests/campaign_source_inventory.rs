//! Integrity checks for the research source manifest under
//! `research/qblock_eval/sources/selected`.

use std::fs;
use std::path::Path;

use serde_json::{json, Value};
use tempfile::TempDir;

mod inventory {
    use std::collections::{BTreeSet, HashMap};
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde::Deserialize;

    const SCHEMA: &str = "isingmark.research-sources/1";
    const MAX_BYTES: u64 = 524_288;
    const TICKET_PREFIXES: [&str; 9] = ["SC", "CPU", "CU", "MT", "DW", "CO", "VAL", "IM", "EXP"];

    #[derive(Deserialize)]
    struct Manifest {
        schema: String,
        digest: String,
        sources: Vec<Source>,
        duplicates: Vec<Duplicate>,
    }

    #[derive(Deserialize)]
    struct Source {
        id: String,
        origin: String,
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

    /// Checks the manifest in `dir` against the copied files and returns every violation.
    pub fn validate(dir: &Path) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let manifest: Manifest = match fs::read(dir.join("manifest.json"))
            .map_err(|e| e.to_string())
            .and_then(|raw| serde_json::from_slice(&raw).map_err(|e| e.to_string()))
        {
            Ok(manifest) => manifest,
            Err(e) => return Err(vec![format!("cannot read manifest.json: {e}")]),
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
            if matches!(source.kind.as_str(), "solver" | "generator" | "postprocess")
                && source.functions.is_empty()
            {
                errors.push(format!("{origin}: {} source has no functions", source.kind));
            }
            if !is_safe_origin(origin) {
                errors.push(format!("unsafe origin {origin:?}"));
                continue;
            }
            check_file(dir, source, &mut errors);
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

        let mut files = BTreeSet::new();
        if let Err(e) = list_files(dir, dir, &mut files) {
            errors.push(format!("cannot list {}: {e}", dir.display()));
        }
        for file in files {
            let listed = manifest.sources.iter().any(|s| s.origin == file);
            if file != "manifest.json" && !listed {
                errors.push(format!("unlisted file {file}"));
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn check_file(dir: &Path, source: &Source, errors: &mut Vec<String>) {
        let origin = &source.origin;
        let content = match fs::read(dir.join(origin)) {
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
        if len > MAX_BYTES {
            errors.push(format!("{origin}: {len} bytes exceeds {MAX_BYTES}"));
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

fn source(id: &str, origin: &str, content: &[u8]) -> Value {
    json!({
        "id": id,
        "origin": origin,
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
        "schema": "isingmark.research-sources/1",
        "research_root": "/research",
        "digest": "blake3",
        "sources": sources,
        "duplicates": duplicates,
        "retained_evidence": [],
        "skipped_checks": []
    })
}

fn write_file(dir: &Path, origin: &str, content: &[u8]) -> TestResult {
    let path = dir.join(origin);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

fn write_manifest(dir: &Path, manifest: &Value) -> TestResult {
    fs::write(
        dir.join("manifest.json"),
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
fn oversize_fails() -> TestResult {
    let tmp = TempDir::new()?;
    let content = vec![b'x'; 524_289];
    write_file(tmp.path(), "big.rs", &content)?;
    write_manifest(
        tmp.path(),
        &manifest(&[source("big", "big.rs", &content)], &[]),
    )?;
    expect_error(tmp.path(), "exceeds 524288")?;
    Ok(())
}

#[test]
fn max_size_file_passes() -> TestResult {
    let tmp = TempDir::new()?;
    let content = vec![b'x'; 524_288];
    write_file(tmp.path(), "big.rs", &content)?;
    write_manifest(
        tmp.path(),
        &manifest(&[source("big", "big.rs", &content)], &[]),
    )?;
    expect_valid(tmp.path())?;
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

#[test]
fn recorded_manifest_is_valid() -> TestResult {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("research/qblock_eval/sources/selected");
    expect_valid(&dir)
}
