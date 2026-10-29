use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_secrets::stored_cipher::app_crypto::{AppCrypto, AppCryptoError, Envelope};
use secrecy::SecretSlice;
use serde_json::{Value, json};

use super::support::invalid;
use crate::helpers::HelperResult;

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

pub(super) fn run_app_crypto(key: &str, steps: &[AppCryptoStep]) -> HelperResult {
    let decoded = STANDARD
        .decode(key)
        .map_err(|error| invalid(&error.to_string()))?;
    let app: AppCrypto =
        AppCrypto::new(&SecretSlice::from(decoded)).map_err(|error| invalid(&error.to_string()))?;
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
