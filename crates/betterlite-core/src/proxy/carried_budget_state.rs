use crate::agents::json::AgentJson;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type MetadataEntries = BTreeMap<String, AgentJson>;

fn metadata_entries<T>(snapshot: &T) -> MetadataEntries
where
    T: Serialize,
{
    serde_json::to_value(snapshot)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default()
}

fn from_metadata<T>(metadata: &MetadataEntries, keys: &[&str]) -> Option<T>
where
    T: for<'de> Deserialize<'de>,
{
    let filtered = metadata
        .iter()
        .filter(|(key, _)| keys.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<MetadataEntries>();
    let value = serde_json::to_value(filtered).ok()?;
    serde_json::from_value(value).ok()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeyBudgetSnapshot {
    #[serde(default, rename = "user_api_key_budget_reset_at")]
    pub budget_reset_at: Option<DateTime<Utc>>,
}

impl KeyBudgetSnapshot {
    #[must_use]
    pub fn metadata_entries(&self) -> MetadataEntries {
        metadata_entries(self)
    }

    #[must_use]
    pub fn from_metadata(metadata: &MetadataEntries) -> Option<Self> {
        from_metadata(metadata, &["user_api_key_budget_reset_at"])
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TeamBudgetSnapshot {
    #[serde(default, rename = "user_api_key_team_budget_reset_at")]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(default, rename = "user_api_key_team_table_max_budget")]
    pub max_budget: Option<Decimal>,
}

impl TeamBudgetSnapshot {
    #[must_use]
    pub fn metadata_entries(&self) -> MetadataEntries {
        metadata_entries(self)
    }

    #[must_use]
    pub fn from_metadata(metadata: &MetadataEntries) -> Option<Self> {
        from_metadata(
            metadata,
            &[
                "user_api_key_team_budget_reset_at",
                "user_api_key_team_table_max_budget",
            ],
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UserBudgetSnapshot {
    #[serde(default, rename = "user_api_key_user_budget_reset_at")]
    pub budget_reset_at: Option<DateTime<Utc>>,
    #[serde(default, rename = "user_api_key_user_table_max_budget")]
    pub max_budget: Option<Decimal>,
    #[serde(default, rename = "user_api_key_user_alias")]
    pub user_alias: Option<String>,
}

impl UserBudgetSnapshot {
    #[must_use]
    pub fn metadata_entries(&self) -> MetadataEntries {
        metadata_entries(self)
    }

    #[must_use]
    pub fn from_metadata(metadata: &MetadataEntries) -> Option<Self> {
        from_metadata(
            metadata,
            &[
                "user_api_key_user_budget_reset_at",
                "user_api_key_user_table_max_budget",
                "user_api_key_user_alias",
            ],
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OrgBudgetSnapshot {
    #[serde(rename = "user_api_key_org_spend")]
    pub spend: Decimal,
    #[serde(default, rename = "user_api_key_org_max_budget")]
    pub max_budget: Option<Decimal>,
}

impl OrgBudgetSnapshot {
    #[must_use]
    pub fn metadata_entries(&self) -> MetadataEntries {
        metadata_entries(self)
    }

    #[must_use]
    pub fn from_metadata(metadata: &MetadataEntries) -> Option<Self> {
        from_metadata(
            metadata,
            &["user_api_key_org_spend", "user_api_key_org_max_budget"],
        )
    }
}

#[cfg(test)]
mod tests;
