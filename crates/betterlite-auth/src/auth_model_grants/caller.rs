use std::collections::BTreeMap;

use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::grant::key_model_aliases;
use crate::auth_model_patterns::user_model::can_user_call_model;
use crate::auth_request_models::RequestedModels;
use crate::entity_model_access::error::EntityModelAccessError;
use crate::entity_model_access::team::{TeamAccessRequest, can_team_access_model};
use crate::resolved_key::auth::UserAPIKeyAuth;
use crate::team_budget_admission::member_scope::check_team_member_model_access;
use crate::team_budget_admission::sources::Membership;
use crate::team_budget_admission::{TeamAdmission, TeamRequest};

use super::access_source::AgentAccessSource;
use super::error::AgentModelGrantsError;
use super::requested;

// _check_agent_caller_model_access: the invoking team's grant then its member scope, else the invoking user
pub async fn check_agent_caller(
    model: &RequestedModels,
    token: Option<&UserAPIKeyAuth>,
    source: Option<&dyn AgentAccessSource>,
    admission: &TeamAdmission<'_>,
    context: &ModelAccessContext<'_>,
) -> Result<(), AgentModelGrantsError> {
    if requested::is_falsey(model) {
        return Ok(());
    }
    let Some(token) = token else {
        return Ok(());
    };
    // agent_caller_auth is empty for every key that is not acting for a caller
    if token.extra.agent_caller.is_none() {
        return Ok(());
    }
    let Some(source) = source else {
        return Err(AgentModelGrantsError::Unavailable);
    };
    let Some(mut caller) = source.caller_auth(token).await? else {
        return Ok(());
    };
    let aliases = key_model_aliases(&token.view.token.aliases.0);
    // the caller team is loaded before the user, and a loaded team ends the check after the member scope
    if let Some(loaded) = source.caller_team(token).await? {
        let team = &loaded.team;
        let no_team_aliases = BTreeMap::new();
        let request = TeamAccessRequest {
            team: Some(team),
            team_aliases: &no_team_aliases,
            key_aliases: &aliases,
            lookup: None,
        };
        can_team_access_model(model, &request, context).await?;
        // the member scope reads the aliases off the token it is handed, as the source forwards the agent key's
        caller.view.token.aliases = token.view.token.aliases.clone();
        let request = TeamRequest {
            team: Some(team),
            token: Some(&caller),
            user_email: None,
            model,
            membership: Membership::Unloaded,
        };
        check_team_member_model_access(admission, &request)
            .await
            .map_err(|error| {
                AgentModelGrantsError::Entity(Box::new(EntityModelAccessError::from_member(error)))
            })?;
        return Ok(());
    }
    let Some(caller_user) = source.caller_user(token).await? else {
        return Ok(());
    };
    AgentModelGrantsError::gate(can_user_call_model(
        model,
        Some(&caller_user),
        &aliases,
        context,
    ))
}
