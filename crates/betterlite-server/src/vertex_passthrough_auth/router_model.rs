use std::fmt::Display;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::resolution::{
    ResolutionContext, ResolutionError, ResolutionRequest, ResolvedModel, resolve,
};
use betterlite_router::passthrough_selection::error::PassthroughSelectionError;
use tracing::debug;

use super::non_empty_text;

// The two fields _resolve_vertex_model_from_router reads; model_info keeps LiteLLM's untyped read.
// No Debug: litellm_params carries api_key and vertex_credentials verbatim.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct Deployment {
    pub litellm_params: JsonObject,
    pub model_info: Option<Json>,
}

// B396's pass-through selection error, kept typed so the router's cause survives the swallow
#[derive(Debug, thiserror::Error)]
#[error("{source}")]
pub struct LookupError {
    #[from]
    source: PassthroughSelectionError,
}

// LiteLLM's Router.get_available_deployment_for_pass_through, which B396 backs
pub trait VertexDeploymentLookup: Send + Sync {
    fn deployment_for_pass_through(&self, model: &str) -> Result<Option<Deployment>, LookupError>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedVertexModel {
    pub encoded_endpoint: String,
    pub endpoint: String,
    pub project: Option<String>,
    pub location: Option<String>,
    pub deployment_model_info: Option<JsonObject>,
}

pub struct VertexModelRequest<'request> {
    pub model_id: &'request str,
    pub encoded_endpoint: String,
    pub endpoint: String,
    pub project: Option<String>,
    pub location: Option<String>,
}

// LiteLLM's _resolve_vertex_model_from_router: a failure after the overrides returns them, since its except returns the rebound locals
#[must_use]
pub fn resolve_vertex_router_model(
    request: VertexModelRequest<'_>,
    router: Option<&dyn VertexDeploymentLookup>,
    resolution: &ResolutionContext<'_>,
) -> ResolvedVertexModel {
    let Some(lookup) = router else {
        return original_model(&request);
    };
    match lookup.deployment_for_pass_through(request.model_id) {
        Ok(Some(deployment)) => resolve_deployment(request, deployment, resolution),
        Ok(None) => original_model(&request),
        Err(error) => failed_model(
            &request,
            request.project.as_deref(),
            request.location.as_deref(),
            &error,
        ),
    }
}

fn resolve_deployment(
    request: VertexModelRequest<'_>,
    deployment: Deployment,
    resolution: &ResolutionContext<'_>,
) -> ResolvedVertexModel {
    let Deployment {
        litellm_params,
        model_info,
    } = deployment;
    let deployment_model_info = model_info_map(model_info);
    let (project, location) = router_values(&litellm_params, &request);
    let Some(model) = litellm_params.get("model").and_then(non_empty_text) else {
        return ResolvedVertexModel {
            encoded_endpoint: request.encoded_endpoint,
            endpoint: request.endpoint,
            project,
            location,
            deployment_model_info,
        };
    };
    let resolved = match resolve_model(model, request.model_id, resolution) {
        Ok(resolved) => resolved,
        Err(error) => {
            return failed_model(&request, project.as_deref(), location.as_deref(), &error);
        }
    };
    let (encoded_endpoint, endpoint) =
        replace_model(&request, &resolved, project.as_deref(), location.as_deref());
    ResolvedVertexModel {
        encoded_endpoint,
        endpoint,
        project,
        location,
        deployment_model_info,
    }
}

// The router values are read before the provider call, and the blanket except returns them as they stand
fn router_values(
    litellm_params: &JsonObject,
    request: &VertexModelRequest<'_>,
) -> (Option<String>, Option<String>) {
    let project = litellm_params
        .get("vertex_project")
        .and_then(non_empty_text)
        .map(str::to_owned)
        .or_else(|| request.project.clone());
    let location = litellm_params
        .get("vertex_location")
        .and_then(non_empty_text)
        .map(str::to_owned)
        .or_else(|| request.location.clone());
    (project, location)
}

fn original_model(request: &VertexModelRequest<'_>) -> ResolvedVertexModel {
    ResolvedVertexModel {
        encoded_endpoint: request.encoded_endpoint.clone(),
        endpoint: request.endpoint.clone(),
        project: request.project.clone(),
        location: request.location.clone(),
        deployment_model_info: None,
    }
}

// The blanket except: one debug line naming the model and the error, never a key or a credential
fn failed_model<Error>(
    request: &VertexModelRequest<'_>,
    project: Option<&str>,
    location: Option<&str>,
    error: &Error,
) -> ResolvedVertexModel
where
    Error: Display,
{
    debug!(
        model_id = request.model_id,
        error = %error,
        "Error resolving vertex model from router"
    );
    ResolvedVertexModel {
        encoded_endpoint: request.encoded_endpoint.clone(),
        endpoint: request.endpoint.clone(),
        project: project.map(str::to_owned),
        location: location.map(str::to_owned),
        deployment_model_info: None,
    }
}

fn model_info_map(model_info: Option<Json>) -> Option<JsonObject> {
    match model_info {
        Some(Json::Object(object)) => Some(object),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => None,
    }
}

// LiteLLM's get_llm_provider(model=model_from_config): the route injects the populated catalog, since only it resolves a bare model
fn resolve_model(
    model: &str,
    model_id: &str,
    resolution: &ResolutionContext<'_>,
) -> Result<ResolvedModel, ResolutionError> {
    let resolved = resolve(
        &ResolutionRequest {
            model,
            custom_llm_provider: None,
            api_base: None,
            api_key: None,
        },
        resolution,
    )?;
    debug!(
        actual_model = %resolved.model,
        custom_llm_provider = %resolved.provider.slug(),
        model_id,
        "get_llm_provider returned"
    );
    Ok(resolved)
}

// str.replace of every occurrence in both endpoints, so a model id inside the publisher part moves too
fn replace_model(
    request: &VertexModelRequest<'_>,
    resolved: &ResolvedModel,
    project: Option<&str>,
    location: Option<&str>,
) -> (String, String) {
    if resolved.model.is_empty() || resolved.model == request.model_id {
        return (request.encoded_endpoint.clone(), request.endpoint.clone());
    }
    debug!(
        model_id = request.model_id,
        actual_model = %resolved.model,
        provider = %resolved.provider.slug(),
        project = ?project,
        location = ?location,
        "Resolved router model"
    );
    (
        request
            .encoded_endpoint
            .replace(request.model_id, &resolved.model),
        request.endpoint.replace(request.model_id, &resolved.model),
    )
}
