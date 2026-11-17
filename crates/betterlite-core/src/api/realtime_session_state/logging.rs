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

impl RealtimeStreaming {
    pub fn store_input(&mut self, message: SessionInput<'_>) {
        self.input_message = match message {
            SessionInput::Record(record) => record.clone(),
            SessionInput::Text(_) => IndexMap::new(),
        };
        self.collect_client_input(message);
        if let Some(logging) = self.logging.as_deref() {
            logging.pre_call(&message, "");
        }
    }

    #[must_use]
    pub fn log_messages(&self) -> bool {
        let Some(logging) = self.logging.as_deref() else {
            return false;
        };
        if !self.input_messages.is_empty() {
            logging.record_model_call_detail(
                "messages",
                Json::List(
                    self.input_messages
                        .iter()
                        .cloned()
                        .map(Json::Object)
                        .collect(),
                ),
            );
        }
        if !self.session_tools.is_empty() || !self.tool_calls.is_empty() {
            logging.record_model_call_detail(
                "realtime_tools",
                Json::List(
                    self.session_tools
                        .iter()
                        .cloned()
                        .map(Json::Object)
                        .collect(),
                ),
            );
            logging.record_model_call_detail(
                "realtime_tool_calls",
                Json::List(self.tool_calls.iter().cloned().map(Json::Object).collect()),
            );
        }
        let work = logging.dispatch_success_handlers(&self.messages);
        let queue = self.logging_worker.as_deref();
        if !queue.is_some_and(|sender| sender.enqueue(work)) {
            return false;
        }
        logging.record_model_call_detail(SUCCESS_LOGGED_KEY, Json::Bool(true));
        true
    }
}
