use async_trait::async_trait;
use betterlite_core::proxy::carried_budget_state::{
    KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};
use rust_decimal::Decimal;

// LiteLLM catches whatever a lookup raises and logs its text, so the failure carries a message
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LookupError {
    pub message: String,
}

pub struct KeyBudgetLookup<'request> {
    pub key: Option<&'request str>,
    pub key_alias: Option<&'request str>,
    pub response_cost: Decimal,
    pub max_budget: Option<Decimal>,
    pub spend: Option<Decimal>,
    pub carried: Option<KeyBudgetSnapshot>,
}

pub struct TeamBudgetLookup<'request> {
    pub team: Option<&'request str>,
    pub team_alias: Option<&'request str>,
    pub spend: Option<Decimal>,
    pub max_budget: Option<Decimal>,
    pub response_cost: Decimal,
    pub carried: Option<TeamBudgetSnapshot>,
}

pub struct UserBudgetLookup<'request> {
    pub user_id: Option<&'request str>,
    pub spend: Option<Decimal>,
    pub max_budget: Option<Decimal>,
    pub response_cost: Decimal,
    pub carried: Option<UserBudgetSnapshot>,
    pub user_email: Option<&'request str>,
}

pub struct OrgBudgetLookup<'request> {
    pub org_id: Option<&'request str>,
    pub response_cost: Decimal,
    pub carried: Option<OrgBudgetSnapshot>,
    pub org_alias: Option<&'request str>,
}

pub struct CustomerBudgetLookup<'request> {
    pub end_user_id: Option<&'request str>,
    pub response_cost: Decimal,
}

// one method per entity, so the tests record which lookups the fan-out started and with what
#[async_trait]
pub trait RequestBudgetLookups: Send + Sync {
    async fn key(&self, lookup: KeyBudgetLookup<'_>) -> Result<(), LookupError>;
    async fn team(&self, lookup: TeamBudgetLookup<'_>) -> Result<(), LookupError>;
    async fn user(&self, lookup: UserBudgetLookup<'_>) -> Result<(), LookupError>;
    async fn org(&self, lookup: OrgBudgetLookup<'_>) -> Result<(), LookupError>;
    async fn customer(&self, lookup: CustomerBudgetLookup<'_>) -> Result<(), LookupError>;
}
