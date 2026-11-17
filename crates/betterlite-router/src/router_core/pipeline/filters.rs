use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_core::route::RouteContext;
use crate::static_eligibility::environment::active_in_environment;
use crate::static_eligibility::error::EligibilityError;
use crate::static_eligibility::params::filter_supported_params;
use crate::static_eligibility::tokens::{ContextWindowExceeded, filter_context_window};
use crate::static_eligibility::web_search::filter_web_search;

// The static gates before any dynamic stage: the environment the model was registered in, then web search.
pub(super) fn static_pool(
    context: &RouteContext<'_>,
    deployments: Vec<DeploymentSpec>,
) -> Result<Vec<DeploymentSpec>, EligibilityError> {
    let mut causes = Vec::new();
    let mut active = Vec::new();
    for deployment in deployments {
        if active_in_environment(&deployment.model_info, context.request.environment)? {
            active.push(deployment);
        }
    }
    Ok(filter_web_search(
        active,
        &context.request.facts.tools,
        &mut causes,
    )?)
}

// Router._pre_call_checks over the pool the callbacks kept: the context window, then the supported response_format.
pub(super) fn pre_call_pool(
    context: &RouteContext<'_>,
    group: &str,
    deployments: Vec<DeploymentSpec>,
) -> Result<Vec<DeploymentSpec>, EligibilityError> {
    if !context.policy.enable_pre_call_checks {
        return Ok(deployments);
    }
    let facts = context.request.facts;
    if facts.messages.is_none() && facts.input.is_none() {
        return Ok(deployments);
    }
    let mut causes = Vec::new();
    let mut rejected = String::new();
    let mut pool = deployments;
    if !facts.compaction_pending {
        let window = filter_context_window(
            group,
            pool,
            facts.input_token_count,
            context.limits,
            &mut causes,
        );
        pool = window.deployments;
        rejected = window.rejected;
    }
    let supported = filter_supported_params(
        pool,
        facts,
        context.policy.drop_params,
        context.params,
        &mut causes,
    );
    if supported.is_empty() && !rejected.is_empty() {
        return Err(EligibilityError::ContextWindow(ContextWindowExceeded {
            causes: rejected,
        }));
    }
    Ok(supported)
}
