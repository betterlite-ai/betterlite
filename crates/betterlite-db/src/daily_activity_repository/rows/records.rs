use super::RowError;
use crate::budget_permission_rows::JsonData;
use crate::daily_activity_contracts::DailyActivityRow;
use chrono::NaiveDateTime;
use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;
use std::collections::BTreeMap;
use tokio_postgres::Row;

#[derive(Debug, Clone, PartialEq)]
pub struct VerificationTokenRow {
    pub token: String,
    pub key_alias: Option<String>,
    pub team_id: Option<String>,
    pub user_id: Option<String>,
    pub metadata: Option<BTreeMap<String, JsonData>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeletedVerificationTokenRow {
    pub row: VerificationTokenRow,
    pub deleted_at: NaiveDateTime,
}

pub fn verification_token_row(row: &Row) -> Result<VerificationTokenRow, RowError> {
    let metadata_text = row
        .try_get::<_, Option<String>>("metadata")
        .map_err(|source| RowError::Decode { source })?;
    let metadata = match metadata_text {
        None => None,
        Some(text) => Some(json_object(&text, "metadata")?),
    };
    Ok(VerificationTokenRow {
        token: row
            .try_get("token")
            .map_err(|source| RowError::Decode { source })?,
        key_alias: row
            .try_get("key_alias")
            .map_err(|source| RowError::Decode { source })?,
        team_id: row
            .try_get("team_id")
            .map_err(|source| RowError::Decode { source })?,
        user_id: row
            .try_get("user_id")
            .map_err(|source| RowError::Decode { source })?,
        metadata,
    })
}

fn json_object(text: &str, column: &str) -> Result<BTreeMap<String, JsonData>, RowError> {
    let value = serde_json::from_str::<JsonData>(text).map_err(|source| RowError::Json {
        column: column.to_owned(),
        source,
    })?;
    match value {
        JsonData::Object(fields) => Ok(fields),
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Array(_) => Err(RowError::ColumnType {
            column: column.to_owned(),
            expected: "a JSON object",
        }),
    }
}

pub fn deleted_verification_token_row(row: &Row) -> Result<DeletedVerificationTokenRow, RowError> {
    Ok(DeletedVerificationTokenRow {
        row: verification_token_row(row)?,
        deleted_at: row
            .try_get("deleted_at")
            .map_err(|source| RowError::Decode { source })?,
    })
}

pub fn daily_activity_row(row: &Row) -> Result<DailyActivityRow, RowError> {
    Ok(DailyActivityRow {
        id: row
            .try_get("id")
            .map_err(|source| RowError::Decode { source })?,
        date: row
            .try_get("date")
            .map_err(|source| RowError::Decode { source })?,
        api_key: row
            .try_get("api_key")
            .map_err(|source| RowError::Decode { source })?,
        model: row
            .try_get("model")
            .map_err(|source| RowError::Decode { source })?,
        model_group: row
            .try_get("model_group")
            .map_err(|source| RowError::Decode { source })?,
        custom_llm_provider: row
            .try_get("custom_llm_provider")
            .map_err(|source| RowError::Decode { source })?,
        mcp_namespaced_tool_name: row
            .try_get("mcp_namespaced_tool_name")
            .map_err(|source| RowError::Decode { source })?,
        endpoint: row
            .try_get("endpoint")
            .map_err(|source| RowError::Decode { source })?,
        prompt_tokens: row
            .try_get("prompt_tokens")
            .map_err(|source| RowError::Decode { source })?,
        completion_tokens: row
            .try_get("completion_tokens")
            .map_err(|source| RowError::Decode { source })?,
        cache_read_input_tokens: row
            .try_get("cache_read_input_tokens")
            .map_err(|source| RowError::Decode { source })?,
        cache_creation_input_tokens: row
            .try_get("cache_creation_input_tokens")
            .map_err(|source| RowError::Decode { source })?,
        compression_saved_tokens: row
            .try_get("compression_saved_tokens")
            .map_err(|source| RowError::Decode { source })?,
        api_requests: row
            .try_get("api_requests")
            .map_err(|source| RowError::Decode { source })?,
        successful_requests: row
            .try_get("successful_requests")
            .map_err(|source| RowError::Decode { source })?,
        failed_requests: row
            .try_get("failed_requests")
            .map_err(|source| RowError::Decode { source })?,
        total_response_time_ms: row
            .try_get("total_response_time_ms")
            .map_err(|source| RowError::Decode { source })?,
        timed_requests: row
            .try_get("timed_requests")
            .map_err(|source| RowError::Decode { source })?,
        compression_savings_spend: decimal_column(row, "compression_savings_spend")?,
        prompt_caching_savings_spend: decimal_column(row, "prompt_caching_savings_spend")?,
        gateway_cache_savings_spend: decimal_column(row, "gateway_injected_caching_savings_spend")?,
        autorouter_savings_spend: decimal_column(row, "autorouter_savings_spend")?,
        spend: decimal_column(row, "spend")?,
    })
}

fn decimal_column(row: &Row, column: &str) -> Result<Decimal, RowError> {
    let value = row
        .try_get::<_, f64>(column)
        .map_err(|source| RowError::Decode { source })?;
    Decimal::from_f64(value).ok_or_else(|| RowError::NonFiniteNumber {
        column: column.to_owned(),
    })
}
