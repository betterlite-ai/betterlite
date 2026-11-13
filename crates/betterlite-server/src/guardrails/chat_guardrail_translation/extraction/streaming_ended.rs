use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_items;

use super::json_access::field;

mod tests;

#[must_use]
pub fn check_streaming_has_ended(responses_so_far: &[StreamItem]) -> bool {
    responses_so_far
        .iter()
        .flat_map(|chunk| stream_item_items(chunk, "choices"))
        .any(|choice| field(choice, "finish_reason").is_some())
}
