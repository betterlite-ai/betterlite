use std::collections::BTreeMap;

use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, PortError};
use betterlite_db::budget_permission_rows::JsonData;

use crate::management_guards::metadata::METADATA_FIELDS;
use crate::management_guards::metadata::has_effect;
use crate::organization_create::records::BUDGET_SCHEMA_FIELDS;
use crate::organization_create::records::to_json_data;

pub const NESTED_BUDGET_FIELD: &str = "litellm_budget_table";
pub const BUDGET_DURATION_FIELD: &str = "budget_duration";
pub const BUDGET_RESET_AT_FIELD: &str = "budget_reset_at";
pub const UPDATED_BY_FIELD: &str = "updated_by";
// _ORG_COLUMN_FIELDS: the organization table columns a request may set directly
pub const ORG_COLUMN_FIELDS: [&str; 2] = ["organization_alias", "models"];

// _BUDGET_SETTABLE_FIELDS: the budget schema's fields minus the key column
#[must_use]
pub fn budget_settable_fields() -> Vec<&'static str> {
    BUDGET_SCHEMA_FIELDS
        .into_iter()
        .filter(|field| *field != "budget_id")
        .collect()
}

// _ORG_METADATA_FIELDS: the management metadata list minus the two budget columns
#[must_use]
pub fn org_metadata_fields() -> Vec<&'static str> {
    let budget_fields = budget_settable_fields();
    METADATA_FIELDS
        .into_iter()
        .filter(|field| !budget_fields.contains(field))
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum BudgetTransformError {
    #[error(transparent)]
    Reset(#[from] PortError),
    #[error("the budget reset time cannot be written")]
    ResetValue,
    #[error("the nested litellm_budget_table value is not a mapping")]
    NestedNotAMapping,
}

// LiteLLM's build_budget_write_data: a sent duration drives budget_reset_at, a reset value and the actor override input
pub fn build_budget_write_data(
    budget_updates: &BTreeMap<String, JsonData>,
    updated_by: &str,
    reset: &dyn BudgetResetCalculator,
) -> Result<BTreeMap<String, JsonData>, BudgetTransformError> {
    let mut data = budget_updates.clone();
    if let Some(duration) = budget_updates.get(BUDGET_DURATION_FIELD) {
        let reset_at = match duration {
            JsonData::String(text) => JsonData::String(reset_value(reset.reset_time(text)?)?),
            JsonData::Null
            | JsonData::Bool(_)
            | JsonData::Number(_)
            | JsonData::Array(_)
            | JsonData::Object(_) => JsonData::Null,
        };
        data.insert(BUDGET_RESET_AT_FIELD.to_owned(), reset_at);
    }
    data.insert(
        UPDATED_BY_FIELD.to_owned(),
        JsonData::String(updated_by.to_owned()),
    );
    Ok(data)
}

fn reset_value(
    timestamp: betterlite_db::budget_permission_rows::Timestamp,
) -> Result<String, BudgetTransformError> {
    match to_json_data(&timestamp) {
        Ok(JsonData::String(text)) => Ok(text),
        Ok(_) => Err(BudgetTransformError::ResetValue),
        Err(error) => {
            tracing::error!(error = %error, "the budget reset time could not be encoded");
            Err(BudgetTransformError::ResetValue)
        }
    }
}

// LiteLLM's handle_nested_budget_structure_in_organization_update_request: the UI's nested budget wins for known fields
pub fn handle_nested_budget_structure_in_organization_update_request(
    raw_data: BTreeMap<String, JsonData>,
) -> Result<BTreeMap<String, JsonData>, BudgetTransformError> {
    let mut transformed = raw_data;
    let Some(nested) = transformed.remove(NESTED_BUDGET_FIELD) else {
        return Ok(transformed);
    };
    if !has_effect(&nested) {
        return Ok(transformed);
    }
    let JsonData::Object(budget_data) = nested else {
        return Err(BudgetTransformError::NestedNotAMapping);
    };
    for (name, value) in budget_data {
        if BUDGET_SCHEMA_FIELDS.contains(&name.as_str()) {
            transformed.insert(name, value);
        }
    }
    Ok(transformed)
}
