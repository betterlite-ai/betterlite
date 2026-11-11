use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{
    BatchFailures, BatchObjectKind, BatchStatus, BatchUsage, RequestCounts,
};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::BatchCreateError;

const GUARDRAIL_METADATA_KEY: &str = "standard_logging_guardrail_information";
const DEFAULT_ENDPOINT: &str = "/v1/chat/completions";
const DEFAULT_COMPLETION_WINDOW: &str = "24h";
const UNSERIALIZABLE: &str = "Unserializable Object";

// the API caller's own create request, carried on litellm_params.original_batch_request
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct OriginalBatchRequest {
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub input_file_id: Option<String>,
    #[serde(default)]
    pub completion_window: Option<String>,
    #[serde(default)]
    pub metadata: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedBatch {
    pub job_arn: String,
    pub status: BatchStatus,
    pub in_progress: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BatchCreateReply {
    pub id: String,
    pub object: BatchObjectKind,
    pub endpoint: String,
    pub errors: Option<BatchFailures>,
    pub input_file_id: String,
    pub completion_window: String,
    pub status: BatchStatus,
    pub output_file_id: Option<String>,
    pub error_file_id: Option<String>,
    pub created_at: i64,
    pub in_progress_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub finalizing_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub expired_at: Option<i64>,
    pub cancelling_at: Option<i64>,
    pub cancelled_at: Option<i64>,
    pub request_counts: Option<RequestCounts>,
    pub metadata: IndexMap<String, String>,
    pub model: Option<String>,
    pub usage: Option<BatchUsage>,
}

#[derive(Debug, Deserialize)]
struct RawCreatedBatch {
    #[serde(rename = "jobArn", default)]
    job_arn: Option<Json>,
    #[serde(default)]
    status: Option<Json>,
}

pub fn decode_created_batch(body: &[u8]) -> Result<CreatedBatch, BatchCreateError> {
    let raw: RawCreatedBatch = serde_json::from_slice(body)
        .map_err(|source| BatchCreateError::ResponseParse { source })?;
    let job_arn = match raw.job_arn {
        None | Some(Json::Null) => String::new(),
        Some(Json::Text(text)) => text,
        Some(_) => return Err(BatchCreateError::JobArnType),
    };
    let (status, in_progress) = match raw.status {
        Some(Json::Text(text)) if text == "InProgress" => (BatchStatus::InProgress, true),
        Some(Json::Text(text)) => (bedrock_batch_status(&text), false),
        _ => (BatchStatus::Validating, false),
    };
    Ok(CreatedBatch {
        job_arn,
        status,
        in_progress,
    })
}

pub fn create_batch_reply(
    body: &[u8],
    original: &OriginalBatchRequest,
    clock: &dyn Fn() -> i64,
) -> Result<BatchCreateReply, BatchCreateError> {
    let created = decode_created_batch(body)?;
    let created_at = clock();
    let in_progress_at = created.in_progress.then(clock);
    let metadata = original
        .metadata
        .as_ref()
        .map_or_else(IndexMap::new, openai_batch_metadata);
    Ok(BatchCreateReply {
        id: created.job_arn,
        object: BatchObjectKind::Batch,
        endpoint: original
            .endpoint
            .clone()
            .unwrap_or_else(|| DEFAULT_ENDPOINT.to_owned()),
        errors: None,
        input_file_id: original.input_file_id.clone().unwrap_or_default(),
        completion_window: original
            .completion_window
            .clone()
            .unwrap_or_else(|| DEFAULT_COMPLETION_WINDOW.to_owned()),
        status: created.status,
        output_file_id: None,
        error_file_id: None,
        created_at,
        in_progress_at,
        expires_at: None,
        finalizing_at: None,
        completed_at: None,
        failed_at: None,
        expired_at: None,
        cancelling_at: None,
        cancelled_at: None,
        request_counts: None,
        metadata,
        model: None,
        usage: None,
    })
}

#[must_use]
pub fn openai_batch_metadata(metadata: &Json) -> IndexMap<String, String> {
    let Json::Object(fields) = metadata else {
        return IndexMap::new();
    };
    let mut sanitized = IndexMap::new();
    for (key, value) in fields {
        if key == GUARDRAIL_METADATA_KEY || matches!(value, Json::Null) {
            continue;
        }
        let text = match value {
            Json::Text(text) => text.clone(),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                safe_dumps(value, DEFAULT_MAX_RECURSE_DEPTH, None)
                    .unwrap_or_else(|_error| UNSERIALIZABLE.to_owned())
            }
        };
        sanitized.insert(key.clone(), text);
    }
    sanitized
}

fn bedrock_batch_status(status: &str) -> BatchStatus {
    // Submitted, Validating and every status AWS adds later stay validating
    match status {
        "Scheduled" | "InProgress" => BatchStatus::InProgress,
        "PartiallyCompleted" | "Completed" => BatchStatus::Completed,
        "Failed" => BatchStatus::Failed,
        "Stopping" => BatchStatus::Cancelling,
        "Stopped" => BatchStatus::Cancelled,
        "Expired" => BatchStatus::Expired,
        _ => BatchStatus::Validating,
    }
}
