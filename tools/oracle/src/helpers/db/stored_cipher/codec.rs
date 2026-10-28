use std::collections::BTreeMap;

use betterlite_secrets::error::SecretError;
use betterlite_secrets::secret_codec::bearer::{decrypt_bearer_token, encrypt_bearer_token};
use betterlite_secrets::secret_codec::secret_map::{
    SecretMap, decode_secret_map, encrypt_secret_map,
};
use betterlite_secrets::secret_codec::{
    CipherSettings, DecryptPolicy, FailureLog, FailureValue, StoredCipher, StoredValue,
    decrypt_if_encrypted_with,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::support::{FixedNonces, KeySource, LogRecorder, resolve};
use crate::helpers::HelperResult;

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

pub(super) fn run_codec(
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
            records: std::sync::Mutex::new(Vec::new()),
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
