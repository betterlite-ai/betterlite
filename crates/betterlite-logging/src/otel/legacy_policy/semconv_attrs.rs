use std::collections::BTreeMap;
use std::slice;

use opentelemetry::trace::Span;
use opentelemetry::{Array, KeyValue, StringValue, Value};

use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::vocabulary::constants::attribute_keys::gen_ai;
use crate::otel::vocabulary::semconv::OTELSemconvCategory;

mod tests;

pub const REQUEST_STREAM: &str = "gen_ai.request.stream";

#[must_use]
pub fn gen_ai_operation_name(call_type: Option<&str>) -> &'static str {
    match call_type.unwrap_or_default() {
        name if name.contains("embedding") => "embeddings",
        name if name.contains("text_completion") => "text_completion",
        _ => "chat",
    }
}

#[must_use]
pub fn semconv_latest_experimental(config: &OpenTelemetryConfig) -> bool {
    config
        .semconv_stability_opt_in
        .contains(&OTELSemconvCategory::GenAiLatestExperimental)
}

pub fn set_semconv_request_attributes<S>(
    span: &mut S,
    optional_params: &BTreeMap<String, OtelModelValue>,
) where
    S: Span,
{
    for (source, target) in [
        ("frequency_penalty", gen_ai::REQUEST_FREQUENCY_PENALTY),
        ("presence_penalty", gen_ai::REQUEST_PRESENCE_PENALTY),
        ("top_k", gen_ai::REQUEST_TOP_K),
        ("seed", gen_ai::REQUEST_SEED),
    ] {
        if let Some(value) = optional_params
            .get(source)
            .filter(|value| !matches!(value, OtelModelValue::Null(())))
        {
            span.safe_set_attribute(target, value);
        }
    }
    if let Some(stop) = optional_params
        .get("stop")
        .filter(|value| !matches!(value, OtelModelValue::Null(())))
    {
        write_stop_sequences(span, stop);
    }
    if optional_params
        .get("stream")
        .and_then(OtelModelValue::as_bool)
        == Some(true)
    {
        span.safe_set_attribute(REQUEST_STREAM, &OtelModelValue::Bool(true));
    }
    write_choice_count(span, optional_params);
}

fn write_stop_sequences<S>(span: &mut S, stop: &OtelModelValue)
where
    S: Span,
{
    let items = stop
        .as_array()
        .map_or_else(|| slice::from_ref(stop), |items| items);
    let sequences: Vec<StringValue> = items
        .iter()
        .map(|item| StringValue::from(stop_text(item)))
        .collect();
    span.set_attribute(KeyValue::new(
        gen_ai::REQUEST_STOP_SEQUENCES,
        Value::Array(Array::from(sequences)),
    ));
}

fn write_choice_count<S>(span: &mut S, optional_params: &BTreeMap<String, OtelModelValue>)
where
    S: Span,
{
    let choice_count = optional_params.get("n").and_then(integer);
    if let Some(count) = choice_count.filter(|count| *count > 1) {
        span.safe_set_attribute(
            gen_ai::REQUEST_CHOICE_COUNT,
            &OtelModelValue::Number(serde_json::Number::from(count)),
        );
    }
}

pub fn set_cache_token_attributes<S>(span: &mut S, payload: &BTreeMap<String, OtelModelValue>)
where
    S: Span,
{
    let Some(usage_object) = payload
        .get("metadata")
        .and_then(OtelModelValue::as_object)
        .and_then(|metadata| metadata.get("usage_object"))
        .and_then(OtelModelValue::as_object)
    else {
        return;
    };
    for (source, target) in [
        (
            "cache_creation_input_tokens",
            gen_ai::USAGE_CACHE_CREATION_TOKENS,
        ),
        (
            "cache_read_input_tokens",
            gen_ai::USAGE_CACHE_READ_INPUT_TOKENS,
        ),
    ] {
        if let Some(value) = usage_object
            .get(source)
            .filter(|value| value.as_decimal().is_some_and(|number| !number.is_zero()))
        {
            span.safe_set_attribute(target, value);
        }
    }
}

fn integer(value: &OtelModelValue) -> Option<i64> {
    if let OtelModelValue::Number(number) = value {
        return number.as_i64();
    }
    None
}

fn stop_text(value: &OtelModelValue) -> String {
    match value {
        OtelModelValue::String(text) => text.clone(),
        OtelModelValue::Bool(true) => "True".to_owned(),
        OtelModelValue::Bool(false) => "False".to_owned(),
        OtelModelValue::Number(number) => number.to_string(),
        OtelModelValue::Null(()) => "None".to_owned(),
        OtelModelValue::Array(_) | OtelModelValue::Object(_) => {
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}
