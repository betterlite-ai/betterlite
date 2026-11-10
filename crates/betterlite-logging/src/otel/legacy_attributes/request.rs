use opentelemetry::trace::Span;

use crate::logging_privacy::internal::{Origin, call_category, is_unbilled};
use crate::otel::integrations::arize_attributes::truthy::is_truthy;
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

use super::context::{AttributeContext, InferenceCall, write_present};

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
    write_payload_attributes(span, context, call);
    write_request_identity(span, context.config, call);
    write_response_identity(span, call.payload, call.response);
    set_service_tier_attributes(span, call.payload);
    write_usage(span, call.kwargs, call.litellm_params, call.response);
}

fn write_payload_attributes<S>(
    span: &mut S,
    context: &AttributeContext<'_>,
    call: &InferenceCall<'_>,
) where
    S: Span,
{
    let metadata = call
        .payload
        .get("metadata")
        .and_then(OtelModelValue::as_object)
        .into_iter()
        .flatten();
    for (key, value) in metadata {
        span.safe_set_attribute(&format!("{}{key}", keys::METADATA_PREFIX), value);
    }
    let hidden = call
        .payload
        .get("hidden_params")
        .filter(|value| is_truthy(value));
    if let Some(found) = hidden
        && let Ok(text) = safe_dumps(found, DEFAULT_MAX_RECURSE_DEPTH, None)
    {
        span.safe_set_attribute(keys::HIDDEN_PARAMS, &OtelModelValue::String(text));
    }
    write_identity_attributes(
        span,
        call.payload,
        call.litellm_params,
        &context.config.baggage_team_metadata_keys,
        &context.config.baggage_metadata_keys,
    );
    let breakdown = call
        .payload
        .get("cost_breakdown")
        .filter(|value| is_truthy(value))
        .and_then(OtelModelValue::as_object);
    let entries = breakdown
        .into_iter()
        .flatten()
        .filter(|(_, value)| !matches!(value, OtelModelValue::Null(())));
    for (key, value) in entries {
        span.safe_set_attribute(&format!("{}{key}", keys::COST_PREFIX), value);
    }
}

fn write_request_identity<S>(span: &mut S, config: &OpenTelemetryConfig, call: &InferenceCall<'_>)
where
    S: Span,
{
    write_present(span, gen_ai::REQUEST_MODEL, call.kwargs.get("model"));
    span.safe_set_attribute(
        keys::REQUEST_TYPE,
        call.payload
            .get("call_type")
            .unwrap_or(&OtelModelValue::Null(())),
    );
    let provider = call
        .litellm_params
        .get("custom_llm_provider")
        .cloned()
        .unwrap_or_else(|| OtelModelValue::String(UNKNOWN_PROVIDER.to_owned()));
    let provider_attribute = if semconv_latest_experimental(config) {
        gen_ai::PROVIDER_NAME
    } else {
        gen_ai::SYSTEM
    };
    span.safe_set_attribute(provider_attribute, &provider);
    write_sampling_parameters(span, call.optional_params);
    write_stream_or_semconv(span, config, call);
    write_present(span, keys::USER, call.optional_params.get("user"));
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
        write_present(span, attribute, optional_params.get(source));
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
        Some(value) if is_truthy(value) => "True",
        None | Some(_) => "False",
    };
    span.safe_set_attribute(
        keys::IS_STREAMING,
        &OtelModelValue::String(streaming.to_owned()),
    );
}

fn write_response_identity<S>(
    span: &mut S,
    payload: &MetadataMap,
    response: Option<&OtelModelValue>,
) where
    S: Span,
{
    let identifier = response_field(response, "id")
        .or_else(|| payload.get("id").filter(|value| is_truthy(value)));
    write_present(span, gen_ai::RESPONSE_ID, identifier);
    write_present(span, litellm::CALL_ID, payload.get("litellm_call_id"));
    write_present(
        span,
        gen_ai::RESPONSE_MODEL,
        response_field(response, "model"),
    );
}

// a truthy field of the returned object; a missing or falsy response reads as absent
fn response_field<'source>(
    response: Option<&'source OtelModelValue>,
    key: &str,
) -> Option<&'source OtelModelValue> {
    response
        .filter(|value| is_truthy(value))
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get(key))
        .filter(|value| is_truthy(value))
}

fn write_usage<S>(
    span: &mut S,
    kwargs: &MetadataMap,
    litellm_params: &MetadataMap,
    response: Option<&OtelModelValue>,
) where
    S: Span,
{
    let Some(returned) = response.filter(|value| is_truthy(value)) else {
        return;
    };
    if is_unbilled_call(kwargs, litellm_params, returned) {
        return;
    }
    let usage = returned
        .as_object()
        .and_then(|fields| fields.get("usage"))
        .filter(|value| is_truthy(value))
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
