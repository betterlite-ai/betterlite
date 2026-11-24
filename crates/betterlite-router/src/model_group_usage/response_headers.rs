use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::remaining_usage_headers::{RemainingUsage, apply_remaining_usage_headers};
use crate::router_core::headers::{
    Prepared, Response, apply_response_model_id, ensure_additional_headers, prepare_response,
    write_hidden_params,
};
use crate::routing_decision_headers::{RoutingValue, apply_quality_headers, complexity_headers};

use super::counters::UsageCounterCache;
use super::error::GroupUsageError;
use super::metadata::GroupMetadata;
use super::remaining::group_remaining_usage;

const METADATA_CHANNELS: [&str; 2] = ["litellm_metadata", "metadata"];

// Everything Router.set_response_headers reads, with the remaining-usage collaborators in place of the router's own.
pub struct ResponseHeadersRequest<'request, M, C> {
    pub response: Response,
    pub model_group: Option<&'request str>,
    pub request_kwargs: Option<&'request JsonObject>,
    pub model_list: Option<&'request [DeploymentSpec]>,
    pub metadata: &'request M,
    pub cache: &'request C,
    pub now: SystemTime,
}

// Router.set_response_headers: the model group, the decision headers and the remaining usage on one response.
pub async fn set_response_headers<M, C>(
    request: ResponseHeadersRequest<'_, M, C>,
) -> Result<Response, GroupUsageError<M::Error, C::Error>>
where
    M: GroupMetadata,
    C: UsageCounterCache,
{
    let ResponseHeadersRequest {
        response,
        model_group,
        request_kwargs,
        model_list,
        metadata,
        cache,
        now,
    } = request;
    let Some(mut prepared) = prepare_response(response) else {
        return Ok(Response::Absent);
    };
    let remaining = match model_group {
        Some(group) => Some(group_remaining_usage(group, model_list, metadata, cache, now).await?),
        None => None,
    };
    stamp_headers(
        &mut prepared,
        model_group,
        request_kwargs,
        remaining.as_ref(),
    );
    Ok(write_hidden_params(prepared))
}

// The model id, the group header, the decision headers, then the remaining usage, in the order the source writes them.
fn stamp_headers(
    prepared: &mut Prepared,
    model_group: Option<&str>,
    request_kwargs: Option<&JsonObject>,
    remaining: Option<&RemainingUsage>,
) {
    apply_response_model_id(prepared, find_deployment_metadata(request_kwargs));
    let headers = ensure_additional_headers(prepared);
    headers.insert(
        "x-litellm-model-group".to_owned(),
        model_group.map_or(Json::Null, |group| Json::Text(group.to_owned())),
    );
    if let Some(kwargs) = request_kwargs {
        let routing = RoutingValue::Object(
            kwargs
                .iter()
                .map(|(key, value)| (key.clone(), routing_value(value)))
                .collect(),
        );
        apply_decision_headers(headers, &routing);
    }
    if let Some(remaining) = remaining {
        apply_remaining_usage_headers(headers, remaining);
    }
}

// apply_quality_router_decision_headers, then the dict.update of complexity_router_decision_headers.
fn apply_decision_headers(headers: &mut JsonObject, routing: &RoutingValue) {
    let mut quality = BTreeMap::new();
    apply_quality_headers(&mut quality, routing);
    for (header, value) in quality {
        let RoutingValue::String(text) = value else {
            continue;
        };
        headers.insert(header, Json::Text(text));
    }
    for (header, value) in complexity_headers(routing) {
        headers.insert(header, Json::Text(value));
    }
}

// find_deployment_metadata: the first bucket carrying a model_info key, litellm_metadata before metadata.
fn find_deployment_metadata(request_kwargs: Option<&JsonObject>) -> Option<&JsonObject> {
    let kwargs = request_kwargs?;
    METADATA_CHANNELS
        .iter()
        .find_map(|channel| match kwargs.get(*channel) {
            Some(Json::Object(bucket)) if bucket.contains_key("model_info") => Some(bucket),
            Some(
                Json::Object(_)
                | Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::Text(_)
                | Json::List(_),
            )
            | None => None,
        })
}

fn routing_value(value: &Json) -> RoutingValue {
    match value {
        Json::Null => RoutingValue::Null,
        Json::Bool(flag) => RoutingValue::Bool(*flag),
        Json::Number(number) => RoutingValue::Number(number.clone()),
        Json::Text(text) => RoutingValue::String(text.clone()),
        Json::List(items) => RoutingValue::Array(items.iter().map(routing_value).collect()),
        Json::Object(fields) => RoutingValue::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), routing_value(value)))
                .collect(),
        ),
    }
}
