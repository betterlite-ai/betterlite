use betterlite_prompts::arguments::json::Json;

use crate::typesafe::TypeSafeGuardrail;

#[must_use]
pub fn exchange_tool_text(
    guardrail: &TypeSafeGuardrail,
    messages: &[Json],
    group: &[usize],
) -> String {
    let mut text = String::new();
    for index in group.iter().skip(1) {
        let Some(message) = messages.get(*index).and_then(Json::as_object) else {
            continue;
        };
        if !matches!(
            message.get("role").and_then(Json::as_str),
            Some("tool" | "function")
        ) {
            continue;
        }
        text.push_str(&(guardrail.content_to_text)(message.get("content")));
    }
    text
}

#[path = "typesafe_text_extraction/tests.rs"]
mod tests;
