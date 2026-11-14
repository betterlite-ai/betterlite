use betterlite_openai::chat::response::ChatResponse;
use serde_json::from_str;
use serde_json::value::RawValue;

use super::{ChatLoopResult, LoggingHandle};

mod tests;

// LiteLLM's _wrap_response_as_fake_stream: a CustomStreamWrapper over a MockResponseIterator
pub trait FakeStreamPort: Send + Sync {
    fn wrap_model_response(
        &self,
        response: Box<RawValue>,
        model: &str,
        provider: &str,
        logging: &LoggingHandle<'_>,
    ) -> ChatLoopResult;

    #[must_use]
    fn wrap(
        &self,
        response: ChatLoopResult,
        model: &str,
        provider: &str,
        logging: &LoggingHandle<'_>,
    ) -> ChatLoopResult {
        match response {
            ChatLoopResult::Stream(stream) => ChatLoopResult::Stream(stream),
            ChatLoopResult::Response(raw) => {
                let is_model_response = from_str::<ChatResponse>(raw.get()).is_ok();
                if is_model_response && matches!(logging, LoggingHandle::Object(_)) {
                    return self.wrap_model_response(raw, model, provider, logging);
                }
                ChatLoopResult::Response(raw)
            }
        }
    }
}
