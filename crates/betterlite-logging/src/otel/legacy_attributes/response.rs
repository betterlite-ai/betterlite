use opentelemetry::trace::Span;

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::transforms::tools::tool_calls_kv_pair;
use crate::otel::transforms::{transform_choices, transform_responses_output};
use crate::otel::vocabulary::constants::attribute_keys::gen_ai;
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::context::truthiness;

mod tests;

pub fn set_response_attributes<S>(span: &mut S, response: Option<&OtelModelValue>)
where
    S: Span,
{
    let Some(present) = response else {
        return;
    };
    let Some(fields) = present.as_object() else {
        return;
    };
    if let Some(choices) = fields.get("choices").filter(|value| truthiness(value)) {
        write_choices(span, choices);
        return;
    }
    if let Some(output) = fields.get("output").filter(|value| truthiness(value)) {
        write_responses_output(span, fields, output);
    }
}

fn write_choices<S>(span: &mut S, choices: &OtelModelValue)
where
    S: Span,
{
    let Some(items) = choices.as_array() else {
        return;
    };
    write_serialized(span, gen_ai::OUTPUT_MESSAGES, &transform_choices(items));
    let finish_reasons: Vec<OtelModelValue> =
        items.iter().filter_map(choice_finish_reason).collect();
    if !finish_reasons.is_empty() {
        write_serialized(span, gen_ai::RESPONSE_FINISH_REASONS, &finish_reasons);
    }
    for choice in items {
        write_choice_tool_calls(span, choice);
    }
}

fn choice_finish_reason(choice: &OtelModelValue) -> Option<OtelModelValue> {
    choice
        .as_object()?
        .get("finish_reason")
        .filter(|value| truthiness(value))
        .cloned()
}

fn write_choice_tool_calls<S>(span: &mut S, choice: &OtelModelValue)
where
    S: Span,
{
    let Some(fields) = choice.as_object() else {
        return;
    };
    if !fields.get("finish_reason").is_some_and(truthiness) {
        return;
    }
    let declared = fields
        .get("message")
        .and_then(OtelModelValue::as_object)
        .and_then(|message| message.get("tool_calls"))
        .filter(|value| truthiness(value))
        .and_then(OtelModelValue::as_array);
    let Some(tool_calls) = declared else {
        return;
    };
    for (key, value) in tool_calls_kv_pair(tool_calls) {
        span.safe_set_attribute(&key, &value);
    }
}

fn write_responses_output<S>(span: &mut S, fields: &MetadataMap, output: &OtelModelValue)
where
    S: Span,
{
    let Some(items) = output.as_array() else {
        return;
    };
    let messages = transform_responses_output(items);
    if !messages.is_empty() {
        write_serialized(span, gen_ai::OUTPUT_MESSAGES, &messages);
    }
    let tool_calls = function_call_tool_calls(items);
    if !tool_calls.is_empty() {
        for (key, value) in tool_calls_kv_pair(&tool_calls) {
            span.safe_set_attribute(&key, &value);
        }
    }
    let status = fields.get("status").filter(|value| truthiness(value));
    if let Some(found) = status {
        write_serialized(span, gen_ai::RESPONSE_FINISH_REASONS, &[found]);
    }
}

fn function_call_tool_calls(items: &[OtelModelValue]) -> Vec<OtelModelValue> {
    let mut tool_calls = Vec::new();
    for item in items {
        let Some(fields) = item.as_object() else {
            continue;
        };
        if fields.get("type").and_then(OtelModelValue::as_str) != Some("function_call") {
            continue;
        }
        let mut function = MetadataMap::new();
        for key in ["arguments", "name"] {
            let value = fields
                .get(key)
                .cloned()
                .unwrap_or_else(|| OtelModelValue::String(String::new()));
            function.insert(key.to_owned(), value);
        }
        tool_calls.push(OtelModelValue::Object(MetadataMap::from([(
            "function".to_owned(),
            OtelModelValue::Object(function),
        )])));
    }
    tool_calls
}

fn write_serialized<S, T>(span: &mut S, key: &str, value: &T)
where
    S: Span,
    T: serde::Serialize,
{
    if let Ok(text) = safe_dumps(value, DEFAULT_MAX_RECURSE_DEPTH, None) {
        span.safe_set_attribute(key, &OtelModelValue::String(text));
    }
}
