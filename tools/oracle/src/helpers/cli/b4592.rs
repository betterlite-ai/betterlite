use std::future::Future;
use std::path::Path;
use std::sync::{Arc, PoisonError};

use betterlite_cli::credential_lifecycle::clear::clear_cli_token;
use betterlite_cli::credential_lifecycle::gateway::VaultGatewayStore;
use betterlite_cli::credential_lifecycle::load::load_cli_token;
use betterlite_cli::credential_lifecycle::outcome::{SecretClear, SecretSave};
use betterlite_cli::credential_lifecycle::save::save_cli_token;
use betterlite_cli::token_records::file::read_token_file;
use betterlite_cli::token_records::record::CliTokenRecord;
use betterlite_secrets::gateway_key::get_litellm_gateway_api_key;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex as AsyncMutex;

use crate::helpers::{Helper, HelperError, HelperResult};

use self::files::{Failures, ScriptedFiles};
use self::vault::{Events, FakeVault, VaultInput};

mod files;
mod vault;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    record: Option<CliTokenRecord>,
    #[serde(default)]
    expected_base_url: Option<String>,
    #[serde(default)]
    vault: VaultInput,
    #[serde(default)]
    failures: Failures,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    Load,
    Save,
    Clear,
    GatewayKey,
}

#[derive(Debug, Clone, Copy)]
enum Credential {
    Load,
    Save,
    Clear,
}

impl Operation {
    const fn credential(self) -> Option<Credential> {
        match self {
            Self::Load => Some(Credential::Load),
            Self::Save => Some(Credential::Save),
            Self::Clear => Some(Credential::Clear),
            Self::GatewayKey => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Named<'save> {
    Save(&'save SecretSave),
    Clear(&'save SecretClear),
}

fn helper_error<M>(message: M) -> Box<HelperError>
where
    M: std::fmt::Display,
{
    Box::new(HelperError {
        class: "CredentialLifecycle".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_string(),
    })
}

fn block_on<F, T>(future: F) -> Result<T, Box<HelperError>>
where
    F: Future<Output = T>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(helper_error)?;
    Ok(runtime.block_on(future))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let directory = tempfile::tempdir().map_err(helper_error)?;
        let path = directory.path().join("token.json");
        if let Some(file) = &input.file {
            std::fs::write(&path, file).map_err(helper_error)?;
        }
        let events = Arc::new(Events::default());
        let files = ScriptedFiles::new(&path, &input.failures, Arc::clone(&events));
        let vault = FakeVault::new(&input.vault, Arc::clone(&events));
        match input.operation.credential() {
            Some(credential) => credential_answer(credential, input, &path, &files, vault),
            None => Ok(gateway_answer(input, &path, &files, vault)),
        }
    }
}

fn credential_answer(
    credential: Credential,
    input: &Input,
    path: &Path,
    files: &ScriptedFiles,
    mut vault: FakeVault,
) -> HelperResult {
    match credential {
        Credential::Load => {
            let loaded = block_on(load_cli_token(files, &mut vault))?;
            let value = loaded.as_ref().map_or(Value::Null, |record| {
                serde_json::to_value(record).unwrap_or(Value::Null)
            });
            Ok(answer(path, &vault, "loaded", value))
        }
        Credential::Save => {
            let Some(record) = input.record.clone() else {
                return Err(helper_error("save needs a record"));
            };
            let outcome = block_on(save_cli_token(files, &mut vault, record))?;
            let labelled = Value::String(outcome_name(Named::Save(&outcome)).to_owned());
            Ok(answer(path, &vault, "outcome", labelled))
        }
        Credential::Clear => {
            let outcome = block_on(clear_cli_token(files, &mut vault))?;
            let labelled = Value::String(outcome_name(Named::Clear(&outcome)).to_owned());
            Ok(answer(path, &vault, "outcome", labelled))
        }
    }
}

fn gateway_answer(input: &Input, path: &Path, files: &ScriptedFiles, vault: FakeVault) -> Value {
    let shared = AsyncMutex::new(vault);
    let store = VaultGatewayStore {
        files,
        vault: &shared,
    };
    let key = get_litellm_gateway_api_key(input.expected_base_url.as_deref(), &store);
    answer(
        path,
        &shared.into_inner(),
        "key",
        key.map_or(Value::Null, Value::String),
    )
}

fn answer(path: &Path, vault: &FakeVault, field: &str, value: Value) -> Value {
    let mut fields = serde_json::Map::new();
    let events = vault
        .events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    fields.insert(
        "events".to_owned(),
        serde_json::to_value(events).unwrap_or(Value::Null),
    );
    fields.insert(
        "file".to_owned(),
        read_token_file(path).map_or(Value::Null, |record| {
            serde_json::to_value(&record).unwrap_or(Value::Null)
        }),
    );
    fields.insert("vault".to_owned(), vault.value());
    fields.insert(field.to_owned(), value);
    Value::Object(fields)
}

const fn outcome_name(value: Named<'_>) -> &'static str {
    match value {
        Named::Save(SecretSave::SecretStored) => "secret_stored",
        Named::Save(SecretSave::KeyringDiscardsWrites) => "keyring_discards_writes",
        Named::Save(SecretSave::KeyringDisabled) | Named::Clear(SecretClear::KeyringDisabled) => {
            "keyring_disabled"
        }
        Named::Save(SecretSave::KeyringNotInstalled)
        | Named::Clear(SecretClear::KeyringNotInstalled) => "keyring_not_installed",
        Named::Save(SecretSave::KeyringUnreachable)
        | Named::Clear(SecretClear::KeyringUnreachable) => "keyring_unreachable",
        Named::Save(SecretSave::CredentialNotSaved(_)) => "credential_not_saved",
        Named::Save(SecretSave::CredentialNotRecorded(_)) => "credential_not_recorded",
        Named::Clear(SecretClear::SecretErased) => "secret_erased",
        Named::Clear(SecretClear::SecretStranded) => "secret_stranded",
        Named::Clear(SecretClear::CredentialNotCleared(_)) => "credential_not_cleared",
    }
}
