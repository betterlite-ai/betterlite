use betterlite_core::api::response_chat_history::history::{HistoryMessage, convert_history};
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::merge_messages::merge_scoped_messages;
use serde_json::Value;

use super::error::RequestRewriteError;
use super::patch::patched_request_fields;
use super::{RequestContext, RequestFields, ScopedRequest, serde_convert};

pub fn patch_or_convert_fields(
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
    let history = merged_messages
        .iter()
        .map(|message| serde_convert::<_, HistoryMessage>(message, "rewritten message"))
        .collect::<Result<Vec<_>, _>>()?;
    let converted = convert_history(&history)?;
    Ok(RequestFields {
        input: converted.input,
        instructions: converted.instructions,
    })
}

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
    let request: Value = serde_convert(data, "request")?;
    patch_or_convert_fields(
        request.get("input"),
        request.get("instructions"),
        &scoped.structured_messages,
        &merged,
        context,
    )
    .map(Some)
}
