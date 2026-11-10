use opentelemetry::trace::Span;

use crate::logging_privacy::internal::{Origin, call_category, is_unbilled};
use crate::otel::legacy_policy::annotations::{
    set_service_tier_attributes, write_identity_attributes,
};
use crate::otel::legacy_policy::config::OpenTelemetryConfig;
use crate::otel::legacy_policy::semconv_attrs::{
    semconv_latest_experimental, set_cache_token_attributes, set_semconv_request_attributes,
};
use crate::otel::model::metadata::wire::MetadataMap;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::vocabulary::constants::attribute_keys::{gen_ai, litellm};
use crate::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};

use super::context::{AttributeContext, InferenceCall, truthiness};

mod tests;

pub mod keys;

pub(super) const UNKNOWN_PROVIDER: &str = "Unknown";
const BACKGROUND_COST_POLL_ORIGIN: &str = "background_response_cost_poll";

pub fn set_request_attributes<S>(
    span: &mut S,
    context: &AttributeContext<'_>,
    call: &InferenceCall<'_>,
) where
    S: Span,
{
    write_metadata(span, call.payload);
    write_hidden_params(span, call.payload);
    write_identity_attributes(
        span,
        call.payload,
        call.litellm_params,
        &context.config.baggage_team_metadata_keys,
        &context.config.baggage_metadata_keys,
    );
    write_cost_breakdown(span, call.payload);
    write_model(span, call.kwargs);
    write_request_type(span, call.payload);
    write_provider(span, context.config, call.litellm_params);
    write_sampling_parameters(span, call.optional_params);
    write_stream_or_semconv(span, context.config, call);
    write_user(span, call.optional_params);
    write_response_id(span, call.payload, call.response);
    write_call_id(span, call.payload);
    write_response_model(span, call.response);
    set_service_tier_attributes(span, call.payload);
    write_usage(span, call.kwargs, call.litellm_params, call.response);
}

fn write_metadata<S>(span: &mut S, payload: &MetadataMap)
where
    S: Span,
{
    let Some(metadata) = payload.get("metadata").and_then(OtelModelValue::as_object) else {
        return;
    };
    for (key, value) in metadata {
        span.safe_set_attribute(&format!("{}{key}", keys::METADATA_PREFIX), value);
    }
}

fn write_hidden_params<S>(span: &mut S, payload: &MetadataMap)
where
    S: Span,
{
    let Some(hidden) = payload
        .get("hidden_params")
        .filter(|value| truthiness(value))
    else {
        return;
    };
    if let Ok(text) = safe_dumps(hidden, DEFAULT_MAX_RECURSE_DEPTH, None) {
        span.safe_set_attribute(keys::HIDDEN_PARAMS, &OtelModelValue::String(text));
    }
}

fn write_cost_breakdown<S>(span: &mut S, payload: &MetadataMap)
where
    S: Span,
{
    let Some(entries) = payload
        .get("cost_breakdown")
        .filter(|value| truthiness(value))
        .and_then(OtelModelValue::as_object)
    else {
        return;
    };
    for (key, value) in entries {
        if !matches!(value, OtelModelValue::Null(())) {
            span.safe_set_attribute(&format!("{}{key}", keys::COST_PREFIX), value);
        }
    }
}

fn write_model<S>(span: &mut S, kwargs: &MetadataMap)
where
    S: Span,
{
    if let Some(model) = kwargs.get("model").filter(|value| truthiness(value)) {
        span.safe_set_attribute(gen_ai::REQUEST_MODEL, model);
    }
}

fn write_request_type<S>(span: &mut S, payload: &MetadataMap)
where
    S: Span,
{
    let call_type = payload
        .get("call_type")
        .cloned()
        .unwrap_or(OtelModelValue::Null(()));
    span.safe_set_attribute(keys::REQUEST_TYPE, &call_type);
}

fn write_provider<S>(span: &mut S, config: &OpenTelemetryConfig, litellm_params: &MetadataMap)
where
    S: Span,
{
    let provider = litellm_params
        .get("custom_llm_provider")
        .cloned()
        .unwrap_or_else(|| OtelModelValue::String(UNKNOWN_PROVIDER.to_owned()));
    let attribute = if semconv_latest_experimental(config) {
        gen_ai::PROVIDER_NAME
    } else {
        gen_ai::SYSTEM
    };
    span.safe_set_attribute(attribute, &provider);
}

fn write_sampling_parameters<S>(span: &mut S, optional_params: &MetadataMap)
where
    S: Span,
{
    for (attribute, source) in [
        (gen_ai::REQUEST_MAX_TOKENS, "max_tokens"),
        (gen_ai::REQUEST_TEMPERATURE, "temperature"),
        (gen_ai::REQUEST_TOP_P, "top_p"),
    ] {
        if let Some(value) = optional_params
            .get(source)
            .filter(|value| truthiness(value))
        {
            span.safe_set_attribute(attribute, value);
        }
    }
}

fn write_stream_or_semconv<S>(span: &mut S, config: &OpenTelemetryConfig, call: &InferenceCall<'_>)
where
    S: Span,
{
    if semconv_latest_experimental(config) {
        set_semconv_request_attributes(span, call.optional_params);
        set_cache_token_attributes(span, call.payload);
        return;
    }
    let streaming = match call.optional_params.get("stream") {
        Some(OtelModelValue::Null(())) => "None",
        Some(value) if truthiness(value) => "True",
        None | Some(_) => "False",
    };
    span.safe_set_attribute(
        keys::IS_STREAMING,
        &OtelModelValue::String(streaming.to_owned()),
    );
}

fn write_user<S>(span: &mut S, optional_params: &MetadataMap)
where
    S: Span,
{
    if let Some(user) = optional_params
        .get("user")
        .filter(|value| truthiness(value))
    {
        span.safe_set_attribute(keys::USER, user);
    }
}

fn write_response_id<S>(span: &mut S, payload: &MetadataMap, response: Option<&OtelModelValue>)
where
    S: Span,
{
    let from_response = response
        .filter(|value| truthiness(value))
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("id"))
        .filter(|value| truthiness(value));
    let identifier = from_response.or_else(|| payload.get("id").filter(|value| truthiness(value)));
    if let Some(found) = identifier {
        span.safe_set_attribute(gen_ai::RESPONSE_ID, found);
    }
}

fn write_call_id<S>(span: &mut S, payload: &MetadataMap)
where
    S: Span,
{
    if let Some(call_id) = payload
        .get("litellm_call_id")
        .filter(|value| truthiness(value))
    {
        span.safe_set_attribute(litellm::CALL_ID, call_id);
    }
}

fn write_response_model<S>(span: &mut S, response: Option<&OtelModelValue>)
where
    S: Span,
{
    let model = response
        .filter(|value| truthiness(value))
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("model"))
        .filter(|value| truthiness(value));
    if let Some(found) = model {
        span.safe_set_attribute(gen_ai::RESPONSE_MODEL, found);
    }
}

fn write_usage<S>(
    span: &mut S,
    kwargs: &MetadataMap,
    litellm_params: &MetadataMap,
    response: Option<&OtelModelValue>,
) where
    S: Span,
{
    let Some(returned) = response.filter(|value| truthiness(value)) else {
        return;
    };
    if is_unbilled_call(kwargs, litellm_params, returned) {
        return;
    }
    let usage = returned
        .as_object()
        .and_then(|fields| fields.get("usage"))
        .filter(|value| truthiness(value))
        .and_then(OtelModelValue::as_object);
    let Some(counts) = usage else {
        return;
    };
    for (attribute, source) in [
        (keys::USAGE_TOTAL_TOKENS, "total_tokens"),
        (gen_ai::USAGE_OUTPUT_TOKENS, "completion_tokens"),
        (gen_ai::USAGE_INPUT_TOKENS, "prompt_tokens"),
    ] {
        let value = counts
            .get(source)
            .cloned()
            .unwrap_or(OtelModelValue::Null(()));
        span.safe_set_attribute(attribute, &value);
    }
}

fn is_unbilled_call(
    kwargs: &MetadataMap,
    litellm_params: &MetadataMap,
    response: &OtelModelValue,
) -> bool {
    let call_type = kwargs.get("call_type").and_then(OtelModelValue::as_str);
    let background = response
        .as_object()
        .and_then(|fields| fields.get("background"))
        .and_then(OtelModelValue::as_bool)
        == Some(true);
    is_unbilled(
        call_category(call_type),
        cost_poll_origin(litellm_params),
        background,
    )
}

// only the origin is read from the merged metadata, and metadata wins over litellm_metadata
fn cost_poll_origin(litellm_params: &MetadataMap) -> Option<Origin> {
    let from_metadata = litellm_params
        .get("metadata")
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("internal_call_origin"));
    let from_litellm_metadata = litellm_params
        .get("litellm_metadata")
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("internal_call_origin"));
    let origin = from_metadata
        .or(from_litellm_metadata)
        .and_then(OtelModelValue::as_str)?;
    (origin == BACKGROUND_COST_POLL_ORIGIN).then_some(Origin::BackgroundResponseCostPoll)
}
