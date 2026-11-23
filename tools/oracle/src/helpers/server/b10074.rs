use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{OsNonces, StoredCipher, decrypt_if_encrypted_with};
use betterlite_server::env_value_codec::for_db::encrypt_env_for_db;
use betterlite_server::env_value_codec::{decrypt_env, encrypt_env};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub(crate) struct Op;

type Cipher = StoredCipher<OsNonces, TracingDiagnostics>;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    salt_key: Option<String>,
    master_key: Option<String>,
    general_settings: Map<String, Value>,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    operation: Operation,
    values: Map<String, Value>,
    new_key: Option<String>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum Operation {
    EncryptEnv,
    DecryptEnv,
    EncryptEnvForDb,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cipher = cipher(input)?;
        let answers = input
            .calls
            .iter()
            .map(|call| answer(&cipher, call))
            .collect();
        Ok(Value::Array(answers))
    }
}

fn cipher(input: &Input) -> Result<Cipher, Box<HelperError>> {
    let mut general = input.general_settings.clone();
    if let Some(master_key) = &input.master_key {
        general.insert("master_key".to_owned(), Value::String(master_key.clone()));
    }
    let lookup: BTreeMap<String, String> = input
        .salt_key
        .iter()
        .map(|salt| ("LITELLM_SALT_KEY".to_owned(), salt.clone()))
        .collect();
    let source = json!({"general_settings": general}).to_string();
    let config = ConfigLoader { lookup }.load(&source).or_value_error()?;
    Ok(StoredCipher {
        settings: config.general_settings.cipher,
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    })
}

fn answer(cipher: &Cipher, call: &Call) -> Value {
    let new_key = call.new_key.as_deref().map(SecretString::from);
    let sealed = match call.operation {
        Operation::EncryptEnv => encrypt_env(&call.values, cipher, new_key.as_ref()),
        Operation::EncryptEnvForDb => encrypt_env_for_db(&call.values, cipher, new_key.as_ref()),
        Operation::DecryptEnv => return Value::Object(decrypt_env(&call.values, cipher)),
    };
    match sealed {
        Ok(values) => {
            let key = write_key(cipher, new_key.as_ref());
            sealed_view(&values, key.as_ref())
        }
        Err(_) => json!({"failed": true}),
    }
}

fn write_key(cipher: &Cipher, new_key: Option<&SecretString>) -> Option<SecretString> {
    new_key
        .filter(|key| !key.expose_secret().is_empty())
        .cloned()
        .or_else(|| cipher.settings.signing_key.clone())
}

fn sealed_view(sealed: &Map<String, Value>, write_key: Option<&SecretString>) -> Value {
    let mut view = Map::new();
    for (name, value) in sealed {
        let entry = match value {
            Value::String(text) => sealed_entry(text, write_key),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::Array(_)
            | Value::Object(_) => Some(value.clone()),
        };
        let Some(entry) = entry else {
            return json!({"failed": true});
        };
        view.insert(name.clone(), entry);
    }
    Value::Object(view)
}

fn sealed_entry(text: &str, write_key: Option<&SecretString>) -> Option<Value> {
    let opened = write_key.and_then(|key| decrypt_if_encrypted_with(text, key))?;
    Some(json!({
        "opened": opened.expose_secret(),
        "v2": text.starts_with("v2:gcm:"),
    }))
}
