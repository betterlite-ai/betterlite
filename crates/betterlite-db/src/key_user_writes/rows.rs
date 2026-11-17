use std::collections::BTreeMap;

use tokio_postgres::Row;

use crate::budget_permission_rows::JsonData;
use crate::key_repository::decode::TokenDecodeError;
use crate::postgres_store::errors::backend;

use super::{KeyUserWriteError, WriteRowError};

pub(super) fn row_text(row: &Row) -> Result<String, KeyUserWriteError> {
    let text: String = row
        .try_get(0)
        .map_err(|driver| KeyUserWriteError::Database {
            source: backend::<WriteRowError>(driver),
        })?;
    Ok(text)
}

pub(super) fn row_json(row: &Row) -> Result<JsonData, KeyUserWriteError> {
    let text = row_text(row)?;
    serde_json::from_str(&text).map_err(|source| {
        KeyUserWriteError::Row(WriteRowError::Key(TokenDecodeError::RowJson(source)))
    })
}

pub(super) fn row_object(row: &Row) -> Result<BTreeMap<String, JsonData>, KeyUserWriteError> {
    match row_json(row)? {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(KeyUserWriteError::Row(WriteRowError::NotAnObject)),
    }
}
