pub mod keys;
mod output;
pub mod text;
pub mod tool_calls;
mod usage;

#[path = "arize_messages/tests.rs"]
mod tests;

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::{LLMObsAttributes, SafeSetAttribute as _};
use keys::{
    INPUT_VALUE, LLM_INPUT_MESSAGES, MESSAGE_CONTENT, MESSAGE_CONTENT_IMAGE_URL,
    MESSAGE_CONTENT_TEXT, MESSAGE_CONTENT_TYPE, MESSAGE_CONTENTS, MESSAGE_NAME, MESSAGE_ROLE,
    MESSAGE_TOOL_CALL_ID,
};
use opentelemetry::trace::Span;

pub struct ArizeOtelAttributes;

impl LLMObsAttributes for ArizeOtelAttributes {
    fn set_messages<S>(span: &mut S, kwargs: &MetadataMap)
    where
        S: Span,
    {
        let found = kwargs
            .get("messages")
            .and_then(OtelModelValue::as_array)
            .filter(|messages| !messages.is_empty());
        let Some(messages) = found else {
            return;
        };
        let last_content = messages
            .last()
            .and_then(OtelModelValue::as_object)
            .and_then(|last_message| last_message.get("content"));
        span.safe_set_attribute(
            INPUT_VALUE,
            &last_content.cloned().unwrap_or(OtelModelValue::Null(())),
        );
        for (index, message) in messages.iter().enumerate() {
            let Some(fields) = message.as_object() else {
                continue;
            };
            let prefix = format!("{LLM_INPUT_MESSAGES}.{index}");
            let role = fields
                .get("role")
                .cloned()
                .unwrap_or(OtelModelValue::Null(()));
            span.safe_set_attribute(&format!("{prefix}.{MESSAGE_ROLE}"), &role);
            let content = fields
                .get("content")
                .cloned()
                .unwrap_or(OtelModelValue::Null(()));
            span.safe_set_attribute(&format!("{prefix}.{MESSAGE_CONTENT}"), &content);
            emit_input_message_extras(span, &prefix, fields);
        }
    }
}

fn emit_input_message_extras<S>(span: &mut S, prefix: &str, message: &MetadataMap)
where
    S: Span,
{
    let name = message
        .get("name")
        .and_then(OtelModelValue::as_str)
        .filter(|name| !name.is_empty());
    if let Some(found_name) = name {
        span.safe_set_attribute(
            &format!("{prefix}.{MESSAGE_NAME}"),
            &OtelModelValue::String(found_name.to_owned()),
        );
    }
    let tool_call_id = message
        .get("tool_call_id")
        .and_then(OtelModelValue::as_str)
        .filter(|tool_call_id| !tool_call_id.is_empty());
    if let Some(found_id) = tool_call_id {
        span.safe_set_attribute(
            &format!("{prefix}.{MESSAGE_TOOL_CALL_ID}"),
            &OtelModelValue::String(found_id.to_owned()),
        );
    }
    tool_calls::emit_message_tool_calls(span, prefix, message);
    emit_message_contents(span, prefix, message);
}

fn emit_message_contents<S>(span: &mut S, prefix: &str, message: &MetadataMap)
where
    S: Span,
{
    let Some(parts) = message.get("content").and_then(OtelModelValue::as_array) else {
        return;
    };
    let contents_prefix = format!("{prefix}.{MESSAGE_CONTENTS}");
    for (part_index, part) in parts.iter().enumerate() {
        let Some(fields) = part.as_object() else {
            continue;
        };
        let part_prefix = format!("{contents_prefix}.{part_index}");
        match fields.get("type").and_then(OtelModelValue::as_str) {
            Some("text" | "input_text") => emit_text_part(span, &part_prefix, fields),
            Some("image_url" | "image" | "input_image") => {
                emit_image_part(span, &part_prefix, fields);
            }
            _ => {}
        }
    }
}

fn emit_text_part<S>(span: &mut S, part_prefix: &str, part: &MetadataMap)
where
    S: Span,
{
    let Some(text) = part.get("text").and_then(OtelModelValue::as_str) else {
        return;
    };
    span.safe_set_attribute(
        &format!("{part_prefix}.{MESSAGE_CONTENT_TYPE}"),
        &OtelModelValue::String("text".to_owned()),
    );
    span.safe_set_attribute(
        &format!("{part_prefix}.{MESSAGE_CONTENT_TEXT}"),
        &OtelModelValue::String(text.to_owned()),
    );
}

fn emit_image_part<S>(span: &mut S, part_prefix: &str, part: &MetadataMap)
where
    S: Span,
{
    let Some(url) = image_url(part) else {
        return;
    };
    span.safe_set_attribute(
        &format!("{part_prefix}.{MESSAGE_CONTENT_TYPE}"),
        &OtelModelValue::String("image".to_owned()),
    );
    span.safe_set_attribute(
        &format!("{part_prefix}.{MESSAGE_CONTENT_IMAGE_URL}"),
        &OtelModelValue::String(url),
    );
}

fn image_url(part: &MetadataMap) -> Option<String> {
    let direct = match part.get("image_url") {
        Some(OtelModelValue::String(text)) => Some(text.clone()),
        Some(OtelModelValue::Object(image)) => image
            .get("url")
            .and_then(OtelModelValue::as_str)
            .map(str::to_owned),
        Some(
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::Array(_),
        )
        | None => None,
    };
    direct
        .filter(|url| !url.is_empty())
        .or_else(|| source_data_url(part))
        .or_else(|| {
            part.get("url")
                .and_then(OtelModelValue::as_str)
                .map(str::to_owned)
        })
        .filter(|url| !url.is_empty())
}

fn source_data_url(part: &MetadataMap) -> Option<String> {
    let source = part.get("source").and_then(OtelModelValue::as_object)?;
    let data = source
        .get("data")
        .and_then(OtelModelValue::as_str)
        .filter(|data| !data.is_empty())?;
    let media_type = source
        .get("media_type")
        .and_then(OtelModelValue::as_str)
        .unwrap_or("image/jpeg");
    Some(format!("data:{media_type};base64,{data}"))
}

pub fn set_response_attributes<S>(span: &mut S, response: &OtelModelValue)
where
    S: Span,
{
    let Some(fields) = response.as_object() else {
        return;
    };
    output::set_choice_outputs(span, fields);
    output::set_image_outputs(span, fields);
    output::set_audio_outputs(span, fields);
    output::set_embedding_outputs(span, fields);
    output::set_structured_outputs(span, fields);
    usage::set_usage_outputs(span, fields);
}
