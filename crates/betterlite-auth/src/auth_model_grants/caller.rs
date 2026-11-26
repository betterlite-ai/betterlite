use std::collections::BTreeMap;

use betterlite_db::team_rows::team::LiteLLMTeamTable;

use crate::auth_model_patterns::decision::ModelAccessContext;
use crate::auth_model_patterns::grant::key_model_aliases;
use crate::auth_model_patterns::user_model::can_user_call_model;
use crate::auth_request_models::RequestedModels;
use crate::auth_team_lookup::lookup::TeamLookup;
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
    teams: Option<&TeamLookup>,
    admission: &TeamAdmission<'_>,
    context: &ModelAccessContext<'_>,
) -> Result<(), AgentModelGrantsError> {
    if requested::is_falsey(model) {
        return Ok(());
    }
    let Some(key) = token else {
        return Ok(());
    };
    // agent_caller_auth is empty for every key that is not acting for a caller
    if key.extra.agent_caller.is_none() {
        return Ok(());
    }
    let Some(access) = source else {
        return Err(AgentModelGrantsError::Unavailable);
    };
    // agent_caller_auth first: a key that resolves to no caller skips rather than widening the check
    let Some(mut caller) = access.caller_auth(key).await? else {
        return Ok(());
    };
    // the caller team is loaded before the user, and a loaded team ends the check after the member scope
    if let Some(loaded) = access.caller_team(key).await? {
        // the member scope reads the aliases off the token it is handed, as the source forwards the agent key's
        caller.view.token.aliases = key.view.token.aliases.clone();
        return check_caller_team(model, &caller, &loaded.team, teams, admission, context).await;
    }
    // no team echoed: the invoking user's own model list decides, and a missing row narrows nothing
    let Some(caller_user) = access.caller_user(key).await? else {
        return Ok(());
    };
    let aliases = key_model_aliases(&key.view.token.aliases.0);
    AgentModelGrantsError::gate(can_user_call_model(
        model,
        Some(&caller_user),
        &aliases,
        context,
    ))
}

// can_team_access_model then the member scope: the agent key's aliases reach both, the team has none
async fn check_caller_team(
    model: &RequestedModels,
    caller: &UserAPIKeyAuth,
    team: &LiteLLMTeamTable,
    teams: Option<&TeamLookup>,
    admission: &TeamAdmission<'_>,
    context: &ModelAccessContext<'_>,
) -> Result<(), AgentModelGrantsError> {
    let aliases = key_model_aliases(&caller.view.token.aliases.0);
    let no_team_aliases = BTreeMap::new();
    let grant = TeamAccessRequest {
        team: Some(team),
        team_aliases: &no_team_aliases,
        key_aliases: &aliases,
        lookup: teams,
    };
    can_team_access_model(model, &grant, context).await?;
    let request = TeamRequest {
        team: Some(team),
        token: Some(caller),
        user_email: None,
        model,
        membership: Membership::Unloaded,
    };
    check_team_member_model_access(admission, &request)
        .await
        .map_err(|error| {
            AgentModelGrantsError::Entity(Box::new(EntityModelAccessError::from_member(error)))
        })
}
