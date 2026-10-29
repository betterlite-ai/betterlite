use std::collections::BTreeMap;
use std::collections::BTreeSet;

use betterlite_config::loader::ConfigLoader;
use betterlite_secrets::callback_vars::{
    Transform, is_sensitive_callback_key, strip_callback_config, transform_callback_vars,
};
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{NonceSource, StoredCipher};
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

impl Helper for Op {
    type Input = CallbackSecretFieldsInput;

    fn run(input: &Self::Input) -> HelperResult {
        run_callback_secret_fields(input)
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CallbackSecretFieldsInput {
    pub(crate) salt_key: Option<String>,
    pub(crate) master_key: Option<String>,
    pub(crate) general_settings: Option<Value>,
    pub(crate) nonce_hex: String,
    pub(crate) extra: Vec<String>,
    pub(crate) steps: Vec<CallbackSecretStep>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CallbackSecretStep {
    Strip { metadata: Value },
    Encrypt { metadata: Value },
    Decrypt { metadata: Value },
    Sensitive { key: String },
}

struct FixedNonces(Vec<u8>);

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = self.0.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}

fn run_callback_secret_fields(input: &CallbackSecretFieldsInput) -> HelperResult {
    let mut general = match &input.general_settings {
        Some(Value::Object(settings)) => settings.clone(),
        Some(_) | None => serde_json::Map::new(),
    };
    if let Some(master_key) = &input.master_key {
        general.insert("master_key".to_owned(), Value::String(master_key.clone()));
    }
    let lookup: BTreeMap<String, String> = input
        .salt_key
        .iter()
        .map(|salt| ("LITELLM_SALT_KEY".to_owned(), salt.clone()))
        .collect();
    let source = json!({"general_settings": general}).to_string();
    let nonce = hex::decode(&input.nonce_hex);
    let loaded = ConfigLoader { lookup }.load(&source);
    let (Ok(config), Ok(nonce_bytes)) = (loaded, nonce) else {
        return Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "the config or nonce_hex does not load".to_owned(),
        }));
    };
    let cipher = StoredCipher {
        settings: config.general_settings.cipher,
        nonces: FixedNonces(nonce_bytes),
        diagnostics: TracingDiagnostics,
    };
    let extra: BTreeSet<String> = input.extra.iter().cloned().collect();
    let results = input
        .steps
        .iter()
        .map(|step| run_step(&cipher, &extra, step));
    Ok(Value::Array(results.collect()))
}

fn run_step(
    cipher: &StoredCipher<FixedNonces, TracingDiagnostics>,
    extra: &BTreeSet<String>,
    step: &CallbackSecretStep,
) -> Value {
    match step {
        CallbackSecretStep::Strip { metadata } => strip_callback_config(metadata),
        CallbackSecretStep::Encrypt { metadata } => {
            transform_callback_vars(metadata, &Transform::Encrypt(cipher))
        }
        CallbackSecretStep::Decrypt { metadata } => {
            transform_callback_vars(metadata, &Transform::Decrypt(cipher))
        }
        CallbackSecretStep::Sensitive { key } => Value::Bool(is_sensitive_callback_key(key, extra)),
    }
}
