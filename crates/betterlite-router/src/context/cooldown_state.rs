mod tests;

use std::future::{Ready, ready};

use crate::cooldown_state::filter::{HealthRouting, without_cooling_down, without_unhealthy};
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

pub(super) const CONFIGURED: bool = true;

// the health filter runs before the cooldown filter, and a route is never a health probe
pub(super) fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Ready<Result<CandidateSet, RoutingError>> {
    let health = route.services.health_inputs();
    let routing = HealthRouting {
        enabled: health.enabled,
        has_allowed_fails_policy: health.has_allowed_fails_policy,
        scoped_groups: health.scoped_groups.as_ref(),
    };
    let deployments = without_unhealthy(candidates.deployments, &health.unhealthy, routing, false);
    let cooling = route.services.cooldown_list();
    ready(Ok(CandidateSet {
        deployments: without_cooling_down(deployments, &cooling),
    }))
}
