use crate::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::de::value::MapDeserializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub trait BudgetSnapshot: Serialize + DeserializeOwned {
    const METADATA_KEYS: &'static [&'static str];

    fn metadata_entries(&self) -> Result<BTreeMap<String, AgentJson>, serde_json::Error> {
        serde_json::from_value(serde_json::to_value(self)?)
    }

    // An invalid carried record means the logger falls back to its own lookup, so the error is not surfaced
    #[must_use]
    fn from_metadata(metadata: &BTreeMap<String, AgentJson>) -> Option<Self> {
        let carried = Self::METADATA_KEYS
            .iter()
            .filter_map(|key| metadata.get_key_value(*key))
            .map(|(key, value)| (key.as_str(), value));
        Self::deserialize(MapDeserializer::<_, serde_json::Error>::new(carried)).ok()
    }
}

// LiteLLM treats missing optional carrier fields like explicit null values.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeyBudgetSnapshot {
    #[serde(
        default,
        rename = "user_api_key_budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<DateTime<Utc>>,
}

impl BudgetSnapshot for KeyBudgetSnapshot {
    const METADATA_KEYS: &'static [&'static str] = &["user_api_key_budget_reset_at"];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TeamBudgetSnapshot {
    #[serde(
        default,
        rename = "user_api_key_team_budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        rename = "user_api_key_team_table_max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
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
        default,
        rename = "user_api_key_user_budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(
        default,
        rename = "user_api_key_user_table_max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub max_budget: Option<Decimal>,
    #[serde(
        default,
        rename = "user_api_key_user_alias",
        deserialize_with = "Option::deserialize"
    )]
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
    #[serde(
        rename = "user_api_key_org_spend",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub spend: Decimal,
    #[serde(
        default,
        rename = "user_api_key_org_max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub max_budget: Option<Decimal>,
}

impl BudgetSnapshot for OrgBudgetSnapshot {
    const METADATA_KEYS: &'static [&'static str] =
        &["user_api_key_org_spend", "user_api_key_org_max_budget"];
}

mod tests;
