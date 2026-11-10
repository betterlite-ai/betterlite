use std::path::Path;

use betterlite_logging::private_json::files::{PrivateFiles, StagedFile};
use toml_edit::{DocumentMut, TableLike};

use super::document::read_settings;
use super::error::CodexSettingsError;
use super::paths::{codex_configure_state_path, resolve_settings_path};
use super::receipt::{CodexReceipt, ReceiptSource, is_ours, read_receipt};
use super::snapshot::{Snapshot, snapshot, with_value};
use super::storage::{remove_path, stage_settings};
use super::{MODEL_PROVIDERS_KEY, OWNED_PATHS};

#[derive(Debug)]
pub struct CodexUnconfigureOutcome {
    pub restored: Vec<String>,
    pub kept: Vec<String>,
    pub file_removed: bool,
}

pub async fn unconfigure_codex_settings(
    settings_path: &Path,
    files: &PrivateFiles,
) -> Result<CodexUnconfigureOutcome, CodexSettingsError> {
    let current = read_settings(settings_path).await?;
    let Some(receipt) = read_receipt(settings_path).await? else {
        return Err(CodexSettingsError::NothingToUndo);
    };
    let state_path = codex_configure_state_path(settings_path)?;
    let ours = owned_now(settings_path, &current, &receipt);
    let restored = restore_owned(&current, &receipt, &ours, &state_path)?;
    let target = resolve_settings_path(settings_path)?;
    let file_removed =
        restored.to_string().trim().is_empty() && !(receipt.file_existed && target.exists());
    let staged = if file_removed {
        None
    } else {
        Some(stage_settings(files, &target, &restored).await?)
    };
    commit_undo(files, &target, staged.as_ref(), &state_path).await?;
    Ok(CodexUnconfigureOutcome {
        restored: ours
            .iter()
            .filter(|path| snapshot(&current, path) != snapshot(&restored, path))
            .map(|path| (*path).to_owned())
            .collect(),
        kept: OWNED_PATHS
            .iter()
            .copied()
            .filter(|path| {
                receipt.written.contains_key(*path)
                    && !ours.contains(path)
                    && snapshot(&current, path).is_some()
            })
            .map(str::to_owned)
            .collect(),
        file_removed,
    })
}

fn owned_now(
    settings_path: &Path,
    current: &DocumentMut,
    receipt: &CodexReceipt,
) -> Vec<&'static str> {
    if !settings_path.exists() {
        return Vec::new();
    }
    OWNED_PATHS
        .iter()
        .copied()
        .filter(|path| receipt.written.contains_key(*path) && is_ours(current, path, receipt))
        .collect()
}

fn restore_owned(
    current: &DocumentMut,
    receipt: &CodexReceipt,
    ours: &[&str],
    state_path: &Path,
) -> Result<DocumentMut, CodexSettingsError> {
    let mut restored = current.clone();
    for path in ours {
        let previous = previous_snapshot(receipt, path, state_path)?;
        restored = with_value(&restored, path, previous.as_ref());
    }
    Ok(without_empty_providers(restored, receipt))
}

fn previous_snapshot(
    receipt: &CodexReceipt,
    path: &str,
    state_path: &Path,
) -> Result<Option<Snapshot>, CodexSettingsError> {
    let Some(text) = receipt
        .previous
        .get(path)
        .and_then(|saved| saved.as_deref())
    else {
        return Ok(None);
    };
    Snapshot::parse(text)
        .map(Some)
        .map_err(|source| CodexSettingsError::Receipt {
            path: state_path.to_owned(),
            source: ReceiptSource::Snapshot(source),
        })
}

fn without_empty_providers(restored: DocumentMut, receipt: &CodexReceipt) -> DocumentMut {
    let empty = restored
        .as_table()
        .get(MODEL_PROVIDERS_KEY)
        .and_then(|providers| providers.as_table_like())
        .is_some_and(TableLike::is_empty);
    if empty && !receipt.providers_existed {
        return with_value(&restored, MODEL_PROVIDERS_KEY, None);
    }
    restored
}

async fn commit_undo(
    files: &PrivateFiles,
    target: &Path,
    staged: Option<&StagedFile>,
    state_path: &Path,
) -> Result<(), CodexSettingsError> {
    let committed = match staged {
        Some(file) => files.commit_staged_json(file, target).await,
        None => remove_path(files, target).await,
    };
    if let Err(source) = committed {
        if let Some(file) = staged {
            drop(files.discard_staged_json(file).await);
        }
        return Err(CodexSettingsError::Undo { source });
    }
    remove_path(files, state_path)
        .await
        .map_err(|source| CodexSettingsError::Undo { source })
}

mod tests;
