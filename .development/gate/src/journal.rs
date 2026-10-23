use std::fs;
use std::path::{Path, PathBuf};

use crate::error::GateError;
use crate::git::common_dir;

pub(crate) fn journal_dir(root: &Path, task_id: &str) -> Result<PathBuf, GateError> {
    let directory = common_dir(root)?.join("betterlite").join(task_id);
    fs::create_dir_all(&directory).map_err(|source| GateError::Write {
        path: directory.clone(),
        source,
    })?;
    Ok(directory)
}
