use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::api::batch_records::{CreateBatchRequest, RetrieveBatchRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum CompletionWindow {
    #[serde(rename = "24h")]
    Day,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub enum BatchEndpoint {
    #[serde(rename = "/v1/chat/completions")]
    ChatCompletions,
    #[serde(rename = "/v1/embeddings")]
    Embeddings,
    #[serde(rename = "/v1/completions")]
    Completions,
    #[serde(rename = "/v1/responses")]
    Responses,
    #[serde(rename = "/v1/ocr")]
    Ocr,
    #[serde(rename = "/v1/images/generations")]
    ImageGenerations,
    #[serde(rename = "/v1/images/edits")]
    ImageEdits,
    #[serde(rename = "/v1/videos/generations")]
    VideoGenerations,
    #[serde(rename = "/v1/videos")]
    Videos,
    #[serde(rename = "/v1/videos/edits")]
    VideoEdits,
    #[serde(rename = "/v1/videos/extensions")]
    VideoExtensions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpiryAnchor {
    CreatedAt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct OutputExpiry {
    pub anchor: ExpiryAnchor,
    pub seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct BatchCreate {
    pub completion_window: CompletionWindow,
    pub endpoint: BatchEndpoint,
    pub input_file_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_expires_after: Option<OutputExpiry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

// LiteLLM's RetrieveBatchRequest and CancelBatchRequest are the same four keys
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct BatchQuery {
    pub batch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_headers: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_body: Option<IndexMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MissingBatchField {
    #[error("the batch request is missing its {field}")]
    Missing { field: &'static str },
}

impl BatchCreate {
    pub fn from_request(body: &CreateBatchRequest) -> Result<Self, MissingBatchField> {
        let Some(completion_window) = body.completion_window else {
            return Err(MissingBatchField::Missing {
                field: "completion_window",
            });
        };
        let Some(endpoint) = body.endpoint else {
            return Err(MissingBatchField::Missing { field: "endpoint" });
        };
        let Some(input_file_id) = body.input_file_id.clone() else {
            return Err(MissingBatchField::Missing {
                field: "input_file_id",
            });
        };
        Ok(Self {
            completion_window,
            endpoint,
            input_file_id,
            metadata: body.metadata.clone(),
            output_expires_after: body.output_expires_after,
            extra_headers: body.extra_headers.clone(),
            extra_body: body.extra_body.clone(),
            timeout: body.timeout,
        })
    }
}

impl BatchQuery {
    pub fn from_request(request: &RetrieveBatchRequest) -> Result<Self, MissingBatchField> {
        let batch_id = request.batch_id.clone();
        batch_id
            .map(|given| Self {
                batch_id: given,
                extra_headers: request.extra_headers.clone(),
                extra_body: request.extra_body.clone(),
                timeout: request.timeout,
            })
            .ok_or(MissingBatchField::Missing { field: "batch_id" })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchStatus {
    Validating,
    Failed,
    InProgress,
    Finalizing,
    Completed,
    Expired,
    Cancelling,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchObjectKind {
    Batch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct RequestCounts {
    pub completed: i64,
    pub failed: i64,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BatchFailure {
    pub code: Option<String>,
    pub line: Option<i64>,
    pub message: Option<String>,
    pub param: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BatchFailures {
    pub data: Option<Vec<BatchFailure>>,
    pub object: Option<String>,
}

// The SDK's token usage rebuilt through LiteLLM's Usage: the chat-style counts stay zero and the SDK's own keys ride along
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BatchUsage {
    #[serde(default)]
    pub prompt_tokens: i64,
    #[serde(default)]
    pub completion_tokens: i64,
    #[serde(default)]
    pub total_tokens: i64,
    pub completion_tokens_details: Option<Value>,
    pub prompt_tokens_details: Option<Value>,
    #[serde(flatten)]
    pub sdk_fields: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BatchObject {
    pub id: String,
    pub completion_window: String,
    pub created_at: i64,
    pub endpoint: String,
    pub input_file_id: String,
    pub object: BatchObjectKind,
    pub status: BatchStatus,
    pub cancelled_at: Option<i64>,
    pub cancelling_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub error_file_id: Option<String>,
    pub errors: Option<BatchFailures>,
    pub expired_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub failed_at: Option<i64>,
    pub finalizing_at: Option<i64>,
    pub in_progress_at: Option<i64>,
    pub metadata: Option<IndexMap<String, String>>,
    pub model: Option<String>,
    pub output_file_id: Option<String>,
    pub request_counts: Option<RequestCounts>,
    pub usage: Option<BatchUsage>,
    #[serde(flatten)]
    pub provider_fields: Map<String, Value>,
}

// LiteLLM returns the SDK's cursor page unchanged, so its items stay the SDK's own JSON and are never rebuilt through LiteLLMBatch
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct BatchPage {
    pub data: Vec<Value>,
    pub has_more: Option<bool>,
    #[serde(flatten)]
    pub provider_fields: Map<String, Value>,
}
