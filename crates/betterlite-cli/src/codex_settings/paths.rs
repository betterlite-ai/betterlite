use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use soft_canonicalize::soft_canonicalize;

use super::error::CodexSettingsError;

pub fn resolve_settings_path(settings_path: &Path) -> Result<PathBuf, CodexSettingsError> {
    soft_canonicalize(settings_path).map_err(|source| CodexSettingsError::Resolve {
        path: settings_path.to_owned(),
        source,
    })
}

pub fn settings_path_text(path: &Path) -> Result<String, CodexSettingsError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| CodexSettingsError::NonUtf8Path {
            path: path.to_owned(),
        })
}

pub fn codex_configure_state_path(settings_path: &Path) -> Result<PathBuf, CodexSettingsError> {
    let target = resolve_settings_path(settings_path)?;
    let digest = hex::encode(Sha256::digest(target.as_os_str().as_encoded_bytes()));
    let parent = target.parent().map_or_else(PathBuf::new, Path::to_path_buf);
    Ok(parent
        .join(".litellm")
        .join(format!("codex_configure_{digest}.json")))
}
