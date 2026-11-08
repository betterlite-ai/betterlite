use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::checks::{Outcome, Selection};
use crate::error::GateError;
use crate::facts::{Facts, Tree};

#[derive(Serialize, Deserialize)]
struct Record {
    head: String,
    base: String,
    #[serde(default)]
    clean: bool,
    only: Vec<String>,
    skip: Vec<String>,
    checks: Vec<CheckRecord>,
}

#[derive(Serialize, Deserialize)]
struct CheckRecord {
    id: String,
    seconds: u64,
    outcome: State,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum State {
    Pass,
    Skip,
    Fail,
}

pub(super) fn start(log_dir: &Path) -> Result<PathBuf, GateError> {
    let runs = log_dir.join("runs");
    fs::create_dir_all(&runs).map_err(|source| GateError::Write {
        path: runs.clone(),
        source,
    })?;
    tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&runs)
        .map(tempfile::TempDir::keep)
        .map_err(|source| GateError::Write { path: runs, source })
}

pub(super) fn save(
    root: &Path,
    log_dir: &Path,
    facts: &Facts,
    selection: &Selection,
    outcomes: &[(String, u64, Outcome)],
) -> Result<(), GateError> {
    let finished = Tree::read(root)?;
    let checks = outcomes
        .iter()
        .map(|(id, seconds, outcome)| CheckRecord {
            id: id.clone(),
            seconds: *seconds,
            outcome: match outcome {
                Outcome::Pass | Outcome::Reused(_) => State::Pass,
                Outcome::Skip(_) => State::Skip,
                Outcome::Fail { .. } => State::Fail,
            },
        })
        .collect();
    let record = Record {
        clean: facts.tree.clean && finished.clean && facts.tree.head == finished.head,
        head: finished.head,
        base: facts.base.clone(),
        only: selection.only.clone(),
        skip: selection.skip.clone(),
        checks,
    };
    let path = log_dir.join("result.json");
    let text = serde_json::to_vec_pretty(&record).map_err(|source| GateError::Json {
        what: "gate history".to_owned(),
        source,
    })?;
    fs::write(&path, text).map_err(|source| GateError::Write { path, source })
}

pub(crate) fn passed(log_dir: &Path, facts: &Facts) -> Result<BTreeMap<String, String>, GateError> {
    let mut found = BTreeMap::new();
    if !facts.tree.clean {
        return Ok(found);
    }
    for run in runs(log_dir)? {
        let path = run.join("result.json");
        if !path.exists() {
            continue;
        }
        let text = fs::read(&path).map_err(|source| GateError::Read {
            path: path.clone(),
            source,
        })?;
        let record: Record = serde_json::from_slice(&text).map_err(|source| GateError::Json {
            what: format!("gate history {}", path.display()),
            source,
        })?;
        if !record.clean || record.head != facts.tree.head || record.base != facts.base {
            continue;
        }
        let name = run
            .file_name()
            .map_or_else(String::new, |name| name.display().to_string());
        for check in record.checks {
            if check.outcome == State::Pass {
                found.insert(check.id, name.clone());
            }
        }
    }
    Ok(found)
}

pub(crate) fn runs(log_dir: &Path) -> Result<BTreeSet<PathBuf>, GateError> {
    let runs = log_dir.join("runs");
    if !runs.is_dir() {
        return Ok(BTreeSet::new());
    }
    let entries = fs::read_dir(&runs).map_err(|source| GateError::Read {
        path: runs.clone(),
        source,
    })?;
    let mut found = BTreeSet::new();
    for entry in entries {
        let run = entry.map_err(|source| GateError::Read {
            path: runs.clone(),
            source,
        })?;
        found.insert(run.path());
    }
    Ok(found)
}

pub(crate) fn unfinished(log_dir: &Path, earlier: &BTreeSet<PathBuf>) -> Result<bool, GateError> {
    Ok(runs(log_dir)?.iter().any(|run| {
        !earlier.contains(run)
            && !run.join("result.json").exists()
            && !run.join("error.txt").exists()
    }))
}
