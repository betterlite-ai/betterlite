mod tests;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};

use crate::deployment_affinity::FilterRequest;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

// the check is built from the router config, so the stage runs and the services say whether one exists
pub(super) const CONFIGURED: bool = true;

// async_callback_filter_deployments runs DeploymentAffinityCheck before PromptCachingDeploymentCheck
pub(super) async fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Result<CandidateSet, RoutingError> {
    let Some(affinity) = route.services.deployment_affinity() else {
        return Ok(candidates);
    };
    // LiteLLM resolves an alias before the callbacks run, so the pool's own model group names the check
    let Some(first) = candidates.deployments.first() else {
        return Ok(candidates);
    };
    let kwargs = route.policy.prompt.request_kwargs.map(router_json);
    let selection = FilterRequest {
        model: first.model_name.as_str(),
        deployments: &candidates.deployments,
        request_kwargs: kwargs.as_ref(),
        target_order: route.request.facts.target_order,
    };
    let deployments = affinity.filter_deployments(&selection).await?;
    Ok(CandidateSet { deployments })
}

// the request arguments the check reads, in the router's own JSON type
fn router_json(object: &Object) -> JsonObject {
    object
        .iter()
        .map(|(key, value)| (key.clone(), Json::from(value)))
        .collect()
}
