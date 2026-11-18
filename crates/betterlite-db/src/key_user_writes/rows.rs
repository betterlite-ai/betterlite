use std::collections::BTreeMap;

use tokio_postgres::Row;

use crate::budget_permission_rows::JsonData;
use crate::identity_rows::user::LiteLLMUserTable;
use crate::key_repository::decode::TokenDecodeError;
use crate::postgres_store::errors::{DbError, backend};
use crate::user_repository::projection::user_record_to_model;

use super::{KeyUserWriteError, WriteRowError};

// only these columns store lists the row models read as plain vectors, so a stored NULL needs the model's default
pub(super) const KEY_LIST_COLUMNS: [&str; 1] = ["models"];
pub(super) const USER_LIST_COLUMNS: [&str; 4] =
    ["models", "teams", "allowed_cache_controls", "policies"];

pub(super) fn row_text(row: &Row) -> Result<String, KeyUserWriteError> {
    row.try_get(0)
        .map_err(|driver| KeyUserWriteError::Database {
            source: backend::<WriteRowError>(driver),
        })
}

pub(super) fn row_json(row: &Row) -> Result<JsonData, KeyUserWriteError> {
    let text = row_text(row)?;
    serde_json::from_str(&text).map_err(|source| {
        KeyUserWriteError::Row(WriteRowError::Key(TokenDecodeError::RowJson(source)))
    })
}

pub(super) fn row_object(row: &Row) -> Result<BTreeMap<String, JsonData>, KeyUserWriteError> {
    let JsonData::Object(fields) = row_json(row)? else {
        return Err(KeyUserWriteError::Row(WriteRowError::NotAnObject));
    };
    Ok(fields)
}

pub(super) fn fill_null_lists(fields: &mut BTreeMap<String, JsonData>, names: &[&str]) {
    for name in names {
        if matches!(fields.get(*name), Some(JsonData::Null)) {
            fields.insert(String::from(*name), JsonData::Array(Vec::new()));
        }
    }
}

pub(super) fn decode_user_row(row: &Row) -> Result<LiteLLMUserTable, KeyUserWriteError> {
    let mut fields = row_object(row)?;
    fill_null_lists(&mut fields, &USER_LIST_COLUMNS);
    user_record_to_model(Some(&JsonData::Object(fields)))
        .map_err(|source| KeyUserWriteError::Row(WriteRowError::User(source)))?
        .ok_or(KeyUserWriteError::Database {
            source: DbError::CreateNull,
        })
}

pub(super) fn relation_id(fields: &BTreeMap<String, JsonData>, name: &str) -> Option<String> {
    match fields.get(name) {
        Some(JsonData::String(value)) => Some(value.clone()),
        _ => None,
    }
}
