mod tests;

use chrono::{DateTime, FixedOffset, NaiveDateTime};
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

pub trait BudgetSnapshot: Serialize + for<'de> Deserialize<'de> + Sized {
    fn metadata_entries(&self) -> Result<BTreeMap<String, Box<RawValue>>, BudgetSnapshotError> {
        let bytes =
            serde_json::to_vec(self).map_err(|source| BudgetSnapshotError::Serialize { source })?;
        serde_json::from_slice(&bytes).map_err(|source| BudgetSnapshotError::Project { source })
    }

    #[must_use]
    fn from_metadata(metadata: &BTreeMap<String, Box<RawValue>>) -> Option<Self> {
        let normalized = Self::normalize_aliases(metadata);
        let bytes = serde_json::to_vec(&normalized).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn normalize_aliases(
        metadata: &BTreeMap<String, Box<RawValue>>,
    ) -> BTreeMap<String, Box<RawValue>>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SnapshotDateTime {
    Offset(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

fn normalized(
    metadata: &BTreeMap<String, Box<RawValue>>,
    aliases: &[(&str, &str)],
) -> BTreeMap<String, Box<RawValue>> {
    let mut object = metadata.clone();
    for (native, alias) in aliases {
        if let Some(alias_value) = object.get(*alias).cloned() {
            object.remove(*native);
            object.insert(String::from(*alias), alias_value);
        } else if let Some(native_value) = object.remove(*native) {
            object.insert(String::from(*alias), native_value);
        } else {
            object.remove(*native);
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
    pub budget_reset_at: Option<SnapshotDateTime>,
}

impl BudgetSnapshot for KeyBudgetSnapshot {
    fn normalize_aliases(
        metadata: &BTreeMap<String, Box<RawValue>>,
    ) -> BTreeMap<String, Box<RawValue>> {
        normalized(
            metadata,
            &[("budget_reset_at", "user_api_key_budget_reset_at")],
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TeamBudgetSnapshot {
    #[serde(
        rename = "user_api_key_team_budget_reset_at",
        alias = "budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<SnapshotDateTime>,
    #[serde(
        rename = "user_api_key_team_table_max_budget",
        alias = "max_budget",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    pub max_budget: Option<Decimal>,
}

impl BudgetSnapshot for TeamBudgetSnapshot {
    fn normalize_aliases(
        metadata: &BTreeMap<String, Box<RawValue>>,
    ) -> BTreeMap<String, Box<RawValue>> {
        normalized(
            metadata,
            &[
                ("budget_reset_at", "user_api_key_team_budget_reset_at"),
                ("max_budget", "user_api_key_team_table_max_budget"),
            ],
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UserBudgetSnapshot {
    #[serde(
        rename = "user_api_key_user_budget_reset_at",
        alias = "budget_reset_at",
        deserialize_with = "Option::deserialize"
    )]
    pub budget_reset_at: Option<SnapshotDateTime>,
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
    fn normalize_aliases(
        metadata: &BTreeMap<String, Box<RawValue>>,
    ) -> BTreeMap<String, Box<RawValue>> {
        normalized(
            metadata,
            &[
                ("budget_reset_at", "user_api_key_user_budget_reset_at"),
                ("max_budget", "user_api_key_user_table_max_budget"),
                ("user_alias", "user_api_key_user_alias"),
            ],
        )
    }
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
    fn normalize_aliases(
        metadata: &BTreeMap<String, Box<RawValue>>,
    ) -> BTreeMap<String, Box<RawValue>> {
        normalized(
            metadata,
            &[
                ("spend", "user_api_key_org_spend"),
                ("max_budget", "user_api_key_org_max_budget"),
            ],
        )
    }
}
