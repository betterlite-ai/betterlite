use betterlite_db::team_rows::team::LiteLLMTeamTable;
use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::resolved_key::auth::UserAPIKeyAuth;

// the budget_alerts type the team checks queue; both carry event_group TEAM and user_email None
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TeamAlertKind {
    TeamBudget,
    SoftBudget,
}

// the CallInfo _team_max_budget_check and _team_soft_budget_check build; the identity comes from the token, not the team row
#[derive(Clone, Debug)]
pub struct TeamAlert {
    pub kind: TeamAlertKind,
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub soft_budget: Option<Decimal>,
    pub token: Option<SecretString>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub organization_id: Option<String>,
    pub key_alias: Option<String>,
    pub alert_emails: Option<Vec<String>>,
}

// B751's BudgetAlert has no TEAM group, team_budget type or alert_emails, so the team alerts get their own supervised sink
pub trait TeamAlertSink: Send + Sync {
    fn schedule(&self, alert: TeamAlert);
}

// the hard alert carries the team maximum only; the soft alert adds the soft budget, the key alias and the recipients
pub(crate) fn token_alert(
    kind: TeamAlertKind,
    token: &UserAPIKeyAuth,
    team: &LiteLLMTeamTable,
    spend: Decimal,
    alert_emails: Option<Vec<String>>,
) -> TeamAlert {
    let key = &token.view.token;
    let (soft_budget, key_alias) = match kind {
        TeamAlertKind::TeamBudget => (None, None),
        TeamAlertKind::SoftBudget => (team.fields.base.soft_budget, key.key_alias.clone()),
    };
    TeamAlert {
        kind,
        spend,
        max_budget: team.fields.base.max_budget,
        soft_budget,
        token: key.token.clone(),
        user_id: key.user_id.clone(),
        team_id: key.team_id.clone(),
        team_alias: token.view.team_alias.clone(),
        organization_id: key.org_id.clone(),
        key_alias,
        alert_emails,
    }
}
