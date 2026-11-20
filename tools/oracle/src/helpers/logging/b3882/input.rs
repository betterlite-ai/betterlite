use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(super) enum Input {
    RemainingBudget {
        max_budget: Option<f64>,
        spend: Option<f64>,
    },
    RemainingHours {
        reset_at: DateTime<Utc>,
        now: DateTime<Utc>,
    },
    Team {
        team_id: String,
        team_alias: Option<String>,
        spend: Option<f64>,
        max_budget: Option<f64>,
        budget_reset_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    Org {
        org_id: String,
        org_alias: Option<String>,
        spend: Option<f64>,
        max_budget: Option<f64>,
        budget_reset_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    Key {
        token: String,
        key_alias: Option<String>,
        spend: Option<f64>,
        max_budget: Option<f64>,
        budget_reset_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    User {
        user_id: String,
        user_email: Option<String>,
        user_alias: Option<String>,
        spend: Option<f64>,
        max_budget: Option<f64>,
        budget_reset_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    Customer {
        end_user_id: String,
        spend: Option<f64>,
        max_budget: Option<f64>,
        budget_reset_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
        enable_end_user_tracking: bool,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    TeamMembers {
        team_id: String,
        team_alias: Option<String>,
        member_count: usize,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    Provider {
        provider: String,
        spend: Option<f64>,
        budget_limit: Option<f64>,
    },
    KeyList {
        entries: Vec<KeyListEntryCall>,
        now: DateTime<Utc>,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    OrgList {
        rows: Vec<OrgRowCall>,
        now: DateTime<Utc>,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
    CustomerRow {
        customer: CustomerRowCall,
        default_budget: Option<BudgetRowCall>,
        now: DateTime<Utc>,
        enable_end_user_tracking: bool,
        labels_by_metric: BTreeMap<String, Vec<String>>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum KeyListEntryCall {
    Record {
        token: String,
        key_alias: Option<String>,
        spend: Option<f64>,
        max_budget: Option<f64>,
        budget_reset_at: Option<DateTime<Utc>>,
    },
    Token {
        token: String,
    },
}

#[derive(Deserialize)]
pub(super) struct OrgRowCall {
    pub(super) organization_id: Option<String>,
    pub(super) organization_alias: Option<String>,
    pub(super) spend: Option<f64>,
    pub(super) budget: Option<BudgetRowCall>,
}

#[derive(Deserialize)]
pub(super) struct CustomerRowCall {
    pub(super) user_id: String,
    pub(super) spend: f64,
    pub(super) budget: Option<BudgetRowCall>,
}

#[derive(Deserialize)]
pub(super) struct BudgetRowCall {
    pub(super) max_budget: Option<f64>,
    pub(super) budget_reset_at: Option<DateTime<Utc>>,
}
