use std::collections::BTreeMap;

use betterlite_config::sections::settings::{JsonValue, Section};
use betterlite_db::postgres_store::errors::backend;
use tokio_postgres::Row;
use tokio_postgres::types::ToSql;

use super::{ConfigSectionTransaction, ConfigStateError};

// _CONFIG_SECTION_LOCK_SQL: one transaction at a time merges a section, so two saves cannot drop each other's keys
pub const CONFIG_SECTION_LOCK_SQL: &str =
    "SELECT 1 AS locked FROM pg_advisory_xact_lock(hashtext($1))";

pub const READ_CONFIG_SQL: &str =
    "SELECT \"param_value\"::text FROM \"LiteLLM_Config\" WHERE \"param_name\" = $1";

pub const UPSERT_CONFIG_SQL: &str = "INSERT INTO \"LiteLLM_Config\" (\"param_name\", \"param_value\") \
     VALUES ($1, $2::text::jsonb) ON CONFLICT (\"param_name\") \
     DO UPDATE SET \"param_value\" = EXCLUDED.\"param_value\"";

impl ConfigSectionTransaction for deadpool_postgres::Transaction<'_> {
    async fn lock_section(&self, section: Section) -> Result<(), ConfigStateError> {
        run(self, CONFIG_SECTION_LOCK_SQL, &[&section.name()])
            .await
            .map(drop)
    }

    async fn stored_values(
        &self,
        section: Section,
    ) -> Result<Option<BTreeMap<String, JsonValue>>, ConfigStateError> {
        let rows = run(self, READ_CONFIG_SQL, &[&section.name()]).await?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let stored: Option<String> =
            row.try_get(0)
                .map_err(|source| ConfigStateError::Database {
                    source: backend(source),
                })?;
        let Some(text) = stored else {
            return Ok(None);
        };
        decode_section_values(section, &text).map(Some)
    }

    async fn write_values(
        &self,
        section: Section,
        values: &BTreeMap<String, JsonValue>,
    ) -> Result<(), ConfigStateError> {
        let section_value = JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        );
        let encoded = encode_config_value(&section_value)?;
        let name = section.name();
        run(self, UPSERT_CONFIG_SQL, &[&name, &encoded])
            .await
            .map(drop)
    }
}

// insert_data stores json.dumps(param) into the Json column, so every config write is one JSON text inside another
pub fn encode_config_value(value: &JsonValue) -> Result<String, ConfigStateError> {
    let inner =
        serde_json::to_string(value).map_err(|source| ConfigStateError::Encode { source })?;
    serde_json::to_string(&JsonValue::String(inner))
        .map_err(|source| ConfigStateError::Encode { source })
}

pub fn decode_section_values(
    section: Section,
    stored: &str,
) -> Result<BTreeMap<String, JsonValue>, ConfigStateError> {
    let parsed: JsonValue =
        serde_json::from_str(stored).map_err(|source| ConfigStateError::Decode { source })?;
    if let JsonValue::String(inner) = &parsed {
        let decoded: JsonValue =
            serde_json::from_str(inner).map_err(|source| ConfigStateError::Decode { source })?;
        return section_object(section, decoded);
    }
    section_object(section, parsed)
}

// A section row holds a mapping; a JSON null reads as a row that was never written
fn section_object(
    section: Section,
    value: JsonValue,
) -> Result<BTreeMap<String, JsonValue>, ConfigStateError> {
    match value {
        JsonValue::Object(entries) => Ok(entries.into_iter().collect()),
        JsonValue::Null => Ok(BTreeMap::new()),
        JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::String(_) | JsonValue::Array(_) => {
            Err(ConfigStateError::InvalidStoredSection {
                section: section.name(),
            })
        }
    }
}

async fn run(
    transaction: &deadpool_postgres::Transaction<'_>,
    sql: &str,
    parameters: &[&(dyn ToSql + Sync)],
) -> Result<Vec<Row>, ConfigStateError> {
    transaction
        .query(sql, parameters)
        .await
        .map_err(|source| ConfigStateError::Database {
            source: backend(source),
        })
}
