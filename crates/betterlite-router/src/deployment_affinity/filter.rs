use std::collections::BTreeSet;

use betterlite_core::api::response_id_codecs::response::get_model_id_from_response_id;
use betterlite_models::model_registration::JsonObject;

use super::DeploymentAffinity;
use super::affinity_inputs;
use super::error::DeploymentAffinityError;
use super::json::previous_response_id;
use super::read::pinned_model_id;
use crate::affinity_claims::identity::{
    AffinityFlag, affinity_cache_key, find_deployment_by_model_id, session_affinity_cache_key,
    session_id_from_request_kwargs, user_key_from_request_kwargs,
};
use crate::deployment_identity::spec::DeploymentSpec;

// the previous-response step: the flag is on and the prior response names a deployment in the pool
pub(super) fn responses_deployment<'pool>(
    flags: &BTreeSet<AffinityFlag>,
    kwargs: &JsonObject,
    deployments: &'pool [DeploymentSpec],
) -> Result<Option<&'pool DeploymentSpec>, DeploymentAffinityError> {
    if !flags.contains(&AffinityFlag::ResponsesApi) {
        return Ok(None);
    }
    let Some(previous) = previous_response_id(kwargs)? else {
        return Ok(None);
    };
    let Some(model_id) = get_model_id_from_response_id(Some(previous)) else {
        return Ok(None);
    };
    let Some(deployment) = find_deployment_by_model_id(deployments, &model_id) else {
        return Ok(None);
    };
    tracing::debug!(
        model_id,
        "DeploymentAffinityCheck: previous_response_id pinning"
    );
    Ok(Some(deployment))
}

// the session step: the flag or a marker activates the pin, and the key carries the caller's scope
pub(super) async fn session_deployment<'pool>(
    service: &DeploymentAffinity,
    flags: &BTreeSet<AffinityFlag>,
    kwargs: &JsonObject,
    deployments: &'pool [DeploymentSpec],
    model_group: &str,
) -> Result<Option<&'pool DeploymentSpec>, DeploymentAffinityError> {
    let inputs = affinity_inputs(flags, kwargs)?;
    if !inputs.session_active {
        return Ok(None);
    }
    let Some(session_id) = session_id_from_request_kwargs(kwargs)? else {
        return Ok(None);
    };
    let scope = inputs.user_key.as_deref();
    let Ok(key) = session_affinity_cache_key(model_group, &session_id, scope) else {
        return Ok(None);
    };
    let Some(model_id) = pinned_model_id(service, &key).await else {
        return Ok(None);
    };
    let Some(deployment) = find_deployment_by_model_id(deployments, &model_id) else {
        return Ok(None);
    };
    tracing::debug!(
        model_id,
        session_id,
        "DeploymentAffinityCheck: session-id affinity hit"
    );
    Ok(Some(deployment))
}

// the caller step: only with the deployment flag, a caller identity and a pin in the pool
pub(super) async fn caller_selection(
    service: &DeploymentAffinity,
    flags: &BTreeSet<AffinityFlag>,
    kwargs: &JsonObject,
    deployments: &[DeploymentSpec],
    model_group: &str,
) -> Result<Vec<DeploymentSpec>, DeploymentAffinityError> {
    if !flags.contains(&AffinityFlag::Deployment) {
        return Ok(deployments.to_vec());
    }
    let Some(caller) = user_key_from_request_kwargs(kwargs)? else {
        return Ok(deployments.to_vec());
    };
    let Ok(key) = affinity_cache_key(model_group, &caller) else {
        return Ok(deployments.to_vec());
    };
    let Some(model_id) = pinned_model_id(service, &key).await else {
        return Ok(deployments.to_vec());
    };
    let Some(deployment) = find_deployment_by_model_id(deployments, &model_id) else {
        tracing::debug!(
            model_id,
            "DeploymentAffinityCheck: pinned deployment not in healthy_deployments"
        );
        return Ok(deployments.to_vec());
    };
    tracing::debug!(model_id, "DeploymentAffinityCheck: caller affinity hit");
    Ok(vec![deployment.clone()])
}
