use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::error::FingerprintError;
use betterlite_guardrails::guardrail_message_scope::fingerprint::{
    stream_item_fingerprint, stream_item_items,
};

use super::delta_tool_calls::streamed_delta_tool_calls;
use super::json_access::field;

mod tests;

pub fn tool_call_fingerprints(
    responses_so_far: &[StreamItem],
) -> Result<Vec<String>, FingerprintError> {
    responses_so_far
        .iter()
        .flat_map(|chunk| stream_item_items(chunk, "choices"))
        .filter_map(|choice| field(choice, "delta"))
        .flat_map(streamed_delta_tool_calls)
        .map(|tool_call| stream_item_fingerprint(&StreamItem::Json(tool_call.clone())))
        .collect()
}
