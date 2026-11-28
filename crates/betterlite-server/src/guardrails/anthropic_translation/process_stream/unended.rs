use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::decode_replacing_invalid;
use serde_json::Value;

use super::StreamContext;
use super::error::{ProcessStreamError, keep_original_response};
use super::prepare::prepare_request_data;
use crate::guardrails::anthropic_translation::sse_events::{SseChunk, streaming_string_so_far};
use crate::guardrails::anthropic_translation::stream_rewrite::ended;
use crate::guardrails::anthropic_translation::usage_response::build_streaming_usage_response;
use crate::guardrails::responses_translation::block_events::guardrail_json::to_guardrail;

// The chunks as the guardrail sees them: a parsed event as its object, bytes as their text, the rest as null.
fn chunk_value(chunk: &SseChunk) -> GuardrailValue {
    match chunk {
        SseChunk::Bytes(bytes) => GuardrailValue::Text(decode_replacing_invalid(bytes)),
        SseChunk::Event(event) => to_guardrail(&Value::Object(event.clone())),
        SseChunk::Other => GuardrailValue::Null,
    }
}

fn text_inputs(text: &str) -> GuardrailApiInputs {
    GuardrailApiInputs {
        texts: Some(vec![text.to_owned()]),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    }
}

// A ModifyResponse without original_response falls back to the usage-only response of the buffered chunks.
fn unended_failure(
    failure: GuardrailFailure,
    responses_so_far: &[SseChunk],
    request_data: Option<&Object>,
) -> ProcessStreamError {
    let needs_usage = matches!(
        &failure,
        GuardrailFailure::ModifyResponse(response) if response.original_response.is_none()
    );
    let fallback = needs_usage
        .then(|| build_streaming_usage_response(responses_so_far, request_data))
        .flatten();
    ProcessStreamError::from(keep_original_response(failure, fallback))
}

fn deliver_text(
    responses_so_far: &mut [SseChunk],
    answer: &GuardrailApiInputs,
    string_so_far: &str,
    context: &StreamContext<'_>,
) -> Result<(), ProcessStreamError> {
    let Some(texts) = answer.texts.as_deref() else {
        return Ok(());
    };
    if !context.deliver {
        return Ok(());
    }
    let unchanged = texts.len() == 1
        && texts
            .first()
            .is_some_and(|first| first.as_str() == string_so_far);
    if unchanged {
        return Ok(());
    }
    let Some(rewritten) = texts.first() else {
        return Ok(());
    };
    ended::rewrite_ended_stream_text(responses_so_far, rewritten, context.guardrail_name)?;
    Ok(())
}

pub(super) async fn process_unended<G>(
    responses_so_far: &mut [SseChunk],
    guardrail: &G,
    context: &StreamContext<'_>,
    logging_obj: Option<&mut GuardrailLogTarget>,
    request_data: Option<&mut Object>,
) -> Result<(), ProcessStreamError>
where
    G: GuardrailContract,
{
    let string_so_far = streaming_string_so_far(responses_so_far);
    let responses_value = GuardrailValue::Array(responses_so_far.iter().map(chunk_value).collect());
    let inputs = text_inputs(&string_so_far);
    let mut fresh = Object::new();
    let prepared = request_data.unwrap_or(&mut fresh);
    prepare_request_data(
        prepared,
        responses_value,
        context.user_api_key_dict,
        "responses",
    );
    let outcome = guardrail
        .apply_guardrail(inputs, prepared, GuardrailInputType::Response, logging_obj)
        .await;
    let answer = match outcome {
        Ok(answer) => answer,
        Err(failure) => {
            return Err(unended_failure(failure, responses_so_far, Some(&*prepared)));
        }
    };
    deliver_text(responses_so_far, &answer, &string_so_far, context)
}
