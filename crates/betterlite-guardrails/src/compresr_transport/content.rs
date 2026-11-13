use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::content_text::{is_all_text_parts, merge_rewritten_text_parts};

mod tests;

// Only a run of text parts can take one compressed string; a non-text part keeps the content as it is
#[must_use]
pub fn replace_text_in_content(content: &GuardrailValue, new_text: &str) -> GuardrailValue {
    match content {
        GuardrailValue::Text(_) => GuardrailValue::Text(new_text.to_owned()),
        GuardrailValue::Array(parts) if is_all_text_parts(content) => {
            GuardrailValue::Array(merge_rewritten_text_parts(parts, new_text))
        }
        _ => content.clone(),
    }
}
