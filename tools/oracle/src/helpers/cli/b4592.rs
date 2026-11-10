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
        let mut vault = FakeVault::new(&input.vault, Arc::clone(&events));
        match input.operation {
            Operation::Load => load_answer(&path, &files, &mut vault, &events),
            Operation::Save => save_answer(input, &path, &files, &mut vault, &events),
            Operation::Clear => clear_answer(&path, &files, &mut vault, &events),
            Operation::GatewayKey => Ok(gateway_answer(input, &path, &files, vault, &events)),
        }
    }
}

fn load_answer(
    path: &Path,
    files: &ScriptedFiles,
    vault: &mut FakeVault,
    events: &Events,
) -> HelperResult {
    let loaded = block_on(load_cli_token(files, vault))?;
    Ok(answer(
        path,
        vault,
        events,
        "loaded",
        loaded.as_ref().map_or(Value::Null, |record| {
            serde_json::to_value(record).unwrap_or(Value::Null)
        }),
    ))
}

fn save_answer(
    input: &Input,
    path: &Path,
    files: &ScriptedFiles,
    vault: &mut FakeVault,
    events: &Events,
) -> HelperResult {
    let Some(record) = input.record.clone() else {
        return Err(helper_error("save needs a record"));
    };
    let outcome = block_on(save_cli_token(files, vault, record))?;
    Ok(answer(
        path,
        vault,
        events,
        "outcome",
        Value::String(save_name(&outcome).to_owned()),
    ))
}

fn clear_answer(
    path: &Path,
    files: &ScriptedFiles,
    vault: &mut FakeVault,
    events: &Events,
) -> HelperResult {
    let outcome = block_on(clear_cli_token(files, vault))?;
    Ok(answer(
        path,
        vault,
        events,
        "outcome",
        Value::String(clear_name(&outcome).to_owned()),
    ))
}

fn gateway_answer(
    input: &Input,
    path: &Path,
    files: &ScriptedFiles,
    vault: FakeVault,
    events: &Events,
) -> Value {
    let shared = AsyncMutex::new(vault);
    let store = VaultGatewayStore {
        files,
        vault: &shared,
    };
    let key = get_litellm_gateway_api_key(input.expected_base_url.as_deref(), &store);
    answer(
        path,
        &shared.into_inner(),
        events,
        "key",
        key.map_or(Value::Null, Value::String),
    )
}

fn answer(path: &Path, vault: &FakeVault, events: &Events, field: &str, value: Value) -> Value {
    let mut fields = serde_json::Map::new();
    fields.insert(
        "events".to_owned(),
        serde_json::to_value(
            events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .unwrap_or(Value::Null),
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

const fn save_name(outcome: &SecretSave) -> &'static str {
    match outcome {
        SecretSave::SecretStored => "secret_stored",
        SecretSave::KeyringDiscardsWrites => "keyring_discards_writes",
        SecretSave::KeyringDisabled => "keyring_disabled",
        SecretSave::KeyringNotInstalled => "keyring_not_installed",
        SecretSave::KeyringUnreachable => "keyring_unreachable",
        SecretSave::CredentialNotSaved(_) => "credential_not_saved",
        SecretSave::CredentialNotRecorded(_) => "credential_not_recorded",
    }
}

const fn clear_name(outcome: &SecretClear) -> &'static str {
    match outcome {
        SecretClear::SecretErased => "secret_erased",
        SecretClear::SecretStranded => "secret_stranded",
        SecretClear::KeyringDisabled => "keyring_disabled",
        SecretClear::KeyringNotInstalled => "keyring_not_installed",
        SecretClear::KeyringUnreachable => "keyring_unreachable",
        SecretClear::CredentialNotCleared(_) => "credential_not_cleared",
    }
}
