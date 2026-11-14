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
const FOCUS_USAGE_SELECT: &str = "SELECT dus.id, dus.date, dus.user_id, dus.api_key, dus.model, \
    dus.model_group, dus.custom_llm_provider, dus.prompt_tokens, dus.completion_tokens, \
    dus.spend::numeric AS spend, dus.api_requests, dus.successful_requests, \
    dus.failed_requests, dus.cache_creation_input_tokens, dus.cache_read_input_tokens, \
    dus.created_at, dus.updated_at, vt.team_id, vt.key_alias AS api_key_alias, tt.team_alias, \
    ut.user_email AS user_email, \
    COALESCE(vt.organization_id, tt.organization_id) AS organization_id, \
    ot.organization_alias AS organization_alias FROM \"LiteLLM_DailyUserSpend\" dus \
    LEFT JOIN \"LiteLLM_VerificationToken\" vt ON dus.api_key = vt.token \
    LEFT JOIN \"LiteLLM_TeamTable\" tt ON vt.team_id = tt.team_id \
    LEFT JOIN \"LiteLLM_UserTable\" ut ON dus.user_id = ut.user_id \
    LEFT JOIN \"LiteLLM_OrganizationTable\" ot \
    ON ot.organization_id = COALESCE(vt.organization_id, tt.organization_id)";

#[derive(Debug, thiserror::Error)]
pub enum FocusUsageError {
    #[error("limit must be non-negative")]
    NegativeLimit,
    #[error("SQL bind index overflow")]
    BindOverflow,
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
struct FocusUsageRow {
    id: String,
    date: String,
    user_id: Option<String>,
    api_key: String,
    model: Option<String>,
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
    organization_id: Option<String>,
    organization_alias: Option<String>,
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
        if let Some(start) = &self.start_time_utc {
            values.push(start);
        }
        if let Some(end) = &self.end_time_utc {
            values.push(end);
        }
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
) -> Result<UsageQuery, FocusUsageError> {
    if limit.is_some_and(|value| value < 0) {
        return Err(FocusUsageError::NegativeLimit);
    }
    let mut filters = Vec::new();
    let mut index = 1_usize;
    if start_time_utc.is_some() {
        filters.push(format!("dus.updated_at >= ${index}::timestamptz"));
        index = index.checked_add(1).ok_or(FocusUsageError::BindOverflow)?;
    }
    if end_time_utc.is_some() {
        filters.push(format!("dus.updated_at <= ${index}::timestamptz"));
        index = index.checked_add(1).ok_or(FocusUsageError::BindOverflow)?;
    }
    let where_clause = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };
    let limit_clause = if limit.is_some() {
        format!(" LIMIT ${index}")
    } else {
        String::new()
    };
    let sql = format!(
        "{FOCUS_USAGE_SELECT}{where_clause} ORDER BY dus.date DESC, dus.created_at DESC{limit_clause}"
    );
    Ok(UsageQuery {
        sql,
        start_time_utc,
        end_time_utc,
        limit,
    })
}

pub async fn get_usage_data(
    store: &PostgresStore,
    limit: Option<i64>,
    start_time_utc: Option<DateTime<Utc>>,
    end_time_utc: Option<DateTime<Utc>>,
) -> Result<UsageData, FocusUsageError> {
    let query = usage_query(limit, start_time_utc, end_time_utc)?;
    let loaded: Result<Vec<Row>, DbError> = {
        let client = store.session().await?;
        let bound = query.bind();
        client.query(&query.sql, &bound).await.map_err(backend)
    };
    let usage = loaded?
        .iter()
        .map(decode_usage_row)
        .collect::<Result<Vec<UsageRow>, FocusUsageError>>()?;
    let fields = ExportFields::default();
    let recovered = fill_missing_api_key_aliases(store, &usage, &fields).await?;
    Ok(recovered)
}

fn decode_columns(row: &Row) -> Result<FocusUsageRow, tokio_postgres::Error> {
    Ok(FocusUsageRow {
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
        organization_id: row.try_get("organization_id")?,
        organization_alias: row.try_get("organization_alias")?,
    })
}

fn decode_usage_row(row: &Row) -> Result<UsageRow, FocusUsageError> {
    let decoded = decode_columns(row).map_err(|driver| FocusUsageError::Row {
        source: DriverError { driver },
    })?;
    let text =
        serde_json::to_string(&decoded).map_err(|source| FocusUsageError::Json { source })?;
    serde_json::from_str(&text).map_err(|source| FocusUsageError::Json { source })
}

mod tests;
