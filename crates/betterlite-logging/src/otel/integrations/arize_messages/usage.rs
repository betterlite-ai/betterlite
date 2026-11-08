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
        Some(usage) if !usage.is_empty() => usage,
        Some(_) | None => return,
    };
    let total = usage
        .get("total_tokens")
        .cloned()
        .unwrap_or(OtelModelValue::Null(()));
    span.safe_set_attribute(keys::LLM_TOKEN_COUNT_TOTAL, &total);
    if let Some(value) =
        nonzero(usage.get("completion_tokens")).or_else(|| nonzero(usage.get("output_tokens")))
    {
        span.safe_set_attribute(keys::LLM_TOKEN_COUNT_COMPLETION, value);
    }
    if let Some(value) =
        nonzero(usage.get("prompt_tokens")).or_else(|| nonzero(usage.get("input_tokens")))
    {
        span.safe_set_attribute(keys::LLM_TOKEN_COUNT_PROMPT, value);
    }
    let completion_details = pick_details(
        usage
            .get("completion_tokens_details")
            .and_then(OtelModelValue::as_object),
        usage
            .get("output_tokens_details")
            .and_then(OtelModelValue::as_object),
    );
    if let Some(value) =
        completion_details.and_then(|details| nonzero(details.get("reasoning_tokens")))
    {
        span.safe_set_attribute(keys::TOKEN_COUNT_REASONING, value);
    }
    let prompt_details = pick_details(
        usage
            .get("prompt_tokens_details")
            .and_then(OtelModelValue::as_object),
        usage
            .get("input_tokens_details")
            .and_then(OtelModelValue::as_object),
    );
    let cache_read = prompt_details
        .and_then(|details| nonzero(details.get("cached_tokens")))
        .or_else(|| nonzero(usage.get("cache_read_input_tokens")));
    if let Some(value) = cache_read {
        span.safe_set_attribute(keys::TOKEN_COUNT_CACHE_READ, value);
    }
    if let Some(value) = nonzero(usage.get("cache_creation_input_tokens")) {
        span.safe_set_attribute(keys::TOKEN_COUNT_CACHE_WRITE, value);
    }
    if let Some(value) = prompt_details.and_then(|details| nonzero(details.get("audio_tokens"))) {
        span.safe_set_attribute(keys::TOKEN_COUNT_AUDIO, value);
    }
}

fn nonzero(value: Option<&OtelModelValue>) -> Option<&OtelModelValue> {
    match value {
        Some(OtelModelValue::Null(())) | None => None,
        Some(OtelModelValue::Number(number)) if number.as_f64() == Some(0.0) => None,
        Some(value) => Some(value),
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
