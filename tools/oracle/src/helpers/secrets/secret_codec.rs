use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_config::loader::ConfigLoader;
use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::bearer::{decrypt_bearer_token, encrypt_bearer_token};
use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use betterlite_secrets::secret_codec::secret_map::{
    SecretMap, decode_secret_map, encrypt_secret_map,
};
use betterlite_secrets::secret_codec::{
    CipherSettings, DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher,
    StoredValue, decrypt_if_encrypted_with,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = SecretCodecInput;

    fn run(input: &Self::Input) -> HelperResult {
        run_codec(input)
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SecretCodecInput {
    pub(crate) salt_key: Option<String>,
    pub(crate) master_key: Option<String>,
    pub(crate) general_settings: Option<Value>,
    pub(crate) nonce_hex: String,
    pub(crate) steps: Vec<SecretStep>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum SecretStep {
    EncryptValue {
        value: Value,
        new_key: Option<String>,
    },
    DecryptValue {
        value: Value,
        field: String,
        report: Report,
        keep_original: bool,
    },
    DecryptWith {
        value: String,
        signing_key: String,
    },
    EncryptBearer {
        value: String,
        prefix: String,
    },
    DecryptBearer {
        value: String,
        prefix: String,
    },
    EncryptMap {
        value: BTreeMap<String, String>,
        new_key: Option<String>,
    },
    DecodeMap {
        value: Value,
        field: String,
    },
}

#[derive(Debug, Clone, Copy, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Report {
    Debug,
    Error,
}

struct FixedNonces(Vec<u8>);

impl NonceSource for FixedNonces {
    fn fill(&self, nonce: &mut [u8]) -> Result<(), SecretError> {
        let bytes = self.0.get(..nonce.len()).ok_or(SecretError::Truncated)?;
        nonce.copy_from_slice(bytes);
        Ok(())
    }
}

struct LogRecorder<'step> {
    field: &'step str,
    records: Mutex<Vec<Value>>,
}

impl SecretDiagnostics for LogRecorder<'_> {
    fn record(&self, level: Level, event: &SecretEvent<'_>) {
        let message = event.message();
        let names_field = !self.field.is_empty() && message.contains(self.field);
        let salt_advice =
            message.contains("https://docs.litellm.ai/docs/proxy/prod#5-set-litellm-salt-key");
        let level_name = match level {
            Level::Debug => "debug",
            Level::Error => "error",
        };
        if let Ok(mut records) = self.records.lock() {
            records.push(json!({"level": level_name, "names_field": names_field, "salt_advice": salt_advice}));
        }
    }
}

fn run_codec(input: &SecretCodecInput) -> HelperResult {
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
    let (Ok(config), Ok(bytes)) = (loaded, nonce) else {
        return Err(Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "the config or nonce_hex does not load".to_owned(),
        }));
    };
    let results = input
        .steps
        .iter()
        .map(|step| run_step(&config.general_settings.cipher, &bytes, step));
    Ok(Value::Array(results.collect()))
}

fn run_step(settings: &CipherSettings, nonce: &[u8], step: &SecretStep) -> Value {
    let field = match step {
        SecretStep::DecryptValue { field, .. } | SecretStep::DecodeMap { field, .. } => field,
        SecretStep::EncryptValue { .. }
        | SecretStep::DecryptWith { .. }
        | SecretStep::EncryptBearer { .. }
        | SecretStep::DecryptBearer { .. }
        | SecretStep::EncryptMap { .. } => "",
    };
    let cipher = StoredCipher {
        settings: settings.clone(),
        nonces: FixedNonces(nonce.to_vec()),
        diagnostics: LogRecorder {
            field,
            records: Mutex::new(Vec::new()),
        },
    };
    let result = call(&cipher, step);
    let logs = cipher.diagnostics.records.into_inner().unwrap_or_default();
    match result {
        Ok(value) => json!({"value": value, "logs": logs}),
        Err(error) => {
            let class = match error {
                SecretError::MapDecode { .. } => "SecretMapDecodeError",
                SecretError::MissingKey => "AttributeError",
                SecretError::BearerSecret
                | SecretError::TooLarge { .. }
                | SecretError::Base64
                | SecretError::Decode { .. }
                | SecretError::Truncated
                | SecretError::Seal { .. }
                | SecretError::LegacySeal { .. }
                | SecretError::Utf8 { .. }
                | SecretError::Random { .. }
                | SecretError::MapEncode { .. } => "ValueError",
            };
            json!({"error": {"class": class, "message": error.to_string()}, "logs": logs})
        }
    }
}

fn call(
    cipher: &StoredCipher<FixedNonces, LogRecorder<'_>>,
    step: &SecretStep,
) -> Result<Value, SecretError> {
    match step {
        SecretStep::EncryptValue { value, new_key } => {
            let key = new_key.as_deref().map(SecretString::from);
            cipher.encrypt_value(stored(value), key.as_ref()).map(plain)
        }
        SecretStep::DecryptValue {
            value,
            field,
            report,
            keep_original,
        } => {
            let policy = decrypt_policy(*report, *keep_original);
            Ok(cipher
                .decrypt_value(stored(value), field, policy)
                .map_or(Value::Null, plain))
        }
        SecretStep::DecryptWith { value, signing_key } => {
            let key = SecretString::from(signing_key.as_str());
            let text = decrypt_if_encrypted_with(value, &key);
            Ok(Value::from(text.as_ref().map(SecretString::expose_secret)))
        }
        SecretStep::EncryptBearer { value, prefix } => {
            encrypt_bearer_token(cipher, &SecretString::from(value.as_str()), prefix)
                .map(Value::String)
        }
        SecretStep::DecryptBearer { value, prefix } => {
            let text = decrypt_bearer_token(cipher, value, prefix);
            Ok(Value::from(text.as_ref().map(SecretString::expose_secret)))
        }
        SecretStep::EncryptMap { value, new_key } => {
            let key = new_key.as_deref().map(SecretString::from);
            let map: SecretMap = value
                .iter()
                .map(|(name, text)| (name.clone(), SecretString::from(text.as_str())))
                .collect();
            encrypt_secret_map(cipher, &map, key.as_ref()).map(Value::String)
        }
        SecretStep::DecodeMap { value, field } => decode_map(cipher, value, field),
    }
}

fn decode_map(
    cipher: &StoredCipher<FixedNonces, LogRecorder<'_>>,
    value: &Value,
    field: &str,
) -> Result<Value, SecretError> {
    let raw = value.to_string();
    let decoded = decode_secret_map(cipher, Some(&raw), field)?;
    Ok(decoded.map_or(Value::Null, |entries| {
        let plain = entries
            .into_iter()
            .map(|(name, text)| (name, Value::from(text.expose_secret())));
        Value::Object(plain.collect())
    }))
}

fn stored(value: &Value) -> StoredValue<Value> {
    match value {
        Value::String(text) => StoredValue::Text(SecretString::from(text.as_str())),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            StoredValue::Other(value.clone())
        }
    }
}

fn plain(value: StoredValue<Value>) -> Value {
    match value {
        StoredValue::Text(text) => Value::from(text.expose_secret()),
        StoredValue::Other(other) => other,
    }
}

const fn decrypt_policy(report: Report, keep_original: bool) -> DecryptPolicy {
    let log = match report {
        Report::Debug => FailureLog::Debug,
        Report::Error => FailureLog::Error,
    };
    let value = if keep_original {
        FailureValue::Original
    } else {
        FailureValue::Nothing
    };
    DecryptPolicy { log, value }
}
