use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::postgres_store::errors::{DbError, backend};
use betterlite_db::postgres_store::session::PostgresStore;
use thiserror::Error;
use tokio_postgres::types::ToSql;

use super::metadata::{flatten_metadata_for_poll, merge_metadata};

const SELECT_METADATA_SQL: &str =
    r#"SELECT metadata::text AS metadata FROM "LiteLLM_UserTable" WHERE user_id = $1"#;
const UPDATE_METADATA_SQL: &str =
    r#"UPDATE "LiteLLM_UserTable" SET metadata = $2::text::jsonb WHERE user_id = $1"#;

#[derive(Debug, Error)]
pub enum PersistError {
    #[error("the stored user metadata cannot be read")]
    Decode(#[source] serde_json::Error),
    #[error("the merged user metadata cannot be written")]
    Encode(#[source] serde_json::Error),
    #[error("the user metadata query failed")]
    Database(#[from] DbError),
}

// LiteLLM's metadata column projection: only a JSON object counts, and a JSON text string is decoded first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserMetadataRow {
    pub metadata: Option<BTreeMap<String, JsonData>>,
}

impl UserMetadataRow {
    pub fn decode(text: &str) -> Result<Self, PersistError> {
        let stored: JsonData = serde_json::from_str(text).map_err(PersistError::Decode)?;
        let JsonData::String(encoded) = stored else {
            return Ok(Self {
                metadata: object_entries(stored),
            });
        };
        let Ok(decoded) = serde_json::from_str::<JsonData>(&encoded) else {
            return Ok(Self::default());
        };
        Ok(Self {
            metadata: object_entries(decoded),
        })
    }
}

// A failed write ends the login the same way as LiteLLM: the error is logged and the login continues.
pub async fn persist_attribution_metadata(
    store: &PostgresStore,
    user_id: &str,
    attribution: &BTreeMap<String, JsonData>,
) {
    if attribution.is_empty() {
        return;
    }
    if let Err(error) = merge_into_user(store, user_id, attribution).await {
        tracing::error!(user_id, error = %error, "failed to persist CLI SSO attribution metadata");
    }
}

async fn merge_into_user(
    store: &PostgresStore,
    user_id: &str,
    attribution: &BTreeMap<String, JsonData>,
) -> Result<(), PersistError> {
    let mut merged = find_user_metadata(store, user_id)
        .await?
        .unwrap_or_default();
    merge_metadata(&mut merged, attribution);
    let text = serde_json::to_string(&JsonData::Object(merged)).map_err(PersistError::Encode)?;
    let client = store.session().await?;
    let parameters: [&(dyn ToSql + Sync); 2] = [&user_id, &text];
    client
        .execute(UPDATE_METADATA_SQL, &parameters)
        .await
        .map_err(backend)?;
    let keys = flatten_metadata_for_poll(attribution)
        .into_keys()
        .collect::<Vec<String>>();
    tracing::info!(user_id, keys = ?keys, "persisted CLI SSO attribution metadata");
    Ok(())
}

async fn find_user_metadata(
    store: &PostgresStore,
    user_id: &str,
) -> Result<Option<BTreeMap<String, JsonData>>, PersistError> {
    let client = store.session().await?;
    let row = client
        .query_opt(SELECT_METADATA_SQL, &[&user_id])
        .await
        .map_err(backend)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let text: String = row.try_get("metadata").map_err(backend)?;
    Ok(UserMetadataRow::decode(&text)?.metadata)
}

fn object_entries(value: JsonData) -> Option<BTreeMap<String, JsonData>> {
    let JsonData::Object(entries) = value else {
        return None;
    };
    Some(entries)
}
