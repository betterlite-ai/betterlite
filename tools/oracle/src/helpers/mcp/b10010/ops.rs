use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, PoisonError};

use betterlite_db::sso_assertion_rows::{AssertionRow, MemoryAssertionRows};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::sso_assertions::rotation::rotate_assertions;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue, decrypt_if_encrypted_with,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::input::{Input, RowSeed};
use super::logs::Logs;

// The oracle case fixes the salt key; rotation moves every row from it to NEW_KEY.
const OLD_KEY: &str = "b10010-salt-key";
const NEW_KEY: &str = "b10010-new-key";

pub(crate) async fn answer(input: &Input, logs: &Logs) -> HelperResult {
    let current = cipher_with(OLD_KEY);
    let mut stored = Vec::with_capacity(input.rows.len());
    for seed in &input.rows {
        stored.push(seed_row(seed, &current)?);
    }
    let rows = MemoryAssertionRows {
        rows: Mutex::new(stored),
        ..MemoryAssertionRows::default()
    };
    let new_key = SecretString::from(NEW_KEY);
    rotate_assertions(&rows, &current, &new_key)
        .await
        .or_value_error()?;
    let rotated = rows
        .rows
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let outcomes: Vec<Value> = rotated.iter().map(|row| outcome(row, &new_key)).collect();
    let entries: Vec<Value> = logs
        .entries()
        .into_iter()
        .map(|(level, message)| json!({ "level": level, "message": message }))
        .collect();
    Ok(json!({
        "rows": outcomes,
        "logs": entries,
        "updates": rows.writes.load(Ordering::SeqCst),
    }))
}

fn seed_row(seed: &RowSeed, current: &DbCipher) -> Result<AssertionRow, Box<HelperError>> {
    let (user_id, assertion_b64) = match seed {
        RowSeed::Plain { user_id, payload } => {
            let plaintext =
                serde_json::to_string(&BTreeMap::from([("assertion", payload.as_str())]))?;
            let stored = StoredValue::<Infallible>::Text(SecretString::from(plaintext));
            let sealed = current.encrypt_value(stored, None).or_value_error()?;
            let text = match sealed {
                StoredValue::Text(ciphertext) => ciphertext.expose_secret().to_owned(),
                StoredValue::Other(never) => match never {},
            };
            (user_id, text)
        }
        RowSeed::Raw { user_id, text } => (user_id, text.clone()),
    };
    Ok(AssertionRow {
        user_id: user_id.clone(),
        assertion_b64,
    })
}

fn outcome(row: &AssertionRow, new_key: &SecretString) -> Value {
    match decrypt_if_encrypted_with(&row.assertion_b64, new_key) {
        Some(plaintext) => json!({
            "kind": "rotated",
            "user_id": row.user_id,
            "plaintext": plaintext.expose_secret(),
        }),
        None => json!({
            "kind": "skipped",
            "user_id": row.user_id,
            "text": row.assertion_b64,
        }),
    }
}

fn cipher_with(key: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
