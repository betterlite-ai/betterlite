mod tests;

use std::future::{Ready, ready};

use betterlite_models::decisions::Json;
use betterlite_models::lookup::cost_map_keys::cost_map_backend_model;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::generalization_integration::model_generalizations::ModelGeneralizations;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

pub(super) const CONFIGURED: bool = true;

// the rule half of get_model_info at selection time, before any filter reads a deployment's model_info
pub(super) fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Ready<Result<CandidateSet, RoutingError>> {
    let Some(generalizations) = route.services.model_generalizations() else {
        return ready(Ok(candidates));
    };
    let deployments = candidates
        .deployments
        .into_iter()
        .map(|deployment| filled(deployment, generalizations))
        .collect();
    ready(Ok(CandidateSet { deployments }))
}

fn filled(
    mut deployment: DeploymentSpec,
    generalizations: &ModelGeneralizations,
) -> DeploymentSpec {
    let Some(model) = cost_map_backend_model(&deployment.litellm_params, &deployment.model_info)
        .map(str::to_owned)
    else {
        return deployment;
    };
    let declared = declared_provider(&deployment).map(str::to_owned);
    let resolved = generalizations.provider(&model, declared.as_deref());
    let proposed = match resolved.answer.as_deref() {
        Some(provider) => generalizations.fill_missing(&model, provider),
        None => generalizations.capabilities(&model),
    };
    for warning in resolved.warnings.iter().chain(&proposed.warnings) {
        tracing::warn!(?warning, "a generalization rule search was skipped");
    }
    if let Some(info) = proposed.answer {
        for (key, value) in info {
            // the engine keeps a rule's model_info as JSON text, which always reads back into a value
            let Ok(json) = serde_json::from_str::<Json>(value.get()) else {
                continue;
            };
            // the deployment's own keys and explicit nulls win, as get_model_info merges the rule entry
            deployment.model_info.entry(key).or_insert(json);
        }
    }
    // a deployment without a provider takes the first routing rule's, as get_llm_provider does
    if declared.is_none()
        && let Some(provider) = resolved.answer
    {
        deployment
            .model_info
            .entry(String::from("litellm_provider"))
            .or_insert(Json::Text(provider));
    }
    deployment
}

// litellm_provider on the deployment wins over custom_llm_provider, and an empty one names no provider
fn declared_provider(deployment: &DeploymentSpec) -> Option<&str> {
    let from_info = match deployment.model_info.get("litellm_provider") {
        Some(Json::Text(provider)) if !provider.is_empty() => Some(provider.as_str()),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    };
    from_info.or_else(
        || match deployment.litellm_params.get("custom_llm_provider") {
            Some(Json::Text(provider)) if !provider.is_empty() => Some(provider.as_str()),
            Some(
                Json::Null
                | Json::Bool(_)
                | Json::Number(_)
                | Json::Text(_)
                | Json::List(_)
                | Json::Object(_),
            )
            | None => None,
        },
    )
}
