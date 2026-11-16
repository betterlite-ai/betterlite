use betterlite_core::api::response_chat_history::history::{HistoryMessage, convert_history};
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::merge_messages::merge_scoped_messages;
use serde_json::Value;

use super::error::RequestRewriteError;
use super::patch::patched_request_fields;
use super::{RequestContext, RequestFields, ScopedRequest, serde_convert};

// A patch that cannot be made falls back to converting the whole message list
pub fn patch_or_convert_request_fields(
    raw_input: Option<&Value>,
    raw_instructions: Option<&Value>,
    original_messages: &[Value],
    merged_messages: &[Value],
    context: &RequestContext<'_>,
) -> Result<RequestFields, RequestRewriteError> {
    if let Some(patched) = patched_request_fields(
        raw_input,
        raw_instructions,
        original_messages,
        merged_messages,
        context,
    )? {
        return Ok(patched);
    }
    let mut history = Vec::with_capacity(merged_messages.len());
    for message in merged_messages {
        history.push(serde_convert::<_, HistoryMessage>(
            message,
            "rewritten message",
        )?);
    }
    let converted =
        convert_history(&history).map_err(|source| RequestRewriteError::History { source })?;
    Ok(RequestFields {
        input: converted.input,
        instructions: converted.instructions,
    })
}

// None when the guardrail returned no messages or only the ones it was sent
pub fn written_back_request_fields(
    data: &Object,
    scoped: &ScopedRequest,
    answer: &GuardrailApiInputs,
    guardrail_covers_full_request: bool,
    context: &RequestContext<'_>,
) -> Result<Option<RequestFields>, RequestRewriteError> {
    let Some(returned) = &answer.structured_messages else {
        return Ok(None);
    };
    if scoped.scoped_messages.as_ref() == Some(returned) {
        return Ok(None);
    }
    let full_length = scoped.structured_messages.len();
    let covers_full_request = scoped.scoped_indices.len() == full_length
        || (guardrail_covers_full_request && returned.len() == full_length);
    let returned_rows: Vec<Value> = serde_convert(returned, "returned messages")?;
    let merged = if covers_full_request {
        returned_rows
    } else {
        merge_scoped_messages(
            &scoped.structured_messages,
            &scoped.scoped_indices,
            &returned_rows,
        )
    };
    let raw_input: Option<Value> = data
        .get("input")
        .map(|input| serde_convert(input, "input"))
        .transpose()?;
    let raw_instructions: Option<Value> = data
        .get("instructions")
        .map(|instructions| serde_convert(instructions, "instructions"))
        .transpose()?;
    patch_or_convert_request_fields(
        raw_input.as_ref(),
        raw_instructions.as_ref(),
        &scoped.structured_messages,
        &merged,
        context,
    )
    .map(Some)
}
