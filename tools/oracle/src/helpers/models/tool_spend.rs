use std::collections::BTreeMap;

use betterlite_db::spend_batches::{
    StatementLimits, spend_log_queue_within_budget, spend_log_row_bytes, spend_log_write_batches,
};
use betterlite_db::tool_spend::index_rows::ToolIndexRow;
use betterlite_models::tool_spend::{
    ToolSpendTime, ToolUsageRequest, build_tool_usage_transaction, decode_tool_spend_time,
};
use betterlite_prompts::response_tool_calls::projections::{ChatToolCall, ToolCallResponse};
use betterlite_prompts::tool_spend::{realtime_tool_call_names, response_tool_call_names};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::helpers::OrHelperError as _;

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    transaction: Option<TransactionInput>,
    rows: Vec<RowInput>,
    max_bytes: Option<usize>,
    max_rows: Option<usize>,
    queued_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionInput {
    request_id: String,
    start_time_iso: String,
    mcp_namespaced_tool_name: Option<String>,
    spend: Decimal,
    total_tokens: i64,
    completion_response: Option<ToolCallResponse>,
    realtime_tool_calls: Option<Vec<ChatToolCall>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowInput {
    request_id: String,
    tool_name: String,
    start_time: String,
    refused: bool,
}

#[derive(Debug, Serialize)]
struct Output {
    transaction: Option<TransactionOutput>,
    sizes: Vec<usize>,
    batches: Option<Vec<Vec<usize>>>,
    trimmed: Option<TrimmedOutput>,
}

#[derive(Debug, Serialize)]
struct TransactionOutput {
    request_id: String,
    date: String,
    start_time: TimeOutput,
    tool_names: Vec<String>,
    spend: String,
    total_tokens: i64,
}

// Comparing the instant keeps the transaction output free of str(datetime)'s offset spelling
#[derive(Debug, Serialize)]
struct TimeOutput {
    epoch_seconds: i64,
    micros: u32,
}

#[derive(Debug, Serialize)]
struct TrimmedOutput {
    start: usize,
    queued_bytes: usize,
}

// A map keyed by a number is the row a JSON serializer refuses, the same way json.dumps refuses a tuple key
#[derive(Debug, Serialize)]
struct Refused {
    #[serde(flatten)]
    keys: BTreeMap<i64, i64>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum MeasuredRow {
    Row(ToolIndexRow),
    Refused(Refused),
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let rows = measured_rows(&input.rows)?;
        let output = Output {
            transaction: transaction_output(input.transaction.as_ref())?,
            sizes: rows.iter().map(spend_log_row_bytes).collect(),
            batches: input
                .max_bytes
                .zip(input.max_rows)
                .map(|(max_bytes, max_rows)| {
                    batch_indices(
                        &rows,
                        StatementLimits {
                            max_bytes,
                            max_rows,
                        },
                    )
                }),
            trimmed: trimmed_output(&rows, input.queued_bytes, input.max_bytes),
        };
        Ok(serde_json::to_value(output)?)
    }
}

fn transaction_output(
    input: Option<&TransactionInput>,
) -> Result<Option<TransactionOutput>, Box<crate::helpers::HelperError>> {
    let Some(request) = input else {
        return Ok(None);
    };
    let response_names = request
        .completion_response
        .as_ref()
        .map_or_else(Vec::new, response_tool_call_names);
    let realtime_names = request
        .realtime_tool_calls
        .as_deref()
        .map_or_else(Vec::new, realtime_tool_call_names);
    let transaction = build_tool_usage_transaction(
        &ToolUsageRequest {
            request_id: &request.request_id,
            start_time_iso: &request.start_time_iso,
            mcp_namespaced_tool_name: request.mcp_namespaced_tool_name.as_deref(),
            spend: request.spend,
            total_tokens: request.total_tokens,
        },
        &response_names,
        &realtime_names,
    );
    Ok(transaction.map(|built| TransactionOutput {
        request_id: built.request_id,
        date: built.date,
        start_time: TimeOutput {
            epoch_seconds: built.start_time.timestamp(),
            micros: built.start_time.timestamp_subsec_micros(),
        },
        tool_names: built.tool_names,
        spend: built.spend.normalize().to_string(),
        total_tokens: built.total_tokens,
    }))
}

fn measured_rows(rows: &[RowInput]) -> Result<Vec<MeasuredRow>, Box<crate::helpers::HelperError>> {
    rows.iter().map(measured_row).collect()
}

fn measured_row(row: &RowInput) -> Result<MeasuredRow, Box<crate::helpers::HelperError>> {
    if row.refused {
        return Ok(MeasuredRow::Refused(Refused {
            keys: BTreeMap::from([(1, 0)]),
        }));
    }
    let ToolSpendTime { start_time, .. } = decode_tool_spend_time(&row.start_time)
        .ok_or("the row start time is not an ISO datetime")
        .or_value_error()?;
    Ok(MeasuredRow::Row(ToolIndexRow {
        request_id: row.request_id.clone(),
        tool_name: row.tool_name.clone(),
        start_time,
    }))
}

fn batch_indices(rows: &[MeasuredRow], limits: StatementLimits) -> Vec<Vec<usize>> {
    let mut batches = Vec::new();
    let mut start = 0;
    for batch in spend_log_write_batches(rows, limits) {
        let end = start.saturating_add(batch.len());
        batches.push((start..end).collect());
        start = end;
    }
    batches
}

fn trimmed_output(
    rows: &[MeasuredRow],
    queued_bytes: Option<usize>,
    max_bytes: Option<usize>,
) -> Option<TrimmedOutput> {
    let trimmed = spend_log_queue_within_budget(rows, queued_bytes?, max_bytes?);
    Some(TrimmedOutput {
        start: rows.len().saturating_sub(trimmed.rows.len()),
        queued_bytes: trimmed.queued_bytes,
    })
}
