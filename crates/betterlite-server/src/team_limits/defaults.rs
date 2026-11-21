use betterlite_auth::user_roles::UserRole;
use betterlite_config::sections::team_limits::Settings;
use serde::{Deserialize, Serialize};

use crate::key_fields::values::Amount;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub user_id: Option<String>,
    pub user_role: Option<UserRole>,
}

// the spellings are the keys of litellm_settings.default_team_params, the names _get_default_team_param looks up
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamParamField {
    Models,
    OrganizationId,
    MaxBudget,
    BudgetDuration,
    TpmLimit,
    RpmLimit,
    TeamMemberPermissions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum TeamParamValue {
    Text(String),
    Integer(i64),
    Money(Amount),
    Texts(Vec<String>),
}

#[must_use]
pub fn default_team_param(settings: &Settings, field: TeamParamField) -> Option<TeamParamValue> {
    let params = settings.default_team_params.as_ref()?;
    match field {
        TeamParamField::Models => params
            .models
            .as_ref()
            .map(|models| TeamParamValue::Texts(models.clone())),
        TeamParamField::OrganizationId => params.organization_id.clone().map(TeamParamValue::Text),
        TeamParamField::MaxBudget => params
            .max_budget
            .map(|budget| TeamParamValue::Money(Amount(budget))),
        TeamParamField::BudgetDuration => params.budget_duration.clone().map(TeamParamValue::Text),
        TeamParamField::TpmLimit => params.tpm_limit.map(TeamParamValue::Integer),
        TeamParamField::RpmLimit => params.rpm_limit.map(TeamParamValue::Integer),
        TeamParamField::TeamMemberPermissions => params
            .team_member_permissions
            .as_ref()
            .map(|routes| TeamParamValue::Texts(routes.clone())),
    }
}

// LiteLLM accepts the caller here and never reads it: availability depends only on the configured list.
#[must_use]
pub fn is_available_team(
    team_id: &str,
    _caller: &Caller,
    available_teams: Option<&[String]>,
) -> bool {
    let Some(teams) = available_teams else {
        return false;
    };
    teams.iter().any(|team| team == team_id)
}

#[must_use]
pub fn should_auto_add_team_creator(caller: &Caller, settings: &Settings) -> bool {
    // an SSO user without an id in the token cannot become a member; an empty id still can
    if caller.user_id.is_none() {
        return false;
    }
    if caller.user_role != Some(UserRole::ProxyAdmin) {
        return true;
    }
    !settings.disable_admin_auto_add
}
