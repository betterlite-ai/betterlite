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
    write_details(span, usage);
}

fn write_completion_and_prompt<S>(span: &mut S, usage: &MetadataMap)
where
    S: Span,
{
    let completion =
        nonzero(usage.get("completion_tokens")).or_else(|| nonzero(usage.get("output_tokens")));
    write_count(span, keys::LLM_TOKEN_COUNT_COMPLETION, completion);
    let prompt = nonzero(usage.get("prompt_tokens")).or_else(|| nonzero(usage.get("input_tokens")));
    write_count(span, keys::LLM_TOKEN_COUNT_PROMPT, prompt);
}

fn write_details<S>(span: &mut S, usage: &MetadataMap)
where
    S: Span,
{
    let completion = pick_details(
        usage
            .get("completion_tokens_details")
            .and_then(OtelModelValue::as_object),
        usage
            .get("output_tokens_details")
            .and_then(OtelModelValue::as_object),
    );
    let reasoning = completion.and_then(|fields| nonzero(fields.get("reasoning_tokens")));
    write_count(span, keys::TOKEN_COUNT_REASONING, reasoning);
    let prompt = pick_details(
        usage
            .get("prompt_tokens_details")
            .and_then(OtelModelValue::as_object),
        usage
            .get("input_tokens_details")
            .and_then(OtelModelValue::as_object),
    );
    let cached = prompt.and_then(|fields| nonzero(fields.get("cached_tokens")));
    let cache_read = cached.or_else(|| nonzero(usage.get("cache_read_input_tokens")));
    write_count(span, keys::TOKEN_COUNT_CACHE_READ, cache_read);
    write_count(
        span,
        keys::TOKEN_COUNT_CACHE_WRITE,
        nonzero(usage.get("cache_creation_input_tokens")),
    );
    let audio = prompt.and_then(|fields| nonzero(fields.get("audio_tokens")));
    write_count(span, keys::TOKEN_COUNT_AUDIO, audio);
}

fn write_count<S>(span: &mut S, key: &str, value: Option<&OtelModelValue>)
where
    S: Span,
{
    if let Some(found) = value {
        span.safe_set_attribute(key, found);
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
