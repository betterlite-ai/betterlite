use std::collections::BTreeMap;

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use opentelemetry::trace::Span;

use super::keys;

pub(super) fn set_usage_outputs<S>(span: &mut S, response: &MetadataMap)
where
    S: Span,
{
    let usage = match response.get("usage").and_then(OtelModelValue::as_object) {
        Some(fields) if !fields.is_empty() => fields,
        Some(_) | None => return,
    };
    let total = usage
        .get("total_tokens")
        .cloned()
        .unwrap_or(OtelModelValue::Null(()));
    span.safe_set_attribute(keys::LLM_TOKEN_COUNT_TOTAL, &total);
    write_completion_and_prompt(span, usage);
    write_reasoning_tokens(span, usage);
    write_prompt_details(span, usage);
}

fn write_completion_and_prompt<S>(span: &mut S, usage: &MetadataMap)
where
    S: Span,
{
    let completion =
        nonzero(usage.get("completion_tokens")).or_else(|| nonzero(usage.get("output_tokens")));
    if let Some(value) = completion {
        span.safe_set_attribute(keys::LLM_TOKEN_COUNT_COMPLETION, value);
    }
    let prompt = nonzero(usage.get("prompt_tokens")).or_else(|| nonzero(usage.get("input_tokens")));
    if let Some(value) = prompt {
        span.safe_set_attribute(keys::LLM_TOKEN_COUNT_PROMPT, value);
    }
}

fn write_reasoning_tokens<S>(span: &mut S, usage: &MetadataMap)
where
    S: Span,
{
    let details = pick_details(
        usage
            .get("completion_tokens_details")
            .and_then(OtelModelValue::as_object),
        usage
            .get("output_tokens_details")
            .and_then(OtelModelValue::as_object),
    );
    let reasoning = details.and_then(|fields| nonzero(fields.get("reasoning_tokens")));
    if let Some(value) = reasoning {
        span.safe_set_attribute(keys::TOKEN_COUNT_REASONING, value);
    }
}

fn write_prompt_details<S>(span: &mut S, usage: &MetadataMap)
where
    S: Span,
{
    let details = pick_details(
        usage
            .get("prompt_tokens_details")
            .and_then(OtelModelValue::as_object),
        usage
            .get("input_tokens_details")
            .and_then(OtelModelValue::as_object),
    );
    let cache_read = details
        .and_then(|fields| nonzero(fields.get("cached_tokens")))
        .or_else(|| nonzero(usage.get("cache_read_input_tokens")));
    if let Some(value) = cache_read {
        span.safe_set_attribute(keys::TOKEN_COUNT_CACHE_READ, value);
    }
    let cache_write = nonzero(usage.get("cache_creation_input_tokens"));
    if let Some(value) = cache_write {
        span.safe_set_attribute(keys::TOKEN_COUNT_CACHE_WRITE, value);
    }
    let audio = details.and_then(|fields| nonzero(fields.get("audio_tokens")));
    if let Some(value) = audio {
        span.safe_set_attribute(keys::TOKEN_COUNT_AUDIO, value);
    }
}

fn nonzero(value: Option<&OtelModelValue>) -> Option<&OtelModelValue> {
    match value {
        Some(OtelModelValue::Null(())) | None => None,
        Some(OtelModelValue::Number(number)) if number.as_f64() == Some(0.0) => None,
        Some(found) => Some(found),
    }
}

fn pick_details<'usage>(
    primary: Option<&'usage BTreeMap<String, OtelModelValue>>,
    fallback: Option<&'usage BTreeMap<String, OtelModelValue>>,
) -> Option<&'usage BTreeMap<String, OtelModelValue>> {
    match primary {
        Some(details) if !details.is_empty() => Some(details),
        Some(_) | None => fallback,
    }
}
