use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;
use indexmap::IndexMap;

use crate::api::decisions_send::error::visible_message;
use crate::cost::realtime::RealtimeBilling;

pub struct WsCallUpdate<'call> {
    pub kwargs: &'call IndexMap<String, Json>,
    pub model: &'call str,
    pub user: Option<&'call str>,
    pub litellm_params: &'call IndexMap<String, Json>,
    pub custom_llm_provider: Option<&'call str>,
}

// B249, B250, B251 and B254 own the logging object methods; these are the calls the native relay makes.
pub trait WsNativeLogging: Send + Sync {
    fn update_from_kwargs(&self, update: WsCallUpdate<'_>) -> Result<(), LoggingStateError>;
    // the masked client frame is handed over before it reaches the backend, with the literal empty API key
    fn pre_call(&self, input: &str, api_key: &str);
    fn record_input_messages(&self, messages: &[IndexMap<String, Json>]);
    // dispatch_success_handlers(messages, prefer_async_handlers=True)
    fn dispatch_success(&self, messages: Vec<IndexMap<String, Json>>);
    fn response_cost(&self, billing: &RealtimeBilling) -> Option<f64>;
    fn record_partial_usage(&self, usage: &Usage, response_cost: f64);
    // dispatch_failure_handlers(exception, traceback, prefer_async_handlers=True)
    fn dispatch_failure(&self, exception: ErrorClass, traceback: String);
}

// LiteLLM formats the mapped exception's traceback; the frames are LiteLLM's own, so betterlite keeps the header and the final line
#[must_use]
pub fn traceback_text(exception: &ErrorClass) -> String {
    format!(
        "Traceback (most recent call last):\n{}",
        visible_message(exception)
    )
}
