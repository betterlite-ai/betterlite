use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;

// one flat object per case: the function names the entity, and each driver reads only the fields it takes
#[derive(Deserialize)]
pub(crate) struct Input {
    pub(super) function: Function,
    #[serde(default)]
    pub(super) response_cost: Decimal,
    #[serde(default)]
    pub(super) noop: bool,
    #[serde(default)]
    pub(super) database: bool,
    #[serde(default)]
    pub(super) labels_by_metric: BTreeMap<String, Vec<String>>,
    pub(super) now: Option<DateTime<Utc>>,
    pub(super) team_id: Option<String>,
    pub(super) team_alias: Option<String>,
    pub(super) team_spend: Option<Decimal>,
    pub(super) team_max_budget: Option<Decimal>,
    pub(super) org_id: Option<String>,
    pub(super) org_alias: Option<String>,
    pub(super) end_user_id: Option<String>,
    #[serde(default)]
    pub(super) enable_prometheus_only: Option<bool>,
    #[serde(default)]
    pub(super) disable_end_user_cost_tracking: Option<bool>,
    pub(super) user_api_key: Option<String>,
    pub(super) user_api_key_alias: Option<String>,
    pub(super) key_max_budget: Option<Decimal>,
    pub(super) key_spend: Option<Decimal>,
    pub(super) user_id: Option<String>,
    pub(super) user_spend: Option<Decimal>,
    pub(super) user_max_budget: Option<Decimal>,
    pub(super) user_email: Option<String>,
    pub(super) default_budget_id: Option<String>,
    pub(super) carried: Option<SnapshotCall>,
    pub(super) lookup: LookupCall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Function {
    Team,
    Org,
    Customer,
    Key,
    User,
    DefaultCustomerBudget,
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

// the carried snapshot every entity shares; each driver takes the fields its own snapshot type has
#[derive(Deserialize)]
pub(crate) struct SnapshotCall {
    #[serde(default)]
    pub(super) budget_reset_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub(super) max_budget: Option<Decimal>,
    #[serde(default)]
    pub(super) spend: Option<Decimal>,
    #[serde(default)]
    pub(super) user_alias: Option<String>,
}
