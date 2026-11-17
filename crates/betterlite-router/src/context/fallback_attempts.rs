use std::future::{Ready, ready};

use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

mod tests;

pub(super) const CONFIGURED: bool = false;

pub(super) fn apply(
    _candidates: CandidateSet,
    _route: &RouteContext<'_>,
) -> Ready<Result<CandidateSet, RoutingError>> {
    ready(Err(RoutingError::NotBuilt {
        entry: "fallback_attempts",
    }))
}
