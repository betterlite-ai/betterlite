use std::collections::BTreeMap;
use std::path::Path;

use crate::agent_launch::codex_proxy_provider;
use betterlite_logging::private_json::error::PrivateJsonError;
use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles, StagedFile};
use secrecy::{ExposeSecret as _, SecretString};
use toml_edit::{DocumentMut, Item, Table, Value};

use super::document::read_settings;
use super::error::CodexSettingsError;
use super::paths::{codex_configure_state_path, resolve_settings_path, settings_path_text};
use super::receipt::{CodexReceipt, RECEIPT_VERSION, is_ours, read_receipt};
use super::snapshot::{Snapshot, fingerprint, snapshot as document_snapshot, with_value};
use super::storage::{remove_path, stage_settings};
use super::version::{CodexVersion, codex_cli_version, require_safe_codex};
use super::{CODEX_PROXY_PROVIDER, MODEL_PROVIDERS_KEY, OWNED_PATHS};

pub struct ConfigureCodexSettings<'request> {
    pub base_url: &'request str,
    pub api_key: &'request SecretString,
    pub model: &'request str,
    pub settings_path: &'request Path,
    pub version: CodexVersion<'request>,
}

pub async fn configure_codex_settings(
    request: &ConfigureCodexSettings<'_>,
    files: &PrivateFiles,
) -> Result<(), CodexSettingsError> {
    let version = codex_cli_version(request.version).await;
    require_safe_codex(version.as_deref())?;
    let current = read_settings(request.settings_path).await?;
    let earlier = read_receipt(request.settings_path).await?;
    let merged = merge_document(&current, request);
    let receipt = build_receipt(request, &current, &merged, earlier.as_ref())?;
    let target = resolve_settings_path(request.settings_path)?;
    let state_path = codex_configure_state_path(request.settings_path)?;
    let staged_receipt = stage_receipt(files, &state_path, &receipt).await?;
    let staged_settings = match stage_settings(files, &target, &merged).await {
        Ok(staged) => staged,
        Err(error) => {
            drop(files.discard_staged_json(&staged_receipt).await);
            return Err(error);
        }
    };
    if let Err(source) = files.commit_staged_json(&staged_receipt, &state_path).await {
        drop(files.discard_staged_json(&staged_receipt).await);
        drop(files.discard_staged_json(&staged_settings).await);
        return Err(CodexSettingsError::WriteReceipt {
            path: state_path,
            source,
        });
    }
    commit_settings(
        files,
        &target,
        &staged_settings,
        &state_path,
        earlier.as_ref(),
    )
    .await
}

fn merge_document(current: &DocumentMut, request: &ConfigureCodexSettings<'_>) -> DocumentMut {
    let model = Snapshot::from_item(&Item::Value(Value::from(request.model)));
    let provider_name = Snapshot::from_item(&Item::Value(Value::from(CODEX_PROXY_PROVIDER)));
    let provider = Snapshot::from_item(&provider_item(request.base_url, request.api_key));
    let without_profile = with_value(current, "profile", None);
    let with_model = with_value(&without_profile, "model", Some(&model));
    let with_provider = with_value(&with_model, "model_provider", Some(&provider_name));
    with_value(&with_provider, "model_providers.litellm", Some(&provider))
}

fn provider_item(base_url: &str, api_key: &SecretString) -> Item {
    let provider = codex_proxy_provider(base_url);
    let mut headers = Table::new();
    headers.insert(
        "Authorization",
        Item::from(format!("Bearer {}", api_key.expose_secret())),
    );
    let mut table = Table::new();
    table.insert("name", Item::from(provider.name));
    table.insert("base_url", Item::from(provider.base_url));
    table.insert("wire_api", Item::from(provider.wire_api));
    table.insert(
        "supports_websockets",
        Item::from(provider.supports_websockets),
    );
    table.insert(
        "requires_openai_auth",
        Item::from(provider.requires_openai_auth),
    );
    table.insert("http_headers", Item::Table(headers));
    Item::Table(table)
}

fn build_receipt(
    request: &ConfigureCodexSettings<'_>,
    current: &DocumentMut,
    merged: &DocumentMut,
    earlier: Option<&CodexReceipt>,
) -> Result<CodexReceipt, CodexSettingsError> {
    let mut previous = BTreeMap::new();
    let mut written = BTreeMap::new();
    for path in owned_paths(current, merged, earlier) {
        let saved = match earlier {
            Some(receipt) if is_ours(current, &path, receipt) => {
                receipt.previous.get(&path).cloned().flatten()
            }
            Some(_) | None => {
                document_snapshot(current, &path).map(|found| found.as_ref().to_owned())
            }
        };
        let merged_fingerprint = fingerprint(document_snapshot(merged, &path).as_ref());
        previous.insert(path.clone(), saved);
        written.insert(path, merged_fingerprint);
    }
    let resolved = resolve_settings_path(request.settings_path)?;
    Ok(CodexReceipt {
        version: RECEIPT_VERSION,
        settings_path: settings_path_text(&resolved)?,
        file_existed: earlier.map_or_else(
            || request.settings_path.exists(),
            |found| found.file_existed,
        ),
        providers_existed: earlier.map_or_else(
            || current.as_table().contains_key(MODEL_PROVIDERS_KEY),
            |found| found.providers_existed,
        ),
        previous,
        written,
    })
}

fn owned_paths(
    current: &DocumentMut,
    merged: &DocumentMut,
    earlier: Option<&CodexReceipt>,
) -> Vec<String> {
    OWNED_PATHS
        .iter()
        .filter(|path| {
            fingerprint(document_snapshot(current, path).as_ref())
                != fingerprint(document_snapshot(merged, path).as_ref())
                || earlier.is_some_and(|receipt| is_ours(current, path, receipt))
        })
        .map(|path| (*path).to_owned())
        .collect()
}

async fn stage_receipt(
    files: &PrivateFiles,
    state_path: &Path,
    receipt: &CodexReceipt,
) -> Result<StagedFile, CodexSettingsError> {
    let parent = state_path.parent().unwrap_or_else(|| Path::new("."));
    stage_error(state_path, files.ensure_private_dir(parent).await)?;
    let document = stage_error(state_path, PrivateDocument::json(receipt))?;
    stage_error(state_path, files.stage_private(state_path, document).await)
}

fn stage_error<T>(
    state_path: &Path,
    result: Result<T, PrivateJsonError>,
) -> Result<T, CodexSettingsError> {
    result.map_err(|source| CodexSettingsError::StageReceipt {
        path: state_path.to_owned(),
        source,
    })
}

async fn commit_settings(
    files: &PrivateFiles,
    target: &Path,
    staged_settings: &StagedFile,
    state_path: &Path,
    earlier: Option<&CodexReceipt>,
) -> Result<(), CodexSettingsError> {
    if let Err(source) = files.commit_staged_json(staged_settings, target).await {
        drop(files.discard_staged_json(staged_settings).await);
        restore_receipt(files, state_path, earlier).await?;
        return Err(CodexSettingsError::WriteSettings {
            path: target.to_owned(),
            source,
        });
    }
    Ok(())
}

async fn restore_receipt(
    files: &PrivateFiles,
    state_path: &Path,
    earlier: Option<&CodexReceipt>,
) -> Result<(), CodexSettingsError> {
    let restore_error = |source| CodexSettingsError::RestoreReceipt {
        path: state_path.to_owned(),
        source,
    };
    let Some(saved) = earlier else {
        return remove_path(files, state_path).await.map_err(restore_error);
    };
    commit_receipt_document(files, state_path, saved)
        .await
        .map_err(restore_error)
}

async fn commit_receipt_document(
    files: &PrivateFiles,
    state_path: &Path,
    receipt: &CodexReceipt,
) -> Result<(), PrivateJsonError> {
    let document = PrivateDocument::json(receipt)?;
    let staged = files.stage_private(state_path, document).await?;
    files.commit_staged_json(&staged, state_path).await
}

mod tests;
