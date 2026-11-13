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

const MODEL_ROUTE_KINDS: &[(&str, InferenceModelKind)] = &[
    ("/images/generations", InferenceModelKind::ImageGeneration),
    ("/images/edits", InferenceModelKind::ImageEdit),
    ("/moderations", InferenceModelKind::Moderation),
    ("/audio/transcriptions", InferenceModelKind::Moderation),
    ("/audio/speech", InferenceModelKind::Speech),
    ("/rerank", InferenceModelKind::Body),
    ("/decisions", InferenceModelKind::Body),
    ("/systemone", InferenceModelKind::Body),
    ("/messages/count_tokens", InferenceModelKind::Body),
    (":countTokens", InferenceModelKind::Path),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InferenceModelKind {
    Completion,
    ImageGeneration,
    ImageEdit,
    Moderation,
    Speech,
    Body,
    Path,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InferenceModelSettings {
    pub completion_model: Option<String>,
    pub image_generation_model: Option<String>,
    pub moderation_model: Option<String>,
}

pub struct ManagedInferenceRequest<'request> {
    pub route: &'request str,
    pub body: &'request IndexMap<String, Json>,
    pub settings: &'request InferenceModelSettings,
    pub cli_model: Option<&'request str>,
    pub path_model: Option<&'request str>,
    pub query_model: Option<&'request str>,
}

#[must_use]
pub fn resolve_inference_model<'model>(
    body_model: Option<&'model str>,
    settings: &'model InferenceModelSettings,
    cli_model: Option<&'model str>,
    endpoint_model: Option<&'model str>,
    kind: InferenceModelKind,
) -> Option<&'model str> {
    match kind {
        InferenceModelKind::ImageGeneration => cli_model
            .or(endpoint_model)
            .or(settings.image_generation_model.as_deref())
            .or(body_model),
        InferenceModelKind::ImageEdit => settings
            .completion_model
            .as_deref()
            .or(cli_model)
            .or(endpoint_model)
            .or(settings.image_generation_model.as_deref())
            .or(body_model),
        InferenceModelKind::Moderation => cli_model
            .or(settings.moderation_model.as_deref())
            .or(body_model),
        InferenceModelKind::Speech => cli_model.or(body_model),
        InferenceModelKind::Body => body_model,
        InferenceModelKind::Path => endpoint_model,
        InferenceModelKind::Completion => settings
            .completion_model
            .as_deref()
            .or(cli_model)
            .or(endpoint_model)
            .or(body_model),
    }
}

pub fn managed_inference_request(
    request: &ManagedInferenceRequest<'_>,
) -> Result<IndexMap<String, Json>, AgentIdentityError> {
    if MANAGED_REALTIME_ROUTES.contains(&request.route) {
        return realtime_request(request);
    }
    let model_route = MANAGED_MODEL_ROUTES.contains(&request.route)
        || check_route_access(request.route, MANAGED_MODEL_PATHS);
    if !model_route {
        return Ok(request.body.clone());
    }
    model_route_request(request)
}

fn realtime_request(
    request: &ManagedInferenceRequest<'_>,
) -> Result<IndexMap<String, Json>, AgentIdentityError> {
    let model = request
        .query_model
        .or_else(|| body_text(request.body, "model"))
        .filter(|model| !model.is_empty());
    let Some(selected_model) = model else {
        return Err(raise_identity_failure(
            AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from(MANAGED_MODEL_REQUIRED),
            },
            IDENTITY_DENIED_STATUS,
        ));
    };
    Ok(augmented_body(request.body, selected_model))
}

fn model_route_request(
    request: &ManagedInferenceRequest<'_>,
) -> Result<IndexMap<String, Json>, AgentIdentityError> {
    let endpoint_model = request
        .path_model
        .or_else(|| query_model_fallback(request.route, request.query_model));
    let effective = resolve_inference_model(
        body_text(request.body, "model"),
        request.settings,
        request.cli_model,
        endpoint_model,
        route_kind(request.route),
    )
    .filter(|model| !model.is_empty());
    let Some(selected_model) = effective else {
        return Err(raise_identity_failure(
            AgentIdentityFailure {
                code: FailureCode::IdentityDenied,
                message: String::from(MANAGED_MODEL_REQUIRED),
            },
            IDENTITY_DENIED_STATUS,
        ));
    };
    Ok(augmented_body(request.body, selected_model))
}

fn route_kind(route: &str) -> InferenceModelKind {
    match MODEL_ROUTE_KINDS
        .iter()
        .find(|(suffix, _)| route.ends_with(suffix))
    {
        Some((_, kind)) => *kind,
        None => InferenceModelKind::Completion,
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

fn augmented_body(body: &IndexMap<String, Json>, model: &str) -> IndexMap<String, Json> {
    let mut augmented = body.clone();
    augmented.insert(String::from("model"), Json::Text(model.to_owned()));
    augmented
}
