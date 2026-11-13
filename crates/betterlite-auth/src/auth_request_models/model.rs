use super::catalog::{CatalogError, Collaborators, DeploymentRecord, ModelCatalog};
use super::sources::{
    RequestSnapshot, extract_model_candidates_from_request,
    request_dispatched_to_pass_through_endpoint,
};
use super::{RequestedModels, format_model_candidates};
use crate::resource_hints::azure::router_model_in_endpoint;
use crate::resource_hints::bedrock::{
    COUNT_TOKENS_MARKERS, extract_model as bedrock_extract_model,
};
use crate::resource_hints::nvidia::{DeploymentHint, model_group_in_path};

pub fn get_model_from_request(
    request: &RequestSnapshot<'_>,
    collaborators: &Collaborators<'_>,
) -> Result<RequestedModels, CatalogError> {
    if request_dispatched_to_pass_through_endpoint(&request.dispatch) {
        return Ok(RequestedModels::None);
    }
    let candidates = extract_model_candidates_from_request(request, collaborators)?;
    let model = route_or_candidate_model(request.route, format_model_candidates(candidates));
    route_family_model(request.route, model, collaborators.catalog)
}

fn route_or_candidate_model(route: &str, model: RequestedModels) -> RequestedModels {
    if !matches!(model, RequestedModels::None) {
        return model;
    }
    if let Some(from_path) = deployment_path_model(route) {
        return RequestedModels::One(from_path);
    }
    let lowered = route.to_lowercase();
    if !lowered.starts_with("/vertex") {
        if let Some(from_path) = google_models_path(route) {
            return RequestedModels::One(from_path);
        }
        if let Some(from_path) = root_models_path(route) {
            return RequestedModels::One(from_path);
        }
    }
    if lowered.starts_with("/vertex")
        && let Some(from_path) = vertex_models_path(route)
    {
        return RequestedModels::One(from_path);
    }
    RequestedModels::None
}

fn route_family_model(
    route: &str,
    model: RequestedModels,
    catalog: Option<&dyn ModelCatalog>,
) -> Result<RequestedModels, CatalogError> {
    let lowered = route.to_lowercase();
    if lowered.starts_with("/bedrock") {
        return Ok(bedrock_route_model(route).map_or(model, RequestedModels::One));
    }
    if lowered.starts_with("/azure/") || lowered.starts_with("/azure_ai/") {
        let azure = azure_route_model(route, catalog)?;
        return Ok(azure.map_or(model, RequestedModels::One));
    }
    if lowered.starts_with("/nvidia_nim/") {
        let nvidia = nvidia_route_model(route, catalog)?;
        return Ok(nvidia.map_or(model, RequestedModels::One));
    }
    Ok(model)
}

fn bedrock_route_model(route: &str) -> Option<String> {
    let endpoint = strip_prefix_ignore_ascii_case(route, "/bedrock/").unwrap_or(route);
    if COUNT_TOKENS_MARKERS
        .iter()
        .any(|marker| endpoint.contains(marker))
    {
        return None;
    }
    bedrock_extract_model(endpoint).ok()
}

fn azure_route_model(
    route: &str,
    catalog: Option<&dyn ModelCatalog>,
) -> Result<Option<String>, CatalogError> {
    let Some(catalog) = catalog else {
        return Ok(None);
    };
    let endpoint = strip_prefix_ignore_ascii_case(route, "/azure_ai/")
        .or_else(|| strip_prefix_ignore_ascii_case(route, "/azure/"))
        .unwrap_or(route);
    let model_names: Vec<String> = catalog.model_names()?.into_iter().collect();
    Ok(router_model_in_endpoint(endpoint, &model_names))
}

fn nvidia_route_model(
    route: &str,
    catalog: Option<&dyn ModelCatalog>,
) -> Result<Option<String>, CatalogError> {
    let Some(catalog) = catalog else {
        return Ok(None);
    };
    let deployments = catalog.model_list()?;
    let hints: Vec<DeploymentHint<'_>> = deployments.iter().map(deployment_hint).collect();
    Ok(model_group_in_path(route, &hints))
}

fn deployment_hint(deployment: &DeploymentRecord) -> DeploymentHint<'_> {
    DeploymentHint {
        model_name: &deployment.model_name,
        custom_llm_provider: deployment.custom_llm_provider.as_deref(),
        model: deployment.model.as_deref(),
    }
}

fn deployment_path_model(route: &str) -> Option<String> {
    let remaining = route.strip_prefix("/openai/deployments/")?;
    let model = remaining.split('/').next()?;
    (!model.is_empty()).then(|| model.to_owned())
}

fn after_google_prefix(text: &str) -> Option<&str> {
    ["/v1beta/models/", "/beta/models/"]
        .iter()
        .find_map(|prefix| text.strip_prefix(*prefix))
}

fn model_before_colon(text: &str) -> Option<&str> {
    let (model, _suffix) = text.split_once(':')?;
    (!model.is_empty()).then_some(model)
}

fn model_until_colon(text: &str) -> Option<&str> {
    let model = text.split(':').next()?;
    (!model.is_empty()).then_some(model)
}

fn google_models_path(route: &str) -> Option<String> {
    let mut remaining = route;
    loop {
        let index = remaining.find('/')?;
        let candidate = remaining.get(index..)?;
        if let Some(model) = after_google_prefix(candidate).and_then(model_before_colon) {
            return Some(model.to_owned());
        }
        remaining = remaining.get(index.saturating_add(1)..)?;
    }
}

fn root_models_path(route: &str) -> Option<String> {
    route
        .strip_prefix("/models/")
        .and_then(model_before_colon)
        .map(str::to_owned)
}

fn vertex_models_path(route: &str) -> Option<String> {
    let (_head, remaining) = route.split_once("/models/")?;
    model_until_colon(remaining).map(str::to_owned)
}

fn strip_prefix_ignore_ascii_case<'text>(text: &'text str, prefix: &str) -> Option<&'text str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| text.get(prefix.len()..).unwrap_or_default())
}
