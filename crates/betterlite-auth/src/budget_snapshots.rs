use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;

pub trait BudgetSnapshot: Serialize + for<'de> Deserialize<'de> + Sized {
    const METADATA_KEYS: &'static [&'static str];

    fn metadata_entries(&self) -> BTreeMap<String, Box<RawValue>> {
        serde_json::to_value(self).map_or_else(
            |_| BTreeMap::new(),
            |value| serde_json::from_value(value).unwrap_or_default(),
        )
    }

    #[must_use]
    fn from_metadata(metadata: &BTreeMap<String, Box<RawValue>>) -> Option<Self> {
        if Self::METADATA_KEYS
            .iter()
            .any(|key| !metadata.contains_key(*key))
        {
            return None;
        }
        let value = serde_json::to_value(metadata).ok()?;
        serde_json::from_value(value).ok()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeyBudgetSnapshot {
    #[serde(rename = "user_api_key_budget_reset_at", alias = "budget_reset_at")]
    pub budget_reset_at: Option<DateTime<Utc>>,
}

impl BudgetSnapshot for KeyBudgetSnapshot {
    const METADATA_KEYS: &'static [&'static str] = &["user_api_key_budget_reset_at"];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TeamBudgetSnapshot {
    #[serde(
        rename = "user_api_key_team_budget_reset_at",
        alias = "budget_reset_at"
    )]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(rename = "user_api_key_team_table_max_budget", alias = "max_budget")]
    pub max_budget: Option<Decimal>,
}

impl BudgetSnapshot for TeamBudgetSnapshot {
    const METADATA_KEYS: &'static [&'static str] = &[
        "user_api_key_team_budget_reset_at",
        "user_api_key_team_table_max_budget",
    ];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UserBudgetSnapshot {
    #[serde(
        rename = "user_api_key_user_budget_reset_at",
        alias = "budget_reset_at"
    )]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(rename = "user_api_key_user_table_max_budget", alias = "max_budget")]
    pub max_budget: Option<Decimal>,
    #[serde(rename = "user_api_key_user_alias", alias = "user_alias")]
    pub user_alias: Option<String>,
}

impl BudgetSnapshot for UserBudgetSnapshot {
    const METADATA_KEYS: &'static [&'static str] = &[
        "user_api_key_user_budget_reset_at",
        "user_api_key_user_table_max_budget",
        "user_api_key_user_alias",
    ];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrgBudgetSnapshot {
    #[serde(rename = "user_api_key_org_spend", alias = "spend")]
    pub spend: Decimal,
    #[serde(rename = "user_api_key_org_max_budget", alias = "max_budget")]
    pub max_budget: Option<Decimal>,
}

impl BudgetSnapshot for OrgBudgetSnapshot {
    const METADATA_KEYS: &'static [&'static str] =
        &["user_api_key_org_spend", "user_api_key_org_max_budget"];
}

mod tests;
