use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tokio_postgres::types::ToSql;

pub const FIELD_ORDER: [&str; 20] = [
    "api_key",
    "session_id",
    "router_name",
    "router_type",
    "model",
    "turn_at",
    "total_tokens",
    "spend",
    "saved_spend",
    "classifier_cost",
    "covered",
    "cache_hit",
    "cache_ttl_seconds",
    "cache_touched",
    "tier",
    "baseline_model",
    "savings_estimated_turns",
    "savings_estimated_actual_spend",
    "savings_estimated_saved_spend",
    "user_id",
];

#[derive(Clone, Serialize, Deserialize)]
pub struct AutoRouterTurnTransaction {
    pub api_key: String,
    pub session_id: String,
    pub router_name: String,
    pub router_type: String,
    pub model: String,
    pub turn_at: DateTime<Utc>,
    pub total_tokens: i64,
    pub spend: Decimal,
    pub saved_spend: Decimal,
    pub classifier_cost: Decimal,
    pub covered: TurnFlag,
    pub cache_hit: TurnFlag,
    pub cache_ttl_seconds: Option<i64>,
    pub cache_touched: TurnFlag,
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub baseline_model: Option<String>,
    #[serde(default)]
    pub savings_estimated_turns: i64,
    #[serde(default)]
    pub savings_estimated_actual_spend: Decimal,
    #[serde(default)]
    pub savings_estimated_saved_spend: Decimal,
    #[serde(default)]
    pub user_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnFlag(pub bool);

#[must_use]
pub fn upsert_params(transaction: &AutoRouterTurnTransaction) -> Vec<Box<dyn ToSql + Sync + Send>> {
    vec![
        Box::new(transaction.api_key.clone()),
        Box::new(transaction.session_id.clone()),
        Box::new(transaction.router_name.clone()),
        Box::new(transaction.router_type.clone()),
        Box::new(transaction.model.clone()),
        Box::new(transaction.turn_at.to_rfc3339()),
        Box::new(transaction.total_tokens),
        Box::new(transaction.spend),
        Box::new(transaction.saved_spend),
        Box::new(transaction.classifier_cost),
        Box::new(i64::from(transaction.covered.0)),
        Box::new(i64::from(transaction.cache_hit.0)),
        Box::new(transaction.cache_ttl_seconds),
        Box::new(i64::from(transaction.cache_touched.0)),
        Box::new(transaction.tier.clone()),
        Box::new(transaction.baseline_model.clone()),
        Box::new(transaction.savings_estimated_turns),
        Box::new(transaction.savings_estimated_actual_spend),
        Box::new(transaction.savings_estimated_saved_spend),
        Box::new(transaction.user_id.clone()),
    ]
}
