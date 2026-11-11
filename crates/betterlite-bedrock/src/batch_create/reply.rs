use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::{
    BatchFailures, BatchObjectKind, BatchStatus, BatchUsage, RequestCounts,
};
use indexmap::IndexMap;
use serde::de::Error as DeserializeError;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::BatchCreateError;

const GUARDRAIL_METADATA_KEY: &str = "standard_logging_guardrail_information";
const DEFAULT_ENDPOINT: &str = "/v1/chat/completions";
const DEFAULT_COMPLETION_WINDOW: &str = "24h";
const UNSERIALIZABLE: &str = "Unserializable Object";

// the API caller's own create request, carried on litellm_params.original_batch_request
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OriginalBatchRequest {
    #[serde(default = "default_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub input_file_id: String,
    #[serde(default = "default_completion_window")]
    pub completion_window: String,
    #[serde(default = "default_metadata")]
    pub metadata: Json,
}

fn default_endpoint() -> String {
    DEFAULT_ENDPOINT.to_owned()
}

fn default_completion_window() -> String {
    DEFAULT_COMPLETION_WINDOW.to_owned()
}

fn default_metadata() -> Json {
    Json::Object(IndexMap::new())
}

impl Default for OriginalBatchRequest {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            input_file_id: String::new(),
            completion_window: default_completion_window(),
            metadata: default_metadata(),
        }
    }
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
    job_arn: Option<String>,
    #[serde(default)]
    status: Option<Json>,
}

pub fn decode_created_batch(body: &[u8]) -> Result<CreatedBatch, BatchCreateError> {
    let document: Box<RawValue> = serde_json::from_slice(body)
        .map_err(|source| BatchCreateError::ResponseParse { source })?;
    // a derived struct with only optional fields would also accept a top-level JSON array
    if !document.get().starts_with('{') {
        return Err(BatchCreateError::ResponseParse {
            source: DeserializeError::custom("the response body must be a JSON object"),
        });
    }
    let raw: RawCreatedBatch = serde_json::from_str(document.get())
        .map_err(|source| BatchCreateError::ResponseParse { source })?;
    let (status, in_progress) = match raw.status {
        Some(Json::Text(text)) if text == "InProgress" => (BatchStatus::InProgress, true),
        Some(Json::Text(text)) => (bedrock_batch_status(&text), false),
        _ => (BatchStatus::Validating, false),
    };
    Ok(CreatedBatch {
        job_arn: raw.job_arn.unwrap_or_default(),
        status,
        in_progress,
    })
}

pub fn create_batch_reply(
    body: &[u8],
    original: OriginalBatchRequest,
    clock: &dyn Fn() -> i64,
) -> Result<BatchCreateReply, BatchCreateError> {
    let created = decode_created_batch(body)?;
    let created_at = clock();
    let in_progress_at = created.in_progress.then(clock);
    let metadata = openai_batch_metadata(&original.metadata);
    Ok(BatchCreateReply {
        id: created.job_arn,
        object: BatchObjectKind::Batch,
        endpoint: original.endpoint,
        errors: None,
        input_file_id: original.input_file_id,
        completion_window: original.completion_window,
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
