use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use opentelemetry::trace::Span;

use super::keys;
use super::tool_calls;

pub(super) fn set_choice_outputs<S>(span: &mut S, response: &MetadataMap)
where
    S: Span,
{
    let empty = MetadataMap::new();
    let choices = response
        .get("choices")
        .and_then(OtelModelValue::as_array)
        .unwrap_or_default();
    for (index, choice) in choices.iter().enumerate() {
        let message = choice
            .as_object()
            .and_then(|choice| choice.get("message"))
            .and_then(OtelModelValue::as_object)
            .unwrap_or(&empty);
        let output_value = choice_output_value(message);
        span.safe_set_attribute(keys::OUTPUT_VALUE, &output_value);
        let prefix = format!("{}.{index}", keys::LLM_OUTPUT_MESSAGES);
        let role = message
            .get("role")
            .cloned()
            .unwrap_or(OtelModelValue::Null(()));
        span.safe_set_attribute(&format!("{prefix}.{}", keys::MESSAGE_ROLE), &role);
        let content = message
            .get("content")
            .cloned()
            .unwrap_or(OtelModelValue::Null(()));
        span.safe_set_attribute(&format!("{prefix}.{}", keys::MESSAGE_CONTENT), &content);
        tool_calls::emit_message_tool_calls(span, &prefix, message);
    }
}

fn choice_output_value(message: &MetadataMap) -> OtelModelValue {
    let content = message.get("content");
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
            .get("tool_calls")
            .and_then(OtelModelValue::as_array)
            .filter(|calls| !calls.is_empty())
            .map(tool_calls::summarize_tool_calls);
        if let Some(summary) = summary {
            return OtelModelValue::String(summary);
        }
    }
    content.cloned().unwrap_or(OtelModelValue::Null(()))
}

pub(super) fn set_image_outputs<S>(span: &mut S, response: &MetadataMap)
where
    S: Span,
{
    let Some(images) = response.get("data").and_then(OtelModelValue::as_array) else {
        return;
    };
    for (index, image) in images.iter().enumerate() {
        let Some(fields) = image.as_object() else {
            continue;
        };
        let url = match fields.get("url").and_then(OtelModelValue::as_str) {
            Some(url) => Some(url.to_owned()),
            None => fields
                .get("b64_json")
                .and_then(OtelModelValue::as_str)
                .filter(|encoded| !encoded.is_empty())
                .map(|encoded| format!("data:image/png;base64,{encoded}")),
        };
        let Some(url) = url.filter(|url| !url.is_empty()) else {
            continue;
        };
        if index == 0 {
            span.safe_set_attribute(keys::OUTPUT_VALUE, &OtelModelValue::String(url.clone()));
        }
        span.safe_set_attribute(
            &format!("{}.{index}", keys::IMAGE_URL),
            &OtelModelValue::String(url),
        );
    }
}

pub(super) fn set_audio_outputs<S>(span: &mut S, response: &MetadataMap)
where
    S: Span,
{
    let Some(audio_items) = response.get("audio").and_then(OtelModelValue::as_array) else {
        return;
    };
    for (index, item) in audio_items.iter().enumerate() {
        let Some(fields) = item.as_object() else {
            continue;
        };
        let url = match fields.get("url").and_then(OtelModelValue::as_str) {
            Some(url) => Some(url.to_owned()),
            None => fields
                .get("b64_json")
                .and_then(OtelModelValue::as_str)
                .filter(|encoded| !encoded.is_empty())
                .map(|encoded| format!("data:audio/wav;base64,{encoded}")),
        };
        if let Some(url) = url.filter(|url| !url.is_empty()) {
            if index == 0 {
                span.safe_set_attribute(keys::OUTPUT_VALUE, &OtelModelValue::String(url.clone()));
            }
            span.safe_set_attribute(
                &format!("{}.{index}", keys::AUDIO_URL),
                &OtelModelValue::String(url),
            );
        }
        if let Some(mime_type) = fields
            .get("mime_type")
            .and_then(OtelModelValue::as_str)
            .filter(|mime_type| !mime_type.is_empty())
        {
            span.safe_set_attribute(
                &format!("{}.{index}", keys::AUDIO_MIME_TYPE),
                &OtelModelValue::String(mime_type.to_owned()),
            );
        }
        if let Some(transcript) = fields
            .get("transcript")
            .and_then(OtelModelValue::as_str)
            .filter(|transcript| !transcript.is_empty())
        {
            span.safe_set_attribute(
                &format!("{}.{index}", keys::AUDIO_TRANSCRIPT),
                &OtelModelValue::String(transcript.to_owned()),
            );
        }
    }
}

pub(super) fn set_embedding_outputs<S>(span: &mut S, response: &MetadataMap)
where
    S: Span,
{
    let Some(items) = response.get("data").and_then(OtelModelValue::as_array) else {
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let Some(fields) = item.as_object() else {
            continue;
        };
        let vector = fields
            .get("embedding")
            .and_then(OtelModelValue::as_array)
            .filter(|values| !values.is_empty());
        if let Some(vector) = vector {
            let text = vector_text(vector);
            if index == 0 {
                span.safe_set_attribute(keys::OUTPUT_VALUE, &OtelModelValue::String(text.clone()));
            }
            span.safe_set_attribute(
                &format!("{}.{index}", keys::EMBEDDING_VECTOR),
                &OtelModelValue::String(text),
            );
        }
        if let Some(text) = fields
            .get("text")
            .and_then(OtelModelValue::as_str)
            .filter(|text| !text.is_empty())
        {
            span.safe_set_attribute(
                &format!("{}.{index}", keys::EMBEDDING_TEXT),
                &OtelModelValue::String(text.to_owned()),
            );
        }
    }
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

pub(super) fn set_structured_outputs<S>(span: &mut S, response: &MetadataMap)
where
    S: Span,
{
    let Some(items) = response.get("output").and_then(OtelModelValue::as_array) else {
        return;
    };
    for (index, item) in items.iter().enumerate() {
        let Some(fields) = item.as_object() else {
            continue;
        };
        let Some(item_type) = fields.get("type").and_then(OtelModelValue::as_str) else {
            continue;
        };
        let prefix = format!("{}.{index}", keys::LLM_OUTPUT_MESSAGES);
        match item_type {
            "reasoning" => emit_reasoning_summaries(span, &prefix, fields),
            "message" => emit_structured_message(span, &prefix, fields),
            _ => {}
        }
    }
}

fn emit_reasoning_summaries<S>(span: &mut S, prefix: &str, item: &MetadataMap)
where
    S: Span,
{
    let Some(summaries) = item.get("summary").and_then(OtelModelValue::as_array) else {
        return;
    };
    for summary in summaries {
        let Some(text) = summary.as_object().and_then(|summary| summary.get("text")) else {
            continue;
        };
        span.safe_set_attribute(
            &format!("{prefix}.{}", keys::MESSAGE_REASONING_SUMMARY),
            text,
        );
    }
}

fn emit_structured_message<S>(span: &mut S, prefix: &str, item: &MetadataMap)
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
    span.safe_set_attribute(keys::OUTPUT_VALUE, &text);
    span.safe_set_attribute(&format!("{prefix}.{}", keys::MESSAGE_CONTENT), &text);
    span.safe_set_attribute(&format!("{prefix}.{}", keys::MESSAGE_ROLE), &role);
}
