mod tests;

use std::future::{Ready, ready};

use crate::cooldown_state::filter::without_cooling_down;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

pub(super) const CONFIGURED: bool = true;

// Router._filter_cooldown_deployments: every deployment the cooldown cache reports cooling down leaves the pool
pub(super) fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Ready<Result<CandidateSet, RoutingError>> {
    let cooling = route.services.cooldown_list();
    ready(Ok(CandidateSet {
        deployments: without_cooling_down(candidates.deployments, &cooling),
    }))
}
