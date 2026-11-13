use std::collections::BTreeMap;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::wire::RowTimestamp;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoRouterSession {
    #[serde(rename = "api_key")]
    pub hashed_api_key: String,
    pub session_id: String,
    pub router_name: String,
    pub router_type: String,
    pub first_turn_at: RowTimestamp,
    pub last_turn_at: RowTimestamp,
    pub last_model: String,
    pub turns: i64,
    #[serde(with = "rust_decimal::serde::float")]
    pub spend: Decimal,
    #[serde(with = "rust_decimal::serde::float")]
    pub saved_spend: Decimal,
    #[serde(
        default = "default_estimated_turns",
        rename = "savings_estimated_turns"
    )]
    pub estimated_turns: i64,
    #[serde(
        default = "default_estimated_actual_spend",
        rename = "savings_estimated_actual_spend",
        with = "rust_decimal::serde::float"
    )]
    pub estimated_actual_spend: Decimal,
    #[serde(
        default = "default_estimated_saved_spend",
        rename = "savings_estimated_saved_spend",
        with = "rust_decimal::serde::float"
    )]
    pub estimated_saved_spend: Decimal,
    #[serde(default, rename = "savings_estimated_baseline_models")]
    pub estimated_baseline_models: BTreeMap<String, i64>,
    #[serde(with = "rust_decimal::serde::float")]
    pub classifier_cost: Decimal,
    pub tier_turns: BTreeMap<String, i64>,
    pub baseline_models: BTreeMap<String, i64>,
}

#[must_use]
pub const fn default_estimated_turns() -> i64 {
    0
}

#[must_use]
pub const fn default_estimated_actual_spend() -> Decimal {
    Decimal::ZERO
}

#[must_use]
pub const fn default_estimated_saved_spend() -> Decimal {
    Decimal::ZERO
}

impl AutoRouterSession {
    #[must_use]
    pub fn baseline_model(&self) -> Option<&str> {
        if self.baseline_models.is_empty() {
            return None;
        }
        if self.estimated_turns < self.turns && self.baseline_models.len() > 1 {
            return None;
        }
        self.baseline_models
            .iter()
            .max_by(|left, right| left.1.cmp(right.1).then_with(|| left.0.cmp(right.0)))
            .map(|(name, _count)| name.as_str())
    }
}
