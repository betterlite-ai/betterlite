use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;

use super::error::TokenRecordError;
use super::file::{prepare_private_directory, private_document};
use super::record::CliTokenRecord;

pub async fn write_token_file(
    files: &PrivateFiles,
    path: &Path,
    record: &CliTokenRecord,
) -> Result<(), TokenRecordError> {
    prepare_private_directory(files, path).await?;
    files
        .write_private(path, private_document(record)?)
        .await
        .map_err(|source| TokenRecordError::Write { source })
}
