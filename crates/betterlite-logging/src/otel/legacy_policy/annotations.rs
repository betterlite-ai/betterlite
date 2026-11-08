use std::collections::BTreeMap;

use betterlite_models::projections::service_tier::resolved_service_tier;
use opentelemetry::trace::Span;

use crate::otel::model::baggage::{filtered_team_metadata_json, promoted_metadata};
use crate::otel::model::metadata::helpers::flatten_metadata;
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::otel::span_attributes::SafeSetAttribute as _;
use crate::otel::vocabulary::constants::attribute_keys::{http, litellm};

mod tests;

pub const TEAM_ID_ATTRIBUTE: &str = "metadata.user_api_key_team_id";
pub const TEAM_ALIAS_ATTRIBUTE: &str = "metadata.user_api_key_team_alias";
pub const MODEL_GROUP_ATTRIBUTE: &str = "litellm.model_group";
pub const REQUESTED_TIER_ATTRIBUTE: &str = "gen_ai.openai.request.service_tier";
pub const SERVED_TIER_ATTRIBUTE: &str = "gen_ai.openai.response.service_tier";

pub fn set_team_attributes<S>(span: &mut S, team_id: Option<&str>, team_alias: Option<&str>)
where
    S: Span,
{
    if let Some(value) = team_id.filter(|text| !text.is_empty()) {
        span.safe_set_attribute(TEAM_ID_ATTRIBUTE, &OtelModelValue::String(value.to_owned()));
    }
    if let Some(value) = team_alias.filter(|text| !text.is_empty()) {
        span.safe_set_attribute(
            TEAM_ALIAS_ATTRIBUTE,
            &OtelModelValue::String(value.to_owned()),
        );
    }
}

pub fn write_team_attributes<S>(span: &mut S, payload: &BTreeMap<String, OtelModelValue>)
where
    S: Span,
{
    let metadata = payload.get("metadata").and_then(OtelModelValue::as_object);
    let team_id = metadata
        .and_then(|fields| fields.get("user_api_key_team_id"))
        .and_then(OtelModelValue::as_str);
    let team_alias = metadata
        .and_then(|fields| fields.get("user_api_key_team_alias"))
        .and_then(OtelModelValue::as_str);
    set_team_attributes(span, team_id, team_alias);
}

pub fn write_identity_attributes<S>(
    span: &mut S,
    payload: &BTreeMap<String, OtelModelValue>,
    litellm_params: &BTreeMap<String, OtelModelValue>,
    baggage_team_metadata_keys: &[String],
    baggage_metadata_keys: &[String],
) where
    S: Span,
{
    let metadata = payload.get("metadata").and_then(OtelModelValue::as_object);
    write_request_route(span, metadata);
    write_team_metadata(span, litellm_params, baggage_team_metadata_keys);
    let flat = flatten_metadata(metadata);
    for (key, value) in promoted_metadata(&flat, baggage_metadata_keys) {
        span.safe_set_attribute(&key, &OtelModelValue::String(value));
    }
    write_model_attributes(span, payload);
}

fn write_request_route<S>(span: &mut S, metadata: Option<&BTreeMap<String, OtelModelValue>>)
where
    S: Span,
{
    let route = metadata
        .and_then(|fields| fields.get("user_api_key_request_route"))
        .and_then(OtelModelValue::as_str)
        .filter(|text| !text.is_empty());
    if let Some(value) = route {
        span.safe_set_attribute(http::ROUTE, &OtelModelValue::String(value.to_owned()));
    }
}

fn write_team_metadata<S>(
    span: &mut S,
    litellm_params: &BTreeMap<String, OtelModelValue>,
    baggage_team_metadata_keys: &[String],
) where
    S: Span,
{
    let raw_metadata = non_empty_map(litellm_params.get("metadata"))
        .or_else(|| non_empty_map(litellm_params.get("litellm_metadata")));
    let team_metadata = filtered_team_metadata_json(
        raw_metadata
            .and_then(|fields| fields.get("user_api_key_team_metadata"))
            .and_then(OtelModelValue::as_object),
        baggage_team_metadata_keys,
    );
    if let Some(encoded) = team_metadata {
        span.safe_set_attribute(litellm::TEAM_METADATA, &OtelModelValue::String(encoded));
    }
}

fn write_model_attributes<S>(span: &mut S, payload: &BTreeMap<String, OtelModelValue>)
where
    S: Span,
{
    let model_group = payload
        .get("model_group")
        .and_then(OtelModelValue::as_str)
        .filter(|text| !text.is_empty());
    if let Some(group) = model_group {
        span.safe_set_attribute(
            MODEL_GROUP_ATTRIBUTE,
            &OtelModelValue::String(group.to_owned()),
        );
    }
    let hidden_model = payload
        .get("hidden_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("litellm_model_name"))
        .and_then(OtelModelValue::as_str)
        .filter(|text| !text.is_empty());
    let provider_model = hidden_model.or_else(|| {
        payload
            .get("model")
            .and_then(OtelModelValue::as_str)
            .filter(|text| !text.is_empty())
    });
    if let Some(model) = provider_model {
        span.safe_set_attribute(
            litellm::PROVIDER_MODEL,
            &OtelModelValue::String(model.to_owned()),
        );
    }
}

pub fn set_service_tier_attributes<S>(span: &mut S, payload: &BTreeMap<String, OtelModelValue>)
where
    S: Span,
{
    let requested = payload
        .get("model_parameters")
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| fields.get("service_tier"))
        .and_then(OtelModelValue::as_str);
    if let Some(tier) = resolved_service_tier(None, None, requested) {
        span.safe_set_attribute(
            REQUESTED_TIER_ATTRIBUTE,
            &OtelModelValue::String(tier.to_owned()),
        );
    }
    let response_tier = object_text(payload, &["response", "service_tier"]);
    let usage_tier = object_text(payload, &["metadata", "usage_object", "service_tier"]);
    if let Some(tier) = resolved_service_tier(response_tier, usage_tier, None) {
        span.safe_set_attribute(
            SERVED_TIER_ATTRIBUTE,
            &OtelModelValue::String(tier.to_owned()),
        );
    }
}

fn non_empty_map(value: Option<&OtelModelValue>) -> Option<&BTreeMap<String, OtelModelValue>> {
    value
        .and_then(OtelModelValue::as_object)
        .filter(|fields| !fields.is_empty())
}

fn object_text<'source>(
    payload: &'source BTreeMap<String, OtelModelValue>,
    path: &[&str],
) -> Option<&'source str> {
    let (last, parents) = path.split_last()?;
    let mut current = payload;
    for key in parents {
        current = current.get(*key)?.as_object()?;
    }
    current
        .get(*last)
        .and_then(OtelModelValue::as_str)
        .filter(|text| !text.is_empty())
}
