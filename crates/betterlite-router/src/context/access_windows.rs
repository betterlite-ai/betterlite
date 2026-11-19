mod tests;

use std::future::{Ready, ready};

use crate::access_windows::team::filter_team_based_models;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

pub(super) const CONFIGURED: bool = true;

// filter_team_based_models: the caller's authenticated team narrows the pool before any static filter runs.
pub(super) fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Ready<Result<CandidateSet, RoutingError>> {
    let filtered = filter_team_based_models(
        candidates.deployments,
        route.request.authority,
        route.request.model,
    )
    .map(|deployments| CandidateSet { deployments })
    .map_err(RoutingError::Catalog);
    ready(filtered)
}
