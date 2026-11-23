use std::collections::{BTreeMap, BTreeSet};

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::session::{
    SessionAggregate, SessionAggregateError, SessionAggregateReader, SessionLookup,
};

// _SPEND_LOG_LIST_COLUMNS plus the enrichment _build_ui_spend_logs_response writes; no Debug, api_key holds a key
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendLogListRow {
    pub request_id: String,
    pub call_type: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(
        default,
        serialize_with = "rust_decimal::serde::float::serialize",
        deserialize_with = "rust_decimal::serde::arbitrary_precision::deserialize"
    )]
    pub spend: Decimal,
    #[serde(default)]
    pub total_tokens: i64,
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(rename = "startTime")]
    pub start_time: Timestamp,
    #[serde(rename = "endTime")]
    pub end_time: Timestamp,
    #[serde(rename = "completionStartTime", default)]
    pub completion_start_time: Option<Timestamp>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub model_group: Option<String>,
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub api_base: Option<String>,
    #[serde(rename = "user", default)]
    pub user: Option<String>,
    #[serde(default)]
    pub metadata: Option<JsonData>,
    #[serde(default)]
    pub cache_hit: Option<String>,
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub request_tags: Option<JsonData>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub end_user: Option<String>,
    #[serde(default)]
    pub requester_ip_address: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub mcp_namespaced_tool_name: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub litellm_call_id: Option<String>,
    #[serde(default)]
    pub request_duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_total_count: Option<i64>,
    #[serde(
        default,
        with = "rust_decimal::serde::float_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_total_spend: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_total_duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_tool_call_count: Option<i64>,
    #[serde(
        default,
        with = "rust_decimal::serde::float_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub mcp_tool_call_spend: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_cache_hit_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_llm_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_agent_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_total_prompt_tokens: Option<i64>,
    #[serde(
        rename = "session_total_completion_tokens",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub session_completion_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_total_tokens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_models: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_models_truncated: Option<bool>,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct SpendLogsPage {
    pub data: Vec<SpendLogListRow>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
    pub total_pages: i64,
    pub total_is_capped: bool,
}

// _build_ui_spend_logs_response: one pair-keyed aggregate query serves the page; a driver failure keeps plain rows
pub async fn build_ui_spend_logs_response<Reader>(
    reader: Option<&Reader>,
    mut page: SpendLogsPage,
    enrich_session_counts: bool,
) -> Result<SpendLogsPage, SessionAggregateError>
where
    Reader: SessionAggregateReader + ?Sized,
{
    let mut aggregates = BTreeMap::new();
    if enrich_session_counts {
        let lookup = session_lookup(&page.data);
        if !lookup.session_ids.is_empty()
            && let Some(enricher) = reader
        {
            match enricher.session_aggregates(&lookup).await {
                Ok(found) => aggregates = group_aggregates(found),
                Err(SessionAggregateError::Database(source)) => {
                    tracing::debug!(
                        error = %source,
                        "Failed to enrich session spend aggregates for spend logs UI"
                    );
                }
                Err(error @ SessionAggregateError::Row(_)) => return Err(error),
            }
        }
        apply_aggregates(&mut page.data, &aggregates);
    }
    Ok(page)
}

// the session ids and api keys the page shows, deduplicated: a row without a session gains nothing
fn session_lookup(rows: &[SpendLogListRow]) -> SessionLookup {
    let session_ids: BTreeSet<&str> = rows
        .iter()
        .filter_map(|row| row.session_id.as_deref())
        .filter(|session_id| !session_id.is_empty())
        .collect();
    let api_keys: BTreeSet<&str> = rows
        .iter()
        .filter_map(|row| row.api_key.as_deref())
        .collect();
    SessionLookup {
        session_ids: session_ids.into_iter().map(str::to_owned).collect(),
        api_keys: api_keys.into_iter().map(str::to_owned).collect(),
    }
}

// the aggregate rows keyed by (session_id, api_key), so a shared session id never merges two keys
fn group_aggregates(
    found: Vec<SessionAggregate>,
) -> BTreeMap<String, BTreeMap<String, SessionAggregate>> {
    let mut by_session: BTreeMap<String, BTreeMap<String, SessionAggregate>> = BTreeMap::new();
    for aggregate in found {
        if aggregate.session_id.is_empty() {
            continue;
        }
        let session_id = aggregate.session_id.clone();
        let api_key = aggregate.api_key.clone();
        by_session
            .entry(session_id)
            .or_default()
            .insert(api_key, aggregate);
    }
    by_session
}

// a row without stats counts one call and gains no other field; a zero MCP count adds neither field
fn apply_aggregates(
    rows: &mut [SpendLogListRow],
    aggregates: &BTreeMap<String, BTreeMap<String, SessionAggregate>>,
) {
    for row in rows {
        let found = row_aggregate(row, aggregates);
        row.session_total_count =
            Some(found.map_or(1, |stats| i64::from(stats.session_total_count)));
        let Some(stats) = found else {
            continue;
        };
        row.session_total_spend = Some(stats.session_total_spend);
        row.session_total_duration_ms = Some(stats.session_total_duration_ms);
        if stats.mcp_tool_call_count != 0_i32 {
            row.mcp_tool_call_count = Some(i64::from(stats.mcp_tool_call_count));
            row.mcp_tool_call_spend = Some(stats.mcp_tool_call_spend);
        }
        row.session_cache_hit_count = Some(i64::from(stats.session_cache_hit_count));
        row.session_llm_count = Some(i64::from(stats.session_llm_count));
        row.session_agent_count = Some(i64::from(stats.session_agent_count));
        row.session_total_prompt_tokens = Some(stats.session_total_prompt_tokens);
        row.session_completion_tokens = Some(stats.session_completion_tokens);
        row.session_total_tokens = Some(stats.session_total_tokens);
        row.session_models = Some(stats.session_models.clone());
        row.session_models_truncated = Some(stats.session_models_truncated);
    }
}

// the aggregate for one page row, which needs both the session id and the key the row shows
fn row_aggregate<'page>(
    row: &SpendLogListRow,
    aggregates: &'page BTreeMap<String, BTreeMap<String, SessionAggregate>>,
) -> Option<&'page SessionAggregate> {
    let session_id = row
        .session_id
        .as_deref()
        .filter(|session_id| !session_id.is_empty())?;
    let api_key = row.api_key.as_deref()?;
    aggregates.get(session_id)?.get(api_key)
}
