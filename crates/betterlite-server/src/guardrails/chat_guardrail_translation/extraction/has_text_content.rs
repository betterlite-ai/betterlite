use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::Object;

use super::json_access::{choice_parts, object_items};

mod tests;

#[must_use]
pub fn has_text_content(response: &Object) -> bool {
    object_items(response, "choices")
        .iter()
        .filter_map(choice_parts)
        .any(|parts| {
            matches!(parts.content, Some(GuardrailValue::Text(text)) if !text.is_empty())
                || !parts.tool_calls.is_empty()
        })
}
