use std::path::Path;

use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};

use super::error::TokenRecordError;

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
