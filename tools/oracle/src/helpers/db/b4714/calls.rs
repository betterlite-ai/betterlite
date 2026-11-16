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

type EnvStore = UserEnvVarsStore<RowStore, RowStore, OsNonces, TracingDiagnostics>;

fn cipher(salt: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(salt.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn store(rows: &RowStore, cipher: DbCipher) -> EnvStore {
    UserEnvVarsStore {
        table: rows.clone(),
        transactions: rows.clone(),
        cipher,
    }
}

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
    let env = store(&rows, cipher(&call.salt_key));
    let values = env
        .get_user_env_vars(&call.user_id, &call.server_id)
        .await
        .or_value_error()?;
    output(&rows, &cipher(&call.salt_key), exposed(&values)?)
}

pub(super) async fn bulk(call: &Bulk) -> HelperResult {
    let rows = RowStore::new(&call.rows);
    let env = store(&rows, cipher(&call.salt_key));
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
        &cipher(&call.salt_key),
        serde_json::to_value(result).or_value_error()?,
    )
}

pub(super) async fn merge(call: &Merge) -> HelperResult {
    let rows = RowStore::new(&call.rows);
    let env = store(&rows, cipher(&call.salt_key));
    let allowed: BTreeSet<String> = call.allowed_names.iter().cloned().collect();
    let merged = env
        .merge_user_env_vars(&call.user_id, &call.server_id, &call.updates, &allowed)
        .await
        .or_value_error()?;
    output(&rows, &cipher(&call.salt_key), exposed(&merged)?)
}

pub(super) async fn delete(call: &Delete) -> HelperResult {
    let rows = RowStore::new(&call.rows);
    let env = store(&rows, cipher(&call.salt_key));
    env.delete_user_env_vars(&call.user_id, &call.server_id)
        .await
        .or_value_error()?;
    output(&rows, &cipher(&call.salt_key), Value::Null)
}
