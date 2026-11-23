use betterlite_models::tool_spend::{
    ToolUsageRequest, ToolUsageTransaction, build_tool_usage_transaction,
};

use crate::model_usage::rollup::build_model_usage_transaction;
use crate::model_usage::{ModelUsagePayload, ModelUsageTransaction};

use super::payload::ToolUsagePayload;
use super::queues::UsageQueue;

mod tests;

#[derive(Debug, thiserror::Error)]
pub enum ToolUsageError {
    #[error("the spend log payload could not be read")]
    Payload(#[source] serde_json::Error),
    #[error("the spend log payload holds no {field}")]
    Field { field: &'static str },
}

// _enqueue_tool_usage_transaction: a tool spend row for the tool index and its daily rollup; a build or enqueue failure is debug-only.
pub async fn enqueue_tool_usage(
    queue: &UsageQueue<ToolUsageTransaction>,
    payload: &[u8],
    response_names: &[String],
    realtime_names: &[String],
) {
    let transaction = match tool_usage_transaction(payload, response_names, realtime_names) {
        Ok(Some(transaction)) => transaction,
        Ok(None) => return,
        Err(error) => {
            tracing::debug!("_enqueue_tool_usage_transaction error (non-blocking): {error}");
            return;
        }
    };
    if let Err(error) = queue.push(transaction).await {
        tracing::debug!("_enqueue_tool_usage_transaction error (non-blocking): {error}");
    }
}

// payload["request_id"], payload["startTime"], payload["spend"] and payload["total_tokens"] are the source's indexed reads.
fn tool_usage_transaction(
    payload: &[u8],
    response_names: &[String],
    realtime_names: &[String],
) -> Result<Option<ToolUsageTransaction>, ToolUsageError> {
    let payload: ToolUsagePayload =
        serde_json::from_slice(payload).map_err(ToolUsageError::Payload)?;
    let request_id = payload.request_id.as_deref().ok_or(ToolUsageError::Field {
        field: "request_id",
    })?;
    let spend = payload
        .spend
        .ok_or(ToolUsageError::Field { field: "spend" })?;
    let total_tokens = payload.total_tokens.ok_or(ToolUsageError::Field {
        field: "total_tokens",
    })?;
    let start_time = payload
        .start_time
        .as_deref()
        .ok_or(ToolUsageError::Field { field: "startTime" })?;
    Ok(build_tool_usage_transaction(
        &ToolUsageRequest {
            request_id,
            start_time_iso: start_time,
            mcp_namespaced_tool_name: payload.mcp_namespaced_tool_name.as_deref(),
            spend,
            total_tokens,
        },
        response_names,
        realtime_names,
    ))
}

// _enqueue_model_usage_transaction: the daily model usage rollup row; a build or enqueue failure is debug-only.
pub async fn enqueue_model_usage(queue: &UsageQueue<ModelUsageTransaction>, payload: &[u8]) {
    let payload: ModelUsagePayload = match serde_json::from_slice(payload) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::debug!("_enqueue_model_usage_transaction error (non-blocking): {error}");
            return;
        }
    };
    let Some(transaction) = build_model_usage_transaction(&payload) else {
        return;
    };
    if let Err(error) = queue.push(transaction).await {
        tracing::debug!("_enqueue_model_usage_transaction error (non-blocking): {error}");
    }
}
