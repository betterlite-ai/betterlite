use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub trait BudgetSnapshot: Serialize + DeserializeOwned {
    const METADATA_KEYS: &'static [&'static str];

    fn metadata_entries(&self) -> Result<Map<String, Value>, serde_json::Error> {
        serde_json::from_value(serde_json::to_value(self)?)
    }

    // An invalid carried record means the logger falls back to its own lookup, so the error is not surfaced
    fn from_metadata(metadata: &Map<String, Value>) -> Option<Self> {
        let carried = Self::METADATA_KEYS
            .iter()
            .filter_map(|key| metadata.get_key_value(*key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Map<String, Value>>();
        serde_json::from_value(Value::Object(carried)).ok()
    }
}

// LiteLLM requires every carried key to be present, so null is a value and a missing key fails the record
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBudgetSnapshot {
    #[serde(
        rename = "user_api_key_budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<DateTime<Utc>>,
}

impl BudgetSnapshot for KeyBudgetSnapshot {
    const METADATA_KEYS: &'static [&'static str] = &["user_api_key_budget_reset_at"];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TeamBudgetSnapshot {
    #[serde(
        rename = "user_api_key_team_budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(
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
#[serde(deny_unknown_fields)]
pub struct UserBudgetSnapshot {
    #[serde(
        rename = "user_api_key_user_budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(
        rename = "user_api_key_user_table_max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub max_budget: Option<Decimal>,
    #[serde(
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
#[serde(deny_unknown_fields)]
pub struct OrgBudgetSnapshot {
    #[serde(
        rename = "user_api_key_org_spend",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub spend: Decimal,
    #[serde(
        rename = "user_api_key_org_max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub max_budget: Option<Decimal>,
}

impl BudgetSnapshot for OrgBudgetSnapshot {
    const METADATA_KEYS: &'static [&'static str] =
        &["user_api_key_org_spend", "user_api_key_org_max_budget"];
}

#[cfg(test)]
mod tests;
