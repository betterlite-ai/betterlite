use super::error::BridgeError;
use crate::api::responses_chat_call::records::CallOutcome;
use crate::api::responses_create_http::config::Headers;
use crate::api::responses_create_http::service::{CreateLive, CreateSynthetic};
use crate::api::responses_provider_selection::ParameterMap;
use async_trait::async_trait;
use betterlite_models::decisions::HiddenParams;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Object;
use serde_json::value::RawValue;
pub struct StampedResponse {
    pub response: ResponsesApiResponse,
    pub hidden: HiddenParams,
}
pub enum Created<Context> {
    Responses(Box<StampedResponse>),
    Raw(Box<RawValue>),
    Live(Box<CreateLive>),
    Synthetic(Box<CreateSynthetic>),
    Chat(Box<CallOutcome<Context>>),
}
pub struct BridgeCall<'call> {
    pub model: &'call str,
    pub input: &'call ResponsesInput,
    pub request: &'call Object,
    pub custom_llm_provider: Option<&'call str>,
    pub is_async: bool,
    pub stream: Option<bool>,
    pub extra_headers: Option<&'call Headers>,
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
