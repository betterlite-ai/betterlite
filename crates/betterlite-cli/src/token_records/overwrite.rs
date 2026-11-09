use std::path::Path;

use betterlite_logging::private_json::files::PrivateFiles;

use super::error::TokenRecordError;
use super::record::{CliTokenRecord, without_secret};

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
