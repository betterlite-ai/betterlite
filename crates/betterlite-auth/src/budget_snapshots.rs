pub mod lax_datetime;
mod tests;

use lax_datetime::LaxDateTime;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum BudgetSnapshotError {
    #[error("failed to serialize budget snapshot")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to project budget snapshot metadata")]
    Project {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct MetadataEntries(BTreeMap<String, Box<RawValue>>);

impl AsRef<BTreeMap<String, Box<RawValue>>> for MetadataEntries {
    fn as_ref(&self) -> &BTreeMap<String, Box<RawValue>> {
        &self.0
    }
}

pub trait BudgetSnapshot: Serialize + for<'de> Deserialize<'de> + Sized {
    const ALIASES: &'static [(&'static str, &'static str)];

    fn metadata_entries(&self) -> Result<MetadataEntries, BudgetSnapshotError> {
        let bytes =
            serde_json::to_vec(self).map_err(|source| BudgetSnapshotError::Serialize { source })?;
        let entries = serde_json::from_slice(&bytes)
            .map_err(|source| BudgetSnapshotError::Project { source })?;
        Ok(MetadataEntries(entries))
    }

    #[must_use]
    fn from_metadata(metadata: &BTreeMap<String, Box<RawValue>>) -> Option<Self> {
        let normalized = normalized(metadata, Self::ALIASES);
        let bytes = serde_json::to_vec(&normalized).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

fn normalized(
    metadata: &BTreeMap<String, Box<RawValue>>,
    aliases: &[(&str, &str)],
) -> BTreeMap<String, Box<RawValue>> {
    let mut object = metadata.clone();
    for (native, alias) in aliases {
        if object.contains_key(*alias) {
            object.remove(*native);
            continue;
        }
        if let Some(native_value) = object.remove(*native) {
            object.insert(String::from(*alias), native_value);
        }
    }
    object
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeyBudgetSnapshot {
    #[serde(
        rename = "user_api_key_budget_reset_at",
        alias = "budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<LaxDateTime>,
}

impl BudgetSnapshot for KeyBudgetSnapshot {
    const ALIASES: &'static [(&'static str, &'static str)] =
        &[("budget_reset_at", "user_api_key_budget_reset_at")];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TeamBudgetSnapshot {
    #[serde(
        rename = "user_api_key_team_budget_reset_at",
        alias = "budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<LaxDateTime>,
    #[serde(
        rename = "user_api_key_team_table_max_budget",
        alias = "max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub max_budget: Option<Decimal>,
}

impl BudgetSnapshot for TeamBudgetSnapshot {
    const ALIASES: &'static [(&'static str, &'static str)] = &[
        ("budget_reset_at", "user_api_key_team_budget_reset_at"),
        ("max_budget", "user_api_key_team_table_max_budget"),
    ];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UserBudgetSnapshot {
    #[serde(
        rename = "user_api_key_user_budget_reset_at",
        alias = "budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<LaxDateTime>,
    #[serde(
        rename = "user_api_key_user_table_max_budget",
        alias = "max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub max_budget: Option<Decimal>,
    #[serde(
        rename = "user_api_key_user_alias",
        alias = "user_alias",
        deserialize_with = "Option::deserialize"
    )]
    pub user_alias: Option<String>,
}

impl BudgetSnapshot for UserBudgetSnapshot {
    const ALIASES: &'static [(&'static str, &'static str)] = &[
        ("budget_reset_at", "user_api_key_user_budget_reset_at"),
        ("max_budget", "user_api_key_user_table_max_budget"),
        ("user_alias", "user_api_key_user_alias"),
    ];
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OrgBudgetSnapshot {
    #[serde(
        rename = "user_api_key_org_spend",
        alias = "spend",
        with = "rust_decimal::serde::arbitrary_precision"
    )]
    pub spend: Decimal,
    #[serde(
        rename = "user_api_key_org_max_budget",
        alias = "max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub max_budget: Option<Decimal>,
}

impl BudgetSnapshot for OrgBudgetSnapshot {
    const ALIASES: &'static [(&'static str, &'static str)] = &[
        ("spend", "user_api_key_org_spend"),
        ("max_budget", "user_api_key_org_max_budget"),
    ];
}
