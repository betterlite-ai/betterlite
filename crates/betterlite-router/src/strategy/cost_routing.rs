use std::future::{Ready, ready};
use std::sync::Arc;
use std::time::SystemTime;

use rand::Rng;

use crate::cost_routing::costs::DeploymentCosts;
use crate::cost_routing::selection::Selection;
use crate::cost_routing::service::CostRouter;
use crate::cost_routing::window::ColdWindows;
use crate::router_core::error::RoutingError;
use crate::strategy::StrategyContext;

mod tests;

// A route carries no model cost map or stored windows, so this arm ranks each deployment by its own mirrored prices over a cold minute
pub(super) fn select<R>(
    context: &StrategyContext<'_>,
    _random: &mut R,
) -> Ready<Result<usize, RoutingError>>
where
    R: Rng,
{
    let costs = DeploymentCosts::from_deployments(context.deployments);
    let router = CostRouter::new(ColdWindows, Arc::new(SystemTime::now));
    let request = Selection {
        group: context.route.request.model,
        deployments: context.deployments,
        // B365 already counted the request; a request it could not count holds the pinned fallback of no tokens
        input_tokens: context
            .route
            .request
            .facts
            .input_token_count
            .map_or(0, |tokens| i64::try_from(tokens).unwrap_or(0)),
    };
    // the pinned selector answers None when no deployment is left, and the pipeline turns an index past the pool into no deployment
    let selected = router
        .select(&costs, &request)
        .map(|choice| choice.unwrap_or(context.deployments.len()));
    ready(selected.map_err(|error| RoutingError::Metric {
        field: error.field(),
    }))
}
