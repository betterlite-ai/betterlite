use crate::deployment_identity::spec::DeploymentSpec;
use crate::least_busy::guard::InFlightCall;
use crate::least_busy::identity::AttemptFacts;
use crate::router_core::attempt::AttemptResult;
use crate::router_core::route::RouteContext;
use crate::strategy::Strategy;

use super::ResolvedRoute;
use super::clients::acquire_client;

// The attempt one route settled on: the deployment, its client, the count it holds and the bindings the caller dispatches.
pub(super) async fn attempt(
    context: &RouteContext<'_>,
    resolved: ResolvedRoute,
    deployment: DeploymentSpec,
    strategy: Option<Strategy>,
) -> AttemptResult {
    let client = acquire_client(context, &resolved.group, &deployment);
    let least_busy = start_count(context, &resolved.group, &deployment).await;
    AttemptResult {
        group: resolved.group,
        deployment,
        aliases: resolved.aliases,
        strategy,
        client,
        override_pre_call: resolved.override_pre_call,
        bindings: resolved.bindings,
        least_busy,
    }
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
    match service.pre_call(&facts).await {
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
