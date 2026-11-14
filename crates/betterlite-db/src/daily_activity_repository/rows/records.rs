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

macro_rules! column {
    ($row:expr, $name:literal) => {
        $row.try_get($name)
            .map_err(|source| RowError::Decode { source })?
    };
}

pub fn verification_token_row(row: &Row) -> Result<VerificationTokenRow, RowError> {
    let metadata_text = row
        .try_get::<_, Option<String>>("metadata")
        .map_err(|source| RowError::Decode { source })?;
    let metadata = match metadata_text {
        None => None,
        Some(text) => json_object(&text, "metadata")?,
    };
    Ok(VerificationTokenRow {
        token: column!(row, "token"),
        key_alias: column!(row, "key_alias"),
        team_id: column!(row, "team_id"),
        user_id: column!(row, "user_id"),
        metadata,
    })
}

fn json_object(text: &str, column: &str) -> Result<Option<BTreeMap<String, JsonData>>, RowError> {
    let value = serde_json::from_str::<JsonData>(text).map_err(|source| RowError::Json {
        column: column.to_owned(),
        source,
    })?;
    match value {
        JsonData::Object(fields) => Ok(Some(fields)),
        JsonData::Null => Ok(None),
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::String(_) | JsonData::Array(_) => {
            Err(RowError::ColumnType {
                column: column.to_owned(),
                expected: "a JSON object",
            })
        }
    }
}

pub fn deleted_verification_token_row(row: &Row) -> Result<DeletedVerificationTokenRow, RowError> {
    Ok(DeletedVerificationTokenRow {
        row: verification_token_row(row)?,
        deleted_at: column!(row, "deleted_at"),
    })
}

pub fn daily_activity_row(row: &Row) -> Result<DailyActivityRow, RowError> {
    Ok(DailyActivityRow {
        id: column!(row, "id"),
        date: column!(row, "date"),
        api_key: column!(row, "api_key"),
        model: column!(row, "model"),
        model_group: column!(row, "model_group"),
        custom_llm_provider: column!(row, "custom_llm_provider"),
        mcp_namespaced_tool_name: column!(row, "mcp_namespaced_tool_name"),
        endpoint: column!(row, "endpoint"),
        prompt_tokens: column!(row, "prompt_tokens"),
        completion_tokens: column!(row, "completion_tokens"),
        cache_read_input_tokens: column!(row, "cache_read_input_tokens"),
        cache_creation_input_tokens: column!(row, "cache_creation_input_tokens"),
        compression_saved_tokens: column!(row, "compression_saved_tokens"),
        api_requests: column!(row, "api_requests"),
        successful_requests: column!(row, "successful_requests"),
        failed_requests: column!(row, "failed_requests"),
        total_response_time_ms: column!(row, "total_response_time_ms"),
        timed_requests: column!(row, "timed_requests"),
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
