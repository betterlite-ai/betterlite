use crate::deployment_identity::spec::DeploymentSpec;
use crate::router_observations::failure::AsyncFailureEvent;
use crate::router_observations::success::SuccessEvent;
use crate::routing_value_store::RouteKey;

use super::error::LeastBusyError;

// is_batch_retrieve_call_type: the batch retrieve wrappers hand a whole job's usage to the callbacks and hold no in-flight count
const BATCH_RETRIEVE_CALL_TYPES: [&str; 2] = ["aretrieve_batch", "retrieve_batch"];

// the two halves _deployment_ref reads, already pulled out of litellm_params by the B016 attempt event
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AttemptFacts<'facts> {
    pub model_group: Option<&'facts str>,
    pub deployment_id: Option<&'facts str>,
    pub call_type: Option<&'facts str>,
}

impl<'facts> AttemptFacts<'facts> {
    // the success callback's event carries the request facts its request recorded
    #[must_use]
    pub const fn from_success(event: &SuccessEvent<'facts>) -> Self {
        Self {
            model_group: event.request.model_group,
            deployment_id: event.request.deployment_id,
            call_type: event.call_type,
        }
    }

    // the failure callback's event carries the attempt's model group and deployment directly
    #[must_use]
    pub const fn from_failure(event: &AsyncFailureEvent<'facts>) -> Self {
        Self {
            model_group: event.model_group,
            deployment_id: event.deployment_id,
            call_type: event.call_type,
        }
    }
}

// the deployment an attempt picked, as the counter key names it
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeploymentRef {
    pub model_group: String,
    pub deployment_id: String,
}

// _deployment_ref: a batch retrieve, or an attempt missing either half, has no counter to hold
#[must_use]
pub fn deployment_ref(facts: &AttemptFacts<'_>) -> Option<DeploymentRef> {
    if facts
        .call_type
        .is_some_and(|call_type| BATCH_RETRIEVE_CALL_TYPES.contains(&call_type))
    {
        return None;
    }
    let (Some(model_group), Some(deployment_id)) = (facts.model_group, facts.deployment_id) else {
        return None;
    };
    Some(DeploymentRef {
        model_group: model_group.to_owned(),
        deployment_id: deployment_id.to_owned(),
    })
}

// _request_count_key
pub fn request_count_key(
    model_group: &str,
    deployment_id: &str,
) -> Result<RouteKey, LeastBusyError> {
    RouteKey::parse(&format!("{model_group}_request_count:{deployment_id}"))
        .map_err(|source| LeastBusyError::Key { source })
}

// _request_count_keys: one key per deployment, in the order the pool holds them
pub fn request_count_keys(
    model_group: &str,
    deployments: &[DeploymentSpec],
) -> Result<Vec<RouteKey>, LeastBusyError> {
    let mut keys = Vec::with_capacity(deployments.len());
    for (index, deployment) in deployments.iter().enumerate() {
        let deployment_id = deployment
            .id()
            .ok_or(LeastBusyError::MissingDeploymentId { index })?;
        keys.push(request_count_key(model_group, deployment_id)?);
    }
    Ok(keys)
}
