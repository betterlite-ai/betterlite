mod tests;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use toml::Table;

use crate::error::GateError;
use crate::git::git;

#[derive(Debug, Default, Deserialize)]
struct Manifest {
    #[serde(default)]
    dependencies: Table,
    #[serde(default, rename = "dev-dependencies")]
    dev_dependencies: Table,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: Table,
    #[serde(default)]
    workspace: WorkspaceDependencies,
}

#[derive(Debug, Default, Deserialize)]
struct WorkspaceDependencies {
    #[serde(default)]
    dependencies: Table,
}

pub(crate) fn dependency_names(
    manifest_text: &str,
    path: &Path,
) -> Result<BTreeSet<String>, GateError> {
    let manifest: Manifest = toml::from_str(manifest_text).map_err(|source| GateError::Toml {
        path: path.to_owned(),
        source: Box::new(source),
    })?;
    let tables = [
        manifest.dependencies,
        manifest.dev_dependencies,
        manifest.build_dependencies,
        manifest.workspace.dependencies,
    ];
    Ok(tables
        .iter()
        .flat_map(Table::keys)
        .filter(|name| !name.starts_with("betterlite"))
        .cloned()
        .collect())
}

pub(crate) fn added_dependencies(root: &Path, base: &str) -> Result<BTreeSet<String>, GateError> {
    let listed = git(
        root,
        &[
            "ls-files",
            "--",
            "Cargo.toml",
            "crates/**/Cargo.toml",
            "tools/**/Cargo.toml",
        ],
    )?;
    let mut now = BTreeSet::new();
    let mut before = BTreeSet::new();
    for manifest in listed.lines() {
        let path = root.join(manifest);
        let text = fs::read_to_string(&path).map_err(|source| GateError::Read {
            path: path.clone(),
            source,
        })?;
        now.extend(dependency_names(&text, &path)?);
        let at_base = format!("{base}:{manifest}");
        if git(root, &["cat-file", "-e", &at_base]).is_ok() {
            before.extend(dependency_names(&git(root, &["show", &at_base])?, &path)?);
        }
    }
    Ok(now.difference(&before).cloned().collect())
}
