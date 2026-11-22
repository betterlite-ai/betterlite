use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

// _DeploymentRow: one LiteLLM_ProxyModelTable row, with model_info as stored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentRow {
    pub model_id: String,
    pub model_name: String,
    #[serde(default)]
    pub model_info: Option<Json>,
}

// _BudgetRow: a LiteLLM_BudgetTable row joined into an access group budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetRow {
    pub budget_id: String,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub max_budget: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::arbitrary_precision_option")]
    pub soft_budget: Option<Decimal>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub budget_reset_at: Option<Timestamp>,
}

// _ModelAccessGroupBudgetRow: the linking row plus its litellm_budget_table relation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupBudgetRow {
    pub access_group_name: String,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub litellm_budget_table: Option<BudgetRow>,
}

// AccessGroupBudget: _budget_or_none's projection of the joined budget.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessGroupBudget {
    pub budget_id: String,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub soft_budget: Option<Decimal>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub budget_reset_at: Option<Timestamp>,
}

// AccessGroupInfo: one access group built from the deployment tags.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessGroupInfo {
    pub access_group: String,
    pub model_names: Vec<String>,
    pub deployment_count: usize,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub spend: Option<Decimal>,
    #[serde(default)]
    pub budget: Option<AccessGroupBudget>,
}

// AccessGroupBudgetResponse: the budget endpoints' per-group answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessGroupBudgetResponse {
    pub access_group: String,
    #[serde(with = "rust_decimal::serde::float")]
    pub spend: Decimal,
    #[serde(default)]
    pub budget: Option<AccessGroupBudget>,
}

// ModelAccessGroupBudget: LiteLLM's type for the linking row on its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAccessGroupBudget {
    pub access_group_name: String,
    #[serde(default, with = "rust_decimal::serde::float")]
    pub spend: Decimal,
    #[serde(default, with = "rust_decimal::serde::float_option")]
    pub max_budget: Option<Decimal>,
}
