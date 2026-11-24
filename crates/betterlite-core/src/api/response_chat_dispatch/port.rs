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
use indexmap::IndexMap;
use serde_json::{Map, Value};

use super::error::PostStreamError;
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

// What responses() can hand back: its Responses object, a chat ModelResponse, its event stream or a cached chat stream
pub enum Answer {
    Response(BridgeResponse),
    Chat(Json),
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
    Chat(Json),
    Stream(Box<dyn AsyncChunkStream + Send>),
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

// ProviderConfigManager.get_provider_chat_config(...).post_stream_processing(stream): B2202's per-provider seam
pub trait PostStreamProcessing: Send + Sync {
    fn post_stream_processing(
        &self,
        tag: ChatConfigTag,
        stream: Box<dyn AsyncChunkStream + Send>,
    ) -> Result<Box<dyn AsyncChunkStream + Send>, PostStreamError>;
}
