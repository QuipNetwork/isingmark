// SPDX-License-Identifier: AGPL-3.0-or-later
//! `campaign family-check`: score bounded oracle fixtures through the registry.
//!
//! A fixture directory holds `<name>.instance.json` files, each with a
//! `<name>.oracle.json` that lists witnesses and their expected validity and exact
//! energy. No miner or solver runs.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use super::super::{
    contracts::{EncodingOutcome, EvalError, EvaluationContext, FamilyEvaluator, Witness},
    instance::{load_instance, read_bounded, Instance},
    CampaignError,
};
use super::FamilyRegistry;

/// The largest number of instance fixtures one check reads.
pub const MAX_FIXTURES: u64 = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Oracle {
    witnesses: Vec<OracleWitness>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OracleWitness {
    spins: Vec<i8>,
    valid: bool,
    energy_milli: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FamilyCheckFailure {
    pub fixture: String,
    /// The oracle witness index, or `None` for a fault of the whole fixture.
    pub witness: Option<usize>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FamilyCheckReport {
    pub family: String,
    /// The revision of the first fixture. Every fixture must name the same one.
    pub revision: String,
    pub fixtures: u64,
    pub witnesses: u64,
    pub failures: Vec<FamilyCheckFailure>,
}

fn fixture_paths(fixtures: &Path) -> Result<Vec<std::path::PathBuf>, CampaignError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(fixtures)? {
        let path = entry?.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".instance.json"))
        {
            if u64::try_from(paths.len()).map_err(|_| CampaignError::Overflow)? >= MAX_FIXTURES {
                return Err(CampaignError::TooLarge {
                    what: "fixtures",
                    limit: MAX_FIXTURES,
                });
            }
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Score each oracle witness and record every mismatch against `name`.
fn check_oracle(
    evaluator: &dyn FamilyEvaluator,
    instance: &Instance,
    oracle: Oracle,
    name: &str,
    report: &mut FamilyCheckReport,
) {
    let context = EvaluationContext {
        tolerances: serde_json::Value::Null,
    };
    for (index, expected) in oracle.witnesses.into_iter().enumerate() {
        report.witnesses += 1;
        let witness = Witness {
            instance_id: instance.instance_id.clone(),
            spins: expected.spins,
            design: None,
            route: None,
            controls: None,
        };
        let reason = match evaluator.evaluate(instance, &witness, &context) {
            Ok(evaluation)
                if evaluation.valid == expected.valid
                    && evaluation.energy_milli == Some(expected.energy_milli) =>
            {
                continue;
            }
            Ok(evaluation) => format!(
                "expected valid {} at {} milli, got valid {} at {:?} milli",
                expected.valid, expected.energy_milli, evaluation.valid, evaluation.energy_milli
            ),
            Err(error) => error.to_string(),
        };
        report.failures.push(FamilyCheckFailure {
            fixture: name.to_owned(),
            witness: Some(index),
            reason,
        });
    }
}

/// Check every fixture of `family` in `fixtures` against its oracle.
///
/// # Errors
/// `family_unsupported` before any file is read, `TooLarge` for more than
/// [`MAX_FIXTURES`] instance files, `InvalidRun` for a directory without fixtures,
/// and I/O or parse errors for unreadable files. Mismatches are report failures.
pub fn family_check(
    registry: &FamilyRegistry,
    family: &str,
    fixtures: &Path,
) -> Result<FamilyCheckReport, CampaignError> {
    if !registry.has_family(family) {
        return Err(EvalError {
            code: "family_unsupported",
            reason: family.to_owned(),
        }
        .into());
    }
    let paths = fixture_paths(fixtures)?;
    let mut report = FamilyCheckReport {
        family: family.to_owned(),
        revision: String::new(),
        fixtures: 0,
        witnesses: 0,
        failures: Vec::new(),
    };
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .trim_end_matches(".instance.json")
            .to_owned();
        report.fixtures += 1;
        let mut fail = |witness, reason: String| {
            report.failures.push(FamilyCheckFailure {
                fixture: name.clone(),
                witness,
                reason,
            });
        };
        let instance = load_instance(&path)?;
        if report.revision.is_empty() {
            report.revision.clone_from(&instance.model_version);
        }
        if instance.family != family || instance.model_version != report.revision {
            fail(
                None,
                format!(
                    "fixture is {} revision {}",
                    instance.family, instance.model_version
                ),
            );
            continue;
        }
        let evaluator = match registry.for_instance(&instance) {
            Ok(evaluator) => evaluator,
            Err(error) => {
                fail(None, error.to_string());
                continue;
            }
        };
        match evaluator.encode(&instance) {
            Ok(EncodingOutcome::Exact(_)) => {}
            Ok(_) => {
                fail(None, "no exact encoding".to_owned());
                continue;
            }
            Err(error) => {
                fail(None, error.to_string());
                continue;
            }
        }
        let oracle_path = path.with_file_name(format!("{name}.oracle.json"));
        let oracle: Oracle = serde_json::from_slice(&read_bounded(&oracle_path, "oracle")?)?;
        check_oracle(evaluator, &instance, oracle, &name, &mut report);
    }
    if report.fixtures == 0 {
        return Err(CampaignError::InvalidRun(format!(
            "{} holds no *.instance.json fixtures",
            fixtures.display()
        )));
    }
    Ok(report)
}
