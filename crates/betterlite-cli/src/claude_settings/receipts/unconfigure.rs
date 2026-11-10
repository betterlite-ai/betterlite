use std::path::{Path, PathBuf};

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::{PrivateDocument, PrivateFiles, StagedFile};
use tokio::task::JoinSet;

use super::super::SettingsDocument;
use super::super::io::{env_object, load_json_or_empty, write_target};
use super::super::paths::{SettingsFileOwner, refuse_while_owned};
use super::error::ReceiptsError;
use super::files::{LandChange, SettingsLanding};
use super::lookup::lookup;
use super::read::read_configure_receipt;
use super::record::{ConfigureReceipt, receipt_document};
use super::restore::{Restored, owned_and_kept, restored_document, withheld_receipt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WithheldCredential {
    pub key: String,
    pub endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnconfigureOutcome {
    pub restored: Vec<String>,
    pub kept: Vec<String>,
    pub withheld: Vec<WithheldCredential>,
    pub file_removed: bool,
}

struct Documents<'input> {
    settings: Option<&'input SettingsDocument>,
    receipt: Option<&'input ConfigureReceipt>,
}

struct UnconfigurePlan {
    current: SettingsDocument,
    ours: Vec<String>,
    kept: Vec<String>,
    restored: Restored,
    target: PathBuf,
    file_removed: bool,
    kept_receipt: Option<ConfigureReceipt>,
}

pub async fn unconfigure_claude_settings(
    settings_path: &Path,
    state_path: &Path,
    owners: &[SettingsFileOwner],
    files: &PrivateFiles,
    landers: &dyn SettingsLanding,
) -> Result<UnconfigureOutcome, ReceiptsError> {
    let plan = plan_off_thread(settings_path, state_path, owners).await?;
    let documents = Documents {
        settings: (!plan.file_removed).then_some(&plan.restored.settings),
        receipt: plan.kept_receipt.as_ref(),
    };
    write_documents(files, landers, state_path, &plan.target, &documents).await?;
    let restored = plan
        .ours
        .iter()
        .filter(|path| lookup(&plan.current, path) != lookup(&plan.restored.settings, path))
        .cloned()
        .collect();
    Ok(UnconfigureOutcome {
        restored,
        kept: plan.kept,
        withheld: plan.restored.withheld,
        file_removed: plan.file_removed,
    })
}

async fn plan_off_thread(
    settings_path: &Path,
    state_path: &Path,
    owners: &[SettingsFileOwner],
) -> Result<UnconfigurePlan, ReceiptsError> {
    let settings = settings_path.to_owned();
    let state = state_path.to_owned();
    let known_owners = owners.to_vec();
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(move || plan_blocking(&settings, &state, &known_owners));
    let joined = tasks
        .join_next()
        .await
        .ok_or(ReceiptsError::PlannerStopped)?;
    joined.map_err(ReceiptsError::Planner)?
}

fn plan_blocking(
    settings_path: &Path,
    state_path: &Path,
    owners: &[SettingsFileOwner],
) -> Result<UnconfigurePlan, ReceiptsError> {
    refuse_while_owned(settings_path, owners)?;
    let Some(receipt) = read_configure_receipt(state_path)? else {
        return Err(ReceiptsError::NoReceipt {
            path: state_path.to_owned(),
        });
    };
    let current = load_json_or_empty(settings_path)?;
    env_object(&current, settings_path)?;
    let (ours, kept) = owned_and_kept(&current, &receipt);
    let restored = restored_document(&current, &receipt, &ours);
    let target = write_target(settings_path)?;
    let file_removed = restored.settings.is_empty() && !(receipt.file_existed && target.exists());
    let kept_receipt = withheld_receipt(&receipt, &restored.withheld);
    Ok(UnconfigurePlan {
        current,
        ours,
        kept,
        restored,
        target,
        file_removed,
        kept_receipt,
    })
}

async fn stage_settings(
    files: &PrivateFiles,
    target: &Path,
    settings: Option<&SettingsDocument>,
) -> Result<Option<StagedFile>, ReceiptsError> {
    let Some(document) = settings else {
        return Ok(None);
    };
    let value = JsonValue::Object(document.clone().into_iter().collect());
    let staged = files
        .stage_private(target, PrivateDocument::json(&value)?)
        .await?;
    Ok(Some(staged))
}

async fn stage_receipt(
    files: &PrivateFiles,
    state_path: &Path,
    receipt: Option<&ConfigureReceipt>,
    staged_settings: Option<&StagedFile>,
) -> Result<Option<StagedFile>, ReceiptsError> {
    let Some(kept) = receipt else {
        return Ok(None);
    };
    let value = receipt_document(kept, state_path)?;
    match files
        .stage_private(state_path, PrivateDocument::json(&value)?)
        .await
    {
        Ok(staged) => Ok(Some(staged)),
        Err(error) => {
            if let Some(staged) = staged_settings {
                files.discard_staged_json(staged).await?;
            }
            Err(ReceiptsError::from(error))
        }
    }
}

async fn write_documents(
    files: &PrivateFiles,
    landers: &dyn SettingsLanding,
    state_path: &Path,
    target: &Path,
    documents: &Documents<'_>,
) -> Result<(), ReceiptsError> {
    let staged_settings = stage_settings(files, target, documents.settings).await?;
    let staged_receipt = stage_receipt(
        files,
        state_path,
        documents.receipt,
        staged_settings.as_ref(),
    )
    .await?;
    land_settings(
        files,
        landers,
        target,
        staged_settings.as_ref(),
        staged_receipt.as_ref(),
    )
    .await?;
    if let Some(staged) = &staged_receipt {
        let change = LandChange::Commit {
            staged,
            path: state_path,
        };
        landers.land(change).await?;
    } else {
        landers
            .land(LandChange::Remove { path: state_path })
            .await?;
    }
    Ok(())
}

async fn land_settings(
    files: &PrivateFiles,
    landers: &dyn SettingsLanding,
    target: &Path,
    staged_settings: Option<&StagedFile>,
    staged_receipt: Option<&StagedFile>,
) -> Result<(), ReceiptsError> {
    let landed = match staged_settings {
        Some(staged) => {
            let change = LandChange::Commit {
                staged,
                path: target,
            };
            landers.land(change).await
        }
        None => landers.land(LandChange::Remove { path: target }).await,
    };
    if let Err(error) = landed {
        if let Some(staged) = staged_receipt {
            files.discard_staged_json(staged).await?;
        }
        return Err(error);
    }
    Ok(())
}
