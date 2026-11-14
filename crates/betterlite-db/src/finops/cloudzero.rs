use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use serde::Serialize;
use tokio_postgres::{Row, types::ToSql};

use crate::budget_permission_rows::JsonData;
use crate::key_attribution::error::KeyAttributionError;
use crate::key_attribution::projection::ExportFields;
use crate::key_attribution::recovery::fill_missing_api_key_aliases;
use crate::postgres_store::diagnostics::DriverError;
use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

pub type UsageRow = BTreeMap<String, JsonData>;
pub type UsageData = Vec<UsageRow>;

// LiteLLM selects dus.spend as a float; betterlite reads money as NUMERIC so the row keeps Decimal.
const CLOUDZERO_USAGE_SELECT: &str = "SELECT dus.id, dus.date, dus.user_id, dus.api_key, dus.model, \
    dus.model_group, dus.custom_llm_provider, dus.prompt_tokens, dus.completion_tokens, \
    dus.spend::numeric AS spend, dus.api_requests, dus.successful_requests, \
    dus.failed_requests, dus.cache_creation_input_tokens, dus.cache_read_input_tokens, \
    dus.created_at, dus.updated_at, vt.team_id, vt.key_alias AS api_key_alias, tt.team_alias, \
    ut.user_email AS user_email FROM \"LiteLLM_DailyUserSpend\" dus \
    LEFT JOIN \"LiteLLM_VerificationToken\" vt ON dus.api_key = vt.token \
    LEFT JOIN \"LiteLLM_TeamTable\" tt ON vt.team_id = tt.team_id \
    LEFT JOIN \"LiteLLM_UserTable\" ut ON dus.user_id = ut.user_id";

const CLOUDZERO_USAGE_FILTER: &str = " WHERE ($1::timestamptz IS NULL OR \
    dus.updated_at >= $1::timestamptz) AND ($2::timestamptz IS NULL OR \
    dus.updated_at <= $2::timestamptz) ORDER BY dus.date DESC, dus.created_at DESC";

#[derive(Debug, thiserror::Error)]
pub enum CloudZeroUsageError {
    #[error("the usage row is not readable")]
    Row {
        #[source]
        source: DriverError,
    },
    #[error("the usage row is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the usage data could not be read")]
    Database {
        #[from]
        source: DbError,
    },
    #[error("the API key aliases could not be recovered")]
    Attribution {
        #[from]
        source: KeyAttributionError,
    },
}

#[derive(Serialize)]
struct CloudZeroUsageRow {
    id: String,
    date: String,
    user_id: Option<String>,
    api_key: String,
    model: String,
    model_group: Option<String>,
    custom_llm_provider: Option<String>,
    prompt_tokens: i64,
    completion_tokens: i64,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    spend: Decimal,
    api_requests: i64,
    successful_requests: i64,
    failed_requests: i64,
    cache_creation_input_tokens: i64,
    cache_read_input_tokens: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    team_id: Option<String>,
    api_key_alias: Option<String>,
    team_alias: Option<String>,
    user_email: Option<String>,
}

#[derive(Debug)]
struct UsageQuery {
    sql: String,
    start_time_utc: Option<DateTime<Utc>>,
    end_time_utc: Option<DateTime<Utc>>,
    limit: Option<i64>,
}

impl UsageQuery {
    fn bind(&self) -> Vec<&(dyn ToSql + Sync)> {
        let mut values: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(3);
        values.push(&self.start_time_utc);
        values.push(&self.end_time_utc);
        if let Some(limit) = &self.limit {
            values.push(limit);
        }
        values
    }
}

fn usage_query(
    limit: Option<i64>,
    start_time_utc: Option<DateTime<Utc>>,
    end_time_utc: Option<DateTime<Utc>>,
) -> UsageQuery {
    let mut sql = format!("{CLOUDZERO_USAGE_SELECT}{CLOUDZERO_USAGE_FILTER}");
    if limit.is_some() {
        sql.push_str(" LIMIT $3");
    }
    UsageQuery {
        sql,
        start_time_utc,
        end_time_utc,
        limit,
    }
}

pub async fn get_usage_data(
    store: &PostgresStore,
    limit: Option<i64>,
    start_time_utc: Option<DateTime<Utc>>,
    end_time_utc: Option<DateTime<Utc>>,
) -> Result<UsageData, CloudZeroUsageError> {
    let query = usage_query(limit, start_time_utc, end_time_utc);
    let loaded: Result<Vec<Row>, DbError> = {
        let client = store.session().await?;
        let bound = query.bind();
        client.query(&query.sql, &bound).await.map_err(backend)
    };
    let usage: UsageData = loaded?
        .iter()
        .map(decode_usage_row)
        .collect::<Result<Vec<UsageRow>, CloudZeroUsageError>>()?;
    let fields = ExportFields::default();
    let recovered = fill_missing_api_key_aliases(store, &usage, &fields).await?;
    Ok(recovered)
}

fn decode_columns(row: &Row) -> Result<CloudZeroUsageRow, tokio_postgres::Error> {
    Ok(CloudZeroUsageRow {
        id: row.try_get("id")?,
        date: row.try_get("date")?,
        user_id: row.try_get("user_id")?,
        api_key: row.try_get("api_key")?,
        model: row.try_get("model")?,
        model_group: row.try_get("model_group")?,
        custom_llm_provider: row.try_get("custom_llm_provider")?,
        prompt_tokens: row.try_get("prompt_tokens")?,
        completion_tokens: row.try_get("completion_tokens")?,
        spend: row.try_get("spend")?,
        api_requests: row.try_get("api_requests")?,
        successful_requests: row.try_get("successful_requests")?,
        failed_requests: row.try_get("failed_requests")?,
        cache_creation_input_tokens: row.try_get("cache_creation_input_tokens")?,
        cache_read_input_tokens: row.try_get("cache_read_input_tokens")?,
        created_at: row.try_get::<_, NaiveDateTime>("created_at")?.and_utc(),
        updated_at: row.try_get::<_, NaiveDateTime>("updated_at")?.and_utc(),
        team_id: row.try_get("team_id")?,
        api_key_alias: row.try_get("api_key_alias")?,
        team_alias: row.try_get("team_alias")?,
        user_email: row.try_get("user_email")?,
    })
}

fn decode_usage_row(row: &Row) -> Result<UsageRow, CloudZeroUsageError> {
    let decoded = decode_columns(row).map_err(|driver| CloudZeroUsageError::Row {
        source: DriverError { driver },
    })?;
    let text =
        serde_json::to_string(&decoded).map_err(|source| CloudZeroUsageError::Json { source })?;
    serde_json::from_str(&text).map_err(|source| CloudZeroUsageError::Json { source })
}

mod tests;
