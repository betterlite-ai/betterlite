use std::collections::BTreeMap;

use betterlite_auth::auth_identity::records::Principal;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::generic_guardrail_contracts::response::coerce_stream_holdback_value;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_translation::StreamTransformSink;
use indexmap::IndexMap;

use super::stream_request::{stream_inputs, stream_request_data};
use crate::guardrails::chat_guardrail_translation::extraction::accumulate_string_content::accumulate_by_choice_index;

mod tests;

// The log line is the only effect, and a test cannot see it.
#[cfg_attr(test, mutants::skip)]
fn warn_when_withheld(returned: usize, expected: usize) {
    if returned < expected {
        tracing::warn!(
            "OpenAI Chat Completions: guardrail returned {returned} transformed texts for {expected} inputs on the streaming transform path; withholding the unmatched choices to fail closed."
        );
    }
}

// A holdback the guardrail sends as a bad count (negative or too large) becomes 0, and a choice it sends none for has none.
fn holdback_by_choice(indices: &[i64], holdback: Vec<i64>) -> IndexMap<i64, i64> {
    indices
        .iter()
        .copied()
        .zip(holdback)
        .map(|(index, chars)| {
            let coerced = coerce_stream_holdback_value(&GuardrailValue::SignedInteger(chars));
            (index, i64::from(coerced))
        })
        .collect()
}

// The chunks are only read, so the guardrail sees the raw accumulated text every round; the result goes out through the sink.
pub async fn process_streaming_transform<G>(
    responses_so_far: &[Object],
    guardrail: &G,
    principal: &Principal,
    sink: &mut StreamTransformSink,
) -> Result<(), GuardrailFailure>
where
    G: GuardrailContract,
{
    let raw_by_index: BTreeMap<i64, String> = accumulate_by_choice_index(responses_so_far)
        .into_iter()
        .collect();
    if raw_by_index.is_empty() {
        sink.mutated_text_per_choice.clear();
        sink.holdback_per_choice.clear();
        return Ok(());
    }
    // Sorted by choice index so the guardrail's texts map back to the right choice.
    let (indices, texts): (Vec<i64>, Vec<String>) = raw_by_index.into_iter().unzip();
    let mut request_data = stream_request_data(responses_so_far, principal);
    let inputs = stream_inputs(texts.clone(), responses_so_far);
    let guardrailed = guardrail
        .apply_guardrail(
            inputs,
            &mut request_data,
            GuardrailInputType::Response,
            None,
        )
        .await?;
    // No texts back means the guardrail changed nothing; a shorter list withholds the unmatched choices rather than emitting them raw.
    let returned_texts = guardrailed.texts.unwrap_or(texts);
    warn_when_withheld(returned_texts.len(), indices.len());
    sink.mutated_text_per_choice = indices.iter().copied().zip(returned_texts).collect();
    sink.holdback_per_choice = holdback_by_choice(
        &indices,
        guardrailed.stream_holdback_chars.unwrap_or_default(),
    );
    Ok(())
}
