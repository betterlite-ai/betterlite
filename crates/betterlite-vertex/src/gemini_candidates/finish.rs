use betterlite_models::call_helpers::finish_reason::FinishReason;

use crate::gemini_candidates::choice::Message;

// B031 supplies the common mapper from B206 so this leaf crate does not depend on the core
pub trait FinishReasonMapper {
    fn map(&self, raw: &str) -> FinishReason;
}

#[must_use]
pub fn check_finish_reason(
    mapper: &dyn FinishReasonMapper,
    message: &Message,
    native: Option<&str>,
) -> FinishReason {
    if message.function_call.is_some() {
        FinishReason::FunctionCall
    } else if message.tool_calls.is_some() {
        FinishReason::ToolCalls
    } else {
        native.map_or(FinishReason::Stop, |raw| mapper.map(raw))
    }
}
