use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

// the same shape the Python helper reads: one function per case, with the entity fields that function takes
#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Team {
        team_id: Option<String>,
        team_alias: Option<String>,
        team_spend: Option<Decimal>,
        team_max_budget: Option<Decimal>,
        response_cost: Decimal,
        carried: Option<TeamSnapshotCall>,
        lookup: LookupCall,
        now: DateTime<Utc>,
        noop: bool,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    Org {
        org_id: Option<String>,
        org_alias: Option<String>,
        response_cost: Decimal,
        carried: Option<OrgSnapshotCall>,
        database: bool,
        lookup: LookupCall,
        now: DateTime<Utc>,
        noop: bool,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    Customer {
        end_user_id: Option<String>,
        response_cost: Decimal,
        enable_prometheus_only: Option<bool>,
        disable_end_user_cost_tracking: Option<bool>,
        lookup: LookupCall,
        now: DateTime<Utc>,
        noop: bool,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    Key {
        user_api_key: Option<String>,
        user_api_key_alias: Option<String>,
        response_cost: Decimal,
        key_max_budget: Option<Decimal>,
        key_spend: Option<Decimal>,
        carried: Option<KeySnapshotCall>,
        lookup: LookupCall,
        now: DateTime<Utc>,
        noop: bool,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    User {
        user_id: Option<String>,
        user_spend: Option<Decimal>,
        user_max_budget: Option<Decimal>,
        response_cost: Decimal,
        carried: Option<UserSnapshotCall>,
        user_email: Option<String>,
        lookup: LookupCall,
        now: DateTime<Utc>,
        noop: bool,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    DefaultCustomerBudget {
        default_budget_id: Option<String>,
        lookup: LookupCall,
    },
}

// the columns a scripted lookup returns, or the empty kind when it finds nothing
#[derive(Clone, Deserialize)]
pub(crate) struct LookupCall {
    pub(super) kind: LookupKind,
    #[serde(default)]
    pub(super) budget_reset_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(super) max_budget: Option<Decimal>,
    #[serde(default)]
    pub(super) spend: Option<Decimal>,
    #[serde(default)]
    pub(super) organization_alias: Option<String>,
    #[serde(default)]
    pub(super) user_email: Option<String>,
    #[serde(default)]
    pub(super) user_alias: Option<String>,
    #[serde(default)]
    pub(super) budget: Option<BudgetRowCall>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LookupKind {
    Record,
    Missing,
}

impl LookupCall {
    pub(super) fn record(&self) -> Option<&Self> {
        if self.kind == LookupKind::Record {
            return Some(self);
        }
        None
    }
}

#[derive(Clone, Deserialize)]
pub(crate) struct BudgetRowCall {
    pub(super) max_budget: Option<Decimal>,
    pub(super) budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub(crate) struct TeamSnapshotCall {
    pub(super) budget_reset_at: Option<DateTime<Utc>>,
    pub(super) max_budget: Option<Decimal>,
}

#[derive(Deserialize)]
pub(crate) struct OrgSnapshotCall {
    pub(super) spend: Decimal,
    pub(super) max_budget: Option<Decimal>,
}

#[derive(Deserialize)]
pub(crate) struct KeySnapshotCall {
    pub(super) budget_reset_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
pub(crate) struct UserSnapshotCall {
    pub(super) budget_reset_at: Option<DateTime<Utc>>,
    pub(super) max_budget: Option<Decimal>,
    pub(super) user_alias: Option<String>,
}
