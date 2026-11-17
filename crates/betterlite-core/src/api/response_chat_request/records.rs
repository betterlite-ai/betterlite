use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::request_params::ResponsesApiRequestParams;
use indexmap::IndexMap;
use serde_json::Value;

use crate::api::response_chat_history::history::HistoryMessage;

pub struct ChatRequestInput<'call> {
    pub model: &'call str,
    pub messages: &'call [HistoryMessage],
    // Json keeps the caller's key order, which decides which of two keys mapping to reasoning wins
    pub optional_params: &'call mut IndexMap<String, Json>,
    pub litellm_params: &'call IndexMap<String, Value>,
    pub headers: &'call IndexMap<String, String>,
    pub reasoning_auto_summary: bool,
}

pub struct ResponsesChatRequest<'state, Client: ?Sized> {
    pub params: ResponsesApiRequestParams,
    // LiteLLM splats the sanitized SDK params next to the Responses fields, so the caller reads them here
    pub litellm_params: IndexMap<String, Value>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub client: Option<&'state Client>,
    pub logging: &'state mut LoggingState,
}
