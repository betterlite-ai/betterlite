use std::str::Utf8Error;

use betterlite_core::cost::recording::PortFailure;
use betterlite_core::openai_pricing::cost_error::CostError;
use betterlite_core::openai_pricing::responses_api::ResponsesApiError;
use betterlite_core::routes::CallType;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::passthrough_handler::types::StreamedResponse;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_openai::compatible_wire::error::CompatibleWireError;
use chrono::{DateTime, FixedOffset};
use rust_decimal::Decimal;
use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::super::base_passthrough_logging::UpstreamChatReply;

// OpenAIConfig.transform_response's arguments as the chat branch hands them over.
pub struct ChatTransformRequest<'request> {
    pub upstream: &'request UpstreamChatReply<'request>,
    pub model: &'request str,
    pub messages: &'request Json,
    pub optional_params: &'request Json,
    pub request_data: &'request JsonObject,
    pub json_mode: bool,
    pub litellm_params: &'request JsonObject,
}

pub struct ResponsesRequest<'request> {
    pub upstream: &'request UpstreamChatReply<'request>,
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub now: DateTime<FixedOffset>,
}

// completion_cost's arguments; a call type is passed only for a Responses reply.
pub struct PricedResponse<'request> {
    pub response: &'request Json,
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub call_type: Option<CallType>,
}

pub struct StandardObjectRequest<'request> {
    pub response: &'request Json,
    pub start_time: DateTime<FixedOffset>,
    pub end_time: DateTime<FixedOffset>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageEndpoint {
    Generation,
    Editing,
}

// The merged calls the OpenAI handlers make, so tests can stand in for each of them.
pub trait OpenAiLoggingTools: Send + Sync {
    fn chat_response(&self, request: &ChatTransformRequest<'_>) -> Result<Json, OpenAiToolsError>;

    fn embedding_response(
        &self,
        response_body: &JsonObject,
    ) -> Result<JsonObject, OpenAiToolsError>;

    fn embedding_cost(
        &self,
        response: &JsonObject,
        model: &str,
        custom_llm_provider: &str,
    ) -> Decimal;

    fn image_cost(
        &self,
        endpoint: ImageEndpoint,
        model: &str,
        request_body: &JsonObject,
    ) -> Decimal;

    fn responses_with_cost(
        &self,
        request: &ResponsesRequest<'_>,
        call_log: &mut LoggingState,
    ) -> Result<(Json, Decimal), OpenAiToolsError>;

    fn terminal_response(&self, all_chunks: &[String]) -> Result<Option<Json>, OpenAiToolsError>;

    fn stream_response(
        &self,
        all_chunks: &[String],
        model: &str,
    ) -> Result<Option<StreamedResponse>, OpenAiToolsError>;

    fn completion_cost(&self, request: &PricedResponse<'_>) -> Result<Decimal, OpenAiToolsError>;

    fn standard_logging_object(
        &self,
        request: &StandardObjectRequest<'_>,
        call_log: &LoggingState,
    ) -> Option<Json>;
}

#[derive(Debug, Error)]
pub enum OpenAiToolsError {
    #[error("the chat reply could not be decoded: {0}")]
    Chat(#[source] CompatibleWireError),
    #[error("the Responses reply could not be built: {0}")]
    Responses(#[source] ResponsesApiError),
    #[error("the response could not be priced: {0}")]
    Cost(#[source] PortFailure),
    #[error("the Responses reply could not be priced: {0}")]
    ResponsesCost(#[source] CostError),
    #[error("the relayed reply is not UTF-8 text: {0}")]
    Text(#[source] Utf8Error),
    #[error("the logged value could not be carried as JSON: {0}")]
    Encoding(#[source] serde_json::Error),
    #[error("the stream assembly waited on work the logging path cannot drive")]
    Pending,
}

// The serde round trip that moves a value between the typed records and the logged JSON.
pub(crate) fn reencode<Source, Target>(value: &Source) -> Result<Target, OpenAiToolsError>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    serde_json::to_string(value)
        .and_then(|encoded| serde_json::from_str(&encoded))
        .map_err(OpenAiToolsError::Encoding)
}
