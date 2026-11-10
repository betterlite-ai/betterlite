use std::path::{Path, PathBuf};

use betterlite_config::sections::settings::JsonValue;

use super::super::choices::{ModelChoice, StaticToken};
use super::super::io::{env_object, load_json_or_empty, write_target};
use super::super::merge::{MergeOptions, merge_claude_settings};
use super::super::paths::{SettingsFileOwner, refuse_while_owned};
use super::super::statusline_install::install_statusline_script;
use super::super::{MODEL_PATHS, SettingsDocument};
use super::error::ReceiptsError;
use super::files::SettingsFiles;
use super::lookup::with;
use super::read::read_configure_receipt;
use super::record::{ConfigureReceipt, build_receipt, owns, receipt_document};
use super::value::ABSENT;

pub struct ConfigureRequest<'input> {
    pub base_url: &'input str,
    pub credential: &'input StaticToken,
    pub model: &'input ModelChoice,
    pub settings_path: &'input Path,
    pub state_path: &'input Path,
    pub owners: &'input [SettingsFileOwner],
    pub binary_path: Option<&'input Path>,
}

struct Plan {
    merged: SettingsDocument,
    receipt: ConfigureReceipt,
    earlier: Option<ConfigureReceipt>,
    target: PathBuf,
}

pub async fn configure_claude_settings(
    request: &ConfigureRequest<'_>,
    files: &dyn SettingsFiles,
) -> Result<(), ReceiptsError> {
    let plan = build_plan(request)?;
    files
        .ensure_dir(state_directory(request.state_path))
        .await?;
    stage_and_commit(files, request.state_path, &plan).await
}

fn build_plan(request: &ConfigureRequest<'_>) -> Result<Plan, ReceiptsError> {
    refuse_while_owned(request.settings_path, request.owners)?;
    let current = load_json_or_empty(request.settings_path)?;
    env_object(&current, request.settings_path)?;
    let earlier = read_configure_receipt(request.state_path)?;
    let existing = unpinned_document(&current, request.model, earlier.as_ref());
    let command = install_statusline_script(request.binary_path)?;
    let merged = merge_claude_settings(
        &existing,
        &MergeOptions {
            base_url: request.base_url,
            credential: request.credential,
            model: request.model,
            tier_model: None,
            status_line: Some(&command),
        },
    );
    let receipt = build_receipt(
        &current,
        &merged,
        earlier.as_ref(),
        request.settings_path.exists(),
    );
    let target = write_target(request.settings_path)?;
    Ok(Plan {
        merged,
        receipt,
        earlier,
        target,
    })
}

async fn stage_and_commit(
    files: &dyn SettingsFiles,
    state_path: &Path,
    plan: &Plan,
) -> Result<(), ReceiptsError> {
    let staged_receipt = files
        .stage(state_path, &receipt_document(&plan.receipt, state_path)?)
        .await?;
    let settings_document = JsonValue::Object(plan.merged.clone().into_iter().collect());
    let staged_settings = match files.stage(&plan.target, &settings_document).await {
        Ok(staged) => staged,
        Err(error) => {
            files.discard(&staged_receipt).await?;
            return Err(error);
        }
    };
    if let Err(error) = files.commit(&staged_receipt, state_path).await {
        files.discard(&staged_settings).await?;
        return Err(error);
    }
    match files.commit(&staged_settings, &plan.target).await {
        Ok(()) => Ok(()),
        Err(settings_error) => {
            rollback(files, state_path, plan.earlier.as_ref(), settings_error).await
        }
    }
}

fn unpinned_document(
    current: &SettingsDocument,
    model: &ModelChoice,
    earlier: Option<&ConfigureReceipt>,
) -> SettingsDocument {
    let mut existing = current.clone();
    if !matches!(model, ModelChoice::UnpinModel) {
        return existing;
    }
    let Some(receipt) = earlier else {
        return existing;
    };
    for path in MODEL_PATHS {
        if owns(current, path, receipt) {
            let previous = receipt.previous.get(path).cloned().unwrap_or(ABSENT);
            with(&mut existing, path, &previous);
        }
    }
    existing
}

fn state_directory(state_path: &Path) -> &Path {
    state_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

async fn rollback(
    files: &dyn SettingsFiles,
    state_path: &Path,
    earlier: Option<&ConfigureReceipt>,
    settings_error: ReceiptsError,
) -> Result<(), ReceiptsError> {
    let restored = match earlier {
        None => files.remove(state_path).await,
        Some(receipt) => restore_receipt(files, state_path, receipt).await,
    };
    match restored {
        Ok(()) => Err(settings_error),
        Err(receipt_error) => Err(ReceiptsError::Inconsistent {
            state_path: state_path.to_owned(),
            settings: Box::new(settings_error),
            receipt: Box::new(receipt_error),
        }),
    }
}

async fn restore_receipt(
    files: &dyn SettingsFiles,
    state_path: &Path,
    receipt: &ConfigureReceipt,
) -> Result<(), ReceiptsError> {
    let document = receipt_document(receipt, state_path)?;
    let staged = files.stage(state_path, &document).await?;
    files.commit(&staged, state_path).await
}
