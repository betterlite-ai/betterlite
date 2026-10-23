use std::fs;
use std::path::Path;

use crate::error::GateError;
use crate::git::git;
use crate::spec::litellm::Litellm;
use crate::yaml::read_yaml;

pub(crate) fn write(root: &Path, checkout: &Path) -> Result<Vec<String>, GateError> {
    let folder = root.join(".development");
    let litellm: Litellm = read_yaml(&folder.join("litellm.yaml"))?;
    let mut arguments = vec![
        "ls-tree",
        "-r",
        "--name-only",
        litellm.commit.as_str(),
        "--",
    ];
    arguments.extend(litellm.roots.iter().map(String::as_str));
    let listing = git(checkout, &arguments)?;
    let paths: Vec<&str> = listing.lines().collect();
    let path = folder.join("litellm-inventory.txt");
    let text = format!("commit {}\n{}\n", litellm.commit, paths.join("\n"));
    fs::write(&path, text).map_err(|source| GateError::Write { path, source })?;
    Ok(vec![format!(
        "wrote .development/litellm-inventory.txt: {} paths at {}",
        paths.len(),
        litellm.commit
    )])
}
