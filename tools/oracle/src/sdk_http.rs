use std::collections::BTreeMap;
use std::str;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use betterlite_models::decisions::Json;
use betterlite_provider::api::vector_store_contracts::config::Params;

use crate::case::Compare;
use betterlite_oracle_runner::case::UpstreamAnswer;
use chat_response_stream::call::ChatResponseStreamCall;
use error::SdkHttpError;
use file_azure_storage::FileAzureStorageCall;
use messages_chat_stream::call::MessagesChatStreamCall;
use messages_stream_logging::call::MessagesStreamLoggingCall;
use messages_stream_logging::relay::call::MessagesNativeStreamCall;
use messages_sync_stream::call;
use responses_agentic_continuation::AgenticContinuationCall;
use result::SdkResult;

pub(crate) mod anthropic_beta_config;
pub(crate) mod call_checks;
pub(crate) mod capture;
pub mod chat_response_stream;
mod child;
pub(crate) mod embedding_executor;
pub mod error;
pub mod file_azure_storage;
pub mod messages_chat_stream;
pub(crate) mod messages_header_stream;
pub(crate) mod messages_stream_factory;
pub mod messages_stream_logging;
pub mod messages_sync_stream;
pub(crate) mod native;
pub(crate) mod python;
pub mod responses_agentic_continuation;
pub mod result;
mod tests;
mod validate;
pub(crate) mod vector_query_embedding;
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
#[serde(rename_all = "snake_case")]
pub enum SdkHttpCall {
    OpenaiChat(OpenaiChat),
    VectorStoreEmbeddingExecutor(VectorStoreEmbeddingCall),
    VectorQueryEmbedding(VectorQueryEmbeddingCall),
    AnthropicBetaConfig(AnthropicBetaConfigCall),
    MessagesHeaderStream(MessagesHeaderStreamCall),
    MessagesStreamFactory(MessagesStreamFactoryCall),
    MessagesStreamLogging(MessagesStreamLoggingCall),
    MessagesNativeStream(MessagesNativeStreamCall),
    MessagesSyncStream(Box<call::MessagesSyncStreamCall>),
    MessagesChatStream(Box<MessagesChatStreamCall>),
    ChatResponseStream(Box<ChatResponseStreamCall>),
    ResponsesAgenticContinuation(Box<AgenticContinuationCall>),
    FileAzureStorage(FileAzureStorageCall),
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

// The factory takes no model, so this one is only carried to the Python call that ignores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesStreamFactoryCall {
    pub(crate) model: String,
    pub(crate) body: Json,
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
    validate::validate(&case, limits)?;
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
