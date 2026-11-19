use crate::context::RouteStage;
use crate::deployment_identity::spec::DeploymentSpec;
use crate::least_busy::guard::InFlightCall;
use crate::least_busy::identity::AttemptFacts;
use crate::router_core::attempt::AttemptResult;
use crate::router_core::error::RoutingError;
use crate::router_core::route::{ResolvedRoute, RouteContext, no_deployment_error, stage_single};
use crate::strategy::Strategy;

use super::clients::acquire_client;

// The attempt one route settled on: the deployment, its client, the count it holds and the bindings the caller dispatches.
pub(super) async fn attempt(
    context: &RouteContext<'_>,
    resolved: ResolvedRoute,
    picked: DeploymentSpec,
    strategy: Option<Strategy>,
) -> Result<AttemptResult, RoutingError> {
    // async_routing_strategy_pre_call_checks: the picked deployment passes the router's checks before it is called or counted
    let Some(deployment) = stage_single(context, RouteStage::ModelRateLimits, picked).await? else {
        return Err(no_deployment_error(context, &resolved));
    };
    let client = acquire_client(context, &resolved.group, &deployment);
    let least_busy = start_count(context, &resolved.group, &deployment).await;
    Ok(AttemptResult {
        group: resolved.group,
        deployment,
        aliases: resolved.aliases,
        strategy,
        client,
        override_pre_call: resolved.override_pre_call,
        bindings: resolved.bindings,
        least_busy,
    })
}

// log_pre_api_call: the attempt holds one in-flight count until its guard releases it, and an unwritable counter only warns
async fn start_count(
    context: &RouteContext<'_>,
    model_group: &str,
    deployment: &DeploymentSpec,
) -> Option<InFlightCall> {
    let service = context.services.least_busy()?;
    let facts = AttemptFacts {
        model_group: Some(model_group),
        deployment_id: deployment.id(),
        call_type: None,
    };
    // the counter write is deep enough that the router's future layout only stays in budget boxed
    match Box::pin(service.pre_call(&facts)).await {
        Ok(started) => {
            if let Some(warning) = started.warning {
                tracing::warn!(
                    ?warning,
                    "least-busy routing could not update the in-flight count"
                );
            }
            started.call
        }
        Err(error) => {
            tracing::warn!(%error, "least-busy routing cannot count this attempt");
            None
        }
    }
}
