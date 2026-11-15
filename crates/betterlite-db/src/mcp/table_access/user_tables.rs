use std::sync::Arc;

use betterlite_redis::config_sync::table_actions::TableActions as _;
use uuid::Uuid;

use crate::budget_permission_rows::JsonData;

use super::actions::_user_credential_actions;
use super::rows::{CREDENTIAL_TABLE, ENV_VAR_TABLE, KeyValues, RowFields};
use super::sql;
use super::store::McpTableStore;
use super::types::McpTableError;

// Prisma's find_unique over the user_id_server_id compound key
pub async fn _db_find_user_credential_row(
    store: &McpTableStore<'_>,
    user_id: &str,
    server_id: &str,
) -> Result<Option<JsonData>, McpTableError> {
    let key = KeyValues::from([
        ("user_id", user_id.to_owned()),
        ("server_id", server_id.to_owned()),
    ]);
    Ok(_db_find_user_credential_rows(store, Some(&key))
        .await?
        .pop())
}

// ast-grep-ignore: no-one-line-functions
pub async fn _db_find_user_credential_rows(
    store: &McpTableStore<'_>,
    filter: Option<&KeyValues>,
) -> Result<Vec<JsonData>, McpTableError> {
    sql::select_keyed(store.reader(), &CREDENTIAL_TABLE, filter).await
}

// the create arm needs a fresh id; on conflict only credential_b64 and updated_at change
pub async fn _db_upsert_user_credential_row(
    store: &McpTableStore<'_>,
    user_id: &str,
    server_id: &str,
    credential_b64: &str,
) -> Result<(), McpTableError> {
    let table = _user_credential_actions(Some(store.client), Arc::clone(&store.publish))?;
    let id = Uuid::new_v4().to_string();
    let data = RowFields::texts(&[
        ("id", id.as_str()),
        ("user_id", user_id),
        ("server_id", server_id),
        ("credential_b64", credential_b64),
    ]);
    drop(table.upsert(KeyValues::new(), data).await?);
    Ok(())
}

// ast-grep-ignore: no-one-line-functions
pub async fn _db_find_user_env_var_rows(
    store: &McpTableStore<'_>,
    filter: Option<&KeyValues>,
) -> Result<Vec<JsonData>, McpTableError> {
    sql::select_keyed(store.reader(), &ENV_VAR_TABLE, filter).await
}
