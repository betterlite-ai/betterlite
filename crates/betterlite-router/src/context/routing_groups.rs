use std::future::{Ready, ready};

use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;
use crate::routing_groups::priority::prioritized;

pub(super) const CONFIGURED: bool = true;

pub(super) fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Ready<Result<CandidateSet, RoutingError>> {
    ready(Ok(narrowed(candidates, route)))
}

// Router._get_routing_group_deployments: a callable group serves only its members, in priority order.
fn narrowed(candidates: CandidateSet, route: &RouteContext<'_>) -> CandidateSet {
    if route.request.specific_deployment {
        return candidates;
    }
    let Some(group) = route.services.routing_group(route.request.model) else {
        return candidates;
    };
    CandidateSet {
        deployments: candidates
            .deployments
            .into_iter()
            .filter(|deployment| {
                group
                    .models
                    .iter()
                    .any(|member| member == &deployment.model_name)
            })
            .map(|deployment| prioritized(group, &deployment))
            .collect(),
    }
}
