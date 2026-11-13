use betterlite_openai::batches::records::{
    BatchObject, BatchObjectKind, BatchStatus, RequestCounts,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Map;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary};

use super::BatchProviderCancelResult;

pub const ASYNC_INVOKE_COMPLETION_WINDOW: &str = "24h";
pub const ASYNC_INVOKE_ENDPOINT: &str = "/v1/embeddings";

// the fields of AWS GetAsyncInvoke's answer that _handle_async_invoke_status reads
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AsyncInvokeStatus {
    pub invocation_arn: String,
    pub model_arn: String,
    pub status: String,
    pub failure_message: Option<String>,
    pub output_data_config: Option<AsyncInvokeOutput>,
    pub submit_time: Option<String>,
    pub last_modified_time: Option<String>,
    pub end_time: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AsyncInvokeOutput {
    pub s3_output_data_config: Option<AsyncInvokeS3Output>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AsyncInvokeS3Output {
    pub s3_uri: Option<String>,
}

#[must_use]
pub fn normalize(
    status: &AsyncInvokeStatus,
    now: i64,
    limits: &ApiLimits,
) -> Result<BatchProviderCancelResult, ApiLimitError> {
    let s3_uri = status
        .output_data_config
        .as_ref()
        .and_then(|output| output.s3_output_data_config.as_ref())
        .and_then(|s3| s3.s3_uri.as_deref())
        .unwrap_or_default();
    let failure_message = status.failure_message.clone().unwrap_or_default();
    let observed = s3_uri.len().saturating_add(failure_message.len());
    limits.check(ApiBoundariesInput::Event(EventBoundary {
        known_bytes: observed,
    }))?;
    let raw_status = status.status.as_str();
    let normalized = normalized_status(raw_status);
    let in_progress_at = match raw_status {
        "InProgress" | "Validating" | "Scheduled" => {
            timestamp(status.last_modified_time.as_deref())
        }
        _ => None,
    };
    let completed_at = match raw_status {
        "Completed" | "PartiallyCompleted" => timestamp(status.end_time.as_deref()),
        _ => None,
    };
    let failed_at = match raw_status {
        "Failed" => timestamp(status.end_time.as_deref()),
        _ => None,
    };
    let metadata = IndexMap::from([
        ("output_file_id".to_owned(), s3_uri.to_owned()),
        ("failure_message".to_owned(), failure_message),
        ("model_arn".to_owned(), status.model_arn.clone()),
    ]);
    let batch = BatchObject {
        id: status.invocation_arn.clone(),
        completion_window: ASYNC_INVOKE_COMPLETION_WINDOW.to_owned(),
        // a response without a readable submitTime is stamped with the injected clock
        created_at: timestamp(status.submit_time.as_deref()).unwrap_or(now),
        endpoint: ASYNC_INVOKE_ENDPOINT.to_owned(),
        input_file_id: String::new(),
        object: BatchObjectKind::Batch,
        status: normalized,
        cancelled_at: None,
        cancelling_at: None,
        completed_at,
        error_file_id: None,
        errors: None,
        expired_at: None,
        expires_at: None,
        failed_at,
        finalizing_at: None,
        in_progress_at,
        metadata: Some(metadata),
        model: None,
        output_file_id: None,
        request_counts: Some(RequestCounts {
            total: 1,
            completed: i64::from(normalized == BatchStatus::Completed),
            failed: i64::from(normalized == BatchStatus::Failed),
        }),
        usage: None,
        provider_fields: Map::new(),
    };
    Ok(BatchProviderCancelResult { batch, model: None })
}

// AWS status values lowercased: unknown values report failed, as the source maps them
fn normalized_status(raw: &str) -> BatchStatus {
    match raw.to_lowercase().as_str() {
        "completed" => BatchStatus::Completed,
        "failed" => BatchStatus::Failed,
        "inprogress" | "in_progress" => BatchStatus::InProgress,
        _ => BatchStatus::Failed,
    }
}

fn timestamp(value: Option<&str>) -> Option<i64> {
    let text = value?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|parsed| parsed.timestamp())
}
