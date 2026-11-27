use std::collections::BTreeMap;

use betterlite_auth::auth_model_patterns::ALL_TEAM_MODELS;
use betterlite_auth::auth_model_patterns::decision::ModelAccessContext;
use betterlite_auth::auth_request_models::RequestedModels;
use betterlite_auth::auth_team_lookup::lookup::TeamLookup;
use betterlite_auth::entity_model_access::grant::text_models;
use betterlite_auth::entity_model_access::team::{TeamAccessRequest, can_team_access_model};
use betterlite_auth::management_routes::key_routes::KeyManagementRoute;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::team_rows::team::{LiteLLMTeamTable, TeamBase};

use crate::team_access::access::{TEAM_ADMIN_ONLY, TeamAccess};
use crate::team_access::caller::TeamCaller;
use crate::team_access::permissions::member_route_allowed;
use crate::team_access::roster::roster_member;

use super::error::KeyReassignmentError;

// the services a team move reads: B800's caller policy, B886's model grant and its access-group lookup
pub struct TeamChangeDeps<'deps> {
    pub access: &'deps TeamAccess,
    pub teams: Option<&'deps TeamLookup>,
    pub context: &'deps ModelAccessContext<'deps>,
}

// validate_key_team_change: models, then rates, then the owner's membership, then the initiator's authority
pub async fn validate_key_team_change(
    key: &LiteLLMVerificationToken,
    team: &LiteLLMTeamTable,
    change_initiated_by: &TeamCaller,
    deps: &TeamChangeDeps<'_>,
) -> Result<(), KeyReassignmentError> {
    let base = &team.fields.base;
    check_models(key, team, deps).await?;
    check_rates(key, base)?;
    let member = roster_member(&base.members_with_roles, key.user_id.as_deref());
    if let Some(owner) = key.user_id.as_deref()
        && member.is_none()
    {
        return Err(KeyReassignmentError::OwnerNotInTeam {
            user_id: owner.to_owned(),
            team_id: base.team_id.clone(),
        });
    }
    if deps
        .access
        .allows(change_initiated_by, base, &TEAM_ADMIN_ONLY)
        .await?
    {
        return Ok(());
    }
    let owner_role = member.map(|found| found.role.clone());
    match member_route_allowed(
        owner_role.as_ref(),
        base,
        KeyManagementRoute::KeyUpdate.value(),
    ) {
        Ok(true) => Ok(()),
        Ok(false) => Err(KeyReassignmentError::InitiatorNotAdmin {
            user_id: change_initiated_by
                .user_id
                .clone()
                .unwrap_or_else(|| String::from("None")),
            team_id: base.team_id.clone(),
        }),
        Err(error) => Err(KeyReassignmentError::from(error)),
    }
}

// the sentinel means "whatever the team allows", so the team's grant never sees it
async fn check_models(
    key: &LiteLLMVerificationToken,
    team: &LiteLLMTeamTable,
    deps: &TeamChangeDeps<'_>,
) -> Result<(), KeyReassignmentError> {
    let aliases = BTreeMap::new();
    for model in text_models(&key.models) {
        if model == ALL_TEAM_MODELS {
            continue;
        }
        let request = TeamAccessRequest {
            team: Some(team),
            team_aliases: &aliases,
            key_aliases: &aliases,
            lookup: deps.teams,
        };
        can_team_access_model(&RequestedModels::One(model), &request, deps.context).await?;
    }
    Ok(())
}

// key.tpm_limit gates both checks; a zero team ceiling means unset, and a zero key rpm means unset too
fn check_rates(
    key: &LiteLLMVerificationToken,
    team: &TeamBase<String>,
) -> Result<(), KeyReassignmentError> {
    let Some(key_tpm) = key.tpm_limit else {
        return Ok(());
    };
    if let Some(team_tpm) = team.tpm_limit.filter(|limit| *limit != 0)
        && key_tpm > team_tpm
    {
        return Err(KeyReassignmentError::TpmExceedsTeam { key_tpm, team_tpm });
    }
    if let Some(team_rpm) = team.rpm_limit.filter(|limit| *limit != 0)
        && let Some(key_rpm) = key.rpm_limit.filter(|limit| *limit != 0)
        && key_rpm > team_rpm
    {
        return Err(KeyReassignmentError::RpmExceedsTeam { key_rpm, team_rpm });
    }
    Ok(())
}
