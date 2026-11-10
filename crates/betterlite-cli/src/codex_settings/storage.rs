use std::path::Path;

use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles, StagedFile};
use toml_edit::DocumentMut;

use super::error::CodexSettingsError;

pub(crate) async fn stage_settings(
    files: &PrivateFiles,
    target: &Path,
    document: &DocumentMut,
) -> Result<StagedFile, CodexSettingsError> {
    let bytes = document.to_string().into_bytes();
    files
        .stage_private(
            target,
            PrivateDocument {
                suffix: None,
                bytes,
            },
        )
        .await
        .map_err(|source| CodexSettingsError::StageSettings {
            path: target.to_owned(),
            source,
        })
}

// PrivateFiles has no plain unlink; discarding a staged path removes it and treats a missing file as done
pub(crate) async fn remove_path(files: &PrivateFiles, path: &Path) -> Result<(), PrivateJsonError> {
    files
        .discard_staged_json(&StagedFile {
            path: path.to_owned(),
        })
        .await
}
