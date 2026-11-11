mod identity;
pub mod keys;
mod mcp;
mod passthrough;
pub(crate) mod truthy;

#[path = "arize_attributes/tests.rs"]
mod tests;

use crate::otel::integrations::arize_messages::set_response_attributes;
use crate::otel::integrations::arize_span_kind::{
    infer_open_inference_span_kind, set_tool_attributes,
};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::{LLMObsAttributes, SafeSetAttribute as _};
use crate::otel::vocabulary::constants::attribute_keys::exception_event;
use opentelemetry::KeyValue;
use opentelemetry::trace::Span;
use truthy::is_truthy;

const MISSING_PAYLOAD: &str = "standard_logging_object not found in kwargs";

pub fn set_attributes<A, S>(
    span: &mut S,
    kwargs: &MetadataMap,
    response: &OtelModelValue,
    redact_messages: bool,
) where
    A: LLMObsAttributes,
    S: Span,
{
    let payload = kwargs
        .get("standard_logging_object")
        .and_then(OtelModelValue::as_object);
    let call_type = payload
        .and_then(|fields| fields.get("call_type"))
        .and_then(OtelModelValue::as_str);
    span.safe_set_attribute(
        keys::OPENINFERENCE_SPAN_KIND,
        &OtelModelValue::String(infer_open_inference_span_kind(call_type).to_string()),
    );
    match payload {
        Some(fields) => set_main_attributes::<A, S>(span, kwargs, fields, response),
        None => record_missing_payload(span),
    }
    identity::set_session_and_user_attrs(span, kwargs, payload);
    identity::set_response_cost_attr(span, payload);
    passthrough::maybe_normalize_passthrough(span, kwargs, response, payload, redact_messages);
    mcp::maybe_set_mcp_tool_attrs(span, response, payload, redact_messages);
}

fn record_missing_payload<S>(span: &mut S)
where
    S: Span,
{
    tracing::error!(
        "[Arize/Phoenix] Failed to set OpenInference span attributes: {MISSING_PAYLOAD}"
    );
    span.add_event(
        exception_event::NAME,
        vec![
            KeyValue::new(exception_event::TYPE, "ValueError"),
            KeyValue::new(exception_event::MESSAGE, MISSING_PAYLOAD),
        ],
    );
}

fn set_main_attributes<A, S>(
    span: &mut S,
    kwargs: &MetadataMap,
    payload: &MetadataMap,
    response: &OtelModelValue,
) where
    A: LLMObsAttributes,
    S: Span,
{
    if let Some(metadata) = payload
        .get("metadata")
        .filter(|value| !matches!(value, OtelModelValue::Null(())))
        && let Ok(text) = serde_json::to_string(metadata)
    {
        span.safe_set_attribute(keys::METADATA, &OtelModelValue::String(text));
    }
    set_request_attributes(span, kwargs, payload);
    set_response_identity(span, response);
    let optional_tools = kwargs
        .get("optional_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("tools"))
        .and_then(OtelModelValue::as_array);
    set_tool_attributes(
        span,
        optional_tools,
        metadata_tools(payload.get("metadata")),
    );
    A::set_messages(span, kwargs);
    set_model_parameters(span, payload.get("model_parameters"));
    set_response_attributes(span, response);
}

fn metadata_tools(metadata: Option<&OtelModelValue>) -> Option<&[OtelModelValue]> {
    metadata
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("llm"))
        .and_then(OtelModelValue::as_object)
        .and_then(|llm| llm.get("tools"))
        .and_then(OtelModelValue::as_array)
}

fn set_request_attributes<S>(span: &mut S, kwargs: &MetadataMap, payload: &MetadataMap)
where
    S: Span,
{
    if let Some(model) = kwargs.get("model").filter(|value| is_truthy(value)) {
        span.safe_set_attribute(keys::LLM_MODEL_NAME, model);
    }
    let call_type = payload
        .get("call_type")
        .cloned()
        .unwrap_or(OtelModelValue::Null(()));
    span.safe_set_attribute(keys::LLM_REQUEST_TYPE, &call_type);
    let provider = kwargs
        .get("litellm_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|params| params.get("custom_llm_provider"))
        .cloned()
        .unwrap_or_else(|| OtelModelValue::String("Unknown".to_owned()));
    span.safe_set_attribute(keys::LLM_PROVIDER, &provider);
    set_optional_params(span, kwargs);
}

fn set_optional_params<S>(span: &mut S, kwargs: &MetadataMap)
where
    S: Span,
{
    let empty = MetadataMap::new();
    let optional_params = kwargs
        .get("optional_params")
        .and_then(OtelModelValue::as_object)
        .unwrap_or(&empty);
    for (attribute, source) in [
        (keys::LLM_REQUEST_MAX_TOKENS, "max_tokens"),
        (keys::LLM_REQUEST_TEMPERATURE, "temperature"),
        (keys::LLM_REQUEST_TOP_P, "top_p"),
    ] {
        if let Some(value) = optional_params.get(source).filter(|value| is_truthy(value)) {
            span.safe_set_attribute(attribute, value);
        }
    }
    let streaming = match optional_params.get("stream") {
        Some(OtelModelValue::Null(())) => "None",
        Some(value) if is_truthy(value) => "True",
        None | Some(_) => "False",
    };
    span.safe_set_attribute(
        keys::LLM_IS_STREAMING,
        &OtelModelValue::String(streaming.to_owned()),
    );
    if let Some(user) = optional_params.get("user").filter(|value| is_truthy(value)) {
        span.safe_set_attribute(keys::LLM_USER, user);
    }
}

fn set_response_identity<S>(span: &mut S, response: &OtelModelValue)
where
    S: Span,
{
    let Some(fields) = response.as_object() else {
        return;
    };
    if let Some(id) = fields.get("id").filter(|value| is_truthy(value)) {
        span.safe_set_attribute(keys::LLM_RESPONSE_ID, id);
    }
    if let Some(model) = fields.get("model").filter(|value| is_truthy(value)) {
        span.safe_set_attribute(keys::LLM_RESPONSE_MODEL, model);
    }
}

fn set_model_parameters<S>(span: &mut S, model_parameters: Option<&OtelModelValue>)
where
    S: Span,
{
    let Some(parameters) = model_parameters.filter(|value| is_truthy(value)) else {
        return;
    };
    if let Ok(text) = serde_json::to_string(parameters) {
        span.safe_set_attribute(
            keys::LLM_INVOCATION_PARAMETERS,
            &OtelModelValue::String(text),
        );
    }
    if let Some(user) = parameters
        .as_object()
        .and_then(|fields| fields.get("user"))
        .filter(|value| is_truthy(value))
    {
        span.safe_set_attribute(keys::USER_ID, user);
    }
}
