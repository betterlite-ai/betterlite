use std::collections::BTreeMap;

use betterlite_models::projections::service_tier::resolved_service_tier;
use opentelemetry::trace::Span;

use crate::otel::legacy_policy::text_field;
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
    write_text(span, TEAM_ID_ATTRIBUTE, team_id);
    write_text(span, TEAM_ALIAS_ATTRIBUTE, team_alias);
}

pub fn write_team_attributes<S>(span: &mut S, payload: &BTreeMap<String, OtelModelValue>)
where
    S: Span,
{
    let metadata = payload.get("metadata").and_then(OtelModelValue::as_object);
    let team_id = metadata.and_then(|fields| text_field(fields, "user_api_key_team_id"));
    let team_alias = metadata.and_then(|fields| text_field(fields, "user_api_key_team_alias"));
    write_text(span, TEAM_ID_ATTRIBUTE, team_id);
    write_text(span, TEAM_ALIAS_ATTRIBUTE, team_alias);
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
    let route = metadata.and_then(|fields| text_field(fields, "user_api_key_request_route"));
    write_text(span, http::ROUTE, route);
    write_team_metadata(span, litellm_params, baggage_team_metadata_keys);
    let flat = flatten_metadata(metadata);
    for (key, value) in promoted_metadata(&flat, baggage_metadata_keys) {
        span.safe_set_attribute(&key, &OtelModelValue::String(value));
    }
    let model_group = text_field(payload, "model_group");
    let hidden_model = payload
        .get("hidden_params")
        .and_then(OtelModelValue::as_object)
        .and_then(|fields| text_field(fields, "litellm_model_name"));
    let provider_model = hidden_model.or_else(|| text_field(payload, "model"));
    write_text(span, MODEL_GROUP_ATTRIBUTE, model_group);
    write_text(span, litellm::PROVIDER_MODEL, provider_model);
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
    let team = raw_metadata
        .and_then(|fields| fields.get("user_api_key_team_metadata"))
        .and_then(OtelModelValue::as_object);
    if let Some(encoded) = filtered_team_metadata_json(team, baggage_team_metadata_keys) {
        span.safe_set_attribute(litellm::TEAM_METADATA, &OtelModelValue::String(encoded));
    }
}

pub fn set_service_tier_attributes<S>(span: &mut S, payload: &BTreeMap<String, OtelModelValue>)
where
    S: Span,
{
    let requested = object_text(payload, &["model_parameters", "service_tier"]);
    let requested_tier = resolved_service_tier(None, None, requested);
    write_text(span, REQUESTED_TIER_ATTRIBUTE, requested_tier);
    let response = object_text(payload, &["response", "service_tier"]);
    let usage = object_text(payload, &["metadata", "usage_object", "service_tier"]);
    let served_tier = resolved_service_tier(response, usage, None);
    write_text(span, SERVED_TIER_ATTRIBUTE, served_tier);
}

fn write_text<S>(span: &mut S, key: &str, value: Option<&str>)
where
    S: Span,
{
    if let Some(text) = value.filter(|text| !text.is_empty()) {
        span.safe_set_attribute(key, &OtelModelValue::String(text.to_owned()));
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
    let fields = parents
        .iter()
        .try_fold(payload, |current, key| current.get(*key)?.as_object())?;
    text_field(fields, last)
}
