use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::mcp_user_env_vars::codec::USER_ENV_VARS_KEY;
use betterlite_db::mcp_user_env_vars::store::{UserEnvVars, UserEnvVarsStore};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, DecryptPolicy, FailureLog, FailureValue, OsNonces, StoredCipher,
    StoredValue,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};

use super::rows::{RowStore, text};
use super::{Bulk, Delete, Get, Merge};

fn exposed(values: &UserEnvVars) -> HelperResult {
    let pairs: BTreeMap<&str, &str> = values
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect();
    serde_json::to_value(pairs).or_value_error()
}

// what the sealed column holds once opened, so a case shows the stored contents and not just the map
fn row_values(cipher: &DbCipher, stored: Option<&str>) -> Value {
    let Some(stored) = stored else {
        return Value::Null;
    };
    let sealed = StoredValue::<()>::Text(SecretString::from(stored.to_owned()));
    let policy = DecryptPolicy {
        log: FailureLog::Debug,
        value: FailureValue::Nothing,
    };
    let Some(StoredValue::Text(decrypted)) =
        cipher.decrypt_value(sealed, USER_ENV_VARS_KEY, policy)
    else {
        return Value::Null;
    };
    let plaintext = decrypted.expose_secret();
    match serde_json::from_str(plaintext) {
        Ok(value) => value,
        Err(_) => Value::String(plaintext.to_owned()),
    }
}

fn text_value(row: &JsonData, name: &str) -> Value {
    match text(row, name) {
        Some(value) => Value::String(value.to_owned()),
        None => Value::Null,
    }
}

fn output(rows: &RowStore, cipher: &DbCipher, result: Value) -> HelperResult {
    let stored = rows.rows();
    let mut views = Vec::with_capacity(stored.len());
    for row in &stored {
        views.push(json!({
            "user_id": text_value(row, "user_id"),
            "server_id": text_value(row, "server_id"),
            "values": row_values(cipher, text(row, "values_b64")),
        }));
    }
    Ok(json!({
        "result": result,
        "rows": views,
        "events": rows.events(),
        "locks": rows.locks(),
    }))
}

pub(super) async fn get(call: &Get) -> HelperResult {
    let rows = RowStore::new(&call.rows);
    let env = UserEnvVarsStore {
        table: rows.clone(),
        transactions: rows.clone(),
        cipher: StoredCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from(call.salt_key.clone())),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        },
    };
    let values = env
        .get_user_env_vars(&call.user_id, &call.server_id)
        .await
        .or_value_error()?;
    output(&rows, &env.cipher, exposed(&values)?)
}

pub(super) async fn bulk(call: &Bulk) -> HelperResult {
    let rows = RowStore::new(&call.rows);
    let env = UserEnvVarsStore {
        table: rows.clone(),
        transactions: rows.clone(),
        cipher: StoredCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from(call.salt_key.clone())),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        },
    };
    let found = env
        .get_user_env_vars_bulk(&call.user_id, &call.server_ids)
        .await
        .or_value_error()?;
    let mut result = BTreeMap::new();
    for (server_id, values) in &found {
        result.insert(server_id.clone(), exposed(values)?);
    }
    output(
        &rows,
        &env.cipher,
        serde_json::to_value(result).or_value_error()?,
    )
}

pub(super) async fn merge(call: &Merge) -> HelperResult {
    let rows = RowStore::new(&call.rows);
    let env = UserEnvVarsStore {
        table: rows.clone(),
        transactions: rows.clone(),
        cipher: StoredCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from(call.salt_key.clone())),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        },
    };
    let updates: UserEnvVars = call
        .updates
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.clone())))
        .collect();
    let allowed: BTreeSet<String> = call.allowed_names.iter().cloned().collect();
    let merged = env
        .merge_user_env_vars(&call.user_id, &call.server_id, &updates, &allowed)
        .await
        .or_value_error()?;
    output(&rows, &env.cipher, exposed(&merged)?)
}

pub(super) async fn delete(call: &Delete) -> HelperResult {
    let rows = RowStore::new(&call.rows);
    let env = UserEnvVarsStore {
        table: rows.clone(),
        transactions: rows.clone(),
        cipher: StoredCipher {
            settings: CipherSettings {
                signing_key: Some(SecretString::from(call.salt_key.clone())),
                algorithm: Algorithm::XSalsa20Poly1305,
            },
            nonces: OsNonces,
            diagnostics: TracingDiagnostics,
        },
    };
    env.delete_user_env_vars(&call.user_id, &call.server_id)
        .await
        .or_value_error()?;
    output(&rows, &env.cipher, Value::Null)
}
