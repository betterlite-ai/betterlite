use std::fs;

use serde::Deserialize;
use toml::Table;

use crate::builtins::Context;
use crate::error::GateError;
use crate::git::git;

#[derive(Debug, Default, Deserialize)]
struct RootManifest {
    #[serde(default)]
    workspace: WorkspaceSection,
}

#[derive(Debug, Default, Deserialize)]
struct WorkspaceSection {
    #[serde(default)]
    lints: Table,
}

pub(crate) fn check(context: &Context<'_>) -> Result<Vec<String>, GateError> {
    if context.task.scope.lint_tables {
        return Ok(Vec::new());
    }
    let root = &context.development.root;
    let path = root.join("Cargo.toml");
    let current_text = if path.exists() {
        Some(fs::read_to_string(&path).map_err(|source| GateError::Read {
            path: path.clone(),
            source,
        })?)
    } else {
        None
    };
    let at_base = format!("{}:Cargo.toml", context.facts.base);
    let base_text = if git(root, &["cat-file", "-e", &at_base]).is_ok() {
        Some(git(root, &["show", &at_base])?)
    } else {
        None
    };
    let mut lint_tables = Vec::new();
    for text in [current_text, base_text] {
        let manifest: RootManifest = match text {
            Some(manifest_text) => {
                toml::from_str(&manifest_text).map_err(|source| GateError::Toml {
                    path: path.clone(),
                    source: Box::new(source),
                })?
            }
            None => RootManifest::default(),
        };
        lint_tables.push(manifest.workspace.lints);
    }
    if lint_tables.first() == lint_tables.last() {
        return Ok(Vec::new());
    }
    Ok(vec![
        "[workspace.lints] changed; only a task with scope.lint_tables may change it".to_owned(),
    ])
}
