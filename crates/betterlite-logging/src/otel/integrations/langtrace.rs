use opentelemetry::trace::Span;
use opentelemetry::{Array, KeyValue, StringValue, Value};

use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;

pub const LANGTRACE_DEFAULT_HOST: &str = "https://app.langtrace.ai";
pub const LANGTRACE_TRACE_PATH: &str = "/api/trace";

const OPERATION_NAME: &str = "gen_ai.operation.name";
const SERVICE_NAME: &str = "langtrace.service.name";
const REQUEST_MODEL: &str = "gen_ai.request.model";
const IS_STREAMING: &str = "llm.is_streaming";
const TEMPERATURE: &str = "gen_ai.request.temperature";
const TOP_K: &str = "llm.top_k";
const TOP_P: &str = "gen_ai.request.top_p";
const USER: &str = "llm.user";
const MAX_TOKENS: &str = "gen_ai.request.max_tokens";
const STOP_REASON: &str = "llm.response.stop_reason";
const FREQUENCY_PENALTY: &str = "llm.frequency_penalty";
const PRESENCE_PENALTY: &str = "llm.presence_penalty";
const PROMPT_EVENT: &str = "gen_ai.content.prompt";
const PROMPTS: &str = "gen_ai.prompt";
const COMPLETION_EVENT: &str = "gen_ai.content.completion";
const COMPLETIONS: &str = "gen_ai.completion";
const RESPONSE_ID: &str = "gen_ai.response_id";
const SYSTEM_FINGERPRINT: &str = "gen_ai.system_fingerprint";
const RESPONSE_MODEL: &str = "gen_ai.response.model";
const USAGE_PROMPT_TOKENS: &str = "gen_ai.usage.prompt_tokens";
const USAGE_COMPLETION_TOKENS: &str = "gen_ai.usage.completion_tokens";
const USAGE_TOTAL_TOKENS: &str = "llm.usage.total_tokens";

const REQUEST_ATTRIBUTES: [(&str, &str); 10] = [
    (REQUEST_MODEL, "model"),
    (IS_STREAMING, "stream"),
    (TEMPERATURE, "temperature"),
    (TOP_K, "top_k"),
    (TOP_P, "top_p"),
    (USER, "user"),
    (MAX_TOKENS, "max_tokens"),
    (STOP_REASON, "stop"),
    (FREQUENCY_PENALTY, "frequency_penalty"),
    (PRESENCE_PENALTY, "presence_penalty"),
];

const RESPONSE_ATTRIBUTES: [(&str, &str); 3] = [
    (RESPONSE_ID, "id"),
    (SYSTEM_FINGERPRINT, "system_fingerprint"),
    (RESPONSE_MODEL, "model"),
];

const USAGE_ATTRIBUTES: [(&str, &str); 3] = [
    (USAGE_PROMPT_TOKENS, "prompt_tokens"),
    (USAGE_COMPLETION_TOKENS, "completion_tokens"),
    (USAGE_TOTAL_TOKENS, "total_tokens"),
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
            span.set_attribute(KeyValue::new(SERVICE_NAME, service.to_owned()));
        }
        span.set_attribute(KeyValue::new(OPERATION_NAME, "chat"));
        for (key, source) in REQUEST_ATTRIBUTES {
            if let Some(value) = options.get(source).and_then(attribute_value) {
                span.set_attribute(KeyValue::new(key, value));
            }
        }
        let declared = options
            .get("messages")
            .and_then(OtelModelValue::as_array)
            .filter(|messages| !messages.is_empty());
        if let Some(messages) = declared
            && let Ok(text) = serde_json::to_string(messages)
        {
            span.add_event(PROMPT_EVENT, vec![KeyValue::new(PROMPTS, text)]);
        }
    }

    pub fn set_response_attributes<S>(span: &mut S, response_obj: &MetadataMap)
    where
        S: Span,
    {
        let completions = completion_entries(response_obj);
        if let Ok(text) = serde_json::to_string(&completions) {
            span.add_event(COMPLETION_EVENT, vec![KeyValue::new(COMPLETIONS, text)]);
        }
        for (key, source) in RESPONSE_ATTRIBUTES {
            if let Some(value) = response_obj.get(source).and_then(attribute_value) {
                span.set_attribute(KeyValue::new(key, value));
            }
        }
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
        for (key, source) in USAGE_ATTRIBUTES {
            if let Some(value) = usage.get(source).and_then(attribute_value) {
                span.set_attribute(KeyValue::new(key, value));
            }
        }
    }
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
