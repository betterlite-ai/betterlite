use std::collections::BTreeSet;

use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use thiserror::Error;
use tokio_postgres::Client;
use tokio_postgres::types::ToSql;

use crate::seed::{FixtureSeed, KeyRow};

mod tests;

pub const SCHEMA: &str = include_str!("../fixtures/daily_usage.sql");

const MAX_ROWS: usize = 256;
const ID_BYTES: usize = 64;
const TEXT_BYTES: usize = 4096;
const CREATED_AT: &str = "2026-01-01T00:00:00Z";

const USER_INSERT: &str = "INSERT INTO \"LiteLLM_DailyUserSpend\" (\"id\",\"user_id\",\"date\",\"api_key\",\"model\",\"model_group\",\"custom_llm_provider\",\"spend\",\"prompt_tokens\",\"completion_tokens\",\"api_requests\",\"successful_requests\",\"failed_requests\",\"created_at\",\"updated_at\") VALUES ($1,$2,$3,$4,$5,$6,$7,$8::numeric::double precision,$9,$10,$11,$12,$13,$14::text::timestamp(3),$15::text::timestamp(3))";

const TEAM_INSERT: &str = "INSERT INTO \"LiteLLM_DailyTeamSpend\" (\"id\",\"team_id\",\"date\",\"api_key\",\"model\",\"model_group\",\"custom_llm_provider\",\"spend\",\"prompt_tokens\",\"completion_tokens\",\"api_requests\",\"successful_requests\",\"failed_requests\",\"created_at\",\"updated_at\") VALUES ($1,$2,$3,$4,$5,$6,$7,$8::numeric::double precision,$9,$10,$11,$12,$13,$14::text::timestamp(3),$15::text::timestamp(3))";

const TAG_INSERT: &str = "INSERT INTO \"LiteLLM_DailyTagSpend\" (\"id\",\"tag\",\"date\",\"api_key\",\"model\",\"model_group\",\"custom_llm_provider\",\"spend\",\"prompt_tokens\",\"completion_tokens\",\"api_requests\",\"successful_requests\",\"failed_requests\",\"created_at\",\"updated_at\") VALUES ($1,$2,$3,$4,$5,$6,$7,$8::numeric::double precision,$9,$10,$11,$12,$13,$14::text::timestamp(3),$15::text::timestamp(3))";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DailyUsageTable {
    User,
    Team,
    Tag,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DailyUsageRow {
    pub table: DailyUsageTable,
    pub id: String,
    pub entity_id: Option<String>,
    pub date: String,
    pub key_index: usize,
    pub model: Option<String>,
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub spend: Decimal,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub api_requests: i64,
    #[serde(default)]
    pub successful_requests: i64,
    #[serde(default)]
    pub failed_requests: i64,
}

#[derive(Debug, Error)]
pub enum DailyUsageError {
    #[error("daily usage fixture has {count} rows, more than the limit of {limit}")]
    TooManyRows { count: usize, limit: usize },
    #[error("daily usage row {row} has key_index {key_index}, but only {keys} keys are seeded")]
    UnknownKey {
        row: usize,
        key_index: usize,
        keys: usize,
    },
    #[error("daily usage row {row} repeats {table:?} id {id}")]
    DuplicateRow {
        row: usize,
        table: DailyUsageTable,
        id: String,
    },
    #[error("daily usage row {row} field {field} is {bytes} bytes, over the limit of {limit}")]
    TextTooLong {
        row: usize,
        field: &'static str,
        bytes: usize,
        limit: usize,
    },
    #[error("daily usage row {row} counter {field} is {value}, but counters cannot be negative")]
    NegativeCounter {
        row: usize,
        field: &'static str,
        value: i64,
    },
    #[error("daily usage fixture database operation failed: {0}")]
    Database(#[source] tokio_postgres::Error),
}

pub fn validate(rows: &[DailyUsageRow], keys: &[KeyRow]) -> Result<(), DailyUsageError> {
    if rows.len() > MAX_ROWS {
        return Err(DailyUsageError::TooManyRows {
            count: rows.len(),
            limit: MAX_ROWS,
        });
    }
    let mut identities = BTreeSet::new();
    for (row_index, row) in rows.iter().enumerate() {
        validate_row(row_index, row, keys.len())?;
        if !identities.insert((row.table, row.id.as_str())) {
            return Err(DailyUsageError::DuplicateRow {
                row: row_index,
                table: row.table,
                id: row.id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_row(
    index: usize,
    row: &DailyUsageRow,
    key_count: usize,
) -> Result<(), DailyUsageError> {
    if row.key_index >= key_count {
        return Err(DailyUsageError::UnknownKey {
            row: index,
            key_index: row.key_index,
            keys: key_count,
        });
    }
    check_text(index, "id", &row.id, ID_BYTES)?;
    check_text(index, "date", &row.date, ID_BYTES)?;
    for (field, value) in [
        ("entity_id", &row.entity_id),
        ("model", &row.model),
        ("model_group", &row.model_group),
        ("custom_llm_provider", &row.custom_llm_provider),
    ] {
        if let Some(text) = value {
            check_text(index, field, text, TEXT_BYTES)?;
        }
    }
    for (field, value) in [
        ("prompt_tokens", row.prompt_tokens),
        ("completion_tokens", row.completion_tokens),
        ("api_requests", row.api_requests),
        ("successful_requests", row.successful_requests),
        ("failed_requests", row.failed_requests),
    ] {
        if value < 0 {
            return Err(DailyUsageError::NegativeCounter {
                row: index,
                field,
                value,
            });
        }
    }
    Ok(())
}

const fn check_text(
    row: usize,
    field: &'static str,
    text: &str,
    limit: usize,
) -> Result<(), DailyUsageError> {
    if text.len() > limit {
        return Err(DailyUsageError::TextTooLong {
            row,
            field,
            bytes: text.len(),
            limit,
        });
    }
    Ok(())
}

pub async fn seed(
    client: &Client,
    rows: &[DailyUsageRow],
    keys: &[KeyRow],
) -> Result<(), DailyUsageError> {
    validate(rows, keys)?;
    for (row_index, row) in rows.iter().enumerate() {
        insert_row(client, row, row_index, keys).await?;
    }
    Ok(())
}

async fn insert_row(
    client: &Client,
    row: &DailyUsageRow,
    row_index: usize,
    keys: &[KeyRow],
) -> Result<(), DailyUsageError> {
    let key = keys.get(row.key_index).ok_or(DailyUsageError::UnknownKey {
        row: row_index,
        key_index: row.key_index,
        keys: keys.len(),
    })?;
    let api_key = FixtureSeed::key_hash(key.token.expose_secret());
    let statement = match row.table {
        DailyUsageTable::User => USER_INSERT,
        DailyUsageTable::Team => TEAM_INSERT,
        DailyUsageTable::Tag => TAG_INSERT,
    };
    let params: &[&(dyn ToSql + Sync)] = &[
        &row.id,
        &row.entity_id,
        &row.date,
        &api_key,
        &row.model,
        &row.model_group,
        &row.custom_llm_provider,
        &row.spend,
        &row.prompt_tokens,
        &row.completion_tokens,
        &row.api_requests,
        &row.successful_requests,
        &row.failed_requests,
        &CREATED_AT,
        &CREATED_AT,
    ];
    client
        .execute(statement, params)
        .await
        .map_err(DailyUsageError::Database)?;
    Ok(())
}
