use std::future::{Ready, ready};

use rand::Rng;

use crate::router_core::error::RoutingError;
use crate::strategy::StrategyContext;
use crate::usage_routing::select::{WindowCounts, select_counts};

mod tests;

// A route reaches no usage window store, so every deployment enters the window at zero, the cold-cache base case
// get_available_deployments starts from; the tokens the request declares are what a tpm limit can still rule out.
pub(super) fn select<R>(
    context: &StrategyContext<'_>,
    _random: &mut R,
) -> Ready<Result<usize, RoutingError>>
where
    R: Rng,
{
    let tokens = context
        .route
        .request
        .facts
        .input_token_count
        .map_or(0, |count| i64::try_from(count).unwrap_or(i64::MAX));
    match select_counts(&WindowCounts::default(), context.deployments, tokens) {
        Ok(Some(index)) => ready(Ok(index)),
        // an index past the pool is how the pipeline reaches its no-deployment error
        Ok(None) => ready(Ok(context.deployments.len())),
        Err(error) => ready(Err(error)),
    }
}
