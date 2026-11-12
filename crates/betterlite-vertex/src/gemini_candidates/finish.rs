use betterlite_models::call_helpers::finish_reason::{FinishReason, map_finish_reason};

use crate::gemini_candidates::choice::Message;

#[must_use]
pub fn check_finish_reason(message: &Message, native: Option<&str>) -> FinishReason {
    if message.function_call.is_some() {
        FinishReason::FunctionCall
    } else if message.tool_calls.is_some() {
        FinishReason::ToolCalls
    } else {
        native.map_or(FinishReason::Stop, |raw| map_finish_reason(raw).reason)
    }
}
