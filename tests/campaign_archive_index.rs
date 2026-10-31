//! Checks for `evaluation/qblock/archive-index.json`, the index of archived research rows, and
//! for the rule that production code reads no research path.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Fails the test with an error. `assert!` is denied in functions that return a `Result`.
macro_rules! ensure {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(format!($($msg)+).into());
        }
    };
    ($cond:expr) => {
        ensure!($cond, "check failed: {}", stringify!($cond))
    };
}

macro_rules! ensure_eq {
    ($left:expr, $right:expr) => {
        ensure_eq!($left, $right, "values differ")
    };
    ($left:expr, $right:expr, $($msg:tt)+) => {
        let (left, right) = (&$left, &$right);
        if left != right {
            return Err(format!("{}: {left:?} != {right:?}", format!($($msg)+)).into());
        }
    };
}

const INDEX: &str = "evaluation/qblock/archive-index.json";
const CELLS: [&str; 7] = [
    "ice-charge",
    "ice-fields-h500",
    "ice-fields-h1000",
    "two-commodity-routing",
    "ice-twisted-w62-dimer1200",
    "ice-twisted-w62-dimer1350",
    "ice-twisted-w45-dimer1200",
];
const ENERGY_FIELDS: [&str; 3] = ["qpu80_best", "qpu80_greedy", "qpu80_pp"];
const RESEARCH_MARKERS: [&str; 5] = ["/home/", "quip-data", "qpu-f17", "qpu-suite", "new-pow"];

#[derive(Deserialize)]
struct Index {
    schema_revision: u32,
    research_root: String,
    source: Source,
    cells: Vec<Cell>,
}

#[derive(Deserialize)]
struct Source {
    path: String,
    blake3: String,
}

#[derive(Deserialize)]
struct Cell {
    cell: String,
    rows: Vec<Row>,
}

#[derive(Deserialize)]
struct Row {
    nonce: String,
    model: String,
    model_blake3: String,
    qpu80_best: String,
    qpu80_greedy: String,
    qpu80_pp: String,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_index() -> Result<Index, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(root().join(INDEX))?)?)
}

fn is_digest(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_relative(path: &str) -> bool {
    !path.starts_with('/')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[test]
fn index_has_seven_cells_of_100_rows() -> TestResult {
    let index = load_index()?;
    ensure_eq!(index.schema_revision, 1);
    let names: Vec<&str> = index.cells.iter().map(|c| c.cell.as_str()).collect();
    ensure_eq!(names, CELLS);
    let mut total = 0;
    for cell in &index.cells {
        ensure_eq!(cell.rows.len(), 100, "{}", cell.cell);
        let nonces: BTreeSet<&str> = cell.rows.iter().map(|r| r.nonce.as_str()).collect();
        ensure_eq!(nonces.len(), 100, "{}: duplicate nonce", cell.cell);
        total += cell.rows.len();
    }
    ensure_eq!(total, 700);
    Ok(())
}

#[test]
fn index_paths_are_relative() -> TestResult {
    let index = load_index()?;
    ensure!(index.research_root.starts_with('/'));
    ensure!(is_relative(&index.source.path), "{}", index.source.path);
    for cell in &index.cells {
        for row in &cell.rows {
            ensure!(is_relative(&row.model), "{}: {}", cell.cell, row.model);
        }
    }
    let raw = fs::read_to_string(root().join(INDEX))?;
    ensure_eq!(raw.matches(&index.research_root).count(), 1);
    let value: Value = serde_json::from_str(&raw)?;
    ensure_eq!(value["research_root"], index.research_root.as_str());
    Ok(())
}

#[test]
fn index_digests_are_well_formed() -> TestResult {
    let index = load_index()?;
    ensure!(is_digest(&index.source.blake3));
    for cell in &index.cells {
        for row in &cell.rows {
            ensure!(is_digest(&row.model_blake3), "{}: {}", cell.cell, row.nonce);
            ensure!(is_digest(&row.nonce), "{}: {}", cell.cell, row.nonce);
            for energy in [&row.qpu80_best, &row.qpu80_greedy, &row.qpu80_pp] {
                ensure!(
                    energy.parse::<i64>().is_ok(),
                    "{}: {}: energy {energy:?}",
                    cell.cell,
                    row.nonce
                );
            }
        }
    }
    let raw: Value = serde_json::from_slice(&fs::read(root().join(INDEX))?)?;
    for cell in raw["cells"].as_array().into_iter().flatten() {
        for row in cell["rows"].as_array().into_iter().flatten() {
            for field in ENERGY_FIELDS {
                ensure!(row[field].is_string(), "{field} is not a string");
            }
            ensure!(row["pp_cpu_s"].is_number());
        }
    }
    Ok(())
}

#[test]
fn fixtures_match_index() -> TestResult {
    let index = load_index()?;
    let mut rows: BTreeMap<(&str, &str), (&str, &str)> = BTreeMap::new();
    for cell in &index.cells {
        for row in &cell.rows {
            rows.insert(
                (cell.cell.as_str(), row.nonce.as_str()),
                (row.model.as_str(), row.model_blake3.as_str()),
            );
        }
    }
    let mut failures = Vec::new();
    for family in fs::read_dir(root().join("problems"))? {
        let dir = family?.path().join("fixtures");
        if !dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "json") {
                check_fixture(&path, &rows, &mut failures)?;
            }
        }
    }
    ensure!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

fn check_fixture(
    path: &Path,
    rows: &BTreeMap<(&str, &str), (&str, &str)>,
    failures: &mut Vec<String>,
) -> TestResult {
    let fixture: Value = serde_json::from_slice(&fs::read(path)?)?;
    let name = path.display();
    for inst in fixture["archived_instances"]
        .as_array()
        .into_iter()
        .flatten()
    {
        let field = |key: &str| inst[key].as_str().unwrap_or_default();
        let (cell, nonce) = (field("cell"), field("nonce"));
        match rows.get(&(cell, nonce)) {
            None => failures.push(format!("{name}: {cell} {nonce} is not in the index")),
            Some((model, digest)) => {
                if field("archive_origin") != *model {
                    failures.push(format!("{name}: {cell} {nonce}: model path differs"));
                }
                if field("archive_blake3") != *digest {
                    failures.push(format!("{name}: {cell} {nonce}: model digest differs"));
                }
            }
        }
    }
    Ok(())
}

fn list_sources(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            list_sources(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

#[test]
fn src_reads_no_research_path() -> TestResult {
    let mut files = Vec::new();
    list_sources(&root().join("src"), &mut files)?;
    ensure!(!files.is_empty());
    let mut hits = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file)?;
        for (number, line) in text.lines().enumerate() {
            if RESEARCH_MARKERS.iter().any(|m| line.contains(m)) {
                hits.push(format!("{}:{}: {line}", file.display(), number + 1));
            }
        }
    }
    ensure!(hits.is_empty(), "{}", hits.join("\n"));
    Ok(())
}
