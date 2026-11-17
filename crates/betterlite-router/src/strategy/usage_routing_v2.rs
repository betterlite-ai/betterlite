use std::future::{Ready, ready};

use rand::Rng;

use crate::router_core::error::RoutingError;
use crate::strategy::StrategyContext;

mod tests;

pub(super) fn select<R>(
    _context: &StrategyContext<'_>,
    _random: &mut R,
) -> Ready<Result<usize, RoutingError>>
where
    R: Rng,
{
    ready(Err(RoutingError::NotBuilt {
        entry: "usage_routing_v2",
    }))
}
