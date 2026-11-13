use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::scoped_indices::ScopeFlags;

use super::extract_inputs::extract_inputs;
use super::extracted_content::ExtractedContent;

mod tests;

pub const AFTER_SCOPING: &str = "no scannable content after message scoping";
pub const NOTHING_SCANNABLE: &str = "no scannable content";

// The messages arrive already scoped, so nothing is skipped by role here; images alone are never scanned and give no reason.
#[must_use]
pub fn not_run_reason(messages: &[GuardrailValue]) -> Option<&'static str> {
    let mut content = ExtractedContent::default();
    for (message_index, message) in messages.iter().enumerate() {
        extract_inputs(message, message_index, &ScopeFlags::default(), &mut content);
    }
    if !content.texts.is_empty() || !content.tool_calls.is_empty() {
        return Some(AFTER_SCOPING);
    }
    content.images.is_empty().then_some(NOTHING_SCANNABLE)
}
