use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::fingerprint::error::FingerprintError;
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};
use betterlite_guardrails::guardrail_translation::StreamingScanKey;

use crate::guardrails::chat_guardrail_translation::extraction::combine_texts::combine_streaming_texts;
use crate::guardrails::chat_guardrail_translation::extraction::first_choice_finished::first_choice_has_finished;
use crate::guardrails::chat_guardrail_translation::extraction::tool_fingerprints::tool_call_fingerprints;

mod tests;

// Texts come from the assembled chunks only; tool calls count once the stream has ended, and until then only flag that some are in flight.
pub fn get_streaming_scan_key(
    responses_so_far: &[StreamItem],
) -> Result<StreamingScanKey, FingerprintError> {
    let chunks: Vec<Object> = responses_so_far
        .iter()
        .filter_map(|item| match item {
            StreamItem::Json(GuardrailValue::Object(chunk)) => Some(chunk.clone()),
            StreamItem::Json(_) | StreamItem::Bytes(_) => None,
        })
        .collect();
    let stream_ended = first_choice_has_finished(responses_so_far);
    let fingerprints = tool_call_fingerprints(responses_so_far)?;
    let tool_calls_in_flight = !fingerprints.is_empty() && !stream_ended;
    Ok(StreamingScanKey {
        texts: combine_streaming_texts(&chunks).into_values().collect(),
        tool_calls: if stream_ended {
            fingerprints
        } else {
            Vec::new()
        },
        stream_ended,
        tool_calls_in_flight,
    })
}
