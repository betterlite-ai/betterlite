use std::path::Path;

use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles, StagedFile};

use super::error::TokenRecordError;
use super::file::prepare_private_directory;
use super::record::CliTokenRecord;

pub async fn stage_token_file(
    files: &PrivateFiles,
    path: &Path,
    record: &CliTokenRecord,
) -> Result<StagedFile, TokenRecordError> {
    prepare_private_directory(files, path).await?;
    let document =
        PrivateDocument::json(record).map_err(|source| TokenRecordError::Encode { source })?;
    files
        .stage_private(path, document)
        .await
        .map_err(|source| TokenRecordError::Stage { source })
}
