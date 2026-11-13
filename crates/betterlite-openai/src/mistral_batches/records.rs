use http::Method;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use crate::api::batch_records::BatchEndpoint;
use crate::openai_base_config::Headers;
fn default_count() -> i64 {
    1
}
// The native spellings of LiteLLM's MistralBatchStatus literal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MistralBatchStatus {
    Queued,
    Running,
    Success,
    Failed,
    TimeoutExceeded,
    CancellationRequested,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MistralBatchError {
    pub message: String,
    #[serde(default = "default_count")]
    pub count: i64,
}
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MistralBatchJob {
    pub id: String,
    #[serde(default)]
    pub input_files: Vec<String>,
    pub endpoint: String,
    #[serde(default)]
    pub model: Option<String>,
    pub status: MistralBatchStatus,
    pub created_at: i64,
    #[serde(default)]
    pub started_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub total_requests: i64,
    #[serde(default)]
    pub completed_requests: i64,
    #[serde(default)]
    pub succeeded_requests: i64,
    #[serde(default)]
    pub failed_requests: i64,
    #[serde(default)]
    pub output_file: Option<String>,
    #[serde(default)]
    pub error_file: Option<String>,
    #[serde(default)]
    pub errors: Vec<MistralBatchError>,
    #[serde(default)]
    pub metadata: Option<IndexMap<String, String>>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MistralCreateBatchJobRequest {
    pub input_files: Vec<String>,
    pub endpoint: BatchEndpoint,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<IndexMap<String, String>>,
}
// The GET request retrieval sends as-is, credentials included
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MistralGetRequest {
    pub method: Method,
    pub url: String,
    pub headers: Headers,
}
