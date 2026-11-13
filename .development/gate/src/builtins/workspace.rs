use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::GateError;
use crate::process::check_command;

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Package {
    pub(super) name: String,
    pub(super) manifest_path: PathBuf,
    pub(super) license: Option<String>,
    pub(super) targets: Vec<Target>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Target {
    pub(super) name: String,
    pub(super) kind: Vec<String>,
    pub(super) src_path: PathBuf,
}

pub(super) fn packages(root: &Path) -> Result<Vec<Package>, GateError> {
    let output = check_command("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .current_dir(root)
        .output()
        .map_err(|source| GateError::Spawn {
            program: "cargo metadata".to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(GateError::Failed {
            program: "cargo metadata".to_owned(),
            stderr: String::from_utf8(output.stderr)
                .unwrap_or_default()
                .trim()
                .to_owned(),
        });
    }
    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).map_err(|source| GateError::Json {
            what: "cargo metadata".to_owned(),
            source,
        })?;
    Ok(metadata.packages)
}
