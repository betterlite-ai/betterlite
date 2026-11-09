use opentelemetry::trace::Span;
use opentelemetry::{Array, KeyValue, StringValue, Value};

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

pub const LANGTRACE_DEFAULT_HOST: &str = "https://app.langtrace.ai";
pub const LANGTRACE_TRACE_PATH: &str = "/api/trace";

const REQUEST_ATTRIBUTES: [(&str, &str); 10] = [
    ("gen_ai.request.model", "model"),
    ("llm.is_streaming", "stream"),
    ("gen_ai.request.temperature", "temperature"),
    ("llm.top_k", "top_k"),
    ("gen_ai.request.top_p", "top_p"),
    ("llm.user", "user"),
    ("gen_ai.request.max_tokens", "max_tokens"),
    ("llm.response.stop_reason", "stop"),
    ("llm.frequency_penalty", "frequency_penalty"),
    ("llm.presence_penalty", "presence_penalty"),
];

const RESPONSE_ATTRIBUTES: [(&str, &str); 3] = [
    ("gen_ai.response_id", "id"),
    ("gen_ai.system_fingerprint", "system_fingerprint"),
    ("gen_ai.response.model", "model"),
];

const USAGE_ATTRIBUTES: [(&str, &str); 3] = [
    ("gen_ai.usage.prompt_tokens", "prompt_tokens"),
    ("gen_ai.usage.completion_tokens", "completion_tokens"),
    ("llm.usage.total_tokens", "total_tokens"),
];

#[derive(serde::Serialize)]
struct Completion<'entry> {
    role: Option<&'entry OtelModelValue>,
    content: Option<&'entry OtelModelValue>,
}

#[must_use]
pub fn langtrace_trace_endpoint(api_host: Option<&str>) -> String {
    let host = api_host
        .unwrap_or(LANGTRACE_DEFAULT_HOST)
        .trim_end_matches('/')
        .to_owned();
    if host.ends_with(LANGTRACE_TRACE_PATH) {
        return host;
    }
    format!("{host}{LANGTRACE_TRACE_PATH}")
}

pub struct LangtraceAttributes;

impl LangtraceAttributes {
    pub fn set_langtrace_attributes<S>(
        span: &mut S,
        kwargs: &MetadataMap,
        response_obj: &MetadataMap,
    ) where
        S: Span,
    {
        // LiteLLM reads the merged request options; optional params win over the call kwargs
        let mut options = kwargs.clone();
        if let Some(optional) = kwargs
            .get("optional_params")
            .and_then(OtelModelValue::as_object)
        {
            options.extend(
                optional
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        let vendor = kwargs
            .get("litellm_params")
            .and_then(OtelModelValue::as_object)
            .and_then(|params| params.get("custom_llm_provider"))
            .and_then(OtelModelValue::as_str);
        Self::set_request_attributes(span, &options, vendor);
        Self::set_response_attributes(span, response_obj);
        Self::set_usage_attributes(span, response_obj);
    }

    pub fn set_request_attributes<S>(span: &mut S, options: &MetadataMap, vendor: Option<&str>)
    where
        S: Span,
    {
        if let Some(service) = vendor {
            span.set_attribute(KeyValue::new("langtrace.service.name", service.to_owned()));
        }
        span.set_attribute(KeyValue::new("gen_ai.operation.name", "chat"));
        set_attributes(span, options, &REQUEST_ATTRIBUTES);
        let declared = options
            .get("messages")
            .and_then(OtelModelValue::as_array)
            .filter(|messages| !messages.is_empty());
        if let Some(messages) = declared
            && let Ok(text) = serde_json::to_string(messages)
        {
            span.add_event(
                "gen_ai.content.prompt",
                vec![KeyValue::new("gen_ai.prompt", text)],
            );
        }
    }

    pub fn set_response_attributes<S>(span: &mut S, response_obj: &MetadataMap)
    where
        S: Span,
    {
        record_completion_event(span, response_obj);
        set_attributes(span, response_obj, &RESPONSE_ATTRIBUTES);
    }

    pub fn set_usage_attributes<S>(span: &mut S, response_obj: &MetadataMap)
    where
        S: Span,
    {
        let Some(usage) = response_obj
            .get("usage")
            .and_then(OtelModelValue::as_object)
        else {
            return;
        };
        set_attributes(span, usage, &USAGE_ATTRIBUTES);
    }
}

fn set_attributes<S>(span: &mut S, source: &MetadataMap, table: &[(&'static str, &'static str)])
where
    S: Span,
{
    for &(key, source_key) in table {
        if let Some(value) = source.get(source_key).and_then(attribute_value) {
            span.set_attribute(KeyValue::new(key, value));
        }
    }
}

fn record_completion_event<S>(span: &mut S, response_obj: &MetadataMap)
where
    S: Span,
{
    let Ok(text) = serde_json::to_string(&completion_entries(response_obj)) else {
        return;
    };
    span.add_event(
        "gen_ai.content.completion",
        vec![KeyValue::new("gen_ai.completion", text)],
    );
}

fn completion_entries(response_obj: &MetadataMap) -> Vec<Completion<'_>> {
    let Some(choices) = response_obj
        .get("choices")
        .and_then(OtelModelValue::as_array)
    else {
        return Vec::new();
    };
    choices
        .iter()
        .filter_map(|choice| {
            let message = choice.as_object()?.get("message")?.as_object()?;
            Some(Completion {
                role: message.get("role"),
                content: message.get("content"),
            })
        })
        .collect()
}

// a value the OTel attribute model cannot hold is skipped, as LiteLLM's SDK span skips it
fn attribute_value(value: &OtelModelValue) -> Option<Value> {
    match value {
        OtelModelValue::String(text) => Some(Value::from(text.clone())),
        OtelModelValue::Bool(flag) => Some(Value::Bool(*flag)),
        OtelModelValue::Number(number) => number
            .as_i64()
            .map(Value::I64)
            .or_else(|| number.as_f64().map(Value::F64)),
        OtelModelValue::Array(items) => string_array(items),
        OtelModelValue::Null(()) | OtelModelValue::Object(_) => None,
    }
}

fn string_array(items: &[OtelModelValue]) -> Option<Value> {
    if items.is_empty() {
        return None;
    }
    let mut values = Vec::with_capacity(items.len());
    for item in items {
        values.push(StringValue::from(item.as_str()?.to_owned()));
    }
    Some(Value::Array(Array::String(values)))
}

#[path = "langtrace/tests.rs"]
mod tests;
