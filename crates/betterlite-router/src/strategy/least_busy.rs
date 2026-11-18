mod tests;

use rand::Rng;

use crate::router_core::error::RoutingError;
use crate::strategy::StrategyContext;

// get_available_deployments and async_get_available_deployments: the deployment holding the fewest in-flight calls
pub(super) async fn select<R>(
    context: &StrategyContext<'_>,
    _random: &mut R,
) -> Result<usize, RoutingError>
where
    R: Rng,
{
    let Some(service) = context.route.services.least_busy() else {
        // a router that keeps no counters holds no call in flight, so the first deployment weighs the least
        return Ok(0);
    };
    let selection = service
        .select(context.model_group, context.deployments)
        .await?;
    if let Some(warning) = &selection.warning {
        // _warn_unreadable: the shared tier failed and this worker's own counts answered instead
        tracing::warn!(
            ?warning,
            "least-busy routing fell back to this worker's own counts"
        );
    }
    Ok(selection.index.unwrap_or(0))
}
