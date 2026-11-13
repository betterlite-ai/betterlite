use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_items;

use super::json_access::field;

mod tests;

// Only choices[0] of each chunk counts, so a later choice that finishes first does not end the stream.
#[must_use]
pub fn first_choice_has_finished(responses_so_far: &[StreamItem]) -> bool {
    responses_so_far
        .iter()
        .filter_map(|chunk| stream_item_items(chunk, "choices").first())
        .any(|choice| field(choice, "finish_reason").is_some())
}
