use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_availability::policy::{
    AllowRequestsOnOutage, PolicyError, allows_requests_during_outage,
};
use betterlite_db::team_rows::archived::LiteLLMTeamTableCachedObj;
use betterlite_db::team_rows::team::{LiteLLMTeamTable, Nullable, TeamBase, TeamFields};

use crate::auth_team_lookup::error::TeamLookupError;
use crate::resolved_key::auth::UserAPIKeyAuth;

pub const UI_TEAM_ID: &str = "litellm-dashboard";

#[derive(Debug, thiserror::Error)]
pub enum TeamFromTokenError {
    #[error("the token carries no team_id")]
    MissingTeamId,
    #[error("the token's team_models entry {index} is not a string")]
    NonStringModel { index: usize },
}

// _team_obj_from_token: the team fields the token itself already carries, and nothing the row alone knows
pub fn team_obj_from_token(
    token: &UserAPIKeyAuth,
) -> Result<LiteLLMTeamTableCachedObj, TeamFromTokenError> {
    let base = team_identity_from_token(token)?;
    let spend = token.view.team_spend;
    let model_max_budget = token
        .view
        .team_model_max_budget
        .as_ref()
        .map(|payload| payload.0.clone());
    let fields = TeamFields {
        base,
        spend,
        max_parallel_requests: None,
        budget_reset_at: None,
        model_id: None,
        model_spend: Nullable(Some(BTreeMap::new())),
        model_max_budget: Nullable(model_max_budget),
        policies: None,
        allow_team_guardrail_config: Nullable(Some(false)),
        litellm_model_table: None,
        object_permission: None,
        object_permission_id: token.view.team_object_permission_id.clone(),
        updated_at: None,
        created_at: None,
    };
    Ok(LiteLLMTeamTableCachedObj {
        team: LiteLLMTeamTable { fields },
        last_refreshed_at: None,
    })
}

fn team_identity_from_token(
    token: &UserAPIKeyAuth,
) -> Result<TeamBase<String>, TeamFromTokenError> {
    let Some(team_id) = token.view.token.team_id.clone() else {
        return Err(TeamFromTokenError::MissingTeamId);
    };
    Ok(TeamBase {
        team_alias: None,
        team_id,
        organization_id: None,
        admins: Vec::new(),
        members: Vec::new(),
        members_with_roles: Vec::new(),
        team_member_permissions: None,
        metadata: token
            .view
            .team_metadata
            .as_ref()
            .map(|payload| payload.0.clone()),
        tpm_limit: token.view.team_tpm_limit,
        rpm_limit: token.view.team_rpm_limit,
        tpd_limit: token.view.team_tpd_limit,
        max_budget: token.view.team_max_budget,
        soft_budget: token.view.team_soft_budget,
        budget_duration: None,
        budget_limits: None,
        models: team_models(&token.view.team_models)?,
        blocked: token.view.team_blocked,
        router_settings: None,
        access_group_ids: None,
        default_team_member_models: None,
    })
}

fn team_models(models: &[JsonData]) -> Result<Vec<String>, TeamFromTokenError> {
    let mut names = Vec::with_capacity(models.len());
    for (index, model) in models.iter().enumerate() {
        let JsonData::String(name) = model else {
            return Err(TeamFromTokenError::NonStringModel { index });
        };
        names.push(name.clone());
    }
    Ok(names)
}

// _token_can_vouch_for_team: the dashboard sentinel always does, a deleted team never does, and a recorded grant does before the outage setting is read
pub fn token_can_vouch_for_team(
    token: &UserAPIKeyAuth,
    lookup_error: &TeamLookupError,
    outage: &AllowRequestsOnOutage,
) -> Result<bool, PolicyError> {
    if token.view.token.team_id.as_deref() == Some(UI_TEAM_ID) {
        return Ok(true);
    }
    if matches!(lookup_error, TeamLookupError::TeamNotFound { .. }) {
        return Ok(false);
    }
    if !token.view.team_models.is_empty() {
        return Ok(true);
    }
    allows_requests_during_outage(outage)
}
