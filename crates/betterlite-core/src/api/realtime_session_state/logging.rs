use std::future::Future;
use std::pin::Pin;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::RealtimeStreaming;

pub const SUCCESS_LOGGED_KEY: &str = "realtime_session_success_logged";

pub type SuccessWork = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionInput<'input> {
    Text(&'input str),
    Record(&'input IndexMap<String, Json>),
}

pub trait RealtimeLogging: Send + Sync {
    fn pre_call(&self, input: &SessionInput<'_>, api_key: &str);
    fn record_model_call_detail(&self, key: &str, value: Json);
    fn dispatch_success_handlers(&self, messages: &[IndexMap<String, Json>]) -> SuccessWork;
}

pub trait RealtimeSuccessQueue: Send + Sync {
    // the bounded worker's admission: a rejected submission leaves realtime_session_success_logged unset
    fn enqueue(&self, work: SuccessWork) -> bool;
}

// LiteLLM keeps a passed mapping as input_message but an empty mapping for a string input
pub(crate) fn retained_input(message: SessionInput<'_>) -> IndexMap<String, Json> {
    match message {
        SessionInput::Record(record) => record.clone(),
        SessionInput::Text(_) => IndexMap::new(),
    }
}

pub(crate) fn record_details(logging: &dyn RealtimeLogging, state: &RealtimeStreaming) {
    if !state.input_messages.is_empty() {
        logging.record_model_call_detail(
            "messages",
            Json::List(
                state
                    .input_messages
                    .iter()
                    .cloned()
                    .map(Json::Object)
                    .collect(),
            ),
        );
    }
    if !state.session_tools.is_empty() || !state.tool_calls.is_empty() {
        logging.record_model_call_detail(
            "realtime_tools",
            Json::List(
                state
                    .session_tools
                    .iter()
                    .cloned()
                    .map(Json::Object)
                    .collect(),
            ),
        );
        logging.record_model_call_detail(
            "realtime_tool_calls",
            Json::List(state.tool_calls.iter().cloned().map(Json::Object).collect()),
        );
    }
}
