use std::collections::BTreeMap;

use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::resolved_key::auth::UserAPIKeyAuth;

use super::money::Money;

// the Litellm_EntityType values the key checks schedule under
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventGroup {
    Key,
    TeamMember,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetAlertKind {
    TokenBudget,
    SoftBudget,
    MaxBudgetAlert,
}

// the CallInfo LiteLLM builds for a budget alert; a nonfinite budget stays nonfinite instead of being rounded
#[derive(Clone, Debug)]
pub struct BudgetAlert {
    pub kind: BudgetAlertKind,
    pub spend: Decimal,
    pub max_budget: Option<Money>,
    pub soft_budget: Option<Money>,
    pub token: Option<SecretString>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub organization_id: Option<String>,
    pub user_email: Option<String>,
    pub key_alias: Option<String>,
    pub event_group: EventGroup,
    pub max_budget_alert_emails: Option<BTreeMap<String, Vec<String>>>,
}

// B4025's sink owns the task that sends the alert; the auth checks only hand it the record
pub trait BudgetAlertSink: Send + Sync {
    fn schedule(&self, alert: BudgetAlert);
}

pub(crate) fn schedule(sink: Option<&dyn BudgetAlertSink>, alert: BudgetAlert) {
    if let Some(alert_sink) = sink {
        alert_sink.schedule(alert);
    }
}

// the identity fields every key alert carries, read from the request's token and the budget overlay
pub(crate) struct KeyAlertFields<'auth> {
    pub(crate) auth: &'auth UserAPIKeyAuth,
    pub(crate) max_budget: Option<Money>,
    pub(crate) soft_budget: Option<Money>,
    pub(crate) user_email: Option<&'auth str>,
    pub(crate) team_alias: Option<&'auth str>,
}

impl KeyAlertFields<'_> {
    pub(crate) fn build(
        &self,
        kind: BudgetAlertKind,
        spend: Decimal,
        max_budget_alert_emails: Option<BTreeMap<String, Vec<String>>>,
    ) -> BudgetAlert {
        let token = &self.auth.view.token;
        BudgetAlert {
            kind,
            spend,
            max_budget: self.max_budget,
            soft_budget: self.soft_budget,
            token: token.token.clone(),
            user_id: token.user_id.clone(),
            team_id: token.team_id.clone(),
            team_alias: self.team_alias.map(str::to_owned),
            organization_id: token.org_id.clone(),
            user_email: self.user_email.map(str::to_owned),
            key_alias: token.key_alias.clone(),
            event_group: EventGroup::Key,
            max_budget_alert_emails,
        }
    }
}
