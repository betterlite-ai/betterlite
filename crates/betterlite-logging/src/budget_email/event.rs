use std::collections::BTreeMap;

use rust_decimal::Decimal;
use secrecy::SecretString;

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum WebhookEventType {
    BudgetCrossed,
    MaxBudgetAlert,
    SoftBudgetCrossed,
    ThresholdCrossed,
    ProjectedLimitExceeded,
    KeyCreated,
    KeyRotated,
    InternalUserCreated,
    SpendTracked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum EntityType {
    Key,
    User,
    EndUser,
    Team,
    TeamMember,
    Organization,
    OrganizationMember,
    Project,
    Tag,
    Agent,
    ModelAccessGroup,
    Proxy,
}

#[derive(Clone, Debug)]
pub struct WebhookEvent {
    pub event: WebhookEventType,
    pub event_message: String,
    pub event_group: EntityType,
    pub spend: Decimal,
    pub max_budget: Option<Decimal>,
    pub soft_budget: Option<Decimal>,
    pub token: Option<SecretString>,
    pub customer_id: Option<String>,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub team_alias: Option<String>,
    pub organization_id: Option<String>,
    pub user_email: Option<String>,
    pub key_alias: Option<String>,
    pub projected_exceeded_date: Option<String>,
    pub projected_spend: Option<Decimal>,
    pub alert_emails: Option<Vec<String>>,
    pub max_budget_alert_emails: Option<BTreeMap<String, Vec<String>>>,
}
