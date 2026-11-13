pub mod keys;

#[path = "weave_attributes/tests.rs"]
mod tests;

use crate::otel::integrations::arize_attributes::set_attributes;
use crate::otel::integrations::arize_messages::keys::{INPUT_VALUE, OUTPUT_VALUE};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::{LLMObsAttributes, SafeSetAttribute as _};
use opentelemetry::trace::Span;

pub struct WeaveOtelAttributes;

impl LLMObsAttributes for WeaveOtelAttributes {
    fn set_messages<S>(span: &mut S, kwargs: &MetadataMap)
    where
        S: Span,
    {
        let mut prompt = MetadataMap::new();
        let messages = kwargs
            .get("messages")
            .and_then(OtelModelValue::as_array)
            .map_or_else(Vec::new, <[OtelModelValue]>::to_vec);
        prompt.insert("messages".to_owned(), OtelModelValue::Array(messages));
        let empty = MetadataMap::new();
        let optional_params = kwargs
            .get("optional_params")
            .and_then(OtelModelValue::as_object)
            .unwrap_or(&empty);
        for name in ["functions", "tools"] {
            if let Some(value) = optional_params
                .get(name)
                .filter(|value| !matches!(value, OtelModelValue::Null(())))
            {
                prompt.insert(name.to_owned(), value.clone());
            }
        }
        if let Ok(text) = serde_json::to_string(&OtelModelValue::Object(prompt)) {
            span.safe_set_attribute(INPUT_VALUE, &OtelModelValue::String(text));
        }
    }
}

pub fn set_weave_specific_attributes<S>(
    span: &mut S,
    kwargs: &MetadataMap,
    response: &OtelModelValue,
) where
    S: Span,
{
    let empty = MetadataMap::new();
    let metadata = kwargs
        .get("metadata")
        .and_then(OtelModelValue::as_object)
        .unwrap_or(&empty);
    set_display_name(span, metadata, kwargs);
    if let Some(session_id) = metadata
        .get("session_id")
        .filter(|value| !matches!(value, OtelModelValue::Null(())))
    {
        span.safe_set_attribute(keys::THREAD_ID, &thread_id(session_id));
        span.safe_set_attribute(keys::IS_TURN, &OtelModelValue::Bool(true));
    }
    write_response_output(span, response);
}

fn set_display_name<S>(span: &mut S, metadata: &MetadataMap, kwargs: &MetadataMap)
where
    S: Span,
{
    let model = kwargs
        .get("model")
        .and_then(OtelModelValue::as_str)
        .unwrap_or("");
    let provider = kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("custom_llm_provider"))
        .and_then(OtelModelValue::as_str)
        .unwrap_or("");
    let display_name = metadata
        .get("display_name")
        .and_then(OtelModelValue::as_str)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            (!model.is_empty()).then(|| {
                if provider.is_empty() {
                    model.to_owned()
                } else {
                    format!("{provider}/{model}")
                }
            })
        });
    if let Some(name) = display_name {
        span.safe_set_attribute(
            keys::DISPLAY_NAME,
            &OtelModelValue::String(name.replace('/', "__")),
        );
    }
}

fn write_response_output<S>(span: &mut S, response: &OtelModelValue)
where
    S: Span,
{
    if response
        .as_object()
        .is_some_and(|fields| !fields.is_empty())
        && let Ok(text) = serde_json::to_string(response)
    {
        span.safe_set_attribute(OUTPUT_VALUE, &OtelModelValue::String(text));
    }
}

fn thread_id(session_id: &OtelModelValue) -> OtelModelValue {
    match session_id {
        OtelModelValue::Array(_) | OtelModelValue::Object(_) => serde_json::to_string(session_id)
            .map_or_else(|_| session_id.clone(), OtelModelValue::String),
        OtelModelValue::Null(())
        | OtelModelValue::Bool(_)
        | OtelModelValue::Number(_)
        | OtelModelValue::String(_) => session_id.clone(),
    }
}

pub fn set_weave_otel_attributes<S>(
    span: &mut S,
    kwargs: &MetadataMap,
    response: &OtelModelValue,
    redact_messages: bool,
) where
    S: Span,
{
    set_attributes::<WeaveOtelAttributes, S>(span, kwargs, response, redact_messages);
    set_weave_specific_attributes(span, kwargs, response);
}
