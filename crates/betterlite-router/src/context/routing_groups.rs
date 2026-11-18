use std::future::{Ready, ready};

use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;
use crate::routing_groups::priority::prioritized;

pub(super) const CONFIGURED: bool = true;

// Router._get_routing_group_deployments: a callable group serves only its members, in priority order.
pub(super) fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Ready<Result<CandidateSet, RoutingError>> {
    if route.request.specific_deployment {
        return ready(Ok(candidates));
    }
    let Some(group) = route.services.routing_group(route.request.model) else {
        return ready(Ok(candidates));
    };
    ready(Ok(CandidateSet {
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
    }))
}
