use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::checks::{Outcome, Selection};
use crate::error::GateError;
use crate::git::git;

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
    base: &str,
    selection: &Selection,
    outcomes: &[(String, u64, Outcome)],
) -> Result<(), GateError> {
    let checks: Vec<_> = outcomes
        .iter()
        .map(|(id, seconds, outcome)| {
            let state = match outcome {
                Outcome::Pass => "pass",
                Outcome::Skip(_) => "skip",
                Outcome::Fail { .. } => "fail",
            };
            serde_json::json!({"id": id, "seconds": seconds, "outcome": state})
        })
        .collect();
    let value = serde_json::json!({
        "head": git(root, &["rev-parse", "HEAD"])?.trim(), "base": base,
        "only": selection.only, "skip": selection.skip, "checks": checks,
    });
    let path = log_dir.join("result.json");
    let text = serde_json::to_vec_pretty(&value).map_err(|source| GateError::Json {
        what: "gate history".to_owned(),
        source,
    })?;
    fs::write(&path, text).map_err(|source| GateError::Write { path, source })
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
