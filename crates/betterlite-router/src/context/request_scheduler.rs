mod tests;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;
use std::future::{Ready, ready};
// Router.scheduler exists on every router, so the stage runs; the queue wraps the call, not the pool
pub(super) const CONFIGURED: bool = true;
// schedule_acompletion polls the queue around the whole route and never filters a deployment out
pub(super) const fn apply(
    candidates: CandidateSet,
    _route: &RouteContext<'_>,
) -> Ready<Result<CandidateSet, RoutingError>> {
    ready(Ok(candidates))
}
