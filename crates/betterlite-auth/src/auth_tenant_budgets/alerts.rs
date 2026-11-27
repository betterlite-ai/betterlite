use rust_decimal::Decimal;
use secrecy::SecretString;

use crate::resolved_key::auth::UserAPIKeyAuth;

// the budget_alerts type each tenant check queues
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TenantAlertKind {
    ProjectBudget,
    OrganizationBudget,
    SoftBudget,
}

// LiteLLM's Litellm_EntityType.PROJECT and ORGANIZATION event groups
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TenantEventGroup {
    Project,
    Organization,
}

// the CallInfo a tenant check builds; the identity comes from the request's token, never the entity row
#[derive(Clone, Debug)]
pub struct TenantAlert {
    pub kind: TenantAlertKind,
    pub event_group: TenantEventGroup,
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub soft_budget: Option<Decimal>,
    pub token: Option<SecretString>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub organization_id: Option<String>,
}

// B4025's sink owns the task that sends the alert; the auth checks only hand it the record
pub trait TenantAlertSink: Send + Sync {
    fn schedule(&self, alert: TenantAlert);
}

pub(crate) fn schedule(sink: Option<&dyn TenantAlertSink>, alert: TenantAlert) {
    if let Some(alert_sink) = sink {
        alert_sink.schedule(alert);
    }
}

// what a check adds to the token identity: the entity's kind, event group and spend facts
pub(crate) struct TenantAlertFields {
    pub(crate) kind: TenantAlertKind,
    pub(crate) event_group: TenantEventGroup,
    pub(crate) spend: Decimal,
    pub(crate) max_budget: Option<Decimal>,
    pub(crate) soft_budget: Option<Decimal>,
    pub(crate) organization_id: Option<String>,
}

// the fields every tenant alert carries from the token, joined to the fields the caller supplies
pub(crate) fn identity_alert(token: &UserAPIKeyAuth, fields: TenantAlertFields) -> TenantAlert {
    let key = &token.view.token;
    TenantAlert {
        kind: fields.kind,
        event_group: fields.event_group,
        spend: fields.spend,
        max_budget: fields.max_budget,
        soft_budget: fields.soft_budget,
        token: key.token.clone(),
        user_id: key.user_id.clone(),
        team_id: key.team_id.clone(),
        team_alias: token.view.team_alias.clone(),
        organization_id: fields.organization_id,
    }
}
