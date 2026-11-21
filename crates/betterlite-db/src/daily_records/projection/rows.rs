use rust_decimal::Decimal;

use crate::daily_spend_sql::entities::{DailySpendEntity, SpendRow};
use crate::daily_spend_sql::records::BaseDailySpendTransaction;
use crate::daily_spend_sql::values::SpendColumnValue;

use super::super::DailyRecordError;
use super::evidence::request_counts;
use super::presence;
use super::{DailyEvidence, DailyPayload};

// The source's BaseDailySpendTransaction: every column of the logged row that the daily rollup keeps.
pub(super) fn base_transaction(
    payload: &DailyPayload,
    date: String,
    evidence: &DailyEvidence,
) -> Result<BaseDailySpendTransaction, DailyRecordError> {
    let Some(api_key) = presence(&payload.api_key) else {
        return Err(DailyRecordError::InvalidField { field: "api_key" });
    };
    let internal = evidence.internal;
    let (api_requests, successful_requests, failed_requests) =
        request_counts(internal, evidence.status);
    Ok(BaseDailySpendTransaction {
        date,
        api_key: api_key.to_owned(),
        model: presence(&payload.model).map(str::to_owned),
        model_group: presence(&payload.model_group).map(str::to_owned),
        mcp_namespaced_tool_name: presence(&payload.mcp_namespaced_tool_name).map(str::to_owned),
        custom_llm_provider: presence(&payload.custom_llm_provider).map(str::to_owned),
        endpoint: evidence.endpoint.clone(),
        prompt_tokens: payload.prompt_tokens.unwrap_or_default(),
        completion_tokens: payload.completion_tokens.unwrap_or_default(),
        cache_read_input_tokens: evidence.cache_read,
        cache_creation_input_tokens: evidence.cache_creation,
        compression_saved_tokens: evidence.compression_saved,
        compression_savings_spend: evidence.savings.compression,
        prompt_caching_savings_spend: evidence.savings.prompt_caching,
        gateway_cache_savings_spend: evidence.savings.gateway_injected_caching,
        autorouter_savings_spend: if internal {
            Decimal::ZERO
        } else {
            evidence.savings.autorouter
        },
        spend: payload.spend.unwrap_or_default(),
        api_requests,
        successful_requests,
        failed_requests,
        total_response_time_ms: evidence.timed.unwrap_or_default(),
        timed_requests: i64::from(evidence.timed.is_some()),
    })
}

// Every key column of a daily row as the source's BaseDailySpendTransaction carries it; money keeps a decimal point through SpendColumnValue::Decimal.
#[must_use]
pub fn base_spend_row(base: &BaseDailySpendTransaction) -> SpendRow {
    let mut row = SpendRow::new();
    insert_key_columns(&mut row, base);
    insert_metric_columns(&mut row, base);
    row
}

fn insert_key_columns(row: &mut SpendRow, base: &BaseDailySpendTransaction) {
    insert_text(row, "date", Some(&base.date));
    insert_text(row, "api_key", Some(&base.api_key));
    insert_text(row, "model", base.model.as_deref());
    insert_text(row, "model_group", base.model_group.as_deref());
    insert_text(
        row,
        "mcp_namespaced_tool_name",
        base.mcp_namespaced_tool_name.as_deref(),
    );
    insert_text(
        row,
        "custom_llm_provider",
        base.custom_llm_provider.as_deref(),
    );
    insert_text(row, "endpoint", base.endpoint.as_deref());
}

fn insert_metric_columns(row: &mut SpendRow, base: &BaseDailySpendTransaction) {
    let counters = [
        ("prompt_tokens", base.prompt_tokens),
        ("completion_tokens", base.completion_tokens),
        ("cache_read_input_tokens", base.cache_read_input_tokens),
        (
            "cache_creation_input_tokens",
            base.cache_creation_input_tokens,
        ),
        ("compression_saved_tokens", base.compression_saved_tokens),
        ("api_requests", base.api_requests),
        ("successful_requests", base.successful_requests),
        ("failed_requests", base.failed_requests),
        ("total_response_time_ms", base.total_response_time_ms),
        ("timed_requests", base.timed_requests),
    ];
    for (column, value) in counters {
        row.insert(column.to_owned(), SpendColumnValue::Int(value));
    }
    let money = [
        ("compression_savings_spend", base.compression_savings_spend),
        (
            "prompt_caching_savings_spend",
            base.prompt_caching_savings_spend,
        ),
        (
            "gateway_injected_caching_savings_spend",
            base.gateway_cache_savings_spend,
        ),
        ("autorouter_savings_spend", base.autorouter_savings_spend),
        ("spend", base.spend),
    ];
    for (column, value) in money {
        row.insert(column.to_owned(), SpendColumnValue::Decimal(value));
    }
}

fn insert_text(row: &mut SpendRow, column: &str, value: Option<&str>) {
    let cell = value.map_or(SpendColumnValue::Null, |text| {
        SpendColumnValue::Text(text.to_owned())
    });
    row.insert(column.to_owned(), cell);
}

// One entity's row: the source spreads the base transaction into the entity TypedDict, adding the id column and, for tags, the request id.
pub(crate) fn entity_spend_row(
    base: &BaseDailySpendTransaction,
    entity: DailySpendEntity,
    identity: Option<&str>,
    request_id: Option<&str>,
) -> SpendRow {
    let mut row = base_spend_row(base);
    match entity {
        DailySpendEntity::User => insert_text(&mut row, "user_id", identity),
        DailySpendEntity::Team => insert_text(&mut row, "team_id", identity),
        DailySpendEntity::Org => insert_text(&mut row, "organization_id", identity),
        DailySpendEntity::EndUser => insert_text(&mut row, "end_user_id", identity),
        DailySpendEntity::Agent => insert_text(&mut row, "agent_id", identity),
        DailySpendEntity::Tag => {
            insert_text(&mut row, "tag", identity);
            insert_text(&mut row, "request_id", request_id);
        }
    }
    row
}

// The queue key the writer builds with an f-string: a None identity or model reads as its text, and a missing endpoint as the empty string.
#[must_use]
pub(crate) fn queue_key(identity: Option<&str>, base: &BaseDailySpendTransaction) -> String {
    format!(
        "{}_{}_{}_{}_{}_{}",
        identity.unwrap_or("None"),
        base.date,
        base.api_key,
        base.model.as_deref().unwrap_or("None"),
        base.custom_llm_provider.as_deref().unwrap_or("None"),
        base.endpoint.as_deref().unwrap_or(""),
    )
}
