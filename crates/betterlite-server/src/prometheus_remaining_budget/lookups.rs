use async_trait::async_trait;
use betterlite_core::proxy::carried_budget_state::{
    KeyBudgetSnapshot, OrgBudgetSnapshot, TeamBudgetSnapshot, UserBudgetSnapshot,
};
use rust_decimal::Decimal;
use serde::Serialize;

// LiteLLM catches whatever a lookup raises and logs its text, so the failure carries a message
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct LookupError {
    pub message: String,
}

// field names are the keyword arguments LiteLLM's _set_*_budget_metrics_after_api_request take
#[derive(Serialize)]
pub struct KeyBudgetLookup<'request> {
    pub user_api_key: Option<&'request str>,
    pub user_api_key_alias: Option<&'request str>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_cost: Decimal,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub key_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub key_spend: Option<Decimal>,
    pub carried: Option<KeyBudgetSnapshot>,
}

#[derive(Serialize)]
pub struct TeamBudgetLookup<'request> {
    pub user_api_team: Option<&'request str>,
    pub user_api_team_alias: Option<&'request str>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub team_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_cost: Decimal,
    pub carried: Option<TeamBudgetSnapshot>,
}

#[derive(Serialize)]
pub struct UserBudgetLookup<'request> {
    pub user_id: Option<&'request str>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_spend: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision_option")]
    pub user_max_budget: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_cost: Decimal,
    pub carried: Option<UserBudgetSnapshot>,
    pub user_email: Option<&'request str>,
}

#[derive(Serialize)]
pub struct OrgBudgetLookup<'request> {
    pub org_id: Option<&'request str>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    pub response_cost: Decimal,
    pub carried: Option<OrgBudgetSnapshot>,
    pub org_alias: Option<&'request str>,
}

#[derive(Serialize)]
pub struct CustomerBudgetLookup<'request> {
    pub end_user_id: Option<&'request str>,
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
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
