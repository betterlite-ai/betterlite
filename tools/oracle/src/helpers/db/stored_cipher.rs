use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_config::loader::ConfigLoader;
use betterlite_secrets::callback::{decrypt_callback_vars, encrypt_callback_vars};
use betterlite_secrets::error::SecretError;
use betterlite_secrets::master_key::matches_raw;
use betterlite_secrets::secret_codec::bearer::{decrypt_bearer_token, encrypt_bearer_token};
use betterlite_secrets::secret_codec::diagnostics::{Level, SecretDiagnostics, SecretEvent};
use betterlite_secrets::secret_codec::secret_map::{
    SecretMap, decode_secret_map, encrypt_secret_map,
};
use betterlite_secrets::secret_codec::{
    CipherSettings, DecryptPolicy, FailureLog, FailureValue, NonceSource, StoredCipher,
    StoredValue, decrypt_if_encrypted_with,
};
use betterlite_secrets::stored_cipher::app_crypto::{AppCrypto, AppCryptoError, Envelope};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Codec {
                salt_key,
                master_key,
                general_settings,
                nonce_hex,
                steps,
            } => run_codec(
                &key_source(salt_key.as_ref(), master_key.as_ref(), general_settings.as_ref()),
                nonce_hex,
                steps,
            ),
            Input::AppCrypto { key, steps } => run_app_crypto(key, steps),
            Input::CallbackVars {
                salt_key,
                master_key,
                general_settings,
                nonce_hex,
                extra,
                steps,
            } => run_callback_vars(
                &key_source(salt_key.as_ref(), master_key.as_ref(), general_settings.as_ref()),
                nonce_hex,
                extra,
                steps,
            ),
            Input::MasterKeyMatch {
                presented,
                master_key,
            } => Ok(Value::Bool(matches_raw(
                presented.as_deref(),
                master_key.as_deref(),
            ))),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Codec {
        salt_key: Option<String>,
        master_key: Option<String>,
        general_settings: Option<Value>,
        nonce_hex: String,
        steps: Vec<CodecStep>,
    },
    AppCrypto {
        key: String,
        steps: Vec<AppCryptoStep>,
    },
    CallbackVars {
        salt_key: Option<String>,
        master_key: Option<String>,
        general_settings: Option<Value>,
        nonce_hex: String,
        extra: Vec<String>,
        steps: Vec<CallbackStep>,
    },
    MasterKeyMatch {
        presented: Option<String>,
        master_key: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CodecStep {
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

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AppCryptoStep {
    Roundtrip {
        data: Value,
        aad: Option<String>,
    },
    AadMismatch {
        data: Value,
        encrypt_aad: Option<String>,
        decrypt_aad: Option<String>,
    },
    Decrypt {
        envelope: Envelope,
        aad: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CallbackStep {
    Encrypt { metadata: Value },
    Decrypt { metadata: Value },
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

struct KeySource<'input> {
    salt_key: Option<&'input str>,
    master_key: Option<&'input str>,
    general_settings: Option<&'input Value>,
}

fn key_source<'input>(
    salt_key: Option<&'input String>,
    master_key: Option<&'input String>,
    general_settings: Option<&'input Value>,
) -> KeySource<'input> {
    let salt = salt_key.map(String::as_str);
    KeySource {
        salt_key: salt,
        master_key: master_key.map(String::as_str),
        general_settings,
    }
}

fn load_cipher_settings(source: &KeySource<'_>) -> Option<CipherSettings> {
    let mut general = match source.general_settings {
        Some(Value::Object(settings)) => settings.clone(),
        Some(_) | None => serde_json::Map::new(),
    };
    if let Some(key) = source.master_key {
        general.insert("master_key".to_owned(), Value::String(key.to_owned()));
    }
    let lookup: BTreeMap<String, String> = source
        .salt_key
        .iter()
        .map(|salt| ("LITELLM_SALT_KEY".to_owned(), (*salt).to_owned()))
        .collect();
    let settings_source = json!({"general_settings": general}).to_string();
    let config = ConfigLoader { lookup }.load(&settings_source).ok()?;
    Some(config.general_settings.cipher)
}

fn invalid(message: &str) -> Box<HelperError> {
    let error = HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}

fn resolve(source: &KeySource<'_>, nonce_hex: &str) -> Result<(CipherSettings, Vec<u8>), Box<HelperError>> {
    let settings = load_cipher_settings(source).ok_or_else(|| invalid("the config does not load"))?;
    let nonce = hex::decode(nonce_hex).map_err(|_error| invalid("nonce_hex is not hex"))?;
    Ok((settings, nonce))
}

fn run_codec(
    source: &KeySource<'_>,
    nonce_hex: &str,
    steps: &[CodecStep],
) -> HelperResult {
    let (settings, nonce) = resolve(source, nonce_hex)?;
    Ok(Value::Array(
        steps
            .iter()
            .map(|step| run_codec_step(&settings, &nonce, step))
            .collect(),
    ))
}

fn run_codec_step(settings: &CipherSettings, nonce: &[u8], step: &CodecStep) -> Value {
    let field = match step {
        CodecStep::DecryptValue { field, .. } | CodecStep::DecodeMap { field, .. } => field,
        CodecStep::EncryptValue { .. }
        | CodecStep::DecryptWith { .. }
        | CodecStep::EncryptBearer { .. }
        | CodecStep::DecryptBearer { .. }
        | CodecStep::EncryptMap { .. } => "",
    };
    let cipher = StoredCipher {
        settings: settings.clone(),
        nonces: FixedNonces(nonce.to_vec()),
        diagnostics: LogRecorder {
            field,
            records: Mutex::new(Vec::new()),
        },
    };
    let result = call_codec(&cipher, step);
    let logs = cipher.diagnostics.records.into_inner().unwrap_or_default();
    secret_result(result, &logs)
}

fn secret_result(result: Result<Value, SecretError>, logs: &[Value]) -> Value {
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

fn call_codec(
    cipher: &StoredCipher<FixedNonces, LogRecorder<'_>>,
    step: &CodecStep,
) -> Result<Value, SecretError> {
    match step {
        CodecStep::EncryptValue { value, new_key } => {
            let key = new_key.as_deref().map(SecretString::from);
            cipher.encrypt_value(stored(value), key.as_ref()).map(plain)
        }
        CodecStep::DecryptValue {
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
        CodecStep::DecryptWith { value, signing_key } => {
            let key = SecretString::from(signing_key.as_str());
            let text = decrypt_if_encrypted_with(value, &key);
            Ok(Value::from(text.as_ref().map(SecretString::expose_secret)))
        }
        CodecStep::EncryptBearer { value, prefix } => {
            encrypt_bearer_token(cipher, &SecretString::from(value.as_str()), prefix)
                .map(Value::String)
        }
        CodecStep::DecryptBearer { value, prefix } => {
            let text = decrypt_bearer_token(cipher, value, prefix);
            Ok(Value::from(text.as_ref().map(SecretString::expose_secret)))
        }
        CodecStep::EncryptMap { value, new_key } => {
            let key = new_key.as_deref().map(SecretString::from);
            let map: SecretMap = value
                .iter()
                .map(|(name, text)| (name.clone(), SecretString::from(text.as_str())))
                .collect();
            encrypt_secret_map(cipher, &map, key.as_ref()).map(Value::String)
        }
        CodecStep::DecodeMap { value, field } => decode_map(cipher, value, field),
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

fn run_app_crypto(key: &str, steps: &[AppCryptoStep]) -> HelperResult {
    let app = AppCrypto::new(&SecretString::from(key))
        .map_err(|error| invalid(&error.to_string()))?;
    let results = steps.iter().map(|step| call_app_crypto(&app, step));
    Ok(Value::Array(results.collect()))
}

fn call_app_crypto(app: &AppCrypto, step: &AppCryptoStep) -> Value {
    match step {
        AppCryptoStep::Roundtrip { data, aad } => {
            let aad_bytes = aad.as_ref().map(String::as_bytes);
            let result = app
                .encrypt_json(data, aad_bytes)
                .and_then(|envelope| app.decrypt_json(&envelope, aad_bytes));
            app_result(result)
        }
        AppCryptoStep::AadMismatch {
            data,
            encrypt_aad,
            decrypt_aad,
        } => {
            let encrypt_bytes = encrypt_aad.as_ref().map(String::as_bytes);
            let decrypt_bytes = decrypt_aad.as_ref().map(String::as_bytes);
            let result = app
                .encrypt_json(data, encrypt_bytes)
                .and_then(|envelope| app.decrypt_json(&envelope, decrypt_bytes));
            app_result(result)
        }
        AppCryptoStep::Decrypt { envelope, aad } => {
            let aad_bytes = aad.as_ref().map(String::as_bytes);
            app_result(app.decrypt_json(envelope, aad_bytes))
        }
    }
}

fn app_result(result: Result<Value, AppCryptoError>) -> Value {
    match result {
        Ok(value) => json!({"value": value}),
        Err(error) => json!({"error": {"class": "ValueError", "message": error.to_string()}}),
    }
}

fn run_callback_vars(
    source: &KeySource<'_>,
    nonce_hex: &str,
    extra: &[String],
    steps: &[CallbackStep],
) -> HelperResult {
    let (settings, nonce) = resolve(source, nonce_hex)?;
    let extra_keys = extra.iter().cloned().collect();
    let cipher = StoredCipher {
        settings,
        nonces: FixedNonces(nonce),
        diagnostics: LogRecorder {
            field: "",
            records: Mutex::new(Vec::new()),
        },
    };
    let results = steps
        .iter()
        .map(|step| call_callback(&cipher, &extra_keys, step));
    Ok(Value::Array(results.collect()))
}

fn call_callback(
    cipher: &StoredCipher<FixedNonces, LogRecorder<'_>>,
    extra: &std::collections::BTreeSet<String>,
    step: &CallbackStep,
) -> Value {
    match step {
        CallbackStep::Encrypt { metadata } => encrypt_callback_vars(metadata, cipher, extra),
        CallbackStep::Decrypt { metadata } => decrypt_callback_vars(metadata, cipher),
    }
}
