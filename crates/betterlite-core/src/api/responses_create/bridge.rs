use async_trait::async_trait;
use betterlite_models::decisions::HiddenParams;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use serde_json::value::RawValue;

use crate::api::responses_chat_call::records::CallOutcome;
use crate::api::responses_create_http::config::Headers;
use crate::api::responses_create_http::service::{CreateLive, CreateSynthetic};
use crate::api::responses_provider_selection::ParameterMap;

use super::error::BridgeError;

// A native response keeps the hidden params the outer wrapper stamps on it
pub struct StampedResponse {
    pub response: ResponsesApiResponse,
    pub hidden: HiddenParams,
}

// What a create call hands back: a native response, a body no typed response decoded, a stream, or the chat bridge's own outcome
pub enum Created<Context> {
    Responses(Box<StampedResponse>),
    Raw(Box<RawValue>),
    Live(Box<CreateLive>),
    Synthetic(Box<CreateSynthetic>),
    Chat(Box<CallOutcome<Context>>),
}

// The arguments litellm_completion_transformation_handler.response_api_handler receives
pub struct BridgeCall<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    pub request: &'call Object,
    pub custom_llm_provider: Option<&'call str>,
    pub is_async: bool,
    pub stream: Option<bool>,
    pub extra_headers: Option<&'call Headers>,
    // The completion arguments: bridge_kwargs plus extra_body, timeout and allowed_openai_params
    pub kwargs: &'call ParameterMap,
}

#[async_trait]
pub trait ChatBridge<Context: Send>: Send + Sync {
    async fn respond(
        &self,
        call: &BridgeCall<'_>,
        context: Context,
    ) -> Result<Created<Context>, BridgeError>;
}

// The composition supplies the bridge; a bridged request without one has no handler to reach
pub struct NoBridge;

#[async_trait]
impl<Context: Send> ChatBridge<Context> for NoBridge {
    async fn respond(
        &self,
        _call: &BridgeCall<'_>,
        _context: Context,
    ) -> Result<Created<Context>, BridgeError> {
        Err(BridgeError {
            message: String::from("the chat completion bridge is not composed"),
        })
    }
}
