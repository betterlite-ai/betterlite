use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::Params;

use crate::case::Compare;
use betterlite_oracle_runner::case::UpstreamAnswer;
use calls::{SdkHttpCall, validate_call};
use error::SdkHttpError;
use result::kinds::SdkResult;

pub(crate) mod anthropic_beta_config;
pub mod assistant_create;
pub mod assistant_dispatch;
pub mod assistant_messages;
pub mod assistant_read_delete;
pub mod assistant_run;
pub mod assistant_threads;
pub(crate) mod call_checks;
pub mod call_kind;
pub mod calls;
pub(crate) mod capture;
pub mod chat_response_stream;
mod child;
pub mod compression_interception;
pub mod context_edit_dispatch;
pub mod decisions_send;
pub(crate) mod embedding_executor;
pub mod error;
pub mod file_azure_storage;
pub mod file_content_stream;
pub mod file_create_list_transport;
pub mod file_http_content;
pub mod file_http_metadata;
pub mod file_http_upload;
pub mod file_provider_read;
pub mod file_provider_write;
pub mod file_storage_service;
pub mod file_stream_logging;
pub mod file_upload_routing;
pub mod fine_tuning_cancel;
pub mod fine_tuning_create;
pub mod fine_tuning_read;
pub mod image_edit_call;
pub mod image_edit_http;
pub mod image_generation_call;
pub mod image_generation_http;
pub mod image_generation_plan;
pub mod image_variations;
pub mod messages_chat_call;
pub mod messages_chat_stream;
pub mod messages_chat_transform_stream;
pub mod messages_context_preparation;
pub(crate) mod messages_header_stream;
pub mod messages_native_execute;
pub mod messages_stream_logging;
pub mod messages_sync_stream;
pub(crate) mod native;
mod native_dispatch;
pub(crate) mod python;
pub mod realtime_http_calls;
pub mod realtime_http_transport;
pub mod rerank_call;
pub mod rerank_http;
pub mod rerank_provider_plan;
pub mod response_chat_dispatch;
pub mod responses_agentic_continuation;
pub mod responses_async_create;
pub mod responses_chat_call;
pub mod responses_compact;
pub mod responses_compact_transport;
pub mod responses_create;
pub mod responses_create_transport;
pub mod responses_lifecycle_read;
pub mod responses_lifecycle_write;
pub mod responses_poll_stream;
pub mod responses_public_dispatch;
pub mod responses_read_transport;
pub mod responses_stream_driver;
pub mod responses_write_transport;
pub mod result;
pub mod speech_call;
pub mod speech_call_plan;
pub mod speech_chat_bridge;
pub mod speech_http;
pub mod store_create_read_transport;
pub mod summary_execution;
mod tests;
pub mod transcription_call;
pub mod transcription_http;
pub mod vector_file_create_list;
pub mod vector_file_read;
pub mod vector_file_read_transport;
pub mod vector_file_write;
pub mod vector_file_write_transport;
pub(crate) mod vector_query_embedding;
pub mod vector_retrieval_context;
pub mod vector_search_transport;
pub mod vector_store_read;
pub mod vector_store_search_create;
pub mod vector_store_transport;
pub mod vector_store_write;
pub(crate) mod wrapper_stream;

#[derive(Debug, Clone)]
pub struct OracleLimits {
    pub max_case_bytes: usize,
    pub max_result_bytes: usize,
    pub max_body_bytes: usize,
    pub max_requests: usize,
    pub max_execution: Duration,
}

impl Default for OracleLimits {
    fn default() -> Self {
        Self {
            max_case_bytes: 0x0010_0000,
            max_result_bytes: 0x0010_0000,
            max_body_bytes: 0x0100_0000,
            max_requests: 128,
            max_execution: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkHttpCase {
    kind: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) call: SdkHttpCall,
    pub(crate) upstream: Vec<UpstreamAnswer>,
    #[serde(default)]
    pub(crate) compare: Compare,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenaiChat {
    pub(crate) model: String,
    pub(crate) messages: Vec<ChatMessage>,
    pub(crate) api_key: String,
    pub(crate) timeout_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorStoreEmbeddingCall {
    pub(crate) model: String,
    pub(crate) query: String,
    #[serde(default)]
    pub(crate) configuration: Params,
    #[serde(default)]
    pub(crate) metadata: Params,
    #[serde(default)]
    pub(crate) router: Option<Vec<RouterDeploymentSpec>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VectorQueryEmbeddingCall {
    pub(crate) query_text: String,
    #[serde(default)]
    pub(crate) litellm_params: Params,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnthropicBetaConfigCall {
    pub(crate) url: BetaConfigUrl,
    pub(crate) steps: Vec<BetaConfigStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesHeaderStreamCall {
    pub(crate) body: Json,
    #[serde(default)]
    pub(crate) buffered_provider_output: Option<Json>,
}

// the call names no address: the harness points the endpoint at its own mock, or leaves it empty
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BetaConfigUrl {
    Mock,
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BetaConfigStep {
    Load,
    Reload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterDeploymentSpec {
    pub(crate) model_name: String,
    pub(crate) litellm_params: RouterDeploymentParams,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouterDeploymentParams {
    pub(crate) model: String,
    pub(crate) api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Python,
    Native,
}

// the left side is always the pinned LiteLLM SDK; --self-check compares it with a second fresh process
pub(crate) const fn sides(self_check: bool) -> [Side; 2] {
    if self_check {
        [Side::Python, Side::Python]
    } else {
        [Side::Python, Side::Native]
    }
}

pub fn decode_sdk_http_case(
    bytes: &[u8],
    limits: &OracleLimits,
) -> Result<SdkHttpCase, SdkHttpError> {
    if bytes.len() > limits.max_case_bytes {
        return Err(SdkHttpError::CaseTooLarge {
            bytes: bytes.len(),
            limit: limits.max_case_bytes,
        });
    }
    let text = str::from_utf8(bytes).map_err(SdkHttpError::Utf8)?;
    let case: SdkHttpCase =
        serde_saphyr::from_str(text).map_err(|source| SdkHttpError::CaseParse(Box::new(source)))?;
    validate(&case, limits)?;
    Ok(case)
}

pub fn decode_sdk_http_result(
    bytes: &[u8],
    limits: &OracleLimits,
) -> Result<SdkResult, SdkHttpError> {
    if bytes.len() > limits.max_result_bytes {
        return Err(SdkHttpError::ResultTooLarge {
            bytes: bytes.len(),
            limit: limits.max_result_bytes,
        });
    }
    serde_json::from_slice(bytes).map_err(SdkHttpError::ResultParse)
}

// the harness owns the child's proxy routing, interpreter startup and loader paths, so a case never sets them
fn reserved_environment(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "ALL_PROXY"
            | "HOME"
            | "HTTP_PROXY"
            | "HTTPS_PROXY"
            | "LITELLM_LOCAL_MODEL_COST_MAP"
            | "NO_PROXY"
            | "PATH"
    ) || upper.starts_with("PYTHON")
        || upper.starts_with("UV_")
}

fn validate(case: &SdkHttpCase, limits: &OracleLimits) -> Result<(), SdkHttpError> {
    let invalid = |message: &str| Err(SdkHttpError::Invalid(message.to_owned()));
    if case.kind != "sdk_http" || case.name.is_empty() {
        return invalid("the case needs kind sdk_http and a name");
    }
    if let Some(name) = case.env.keys().find(|name| reserved_environment(name)) {
        return invalid(&format!("the case env may not set {name}"));
    }
    validate_call(&case.call, limits)?;
    if case.upstream.len() > limits.max_requests {
        return invalid("the case scripts more provider answers than the request limit");
    }
    for answer in &case.upstream {
        if answer.body.is_some() == answer.events.is_some() {
            return invalid("an upstream answer needs exactly one of body or events");
        }
        if !(100..=599).contains(&answer.status) {
            return invalid("an upstream answer has an invalid status");
        }
    }
    if !case.compare.headers.is_empty() || !case.compare.headers_present.is_empty() {
        return invalid("compare.headers and compare.headers_present describe gateway responses");
    }
    for ignored in &case.compare.ignore {
        if !ignored.pointer.is_empty() && !ignored.pointer.starts_with('/') {
            return invalid("compare.ignore pointers must be JSON pointers");
        }
        if ignored.why.is_empty() {
            return invalid("compare.ignore entries need a why");
        }
    }
    Ok(())
}
