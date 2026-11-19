use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

// the example LiteLLM appends to every rejection
pub const EXAMPLE_URL: &str = "https://docs.litellm.ai/docs/proxy/users";

type Fields = BTreeMap<String, JsonData>;

#[derive(Debug, thiserror::Error)]
#[error("Invalid model_max_budget: {}. Example of valid model_max_budget: {}", .detail, EXAMPLE_URL)]
pub struct ModelBudgetError {
    pub detail: String,
}

fn invalid(model: &str, field: &str, phrase: &str) -> ModelBudgetError {
    ModelBudgetError {
        detail: format!("{model}.{field} must be {phrase}"),
    }
}

// a None or empty map needs no license and no validation; a populated one must fit BudgetConfig
pub fn validate_model_max_budget(
    model_max_budget: Option<&Fields>,
) -> Result<(), ModelBudgetError> {
    let Some(budgets) = model_max_budget.filter(|entries| !entries.is_empty()) else {
        return Ok(());
    };
    for (model, config) in budgets {
        validate_entry(model, config)?;
    }
    Ok(())
}

fn validate_entry(model: &str, config: &JsonData) -> Result<(), ModelBudgetError> {
    let JsonData::Object(fields) = config else {
        return Err(ModelBudgetError {
            detail: format!("{model} must map to an object"),
        });
    };
    validate_budget_fields(model, fields)?;
    validate_rate_limits(model, fields)
}

// budget_limit is converted with float() before BudgetConfig sees it and then replaces max_budget
fn validate_budget_fields(model: &str, fields: &Fields) -> Result<(), ModelBudgetError> {
    let limit = fields.get("budget_limit");
    if limit.is_some_and(|value| !is_numeric(value)) {
        return Err(invalid(model, "budget_limit", "a number"));
    }
    if limit.is_none()
        && fields
            .get("max_budget")
            .is_some_and(|value| !is_number_or_null(value))
    {
        return Err(invalid(model, "max_budget", "a number"));
    }
    // time_period replaces budget_duration the same way
    let period = fields.get("time_period");
    if period.is_some_and(|value| !is_string_or_null(value)) {
        return Err(invalid(model, "time_period", "a string"));
    }
    if period.is_none()
        && fields
            .get("budget_duration")
            .is_some_and(|value| !is_string_or_null(value))
    {
        return Err(invalid(model, "budget_duration", "a string"));
    }
    Ok(())
}

fn validate_rate_limits(model: &str, fields: &Fields) -> Result<(), ModelBudgetError> {
    for limit_field in ["tpm_limit", "rpm_limit"] {
        if fields
            .get(limit_field)
            .is_some_and(|value| !is_integer_or_null(value))
        {
            return Err(invalid(model, limit_field, "an integer"));
        }
    }
    Ok(())
}

// float() accepts numbers and numeric text; nothing else reaches BudgetConfig
fn is_numeric(value: &JsonData) -> bool {
    match value {
        JsonData::Number(_) => true,
        JsonData::String(text) => text.trim().parse::<f64>().is_ok(),
        JsonData::Null | JsonData::Bool(_) | JsonData::Array(_) | JsonData::Object(_) => false,
    }
}

const fn is_number_or_null(value: &JsonData) -> bool {
    match value {
        JsonData::Number(_) | JsonData::Null => true,
        JsonData::Bool(_) | JsonData::String(_) | JsonData::Array(_) | JsonData::Object(_) => false,
    }
}

const fn is_string_or_null(value: &JsonData) -> bool {
    match value {
        JsonData::String(_) | JsonData::Null => true,
        JsonData::Bool(_) | JsonData::Number(_) | JsonData::Array(_) | JsonData::Object(_) => false,
    }
}

fn is_integer_or_null(value: &JsonData) -> bool {
    match value {
        JsonData::Number(number) => number.as_i64().is_some() || number.as_u64().is_some(),
        JsonData::Null => true,
        JsonData::Bool(_) | JsonData::String(_) | JsonData::Array(_) | JsonData::Object(_) => false,
    }
}
