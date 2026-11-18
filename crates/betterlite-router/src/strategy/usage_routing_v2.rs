use std::future::{Ready, ready};

use rand::Rng;

use crate::router_core::error::RoutingError;
use crate::strategy::StrategyContext;
use crate::usage_routing_v2::selection::Selection;

mod tests;

// A route carries no counter store, so this arm answers what the pinned selector answers to a cache that holds no
// window value: every deployment counts as unused and the limits leave only the cold pool.
pub(super) fn select<R>(
    context: &StrategyContext<'_>,
    random: &mut R,
) -> Ready<Result<usize, RoutingError>>
where
    R: Rng,
{
    let selection = Selection {
        group: context.route.request.model,
        deployments: context.deployments,
        // B365 already counted the request; a request it could not count falls back to no tokens
        input_tokens: context
            .route
            .request
            .facts
            .input_token_count
            .map_or(0, |tokens| i64::try_from(tokens).unwrap_or(0)),
        tpm: None,
        rpm: None,
    };
    // a None answer becomes an index past the pool, which the pipeline turns into its own no-deployment error
    let selected = selection
        .choose(random)
        .map(|choice| choice.unwrap_or(context.deployments.len()));
    ready(selected.map_err(|error| RoutingError::Metric {
        field: error.field(),
    }))
}
