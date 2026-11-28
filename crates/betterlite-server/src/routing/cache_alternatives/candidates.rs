use betterlite_anthropic::cache_aware_routing::AnthropicCacheRouting;
use betterlite_anthropic::cache_aware_routing::deployment::PredictionDeployment;
use betterlite_anthropic::cache_aware_routing::services::PredictionServices;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_prompts::message_content::Message;
use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::router_predicates::JsonMap;

use crate::routing::cache_alternatives::error::CacheAlternativesError;
use crate::routing::cache_alternatives::host::CacheRoutingHost;

// _Candidate: one model group, the tier it competes under and the single deployment that serves it.
#[derive(Clone, Debug)]
pub(crate) struct Candidate {
    pub(crate) model: String,
    pub(crate) tier: String,
    pub(crate) deployment: DeploymentSpec,
}

// _candidate: a model group qualifies only when exactly one deployment serves it and that deployment can be predicted for.
pub(crate) fn candidate<Host: CacheRoutingHost + ?Sized>(
    host: &Host,
    services: &PredictionServices<'_>,
    tier: &str,
    model: &str,
    request_kwargs: &JsonMap,
) -> Result<Option<Candidate>, CacheAlternativesError> {
    let deployments = host
        .deployments_for_request(model, request_kwargs)
        .map_err(|source| CacheAlternativesError::Deployments {
            model: model.to_owned(),
            source,
        })?;
    let [deployment] = deployments.as_slice() else {
        return Ok(None);
    };
    let prediction = PredictionDeployment::read(&deployment.litellm_params, &deployment.model_info);
    if prediction.blocked
        || prediction.deployment_id.is_empty()
        || !AnthropicCacheRouting::supports(&prediction, services)
    {
        return Ok(None);
    }
    Ok(Some(Candidate {
        model: model.to_owned(),
        tier: tier.to_owned(),
        deployment: deployment.clone(),
    }))
}

// _available: the caller may call the model group, and one healthy deployment is the candidate's own.
pub(crate) async fn available<Host: CacheRoutingHost + ?Sized>(
    host: &Host,
    candidate: &Candidate,
    caller: &UserAPIKeyAuth,
    request_kwargs: &JsonMap,
    messages: Option<&[Message]>,
) -> Result<bool, CacheAlternativesError> {
    host.can_key_call_model(caller, &candidate.model)
        .await
        .map_err(|source| CacheAlternativesError::Access {
            model: candidate.model.clone(),
            source,
        })?;
    let healthy = host
        .healthy_deployments(&candidate.model, messages, request_kwargs)
        .await
        .map_err(|source| CacheAlternativesError::Healthy {
            model: candidate.model.clone(),
            source,
        })?;
    let wanted = PredictionDeployment::read(
        &candidate.deployment.litellm_params,
        &candidate.deployment.model_info,
    )
    .deployment_id;
    Ok(healthy
        .iter()
        .any(|deployment| deployment.id() == Some(wanted)))
}
