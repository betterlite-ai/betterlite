use std::collections::BTreeMap;

use betterlite_error::proxy_error_types::{ModelAccessObject, ProxyErrorTypes};
use betterlite_error::proxy_exception::ExceptionCode;
// ast-grep-ignore: no-long-names
use betterlite_error::proxy_exception::model_access_denied::ModelAccessDeniedProxyException;

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
    let Some(token) = token else {
        return Ok(());
    };
    let Some(agent_id) = token
        .view
        .token
        .agent_id
        .as_deref()
        .filter(|agent_id| !agent_id.is_empty())
    else {
        return Ok(());
    };
    let Some(source) = source else {
        return Err(AgentModelGrantsError::Unavailable);
    };
    // the managed record wins; only an unmanaged agent asks the registry resolver
    let managed = source.managed_policy(Some(token));
    let ceilings = match managed {
        Some(policy) => source.managed_ceilings(&policy).await?,
        None => match source.unmanaged_ceiling(agent_id).await? {
            Some(ceiling) => vec![ceiling],
            None => Vec::new(),
        },
    };
    let team_aliases = token
        .view
        .team_model_aliases
        .as_ref()
        .map(|payload| key_model_aliases(&payload.0))
        .unwrap_or_default();
    let dispatched = resolve_team_alias(
        model,
        &team_aliases,
        token.view.token.team_id.as_deref(),
        context.view,
    );
    let targets = key_model_aliases(&token.view.token.aliases.0);
    for ceiling in ceilings {
        if ceiling.models.is_empty() {
            return Err(AgentModelGrantsError::Denied(Box::new(no_models_denial(
                model, agent_id,
            ))));
        }
        let grant = ModelGrantPattern {
            // the resolver returns a sorted set, which is the source's `sorted(ceiling.models)`
            models: ceiling.models.iter().cloned().collect(),
            team_model_aliases: BTreeMap::new(),
            key_model_aliases: targets.clone(),
            team_id: token.view.token.team_id.clone(),
        };
        AgentModelGrantsError::gate(can_object_call_model(
            &dispatched,
            &grant,
            ModelAccessObject::Agent,
            context,
            0,
        ))?;
    }
    Ok(())
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
