use std::future::{Ready, ready};

use rand::Rng;

use crate::lar1_routing::metadata::Lar1Metadata;
use crate::lar1_routing::strategy::{Lar1Strategy, pick};
use crate::router_core::error::RoutingError;
use crate::strategy::StrategyContext;

mod tests;

// A route carries no metadata or thresholds, so this arm uses the declared defaults
pub(super) fn select<R>(
    context: &StrategyContext<'_>,
    _random: &mut R,
) -> Ready<Result<usize, RoutingError>>
where
    R: Rng,
{
    let target = Lar1Strategy::default().target(&Lar1Metadata::default());
    // an empty pool keeps index 0, which the pipeline turns into its no-deployment error
    let index = pick(context.deployments, target).map_or(0, |(index, _exact)| index);
    ready(Ok(index))
}
