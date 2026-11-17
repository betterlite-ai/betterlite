use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::chunk::field_mut;

// LiteLLM's _apply_sanitized_content: every non-empty choice message carries the sanitized text.
pub fn apply_sanitized(body: &mut GuardrailValue, sanitized: &str) {
    let Some(GuardrailValue::Array(choices)) = field_mut(body, "choices") else {
        return;
    };
    for choice in choices {
        let Some(message) = field_mut(choice, "message") else {
            continue;
        };
        let Some(content) = field_mut(message, "content") else {
            continue;
        };
        if let GuardrailValue::Text(text) = content
            && !text.is_empty()
        {
            sanitized.clone_into(text);
        }
    }
}
