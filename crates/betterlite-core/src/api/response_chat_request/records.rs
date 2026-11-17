use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use indexmap::IndexMap;
use serde_json::Value;

use crate::api::response_chat_history::history::HistoryMessage;

pub struct ChatRequestInput<'a> {
    pub model: &'a str,
    pub messages: &'a [HistoryMessage],
    pub optional_params: &'a mut IndexMap<String, Value>,
    pub litellm_params: &'a IndexMap<String, Value>,
    pub headers: &'a IndexMap<String, String>,
    pub reasoning_auto_summary: bool,
}

pub struct ResponsesChatRequest<'a, Client: ?Sized> {
    pub params: ResponsesApiRequestParams,
    // LiteLLM splats the sanitized SDK params next to the Responses fields, so the caller reads them here
    pub litellm_params: IndexMap<String, Value>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub client: Option<&'a Client>,
    pub logging: &'a mut LoggingState,
}
