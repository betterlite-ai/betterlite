use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use opentelemetry::trace::Span;

use super::keys;
use super::tool_calls;

fn for_each_entry<S, F>(span: &mut S, response: &MetadataMap, field: &str, mut visit: F)
where
    S: Span,
    F: FnMut(&mut S, &MetadataMap, usize),
{
    let Some(entries) = response.get(field).and_then(OtelModelValue::as_array) else {
        return;
    };
    for (index, entry) in entries.iter().enumerate() {
        let Some(fields) = entry.as_object() else {
            continue;
        };
        visit(span, fields, index);
    }
}

pub(super) fn set_output_attributes<S>(span: &mut S, response: &MetadataMap)
where
    S: Span,
{
    for_each_entry(span, response, "choices", write_choice_entry);
    for_each_entry(span, response, "data", |target, fields, index| {
        write_media_value(target, index, image_url(fields), keys::IMAGE_URL);
    });
    for_each_entry(span, response, "audio", write_audio_entry);
    for_each_entry(span, response, "data", write_embedding_entry);
    for_each_entry(span, response, "output", write_structured_entry);
}

fn write_choice_entry<S>(span: &mut S, fields: &MetadataMap, index: usize)
where
    S: Span,
{
    let message = fields.get("message").and_then(OtelModelValue::as_object);
    span.safe_set_attribute(keys::OUTPUT_VALUE, &choice_output_value(message));
    let prefix = format!("{}.{index}", keys::LLM_OUTPUT_MESSAGES);
    let role = message
        .and_then(|message_fields| message_fields.get("role"))
        .cloned()
        .unwrap_or(OtelModelValue::Null(()));
    span.safe_set_attribute(&format!("{prefix}.{}", keys::MESSAGE_ROLE), &role);
    let content = message
        .and_then(|message_fields| message_fields.get("content"))
        .cloned()
        .unwrap_or(OtelModelValue::Null(()));
    span.safe_set_attribute(&format!("{prefix}.{}", keys::MESSAGE_CONTENT), &content);
    if let Some(message_fields) = message {
        tool_calls::emit_message_tool_calls(span, &prefix, message_fields);
    }
}

fn choice_output_value(message: Option<&MetadataMap>) -> OtelModelValue {
    let content = message.and_then(|message_fields| message_fields.get("content"));
    let blank = match content {
        None | Some(OtelModelValue::Null(())) => true,
        Some(OtelModelValue::String(text)) => text.is_empty(),
        Some(
            OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::Array(_)
            | OtelModelValue::Object(_),
        ) => false,
    };
    if blank {
        let summary = message
            .and_then(|message_fields| message_fields.get("tool_calls"))
            .and_then(OtelModelValue::as_array)
            .filter(|calls| !calls.is_empty())
            .map(tool_calls::summarize_tool_calls);
        if let Some(summary_text) = summary {
            return OtelModelValue::String(summary_text);
        }
    }
    content.cloned().unwrap_or(OtelModelValue::Null(()))
}

pub(super) fn image_url(part: &MetadataMap) -> Option<String> {
    inline_image_url(part)
        .filter(|url| !url.is_empty())
        .or_else(|| source_data_url(part))
        .or_else(|| part_url(part))
}

fn part_url(part: &MetadataMap) -> Option<String> {
    part.get("url")
        .and_then(OtelModelValue::as_str)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
}

fn inline_image_url(part: &MetadataMap) -> Option<String> {
    match part.get("image_url") {
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
    }
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

fn write_audio_entry<S>(span: &mut S, fields: &MetadataMap, index: usize)
where
    S: Span,
{
    let url = data_url(fields, "data:audio/wav;base64,");
    write_media_value(span, index, url, keys::AUDIO_URL);
    let text_fields = [
        ("mime_type", keys::AUDIO_MIME_TYPE),
        ("transcript", keys::AUDIO_TRANSCRIPT),
    ];
    for (field, key) in text_fields {
        write_nonempty_text(span, fields, field, key, index);
    }
}

fn write_media_value<S>(span: &mut S, index: usize, text: Option<String>, key: &str)
where
    S: Span,
{
    if let Some(found) = text {
        let value = OtelModelValue::String(found);
        if index == 0 {
            span.safe_set_attribute(keys::OUTPUT_VALUE, &value);
        }
        span.safe_set_attribute(&format!("{key}.{index}"), &value);
    }
}

fn write_nonempty_text<S>(span: &mut S, fields: &MetadataMap, field: &str, key: &str, index: usize)
where
    S: Span,
{
    let value = fields
        .get(field)
        .and_then(OtelModelValue::as_str)
        .filter(|text| !text.is_empty());
    if let Some(text) = value {
        span.safe_set_attribute(
            &format!("{key}.{index}"),
            &OtelModelValue::String(text.to_owned()),
        );
    }
}

fn data_url(fields: &MetadataMap, prefix: &str) -> Option<String> {
    let direct = fields
        .get("url")
        .and_then(OtelModelValue::as_str)
        .map(str::to_owned);
    let encoded = fields
        .get("b64_json")
        .and_then(OtelModelValue::as_str)
        .filter(|value| !value.is_empty());
    direct
        .or_else(|| encoded.map(|base64| format!("{prefix}{base64}")))
        .filter(|candidate| !candidate.is_empty())
}

fn write_embedding_entry<S>(span: &mut S, fields: &MetadataMap, index: usize)
where
    S: Span,
{
    write_media_value(span, index, vector_value(fields), keys::EMBEDDING_VECTOR);
    write_nonempty_text(span, fields, "text", keys::EMBEDDING_TEXT, index);
}

fn vector_value(fields: &MetadataMap) -> Option<String> {
    fields
        .get("embedding")
        .and_then(OtelModelValue::as_array)
        .filter(|values| !values.is_empty())
        .map(vector_text)
}

fn vector_text(values: &[OtelModelValue]) -> String {
    let parts = values
        .iter()
        .map(|value| match value {
            OtelModelValue::Number(number) => number.to_string(),
            OtelModelValue::Bool(flag) => flag.to_string(),
            OtelModelValue::String(text) => serde_json::to_string(text).unwrap_or_default(),
            OtelModelValue::Null(()) | OtelModelValue::Array(_) | OtelModelValue::Object(_) => {
                serde_json::to_string(value).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(", "))
}

fn write_structured_entry<S>(span: &mut S, fields: &MetadataMap, index: usize)
where
    S: Span,
{
    match fields.get("type").and_then(OtelModelValue::as_str) {
        Some("reasoning") => emit_reasoning_summaries(span, index, fields),
        Some("message") => emit_structured_message(span, index, fields),
        _ => {}
    }
}

fn emit_reasoning_summaries<S>(span: &mut S, index: usize, item: &MetadataMap)
where
    S: Span,
{
    let Some(summaries) = item.get("summary").and_then(OtelModelValue::as_array) else {
        return;
    };
    let prefix = format!("{}.{index}", keys::LLM_OUTPUT_MESSAGES);
    for summary in summaries {
        let text = summary.as_object().and_then(|fields| fields.get("text"));
        if let Some(found_text) = text {
            span.safe_set_attribute(
                &format!("{prefix}.{}", keys::MESSAGE_REASONING_SUMMARY),
                found_text,
            );
        }
    }
}

fn emit_structured_message<S>(span: &mut S, index: usize, item: &MetadataMap)
where
    S: Span,
{
    let Some(content) = item.get("content") else {
        return;
    };
    let text = content
        .as_array()
        .and_then(|parts| parts.first())
        .and_then(OtelModelValue::as_object)
        .and_then(|part| part.get("text"))
        .cloned()
        .unwrap_or(OtelModelValue::Null(()));
    let role = item
        .get("role")
        .cloned()
        .unwrap_or_else(|| OtelModelValue::String("assistant".to_owned()));
    let prefix = format!("{}.{index}", keys::LLM_OUTPUT_MESSAGES);
    span.safe_set_attribute(keys::OUTPUT_VALUE, &text);
    span.safe_set_attribute(&format!("{prefix}.{}", keys::MESSAGE_CONTENT), &text);
    span.safe_set_attribute(&format!("{prefix}.{}", keys::MESSAGE_ROLE), &role);
}
