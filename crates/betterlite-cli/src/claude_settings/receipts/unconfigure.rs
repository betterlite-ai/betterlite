use std::collections::BTreeMap;
use std::path::Path;

use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::private_json::files::StagedFile;

use super::super::io::{env_object, load_json_or_empty, write_target};
use super::super::paths::{SettingsFileOwner, refuse_while_owned};
use super::super::{
    ANTHROPIC_BASE_URL_KEY, BASE_URL_PATH, CREDENTIAL_PATHS, ENV_KEY, OWNED_PATHS, SettingsDocument,
};
use super::error::ReceiptsError;
use super::files::SettingsFiles;
use super::lookup::{env_is_empty, lookup, with};
use super::read::read_configure_receipt;
use super::record::{ConfigureReceipt, owns, receipt_document};
use super::value::{ABSENT, OwnedValue, fingerprint};

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

struct Restored {
    settings: SettingsDocument,
    withheld: Vec<WithheldCredential>,
}

pub async fn unconfigure_claude_settings(
    settings_path: &Path,
    state_path: &Path,
    owners: &[SettingsFileOwner],
    files: &dyn SettingsFiles,
) -> Result<UnconfigureOutcome, ReceiptsError> {
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
    let file_removed = write_documents(files, state_path, &target, &receipt, &restored).await?;
    let restored_paths = ours
        .iter()
        .filter(|path| lookup(&current, path) != lookup(&restored.settings, path))
        .cloned()
        .collect();
    Ok(UnconfigureOutcome {
        restored: restored_paths,
        kept,
        withheld: restored.withheld,
        file_removed,
    })
}

fn owned_and_kept(
    current: &SettingsDocument,
    receipt: &ConfigureReceipt,
) -> (Vec<String>, Vec<String>) {
    let mut ours = Vec::new();
    let mut kept = Vec::new();
    for path in written_paths(receipt) {
        let still_ours = owns(current, &path, receipt);
        let user_kept = !still_ours && lookup(current, &path).present;
        if user_kept {
            kept.push(path.clone());
        }
        if still_ours {
            ours.push(path);
        }
    }
    (ours, kept)
}

fn restored_document(
    current: &SettingsDocument,
    receipt: &ConfigureReceipt,
    ours: &[String],
) -> Restored {
    let mut put_back = current.clone();
    for path in ours {
        let previous = receipt.previous.get(path).cloned().unwrap_or(ABSENT);
        with(&mut put_back, path, &previous);
    }
    let url_after = lookup(&put_back, BASE_URL_PATH);
    let mut withheld = Vec::new();
    for path in CREDENTIAL_PATHS {
        let previous = receipt.previous.get(path).cloned().unwrap_or(ABSENT);
        if !previous.present || !ours.iter().any(|ours_path| ours_path == path) {
            continue;
        }
        let endpoint = receipt.endpoints.get(path).cloned().unwrap_or(ABSENT);
        if endpoint != url_after {
            withheld.push(WithheldCredential {
                key: path.to_owned(),
                endpoint: endpoint_text(&endpoint),
            });
        }
    }
    let mut settings = put_back;
    for item in &withheld {
        with(&mut settings, &item.key, &ABSENT);
    }
    if env_is_empty(&settings) && !receipt.env_was_object {
        let env = OwnedValue {
            present: receipt.env_present,
            value: JsonValue::Null,
        };
        with(&mut settings, ENV_KEY, &env);
    }
    Restored { settings, withheld }
}

async fn write_documents(
    files: &dyn SettingsFiles,
    state_path: &Path,
    target: &Path,
    receipt: &ConfigureReceipt,
    restored: &Restored,
) -> Result<bool, ReceiptsError> {
    let file_removed = restored.settings.is_empty() && !(receipt.file_existed && target.exists());
    let kept_document = withheld_receipt(receipt, &restored.withheld)
        .map(|record| receipt_document(&record, state_path))
        .transpose()?;
    let staged_settings = if file_removed {
        None
    } else {
        let document = JsonValue::Object(restored.settings.clone().into_iter().collect());
        Some(files.stage(target, &document).await?)
    };
    let staged_receipt = match &kept_document {
        None => None,
        Some(document) => match files.stage(state_path, document).await {
            Ok(staged) => Some(staged),
            Err(error) => {
                discard_staged(files, staged_settings.as_ref()).await?;
                return Err(error);
            }
        },
    };
    land_settings(
        files,
        target,
        staged_settings.as_ref(),
        staged_receipt.as_ref(),
    )
    .await?;
    match &staged_receipt {
        Some(staged) => files.commit(staged, state_path).await?,
        None => files.remove(state_path).await?,
    }
    Ok(file_removed)
}

async fn land_settings(
    files: &dyn SettingsFiles,
    target: &Path,
    staged_settings: Option<&StagedFile>,
    staged_receipt: Option<&StagedFile>,
) -> Result<(), ReceiptsError> {
    let landed = match staged_settings {
        Some(staged) => files.commit(staged, target).await,
        None => files.remove(target).await,
    };
    if let Err(error) = landed {
        if let Some(staged) = staged_receipt {
            files.discard(staged).await?;
        }
        return Err(error);
    }
    Ok(())
}

async fn discard_staged(
    files: &dyn SettingsFiles,
    staged: Option<&StagedFile>,
) -> Result<(), ReceiptsError> {
    match staged {
        Some(record) => files.discard(record).await,
        None => Ok(()),
    }
}

fn written_paths(receipt: &ConfigureReceipt) -> Vec<String> {
    let mut paths: Vec<String> = OWNED_PATHS
        .iter()
        .filter(|path| receipt.written.contains_key(**path))
        .map(|path| (*path).to_owned())
        .collect();
    for path in receipt.written.keys() {
        if !OWNED_PATHS.contains(&path.as_str()) {
            paths.push(path.clone());
        }
    }
    paths
}

fn withheld_receipt(
    receipt: &ConfigureReceipt,
    withheld: &[WithheldCredential],
) -> Option<ConfigureReceipt> {
    if withheld.is_empty() {
        return None;
    }
    let absent = fingerprint(&ABSENT);
    Some(ConfigureReceipt {
        file_existed: receipt.file_existed,
        env_present: receipt.env_present,
        env_was_object: receipt.env_was_object,
        previous: receipt.previous.clone(),
        written: withheld
            .iter()
            .map(|item| (item.key.clone(), absent.clone()))
            .collect::<BTreeMap<String, String>>(),
        endpoints: receipt.endpoints.clone(),
    })
}

fn endpoint_text(endpoint: &OwnedValue) -> String {
    if !endpoint.present {
        return format!("no {ANTHROPIC_BASE_URL_KEY} (Anthropic's default endpoint)");
    }
    match &endpoint.value {
        JsonValue::String(text) => text.clone(),
        other @ (JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_)) => other.to_string(),
    }
}
