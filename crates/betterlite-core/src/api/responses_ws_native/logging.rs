use betterlite_error::error::ErrorClass;
use betterlite_logging::logging_state::LoggingStateError;
use betterlite_models::decisions::Json;
use betterlite_models::usage_sum::Usage;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::api::decisions_send::error::visible_message;
use crate::cost::realtime::RealtimeBilling;

pub struct WsCallUpdate<'call> {
    pub kwargs: &'call IndexMap<String, Json>,
    pub model: &'call str,
    pub user: Option<&'call str>,
    pub litellm_params: &'call IndexMap<String, Json>,
    pub custom_llm_provider: Option<&'call str>,
}

pub trait WsNativeLogging: Send + Sync {
    fn update_from_kwargs(&self, update: WsCallUpdate<'_>) -> Result<(), LoggingStateError>;
    fn pre_call(&self, input: &str, api_key: &str);
    fn record_input_messages(&self, messages: &[IndexMap<String, Json>]);
    fn dispatch_success(&self, messages: Vec<IndexMap<String, Json>>, prefer_async: bool);
    fn response_cost(&self, billing: &RealtimeBilling) -> Option<Decimal>;
    fn record_partial_usage(&self, usage: &Usage, response_cost: Decimal);
    fn dispatch_failure(&self, exception: ErrorClass, traceback: String, prefer_async: bool);
}

#[must_use]
pub fn traceback_text(exception: &ErrorClass) -> String {
    format!(
        "Traceback (most recent call last):\n{}",
        visible_message(exception)
    )
}
