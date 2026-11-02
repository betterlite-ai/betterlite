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
