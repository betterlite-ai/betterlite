use betterlite_models::decisions::Json;

use super::fields::{list, object};
use crate::api::responses_tool_namespaces::values::has_value;

#[must_use]
pub fn reasoning_ended(chunk: &Json) -> bool {
    let Some(choice) = object(chunk)
        .and_then(|fields| fields.get("choices"))
        .and_then(list)
        .and_then(|choices| choices.first())
        .and_then(object)
    else {
        return false;
    };
    let delta = choice.get("delta").and_then(object);
    if delta.is_some_and(|fields| {
        fields.get("reasoning_content").is_some_and(has_value)
            || fields.get("thinking_blocks").is_some_and(has_value)
    }) {
        return false;
    }
    let content = delta.is_some_and(|fields| fields.get("content").is_some_and(has_value));
    let function_call =
        delta.is_some_and(|fields| fields.get("function_call").is_some_and(has_value));
    let tool_calls = delta.is_some_and(|fields| fields.get("tool_calls").is_some_and(has_value));
    let finished = choice
        .get("finish_reason")
        .is_some_and(|value| !matches!(value, Json::Null));
    content || function_call || tool_calls || finished
}
