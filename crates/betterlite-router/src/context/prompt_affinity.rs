mod tests;

use crate::prompt_affinity::service::FilterRequest;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

// the check is built from the router config, so the stage runs and the services say whether one exists
pub(super) const CONFIGURED: bool = true;

// async_callback_filter_deployments: the check replaces the pool before the pre-call checks run
pub(super) async fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Result<CandidateSet, RoutingError> {
    let Some(affinity) = route.services.prompt_affinity() else {
        return Ok(candidates);
    };
    // _target_order is the only stand-down a route can carry; no priority group reaches it
    let selection = FilterRequest {
        model: route.request.model,
        deployments: &candidates.deployments,
        messages: route.policy.prompt.messages,
        request_kwargs: route.policy.prompt.request_kwargs,
        priority_group: false,
        target_order: route.request.facts.target_order,
    };
    let deployments = affinity.filter_deployments(&selection).await?;
    Ok(CandidateSet { deployments })
}
