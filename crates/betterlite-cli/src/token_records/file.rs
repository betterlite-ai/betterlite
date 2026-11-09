use std::fs;
use std::path::{Path, PathBuf};

use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles, StagedFile};

use super::error::TokenRecordError;
use super::record::{CliTokenRecord, without_secret};

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

pub async fn stage_token_file(
    files: &PrivateFiles,
    path: &Path,
    record: &CliTokenRecord,
) -> Result<StagedFile, TokenRecordError> {
    let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
    files
        .ensure_private_dir(&directory)
        .await
        .map_err(|source| TokenRecordError::Directory { source })?;
    let document =
        PrivateDocument::json(record).map_err(|source| TokenRecordError::Stage { source })?;
    files
        .stage_private(path, document)
        .await
        .map_err(|source| TokenRecordError::Stage { source })
}

pub async fn commit_token_file(
    files: &PrivateFiles,
    staged: &StagedFile,
    path: &Path,
) -> Result<(), TokenRecordError> {
    files
        .commit_staged_json(staged, path)
        .await
        .map_err(|source| TokenRecordError::Commit { source })
}

pub async fn write_token_file(
    files: &PrivateFiles,
    path: &Path,
    record: &CliTokenRecord,
) -> Result<(), TokenRecordError> {
    let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
    files
        .ensure_private_dir(&directory)
        .await
        .map_err(|source| TokenRecordError::Directory { source })?;
    let document =
        PrivateDocument::json(record).map_err(|source| TokenRecordError::Write { source })?;
    files
        .write_private(path, document)
        .await
        .map_err(|source| TokenRecordError::Write { source })
}

pub async fn overwrite_token_file_secret(
    files: &PrivateFiles,
    path: &Path,
    record: &CliTokenRecord,
) -> Result<(), TokenRecordError> {
    let stripped = without_secret(record);
    files
        .overwrite_private_json(path, &stripped)
        .await
        .map_err(|source| TokenRecordError::Overwrite { source })
}
