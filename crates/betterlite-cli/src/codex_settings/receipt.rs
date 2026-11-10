use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs;
use toml_edit::DocumentMut;

use super::OWNED_PATHS;
use super::error::CodexSettingsError;
use super::paths::{codex_configure_state_path, resolve_settings_path, settings_path_text};
use super::snapshot::{self, Snapshot, SnapshotError};

pub const RECEIPT_VERSION: u8 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexReceipt {
    #[serde(default = "default_version")]
    pub version: u8,
    pub settings_path: String,
    pub file_existed: bool,
    pub providers_existed: bool,
    pub previous: BTreeMap<String, Option<String>>,
    pub written: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ReceiptSource {
    #[error("it cannot be read: {0}")]
    Read(#[source] io::Error),
    #[error("it is not valid receipt JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("it declares receipt version {0}, and betterlite writes version 1")]
    Version(u8),
    #[error("it names another settings file")]
    Scope,
    #[error("its previous and written maps name different fields")]
    Keys,
    #[error("it names fields betterlite does not own")]
    Ownership,
    #[error("a saved value is not a TOML snapshot")]
    Snapshot(#[source] SnapshotError),
}

const fn default_version() -> u8 {
    1
}

pub async fn read_receipt(
    settings_path: &Path,
) -> Result<Option<CodexReceipt>, CodexSettingsError> {
    let state_path = codex_configure_state_path(settings_path)?;
    let bytes = match fs::read(&state_path).await {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(CodexSettingsError::Receipt {
                path: state_path,
                source: ReceiptSource::Read(source),
            });
        }
    };
    let receipt = serde_json::from_slice::<CodexReceipt>(&bytes).map_err(|source| {
        CodexSettingsError::Receipt {
            path: state_path.clone(),
            source: ReceiptSource::Json(source),
        }
    })?;
    validate(&receipt, settings_path, &state_path)?;
    Ok(Some(receipt))
}

fn validate(
    receipt: &CodexReceipt,
    settings_path: &Path,
    state_path: &Path,
) -> Result<(), CodexSettingsError> {
    validate_scope(receipt, settings_path, state_path)?;
    validate_fields(receipt, state_path)
}

fn validate_scope(
    receipt: &CodexReceipt,
    settings_path: &Path,
    state_path: &Path,
) -> Result<(), CodexSettingsError> {
    if receipt.version != RECEIPT_VERSION {
        return Err(CodexSettingsError::Receipt {
            path: state_path.to_owned(),
            source: ReceiptSource::Version(receipt.version),
        });
    }
    let resolved = resolve_settings_path(settings_path)?;
    if receipt.settings_path != settings_path_text(&resolved)? {
        return Err(CodexSettingsError::Receipt {
            path: state_path.to_owned(),
            source: ReceiptSource::Scope,
        });
    }
    Ok(())
}

fn validate_fields(receipt: &CodexReceipt, state_path: &Path) -> Result<(), CodexSettingsError> {
    if !receipt.previous.keys().eq(receipt.written.keys()) {
        return Err(CodexSettingsError::Receipt {
            path: state_path.to_owned(),
            source: ReceiptSource::Keys,
        });
    }
    if !receipt
        .written
        .keys()
        .all(|path| OWNED_PATHS.contains(&path.as_str()))
    {
        return Err(CodexSettingsError::Receipt {
            path: state_path.to_owned(),
            source: ReceiptSource::Ownership,
        });
    }
    for saved in receipt.previous.values().flatten() {
        Snapshot::parse(saved).map_err(|source| CodexSettingsError::Receipt {
            path: state_path.to_owned(),
            source: ReceiptSource::Snapshot(source),
        })?;
    }
    Ok(())
}

#[must_use]
pub fn is_ours(document: &DocumentMut, path: &str, receipt: &CodexReceipt) -> bool {
    receipt.written.get(path)
        == Some(&snapshot::fingerprint(
            snapshot::snapshot(document, path).as_ref(),
        ))
}

mod tests;
