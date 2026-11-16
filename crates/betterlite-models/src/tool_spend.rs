use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::savings_evidence::billed_at::decode_billed_at;

mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUsageTransaction {
    pub request_id: String,
    pub date: String,
    pub start_time: DateTime<Utc>,
    pub tool_names: Vec<String>,
    pub spend: Decimal,
    pub total_tokens: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpendTime {
    pub date: String,
    pub start_time: DateTime<Utc>,
}

// The day is the original text's date prefix, so the rollup lands in the daily writer's bucket
#[must_use]
pub fn decode_tool_spend_time(text: &str) -> Option<ToolSpendTime> {
    let start_time = decode_billed_at(text)?;
    let date = text.split_once('T').map_or(text, |(prefix, _rest)| prefix);
    Some(ToolSpendTime {
        date: date.to_owned(),
        start_time,
    })
}

pub struct ToolUsageRequest<'request> {
    pub request_id: &'request str,
    pub start_time_iso: &'request str,
    pub mcp_namespaced_tool_name: Option<&'request str>,
    pub spend: Decimal,
    pub total_tokens: i64,
}

// None when the request declared tools but invoked none, and for a start time fromisoformat rejects
#[must_use]
pub fn build_tool_usage_transaction(
    request: &ToolUsageRequest<'_>,
    response_names: &[String],
    realtime_names: &[String],
) -> Option<ToolUsageTransaction> {
    let mut tool_names: Vec<String> = Vec::new();
    let mcp_name = request
        .mcp_namespaced_tool_name
        .map(str::trim)
        .filter(|name| !name.is_empty());
    for name in mcp_name
        .into_iter()
        .chain(response_names.iter().map(String::as_str))
        .chain(realtime_names.iter().map(String::as_str))
    {
        if !tool_names.iter().any(|existing| existing == name) {
            tool_names.push(name.to_owned());
        }
    }
    if tool_names.is_empty() {
        return None;
    }
    let decoded = decode_tool_spend_time(request.start_time_iso)?;
    Some(ToolUsageTransaction {
        request_id: request.request_id.to_owned(),
        date: decoded.date,
        start_time: decoded.start_time,
        tool_names,
        spend: request.spend,
        total_tokens: request.total_tokens,
    })
}
