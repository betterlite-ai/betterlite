use std::collections::BTreeSet;
use std::fs;
use std::iter;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::GateError;
use crate::process::check_command;

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    manifest_path: PathBuf,
    license: Option<String>,
    targets: Vec<Target>,
}

#[derive(Debug, Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct MemberManifest {
    #[serde(default)]
    lints: MemberLints,
}

#[derive(Debug, Default, Deserialize)]
struct MemberLints {
    #[serde(default)]
    workspace: bool,
}

fn package_problems(package: &Package) -> Result<Vec<String>, GateError> {
    let binaries = package
        .targets
        .iter()
        .filter(|target| target.kind.iter().any(|kind| kind == "bin"));
    let names: BTreeSet<&str> = iter::once(package.name.as_str())
        .chain(binaries.map(|target| target.name.as_str()))
        .collect();
    let mut problems: Vec<String> = names
        .into_iter()
        .filter(|name| name.to_ascii_lowercase().contains("litellm"))
        .map(|name| format!("{name}: keep the LiteLLM name out of crate and binary names"))
        .collect();
    if package.license.as_deref() != Some("MIT") {
        problems.push(format!(
            "{}: license must be MIT through license.workspace = true",
            package.name
        ));
    }
    let path = &package.manifest_path;
    let text = fs::read_to_string(path).map_err(|source| GateError::Read {
        path: path.clone(),
        source,
    })?;
    let manifest: MemberManifest = toml::from_str(&text).map_err(|source| GateError::Toml {
        path: path.clone(),
        source: Box::new(source),
    })?;
    if !manifest.lints.workspace {
        problems.push(format!(
            "{}: add [lints] workspace = true so the workspace lint table applies",
            package.name
        ));
    }
    Ok(problems)
}

pub(crate) fn check(root: &Path) -> Result<Vec<String>, GateError> {
    if !root.join("Cargo.toml").exists() {
        return Ok(vec!["no Cargo.toml at the repository root".to_owned()]);
    }
    let output = check_command("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
        .current_dir(root)
        .output()
        .map_err(|source| GateError::Spawn {
            program: "cargo metadata".to_owned(),
            source,
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8(output.stderr).unwrap_or_default();
        return Ok(vec![format!("cargo metadata failed: {}", stderr.trim())]);
    }
    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).map_err(|source| GateError::Json {
            what: "cargo metadata".to_owned(),
            source,
        })?;
    let mut problems = Vec::new();
    for package in &metadata.packages {
        problems.extend(package_problems(package)?);
    }
    Ok(problems)
}
