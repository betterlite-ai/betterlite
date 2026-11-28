use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::{Object, StreamItem};
use uuid::Uuid;

use super::super::failure_parts;
use crate::guardrails::chat_guardrail_translation::block_framing::blocked_identity::FreshIdentity;
use crate::guardrails::chat_guardrail_translation::rewrites::response::{
    ChatResponse, ContentPart, MessageContent, ResponseMessage, ResponseToolCall,
};
use crate::guardrails::responses_translation::block_events::BlockStamp;
use crate::guardrails::responses_translation::block_events::guardrail_json::to_guardrail;
use crate::sse_error_frames::{SseFailure, sse_error_payload};

// LiteLLM draws the chat id from uuid4 and the created time from the clock; both come out of the stamp
pub(super) fn fresh_identity(stamp: &BlockStamp) -> FreshIdentity {
    let mut hasher = DefaultHasher::new();
    stamp.response_id.hash(&mut hasher);
    stamp.message_id.hash(&mut hasher);
    FreshIdentity {
        id: Uuid::from_u128(u128::from(hasher.finish())),
        created: stamp.created_at,
    }
}

// LiteLLM's chat build_stream_error_items feeds sse_error_payload's object into its error frame
pub(super) fn failure_error_object(failure: &GuardrailFailure) -> Option<Object> {
    let (status, detail) = failure_parts(failure)?;
    let (_, fields) = sse_error_payload(&SseFailure {
        status_code: Some(status),
        code: None,
        error_type: None,
        param: None,
        detail: Some(Box::new(detail)),
        provider_specific_fields: None,
    });
    Some(
        fields
            .into_iter()
            .map(|(key, value)| (key, to_guardrail(&value)))
            .collect(),
    )
}

pub(super) fn object_chunks(items: &[StreamItem]) -> (Vec<usize>, Vec<Object>) {
    let mut positions = Vec::new();
    let mut chunks = Vec::new();
    for (position, item) in items.iter().enumerate() {
        let StreamItem::Json(GuardrailValue::Object(fields)) = item else {
            continue;
        };
        positions.push(position);
        chunks.push(fields.clone());
    }
    (positions, chunks)
}

pub(super) fn write_object_chunks(
    items: &mut [StreamItem],
    positions: &[usize],
    chunks: &[Object],
) {
    for (position, chunk) in positions.iter().zip(chunks) {
        let Some(slot) = items.get_mut(*position) else {
            continue;
        };
        *slot = StreamItem::Json(GuardrailValue::Object(chunk.clone()));
    }
}

// Only the leaves B3668 rewrites go back, so every field the typed view drops stays as the provider sent it
pub(super) fn write_back(
    response: &mut GuardrailValue,
    before: &ChatResponse,
    after: &ChatResponse,
) {
    let GuardrailValue::Object(fields) = response else {
        return;
    };
    let Some(GuardrailValue::Array(choices)) = fields.get_mut("choices") else {
        return;
    };
    for (slot, (was, now)) in choices
        .iter_mut()
        .zip(before.choices.iter().zip(&after.choices))
    {
        let GuardrailValue::Object(choice) = slot else {
            continue;
        };
        let Some(GuardrailValue::Object(message)) = choice.get_mut("message") else {
            continue;
        };
        write_content(message.get_mut("content"), &was.message, &now.message);
        write_tool_calls(message.get_mut("tool_calls"), &was.message, &now.message);
    }
}

fn write_content(slot: Option<&mut GuardrailValue>, was: &ResponseMessage, now: &ResponseMessage) {
    let Some(target) = slot else {
        return;
    };
    let (Some(was_content), Some(now_content)) = (was.content.as_ref(), now.content.as_ref())
    else {
        return;
    };
    if let (MessageContent::Text(old), MessageContent::Text(new)) = (was_content, now_content) {
        if old != new {
            *target = GuardrailValue::Text(new.clone());
        }
        return;
    }
    if let (MessageContent::Parts(old), MessageContent::Parts(new)) = (was_content, now_content) {
        write_parts(target, old, new);
    }
}

fn write_parts(slot: &mut GuardrailValue, before: &[ContentPart], after: &[ContentPart]) {
    let GuardrailValue::Array(parts) = slot else {
        return;
    };
    for (part, (was, now)) in parts.iter_mut().zip(before.iter().zip(after)) {
        let (Some(was_text), Some(now_text)) = (was.text.as_ref(), now.text.as_ref()) else {
            continue;
        };
        if was_text == now_text {
            continue;
        }
        let GuardrailValue::Object(fields) = part else {
            continue;
        };
        fields.insert("text".to_owned(), GuardrailValue::Text(now_text.clone()));
    }
}

fn write_tool_calls(
    slot: Option<&mut GuardrailValue>,
    was: &ResponseMessage,
    now: &ResponseMessage,
) {
    let (Some(target), Some(was_calls), Some(now_calls)) =
        (slot, was.tool_calls.as_ref(), now.tool_calls.as_ref())
    else {
        return;
    };
    let GuardrailValue::Array(calls) = target else {
        return;
    };
    for (call, (was_call, now_call)) in calls.iter_mut().zip(was_calls.iter().zip(now_calls)) {
        write_call(call, was_call, now_call);
    }
}

fn write_call(call: &mut GuardrailValue, was: &ResponseToolCall, now: &ResponseToolCall) {
    let (Some(was_function), Some(now_function)) = (was.function.as_ref(), now.function.as_ref())
    else {
        return;
    };
    let GuardrailValue::Object(call_fields) = call else {
        return;
    };
    let Some(GuardrailValue::Object(function)) = call_fields.get_mut("function") else {
        return;
    };
    write_function_text(
        function,
        "name",
        was_function.name.as_deref(),
        now_function.name.as_deref(),
    );
    write_function_text(
        function,
        "arguments",
        was_function.arguments.as_deref(),
        now_function.arguments.as_deref(),
    );
}

fn write_function_text(fields: &mut Object, key: &str, was: Option<&str>, now: Option<&str>) {
    let Some(replacement) = now else {
        return;
    };
    if was == Some(replacement) {
        return;
    }
    fields.insert(key.to_owned(), GuardrailValue::Text(replacement.to_owned()));
}
