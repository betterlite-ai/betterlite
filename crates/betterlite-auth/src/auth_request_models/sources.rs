use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::catalog::{CatalogError, Collaborators};
use super::managed::{cache_prediction_model_candidates, extract_models_from_managed_resource_id};
use super::{
    BODY_TARGET_MODEL_ROUTE_MARKERS, COMPLETION_MODEL_ROUTE_MARKERS, HEADER_OR_QUERY_ROUTE_MARKERS,
    MODEL_ROUTING_HEADER_NAME, MODEL_ROUTING_ID_FIELDS, QUERY_TARGET_MODEL_ROUTE_MARKERS,
    SESSION_MODEL_ROUTE_MARKERS, append_model_candidates, case_insensitive_mapping_value,
    dedupe_model_candidates, is_falsey, route_matches_any_marker, route_uses_model_routing_sources,
};

pub struct DispatchContext<'context> {
    // the dispatched endpoint's pass-through marker value, absent when it carries none
    pub endpoint_marker: Option<&'context Json>,
    pub path_params: &'context BTreeSet<String>,
}

#[must_use]
pub fn request_dispatched_to_pass_through_endpoint(context: &DispatchContext<'_>) -> bool {
    matches!(context.endpoint_marker, Some(Json::Bool(true)))
}

#[must_use]
pub fn request_dispatched_to_provider_pass_through(context: &DispatchContext<'_>) -> bool {
    context.path_params.contains("endpoint")
}

pub struct RequestSnapshot<'request> {
    pub route: &'request str,
    pub body: &'request JsonObject,
    pub headers: Option<&'request JsonObject>,
    pub query: Option<&'request JsonObject>,
    pub dispatch: DispatchContext<'request>,
    pub team_id: Option<&'request str>,
}

pub fn extract_model_candidates_from_request(
    request: &RequestSnapshot<'_>,
    collaborators: &Collaborators<'_>,
) -> Result<Vec<String>, CatalogError> {
    if request.route == "/cost/predict-cache" {
        let prediction = cache_prediction_model_candidates(
            request.body,
            collaborators.catalog,
            request.team_id,
        )?;
        return Ok(dedupe_model_candidates(&prediction));
    }
    let mut candidates = Vec::new();
    append_body_model_sources(request.route, request.body, &mut candidates);
    if route_uses_model_routing_sources(request.route) {
        append_header_and_query_sources(request.route, request, &mut candidates);
        append_managed_id_sources(request.body, collaborators, &mut candidates);
    }
    Ok(dedupe_model_candidates(&candidates))
}

fn append_body_model_sources(route: &str, body: &JsonObject, candidates: &mut Vec<String>) {
    let body_model = body.get("model");
    if let Some(value) = body_model {
        append_model_candidates(candidates, value);
    }
    let body_target_route = route_matches_any_marker(route, &BODY_TARGET_MODEL_ROUTE_MARKERS);
    if (body_target_route || body_model.is_none_or(is_falsey))
        && let Some(value) = body.get("target_model_names")
    {
        append_model_candidates(candidates, value);
    }
    if route_matches_any_marker(route, &SESSION_MODEL_ROUTE_MARKERS)
        && let Some(Json::Object(session)) = body.get("session")
        && let Some(value) = session.get("model")
    {
        append_model_candidates(candidates, value);
    }
    if route_matches_any_marker(route, &COMPLETION_MODEL_ROUTE_MARKERS)
        && let Some(Json::Object(completion)) = body.get("completion")
        && let Some(value) = completion.get("model")
    {
        append_model_candidates(candidates, value);
    }
}

fn append_header_and_query_sources(
    route: &str,
    request: &RequestSnapshot<'_>,
    candidates: &mut Vec<String>,
) {
    if route_matches_any_marker(route, &HEADER_OR_QUERY_ROUTE_MARKERS) {
        if let Some(value) = case_insensitive_mapping_value(request.query, "model") {
            append_model_candidates(candidates, value);
        }
        if let Some(value) =
            case_insensitive_mapping_value(request.headers, MODEL_ROUTING_HEADER_NAME)
        {
            append_model_candidates(candidates, value);
        }
    }
    if route_matches_any_marker(route, &QUERY_TARGET_MODEL_ROUTE_MARKERS)
        && let Some(value) = case_insensitive_mapping_value(request.query, "target_model_names")
    {
        append_model_candidates(candidates, value);
    }
}

fn append_managed_id_sources(
    body: &JsonObject,
    collaborators: &Collaborators<'_>,
    candidates: &mut Vec<String>,
) {
    for field in MODEL_ROUTING_ID_FIELDS {
        if let Some(value) = body.get(field) {
            let hints = extract_models_from_managed_resource_id(
                value,
                field,
                collaborators.catalog,
                collaborators.decoders,
            );
            let hints = hints.into_iter().map(Json::Text).collect();
            append_model_candidates(candidates, &Json::List(hints));
        }
    }
}
