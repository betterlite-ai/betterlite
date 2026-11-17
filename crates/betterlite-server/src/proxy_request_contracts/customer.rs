use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};

use super::budget::{BUDGET_FIELDS, BudgetNewRequest};
use super::{ContractError, Fields, decode, is_set, require_object, supplied_fields};
use crate::key_fields::values::{Amount, LiteLLMObjectPermissionBase};

const CONFLICT: &str = "Set either 'max_budget' or 'budget_id', not both.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AllowedModelRegion {
    Eu,
    Us,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NewCustomerRequest {
    #[serde(flatten)]
    pub budget: BudgetNewRequest,
    pub user_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub blocked: bool,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub spend: Option<Amount>,
    #[serde(default)]
    pub allowed_model_region: Option<AllowedModelRegion>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub object_permission: Option<LiteLLMObjectPermissionBase>,
}

impl NewCustomerRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        check_budget_choice(&fields)?;
        let mut request: Self = decode(&fields)?;
        request.budget.supplied = supplied_fields(&fields, &BUDGET_FIELDS);
        Ok(request)
    }
}

// The before-validator sees the raw body, so an explicit null max_budget leaves budget_id free and a 0.0 max_budget does not.
fn check_budget_choice(fields: &Fields) -> Result<(), ContractError> {
    if is_set(fields, "max_budget") && is_set(fields, "budget_id") {
        return Err(ContractError::from(CONFLICT));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
