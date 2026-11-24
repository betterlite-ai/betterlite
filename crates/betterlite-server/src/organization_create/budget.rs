use std::collections::BTreeMap;

use betterlite_auth::auth_user_lookup::ports::{BudgetResetCalculator, PortError};
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable, Timestamp};
use betterlite_db::key_user_writes::json_text::SerdeJsonEncoder;
use betterlite_db::key_user_writes::json_text::jsonify_object;

use crate::management_guards::metadata::METADATA_FIELDS;
use crate::management_guards::metadata::has_effect;
use crate::organization_create::records::{
    BUDGET_SCHEMA_FIELDS, NewOrganizationRequest, amount_of, decimal_of,
};

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

fn reset_value(timestamp: Timestamp) -> Result<String, BudgetTransformError> {
    match serde_json::to_value(&timestamp).and_then(serde_json::from_value::<JsonData>) {
        Ok(JsonData::String(text)) => Ok(text),
        Ok(_) => Err(BudgetTransformError::ResetValue),
        Err(error) => {
            tracing::error!(error = %error, "the budget reset time could not be encoded");
            Err(BudgetTransformError::ResetValue)
        }
    }
}

// ast-grep-ignore: no-long-names
pub fn handle_nested_budget_structure_in_organization_update_request(
    raw_data: BTreeMap<String, JsonData>,
) -> Result<BTreeMap<String, JsonData>, BudgetTransformError> {
    // the UI's nested budget wins over the top-level value for every known budget field
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

// the request's LiteLLM_BudgetTable columns, jsonified, with the actor columns the create always writes
#[must_use]
pub fn budget_write_payload(
    fields: &BTreeMap<String, JsonData>,
    actor: &str,
) -> BTreeMap<String, JsonData> {
    let sent: BTreeMap<String, JsonData> = fields
        .iter()
        .filter(|(name, _)| BUDGET_SCHEMA_FIELDS.contains(&name.as_str()))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let mut data = jsonify_object(&sent, &SerdeJsonEncoder);
    data.insert("created_by".to_owned(), JsonData::String(actor.to_owned()));
    data.insert("updated_by".to_owned(), JsonData::String(actor.to_owned()));
    data
}

// the created row, projected onto LiteLLM_BudgetTable: allowed_models comes from the column default
#[must_use]
pub fn created_budget_row(budget_id: &str, request: &NewOrganizationRequest) -> LiteLLMBudgetTable {
    LiteLLMBudgetTable {
        budget_id: Some(budget_id.to_owned()),
        soft_budget: request
            .soft_budget
            .as_ref()
            .and_then(amount_of)
            .and_then(decimal_of),
        max_budget: request
            .max_budget
            .as_ref()
            .and_then(amount_of)
            .and_then(decimal_of),
        max_parallel_requests: request.max_parallel_requests,
        tpm_limit: request.tpm_limit,
        rpm_limit: request.rpm_limit,
        tpd_limit: request.tpd_limit,
        model_max_budget: request.model_max_budget.clone(),
        budget_duration: request.budget_duration.clone(),
        allowed_models: Some(request.allowed_models.clone().unwrap_or_default()),
        temp_budget_increase: request
            .temp_budget_increase
            .as_ref()
            .and_then(amount_of)
            .and_then(decimal_of),
        temp_budget_expiry: request.temp_budget_expiry,
    }
}
