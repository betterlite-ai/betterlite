use crate::otel::integrations::arize_messages::keys::{INPUT_VALUE, OUTPUT_VALUE};
use crate::otel::integrations::arize_messages::text::coerce_text;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use opentelemetry::trace::Span;

use super::keys;
use super::truthy::is_truthy;

const MCP_TOOL_CALL_TYPE: &str = "call_mcp_tool";

pub(super) fn maybe_set_mcp_tool_attrs<S>(
    span: &mut S,
    response: &OtelModelValue,
    standard_logging_object: Option<&MetadataMap>,
    redact_messages: bool,
) where
    S: Span,
{
    let Some(payload) = standard_logging_object else {
        return;
    };
    if payload.get("call_type").and_then(OtelModelValue::as_str) != Some(MCP_TOOL_CALL_TYPE) {
        return;
    }
    let Some(tool_call) = payload
        .get("metadata")
        .and_then(OtelModelValue::as_object)
        .and_then(|metadata| metadata.get("mcp_tool_call_metadata"))
        .and_then(OtelModelValue::as_object)
    else {
        return;
    };
    let tool_name = tool_call
        .get("name")
        .filter(|value| is_truthy(value))
        .or_else(|| {
            tool_call
                .get("namespaced_tool_name")
                .filter(|value| is_truthy(value))
        });
    if let Some(name) = tool_name {
        span.safe_set_attribute(keys::TOOL_NAME, name);
    }
    if redact_messages {
        return;
    }
    set_input_value(span, tool_call);
    set_mcp_tool_output(span, response);
}

fn set_input_value<S>(span: &mut S, tool_call: &MetadataMap)
where
    S: Span,
{
    if let Some(arguments) = tool_call
        .get("arguments")
        .filter(|value| !matches!(value, OtelModelValue::Null(())))
        && let Ok(text) = serde_json::to_string(arguments)
    {
        span.safe_set_attribute(INPUT_VALUE, &OtelModelValue::String(text));
        span.safe_set_attribute(
            keys::INPUT_MIME_TYPE,
            &OtelModelValue::String(keys::MIME_TYPE_JSON.to_owned()),
        );
    }
}

fn set_mcp_tool_output<S>(span: &mut S, response: &OtelModelValue)
where
    S: Span,
{
    let Some(fields) = response.as_object() else {
        return;
    };
    let content = fields
        .get("content")
        .filter(|value| !matches!(value, OtelModelValue::Null(())));
    let content_text = content.and_then(coerce_text);
    if let Some(found_text) = content_text.filter(|text| !text.is_empty())
        && has_only_text_parts(content)
    {
        write_output(span, &found_text, keys::MIME_TYPE_TEXT);
        return;
    }
    let structured = if fields.contains_key("structured_content") {
        fields.get("structured_content")
    } else {
        fields.get("structuredContent")
    };
    let structured_value = structured.filter(|value| !matches!(value, OtelModelValue::Null(())));
    let payload = match content {
        Some(value) if is_truthy(value) => Some(value),
        other => structured_value.or(other),
    };
    let Some(output_payload) = payload else {
        return;
    };
    if let Ok(output_text) = serde_json::to_string(output_payload) {
        write_output(span, &output_text, keys::MIME_TYPE_JSON);
    }
}

fn write_output<S>(span: &mut S, text: &str, mime_type: &str)
where
    S: Span,
{
    span.safe_set_attribute(OUTPUT_VALUE, &OtelModelValue::String(text.to_owned()));
    span.safe_set_attribute(
        keys::OUTPUT_MIME_TYPE,
        &OtelModelValue::String(mime_type.to_owned()),
    );
}

fn has_only_text_parts(content: Option<&OtelModelValue>) -> bool {
    let Some(parts) = content.and_then(OtelModelValue::as_array) else {
        return true;
    };
    parts
        .iter()
        .all(|part| coerce_text(&OtelModelValue::Array(vec![part.clone()])).is_some())
}
