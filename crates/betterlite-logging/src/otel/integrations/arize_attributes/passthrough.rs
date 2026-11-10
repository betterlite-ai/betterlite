use crate::otel::integrations::arize_messages::keys::{
    INPUT_VALUE, LLM_INPUT_MESSAGES, LLM_OUTPUT_MESSAGES, MESSAGE_CONTENT, MESSAGE_ROLE,
    OUTPUT_VALUE,
};
use crate::otel::integrations::arize_messages::text::coerce_text;
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use opentelemetry::trace::Span;

use super::truthy::is_truthy;

// passthrough is the spelling real call types use: allm_passthrough_route
const PASSTHROUGH_KEYWORDS: [&str; 2] = ["passthrough", "pass_through"];
const OUTPUT_MESSAGE_INDEX: &str = "0";

pub(super) fn maybe_normalize_passthrough<S>(
    span: &mut S,
    kwargs: &MetadataMap,
    response: &OtelModelValue,
    standard_logging_object: Option<&MetadataMap>,
    redact_messages: bool,
) where
    S: Span,
{
    let Some(call_type) = standard_logging_object
        .and_then(|payload| payload.get("call_type"))
        .and_then(OtelModelValue::as_str)
    else {
        return;
    };
    let lowered = call_type.to_lowercase();
    if !PASSTHROUGH_KEYWORDS
        .iter()
        .any(|keyword| lowered.contains(keyword))
    {
        return;
    }
    if redact_messages {
        return;
    }
    set_input_attributes(span, kwargs);
    let Some(parsed) = parse_passthrough_response(response, kwargs) else {
        return;
    };
    set_output_attributes(span, &parsed);
}

fn set_input_attributes<S>(span: &mut S, kwargs: &MetadataMap)
where
    S: Span,
{
    let Some(messages) = passthrough_messages(kwargs) else {
        return;
    };
    if let Some(text) = last_message_text(messages) {
        span.safe_set_attribute(INPUT_VALUE, &OtelModelValue::String(text));
    }
    for (index, message) in messages.iter().enumerate() {
        let Some(fields) = message.as_object() else {
            continue;
        };
        write_input_message(span, index, fields);
    }
}

fn passthrough_messages(kwargs: &MetadataMap) -> Option<&[OtelModelValue]> {
    kwargs
        .get("additional_args")
        .and_then(OtelModelValue::as_object)
        .and_then(|args| args.get("complete_input_dict"))
        .and_then(OtelModelValue::as_object)
        .and_then(|input| input.get("messages"))
        .and_then(OtelModelValue::as_array)
        .filter(|messages| !messages.is_empty())
}

fn last_message_text(messages: &[OtelModelValue]) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter_map(OtelModelValue::as_object)
        .find_map(|message| {
            message
                .get("content")
                .and_then(coerce_text)
                .filter(|text| !text.is_empty())
        })
}

fn write_input_message<S>(span: &mut S, index: usize, fields: &MetadataMap)
where
    S: Span,
{
    let prefix = format!("{LLM_INPUT_MESSAGES}.{index}");
    if let Some(role) = fields.get("role").filter(|value| is_truthy(value)) {
        span.safe_set_attribute(&format!("{prefix}.{MESSAGE_ROLE}"), role);
    }
    if let Some(text) = fields.get("content").and_then(coerce_text) {
        span.safe_set_attribute(
            &format!("{prefix}.{MESSAGE_CONTENT}"),
            &OtelModelValue::String(text),
        );
    }
}

fn parse_passthrough_response(
    response: &OtelModelValue,
    kwargs: &MetadataMap,
) -> Option<MetadataMap> {
    if let Some(fields) = response.as_object()
        && let Some(answer) = response_candidate(fields)
    {
        return Some(answer);
    }
    match kwargs.get("original_response") {
        Some(OtelModelValue::Object(fields)) => Some(fields.clone()),
        Some(OtelModelValue::String(text)) => parse_object(text),
        _ => None,
    }
}

fn response_candidate(fields: &MetadataMap) -> Option<MetadataMap> {
    let wrapped = fields.contains_key("response")
        && !fields.contains_key("content")
        && !fields.contains_key("choices");
    if !wrapped {
        return Some(fields.clone());
    }
    match fields.get("response") {
        Some(OtelModelValue::Object(inner)) => Some(inner.clone()),
        Some(OtelModelValue::String(text)) => parse_object(text),
        _ => None,
    }
}

fn parse_object(text: &str) -> Option<MetadataMap> {
    match serde_json::from_str::<OtelModelValue>(text) {
        Ok(OtelModelValue::Object(fields)) => Some(fields),
        Ok(
            OtelModelValue::Null(())
            | OtelModelValue::Bool(_)
            | OtelModelValue::Number(_)
            | OtelModelValue::String(_)
            | OtelModelValue::Array(_),
        )
        | Err(_) => None,
    }
}

fn set_output_attributes<S>(span: &mut S, response: &MetadataMap)
where
    S: Span,
{
    let joined = content_text(response);
    if !joined.is_empty() {
        write_message_output(span, &joined, response.get("role"));
    }
    let Some(message) = choice_message(response) else {
        return;
    };
    let Some(text) = message
        .get("content")
        .and_then(coerce_text)
        .filter(|text| !text.is_empty())
    else {
        return;
    };
    write_message_output(span, &text, message.get("role"));
}

fn content_text(response: &MetadataMap) -> String {
    let Some(parts) = response.get("content").and_then(OtelModelValue::as_array) else {
        return String::new();
    };
    parts
        .iter()
        .filter_map(|part| part.as_object()?.get("text")?.as_str())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn choice_message(response: &MetadataMap) -> Option<&MetadataMap> {
    response
        .get("choices")
        .and_then(OtelModelValue::as_array)
        .and_then(|choices| choices.first())
        .and_then(OtelModelValue::as_object)
        .and_then(|choice| choice.get("message"))
        .and_then(OtelModelValue::as_object)
}

fn write_message_output<S>(span: &mut S, text: &str, role: Option<&OtelModelValue>)
where
    S: Span,
{
    let prefix = format!("{LLM_OUTPUT_MESSAGES}.{OUTPUT_MESSAGE_INDEX}");
    let message_role = role
        .cloned()
        .unwrap_or_else(|| OtelModelValue::String("assistant".to_owned()));
    span.safe_set_attribute(OUTPUT_VALUE, &OtelModelValue::String(text.to_owned()));
    span.safe_set_attribute(&format!("{prefix}.{MESSAGE_ROLE}"), &message_role);
    span.safe_set_attribute(
        &format!("{prefix}.{MESSAGE_CONTENT}"),
        &OtelModelValue::String(text.to_owned()),
    );
}
