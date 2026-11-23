use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use betterlite_server::key_fields::decode::TypedValue;
use betterlite_server::key_patch_fields::error::KeyPatchError;
use betterlite_server::key_patch_fields::metadata::PatchWarning;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::CallFailed;

pub(super) type Cipher = StoredCipher<OsNonces, TracingDiagnostics>;

pub(super) fn envelope(
    value: Value,
    error: Option<Value>,
    warnings: Vec<Value>,
    effects: Vec<Value>,
) -> Value {
    json!({
        "value": value,
        "error": error,
        "warnings": warnings,
        "effects": effects,
    })
}

pub(super) fn error_json(error: &KeyPatchError) -> Value {
    let (status, _) = error.response();
    let exception = error.proxy_exception();
    let (kind, param, code) = exception.map_or((None, None, None), |exception| {
        (
            Some(exception.error_type),
            exception.param,
            Some(exception.code),
        )
    });
    json!({"status": status, "type": kind, "param": param, "code": code})
}

pub(super) fn warnings_json(warnings: &[PatchWarning]) -> Vec<Value> {
    warnings
        .iter()
        .map(|warning| Value::String(warning.subject().to_owned()))
        .collect()
}

pub(super) fn typed_body(body: &Value) -> Result<TypedValue, CallFailed> {
    serde_json::from_value(body.clone())
        .map(|value| TypedValue { json: value })
        .map_err(CallFailed::from)
}

pub(super) fn amount_value(amount: Option<Decimal>) -> Value {
    let Some(finite) = amount.and_then(|value| value.to_f64()) else {
        return Value::Null;
    };
    Value::from(finite)
}

pub(super) fn cipher() -> Cipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from("b894-oracle-signing-key")),
            algorithm: Algorithm::Aes256Gcm,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
