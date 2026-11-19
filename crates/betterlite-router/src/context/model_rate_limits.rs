mod tests;

use betterlite_tokenizer::reservation::RequestBody;

use crate::io_token_reservations::error::IoPreCallError;
use crate::model_rate_limits::error::ModelRateLimitError;
use crate::router_core::candidates::CandidateSet;
use crate::router_core::error::RoutingError;
use crate::router_core::route::RouteContext;

// the check is built from the router config, so the stage runs and the services say whether one exists
pub(super) const CONFIGURED: bool = true;

// async_routing_strategy_pre_call_checks: the one deployment about to be called passes the registered check
pub(super) async fn apply(
    candidates: CandidateSet,
    route: &RouteContext<'_>,
) -> Result<CandidateSet, RoutingError> {
    let Some(check) = route.services.model_rate_limits() else {
        return Ok(candidates);
    };
    let mut deployments = Vec::with_capacity(candidates.deployments.len());
    for deployment in candidates.deployments {
        let mut request = RequestBody::new();
        let failure = check.async_pre_call_check(&deployment, &mut request).await;
        if allowed(failure) {
            deployments.push(deployment);
        }
    }
    Ok(CandidateSet { deployments })
}

// pre_call_check raises a rate limit and returns the deployment for every other failure it cannot read
fn allowed(failure: Result<(), ModelRateLimitError>) -> bool {
    match failure {
        Err(ModelRateLimitError::Exceeded(_)) => false,
        Err(ModelRateLimitError::IoToken(error)) => !matches!(error, IoPreCallError::RateLimit(_)),
        Ok(()) | Err(ModelRateLimitError::Limit { .. }) => true,
    }
}
