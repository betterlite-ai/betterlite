use std::collections::BTreeSet;

use betterlite_config::sections::settings::JsonValue;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::identity_rows::Payload;
use serde::Deserialize;

use super::{ContractError, decode, require_object, supplied_fields};
use crate::key_fields::values::Amount;

// The declaration order LiteLLM gives the model, which B829's settings route lists its fields in.
pub(crate) const BUDGET_FIELDS: [&str; 10] = [
    "budget_id",
    "max_budget",
    "soft_budget",
    "max_parallel_requests",
    "tpm_limit",
    "rpm_limit",
    "tpd_limit",
    "budget_duration",
    "model_max_budget",
    "budget_reset_at",
];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BudgetNewRequest {
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub max_budget: Option<Amount>,
    #[serde(default)]
    pub soft_budget: Option<Amount>,
    #[serde(default)]
    pub max_parallel_requests: Option<i64>,
    #[serde(default)]
    pub tpm_limit: Option<i64>,
    #[serde(default)]
    pub rpm_limit: Option<i64>,
    #[serde(default)]
    pub tpd_limit: Option<i64>,
    #[serde(default)]
    pub budget_duration: Option<String>,
    #[serde(default)]
    pub model_max_budget: Option<Payload>,
    #[serde(default)]
    pub budget_reset_at: Option<Timestamp>,
    // pydantic's model_fields_set for the keys above: an omitted field and an explicit null both stay None, and only this says which one the body carried.
    #[serde(skip)]
    pub supplied: BTreeSet<String>,
}

impl BudgetNewRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let mut request: Self = decode(&fields)?;
        request.supplied = supplied_fields(&fields, &BUDGET_FIELDS);
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BudgetRequest {
    pub budgets: Vec<String>,
}

impl BudgetRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BudgetDeleteRequest {
    pub id: String,
}

impl BudgetDeleteRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }
}

mod tests;
