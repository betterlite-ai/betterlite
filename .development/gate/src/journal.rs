use std::fs;
use std::path::{Path, PathBuf};

use crate::error::GateError;
use crate::git::git;

pub(crate) fn journal_dir(root: &Path, task_id: &str) -> Result<PathBuf, GateError> {
    let common = git(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let directory = PathBuf::from(common.trim())
        .join("betterlite")
        .join(task_id);
    fs::create_dir_all(&directory).map_err(|source| GateError::Write {
        path: directory.clone(),
        source,
    })?;
    Ok(directory)
}
