use std::error::Error;

use async_trait::async_trait;
use betterlite_context::chat_config_selection::facts::{BedrockFacts, ModelFamilyProbe};
use betterlite_context::chat_config_selection::json_provider::JsonProviderProjection;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_context::chat_config_selection::vertex::VertexModelLists;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use betterlite_openai::chat_choices::wire::{ChoiceLogprobs, DeltaAudio, DeltaFunction};
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, Usage};
use betterlite_streaming::stream_tool_assembly::records::CustomOutput;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::skip_serializing_none;

use super::error::{BridgeStreamError, PostStreamError};
use crate::api::chat_response_stream::source::AsyncChunkStream;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::response_chat_result::records::ModelResponse;

// The flattened request_data LiteLLM hands to responses()/aresponses()
pub struct ResponsesCall<'call, Client: ?Sized> {
    pub params: &'call ResponsesApiRequestParams,
    pub litellm_params: &'call IndexMap<String, Value>,
    pub extra_headers: Option<&'call IndexMap<String, String>>,
    pub client: Option<&'call Client>,
    pub logging: &'call mut LoggingState,
}

// LiteLLM's ModelResponse as responses() returns it: every choice, message and usage field its types declare
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: Option<String>,
    pub system_fingerprint: Option<String>,
    pub choices: Vec<ChatChoice>,
    pub usage: Option<Usage>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatChoice {
    pub index: i64,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
    pub logprobs: Option<ChoiceLogprobs>,
    pub provider_specific_fields: Option<JsonObject>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

// Message: the payloads a replayed delta carries stay typed, and every other provider key round-trips through extra
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatMessage {
    pub content: Option<String>,
    pub role: Option<String>,
    pub function_call: Option<DeltaFunction>,
    pub tool_calls: Option<Vec<ChatToolCall>>,
    pub audio: Option<DeltaAudio>,
    pub reasoning_content: Option<String>,
    pub thinking_blocks: Option<Vec<ChatThinkingBlock>>,
    pub images: Option<Vec<ChatImage>>,
    pub reasoning_items: Option<Vec<ChatReasoningItem>>,
    pub annotations: Option<Vec<ChatAnnotation>>,
    pub provider_specific_fields: Option<JsonObject>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

// ChatCompletionThinkingBlock and ChatCompletionRedactedThinkingBlock share these declared keys
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatThinkingBlock {
    pub r#type: String,
    pub thinking: Option<String>,
    pub signature: Option<String>,
    pub data: Option<String>,
    pub cache_control: Option<JsonObject>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

// LiteLLM's Message declares these payloads, so they stay typed here and only keys no type names reach extra
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatImage {
    pub image_url: ChatImageUrl,
    pub index: i64,
    pub r#type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatImageUrl {
    pub url: String,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatAnnotation {
    #[serde(default)]
    pub r#type: String,
    pub url_citation: Option<ChatUrlCitation>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatUrlCitation {
    pub start_index: Option<i64>,
    pub end_index: Option<i64>,
    pub url: Option<String>,
    pub title: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatReasoningItem {
    pub r#type: String,
    pub id: Option<String>,
    pub encrypted_content: Option<String>,
    pub summary: Vec<ChatReasoningSummary>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatReasoningSummary {
    pub r#type: String,
    pub text: Option<String>,
}
// ChatCompletionMessageToolCall keeps the keys its constructor does not name, such as index
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatToolCall {
    pub id: String,
    pub r#type: String,
    pub function: Option<DeltaFunction>,
    pub custom: Option<CustomOutput>,
    #[serde(flatten)]
    pub extra: JsonObject,
}

// What responses() can hand back: its Responses object, a chat ModelResponse, its event stream or a cached chat stream
pub enum Answer {
    Response(Box<BridgeResponse>),
    Chat(Box<ChatResponse>),
    Events(Box<dyn ResponsesEventStream>),
    Cached(Box<dyn AsyncChunkStream + Send>),
}

pub struct BridgeResponse {
    pub payload: ResponsePayload,
    // the response object's own hidden parameters, which keep their keys over the stream's
    pub existing_hidden: Map<String, Value>,
    // the streaming iterator's hidden parameters, which fill the keys the response leaves absent
    pub stream_hidden: Map<String, Value>,
}

pub enum ResponsePayload {
    Typed(Box<ResponsesApiResponse>),
    Dict(Json),
}

pub enum BridgeOutput {
    Response(Box<ModelResponse>),
    Chat(Box<ChatResponse>),
    Stream(BridgeStream),
}

// One step of a bridge-built stream, typed until the AsyncChunkStream adapter turns it into the wire's text
#[async_trait]
pub trait TypedChunkStream: Send {
    async fn next_chunk(&mut self) -> Result<Option<Json>, BridgeStreamError>;
}

// The delivered stream: the bridge's own streams keep their typed failures, a wrapped or cached one only has the wire's
pub enum BridgeStream {
    Typed(Box<dyn TypedChunkStream + Send>),
    Wire(Box<dyn AsyncChunkStream + Send>),
}

impl BridgeStream {
    pub async fn next(&mut self) -> Result<Option<Json>, BridgeStreamError> {
        match self {
            Self::Typed(stream) => stream.next_chunk().await,
            Self::Wire(stream) => stream
                .next_chunk()
                .await
                .map_err(|failure| BridgeStreamError::Upstream { failure }),
        }
    }
}

// The outward wire-error adapter: HTTP and provider post-processing read every step as a StreamFailure
#[async_trait]
impl AsyncChunkStream for BridgeStream {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure> {
        self.next().await.map_err(StreamFailure::from)
    }
}

// The Responses iterator's next decoded SSE frame, still as the bytes B920 reads
#[async_trait]
pub trait ResponsesEventStream: Send {
    async fn next_event(&mut self) -> Option<Result<Vec<u8>, StreamFailure>>;

    fn hidden_params(&self) -> &Map<String, Value>;
}

// responses() and aresponses(): the one seam a provider executes behind, injected so unit tests name every answer
#[async_trait]
pub trait ResponsesExecution<Client: ?Sized>: Sync {
    type Error: Error + Send + 'static;

    async fn responses(&self, call: ResponsesCall<'_, Client>) -> Result<Answer, Self::Error>;

    async fn aresponses(&self, call: ResponsesCall<'_, Client>) -> Result<Answer, Self::Error>;
}

// The provider chat config a stream's post-processing needs, selected with B2230
pub struct PostStreamDeps<'deps> {
    pub families: &'deps (dyn ModelFamilyProbe + Send + Sync),
    pub base_model: Option<&'deps str>,
    pub bedrock: BedrockFacts<'deps>,
    pub vertex: VertexModelLists<'deps>,
    pub json_providers: &'deps (dyn JsonProviderProjection + Send + Sync),
    pub processing: &'deps dyn PostStreamProcessing,
}

// ProviderConfigManager.get_provider_chat_config(...).post_stream_processing(stream): B2202's per-provider seam, where a pass-through returns the typed stream unchanged
pub trait PostStreamProcessing: Send + Sync {
    fn post_stream_processing(
        &self,
        tag: ChatConfigTag,
        stream: BridgeStream,
    ) -> Result<BridgeStream, PostStreamError>;
}
