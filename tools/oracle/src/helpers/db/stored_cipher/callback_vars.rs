use std::collections::BTreeSet;
use std::sync::Mutex;

use betterlite_secrets::callback::{
    decrypt_callback_vars, encrypt_callback_vars, is_sensitive_callback_key,
};
use betterlite_secrets::secret_codec::StoredCipher;
use serde_json::Value;

use super::cipher_settings::{KeySource, resolve};
use super::fixed_nonces::FixedNonces;
use super::log_recorder::LogRecorder;
use crate::helpers::HelperResult;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CallbackStep {
    Encrypt { metadata: Value },
    Decrypt { metadata: Value },
    Sensitive { key: String },
}

pub(super) fn run_callback_vars(
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
    extra: &BTreeSet<String>,
    step: &CallbackStep,
) -> Value {
    match step {
        // The pinned encrypt_callback_vars/decrypt_callback_vars never forward an extra set.
        CallbackStep::Encrypt { metadata } => {
            encrypt_callback_vars(metadata, cipher, &BTreeSet::new())
        }
        CallbackStep::Decrypt { metadata } => decrypt_callback_vars(metadata, cipher),
        CallbackStep::Sensitive { key } => Value::Bool(is_sensitive_callback_key(key, extra)),
    }
}
