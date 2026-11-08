use std::collections::BTreeSet;
use std::fs;
use std::iter;
use std::path::Path;

use serde::Deserialize;

use crate::builtins::workspace::{self, Package};
use crate::error::GateError;

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
    let mut problems = Vec::new();
    for package in &workspace::packages(root)? {
        problems.extend(package_problems(package)?);
    }
    Ok(problems)
}
