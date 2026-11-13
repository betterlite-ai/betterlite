use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::routes::{MANAGED_MODEL_PATHS, MANAGED_MODEL_ROUTES, MANAGED_REALTIME_ROUTES};
use crate::agent_identity::error::{
    AgentIdentityError, IDENTITY_DENIED_STATUS, raise_identity_failure,
};
use crate::identity::{AgentIdentityFailure, FailureCode};
use crate::route_match::templates::check_route_access;

pub const MANAGED_MODEL_REQUIRED: &str =
    "Managed inference requires an explicit or configured model";

const COMPLETION_KIND: &str = "completion";

// the kind names are LiteLLM's Literal values, matched by route suffix in _MODEL_ROUTE_KINDS
const MODEL_ROUTE_KINDS: &[(&str, &str)] = &[
    ("/images/generations", "image_generation"),
    ("/images/edits", "image_edit"),
    ("/moderations", "moderation"),
    ("/audio/transcriptions", "moderation"),
    ("/audio/speech", "speech"),
    ("/rerank", "body"),
    ("/decisions", "body"),
    ("/systemone", "body"),
    ("/messages/count_tokens", "body"),
    (":countTokens", "path"),
];

// betterlite-core owns resolve_inference_model, so its single copy is handed in at the call site
pub type ModelResolver<'resolver> =
    dyn Fn(Option<&str>, Option<&str>, Option<&str>, &str) -> Option<String> + 'resolver;

pub struct ManagedInferenceRequest<'request> {
    pub route: &'request str,
    pub body: &'request IndexMap<String, Json>,
    pub cli_model: Option<&'request str>,
    pub path_model: Option<&'request str>,
    pub query_model: Option<&'request str>,
}

pub fn managed_inference_request(
    request: &ManagedInferenceRequest<'_>,
    resolve_model: &ModelResolver<'_>,
) -> Result<IndexMap<String, Json>, AgentIdentityError> {
    let model = if MANAGED_REALTIME_ROUTES.contains(&request.route) {
        request
            .query_model
            .or_else(|| body_text(request.body, "model"))
            .map(str::to_owned)
    } else if MANAGED_MODEL_ROUTES.contains(&request.route)
        || check_route_access(request.route, MANAGED_MODEL_PATHS)
    {
        let endpoint_model = request
            .path_model
            .or_else(|| query_model_fallback(request.route, request.query_model));
        resolve_model(
            body_text(request.body, "model"),
            request.cli_model,
            endpoint_model,
            route_kind(request.route),
        )
    } else {
        return Ok(request.body.clone());
    };
    resolved_body(request.body, model.as_deref())
}

fn resolved_body(
    body: &IndexMap<String, Json>,
    model: Option<&str>,
) -> Result<IndexMap<String, Json>, AgentIdentityError> {
    let Some(selected_model) = model.filter(|candidate| !candidate.is_empty()) else {
        return Err(raise_identity_failure(
            AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from(MANAGED_MODEL_REQUIRED),
            },
            IDENTITY_DENIED_STATUS,
        ));
    };
    let mut augmented = body.clone();
    augmented.insert(String::from("model"), Json::Text(selected_model.to_owned()));
    Ok(augmented)
}

fn route_kind(route: &str) -> &'static str {
    match MODEL_ROUTE_KINDS
        .iter()
        .find(|(suffix, _)| route.ends_with(suffix))
    {
        Some((_, kind)) => kind,
        None => COMPLETION_KIND,
    }
}

fn query_model_fallback<'model>(
    route: &str,
    query_model: Option<&'model str>,
) -> Option<&'model str> {
    let uses_query_model = route.ends_with("/completions")
        || route.ends_with("/embeddings")
        || route.ends_with("/images/generations")
        || route.ends_with("/images/edits");
    if uses_query_model { query_model } else { None }
}

fn body_text<'body>(body: &'body IndexMap<String, Json>, key: &str) -> Option<&'body str> {
    match body.get(key) {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
