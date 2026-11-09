use std::fs;
use std::path::{Path, PathBuf};

use betterlite_logging::private_json::files::PrivateFiles;

use super::error::TokenRecordError;
use super::record::CliTokenRecord;

#[must_use]
pub fn token_file_path(home: &Path) -> PathBuf {
    let mut path = home.to_path_buf();
    path.extend([".litellm", "token.json"]);
    path
}

#[must_use]
pub fn read_token_file(path: &Path) -> Option<CliTokenRecord> {
    let Ok(raw) = fs::read_to_string(path) else {
        return None;
    };
    serde_json::from_str(&raw).ok()
}

pub(super) async fn prepare_private_directory(
    files: &PrivateFiles,
    path: &Path,
) -> Result<(), TokenRecordError> {
    let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
    files
        .ensure_private_dir(&directory)
        .await
        .map_err(|source| TokenRecordError::Directory { source })
}
