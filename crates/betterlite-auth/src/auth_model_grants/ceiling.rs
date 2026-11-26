use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::{ModelAccessObject, ProxyErrorTypes};
use betterlite_error::proxy_exception::ExceptionCode;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;

use crate::agent_access_groups::ceiling::AgentAccessGroupCeiling;
use crate::auth_model_patterns::decision::{ModelAccessContext, can_object_call_model};
use crate::auth_model_patterns::denial::model_denied_client_message;
use crate::auth_model_patterns::grant::{ModelGrantPattern, key_model_aliases, resolve_team_alias};
use crate::auth_request_models::RequestedModels;
use crate::resolved_key::auth::UserAPIKeyAuth;

use super::access_source::AgentAccessSource;
use super::error::AgentModelGrantsError;
use super::requested;

// _check_agent_access_group_model_access: a falsey model, token or agent id skips; every ceiling must allow
pub async fn check_agent_ceiling(
    model: &RequestedModels,
    token: Option<&UserAPIKeyAuth>,
    source: Option<&dyn AgentAccessSource>,
    context: &ModelAccessContext<'_>,
) -> Result<(), AgentModelGrantsError> {
    if requested::is_falsey(model) {
        return Ok(());
    }
    let Some(key) = token else {
        return Ok(());
    };
    let Some(agent_id) = key
        .view
        .token
        .agent_id
        .as_deref()
        .filter(|agent_id| !agent_id.is_empty())
    else {
        return Ok(());
    };
    let Some(access) = source else {
        return Err(AgentModelGrantsError::Unavailable);
    };
    let ceilings = ceilings_for(key, agent_id, access).await?;
    check_every_ceiling(model, key, agent_id, &ceilings, context)
}

// the managed record wins; only an unmanaged agent asks the registry resolver, and no group means no ceiling
async fn ceilings_for(
    key: &UserAPIKeyAuth,
    agent_id: &str,
    access: &dyn AgentAccessSource,
) -> Result<Vec<AgentAccessGroupCeiling>, AgentModelGrantsError> {
    let Some(policy) = access.managed_policy(Some(key)) else {
        return Ok(access
            .unmanaged_ceiling(agent_id)
            .await?
            .map_or_else(Vec::new, |ceiling| vec![ceiling]));
    };
    Ok(access.managed_ceilings(&policy).await?)
}

// the team alias resolves once, then each ceiling's models are checked; an empty ceiling denies the request
fn check_every_ceiling(
    model: &RequestedModels,
    key: &UserAPIKeyAuth,
    agent_id: &str,
    ceilings: &[AgentAccessGroupCeiling],
    context: &ModelAccessContext<'_>,
) -> Result<(), AgentModelGrantsError> {
    let team_aliases = key
        .view
        .team_model_aliases
        .as_ref()
        .map(|payload| key_model_aliases(&payload.0))
        .unwrap_or_default();
    let dispatched = resolve_team_alias(
        model,
        &team_aliases,
        key.view.token.team_id.as_deref(),
        context.view,
    );
    for ceiling in ceilings {
        if ceiling.models.is_empty() {
            return Err(AgentModelGrantsError::Denied(Box::new(no_models_denial(
                model, agent_id,
            ))));
        }
        check_ceiling(ceiling, &dispatched, key, context)?;
    }
    Ok(())
}

// _can_object_call_model with object_type agent: the ceiling's models and the key's own aliases
fn check_ceiling(
    ceiling: &AgentAccessGroupCeiling,
    dispatched: &RequestedModels,
    key: &UserAPIKeyAuth,
    context: &ModelAccessContext<'_>,
) -> Result<(), AgentModelGrantsError> {
    let grant = ModelGrantPattern {
        // the resolver returns a sorted set, which is the source's `sorted(ceiling.models)`
        models: ceiling.models.iter().cloned().collect(),
        team_model_aliases: BTreeMap::new(),
        key_model_aliases: key_model_aliases(&key.view.token.aliases.0),
        team_id: key.view.token.team_id.clone(),
    };
    AgentModelGrantsError::gate(can_object_call_model(
        dispatched,
        &grant,
        ModelAccessObject::Agent,
        context,
        0,
    ))
}

// the client sees the safe model-access message; the group diagnostic stays on the exception
fn no_models_denial(
    model: &RequestedModels,
    agent_id: &str,
    // ast-grep-ignore: no-long-names
) -> ModelAccessDeniedProxyException {
    // ast-grep-ignore: no-long-names
    ModelAccessDeniedProxyException::new(
        model_denied_client_message(&requested::requested_text(model)),
        format!("agent {agent_id} access groups grant no models"),
        ProxyErrorTypes::AgentModelAccessDenied.to_string(),
        Some("model".to_owned()),
        ExceptionCode::Status(403),
    )
}
