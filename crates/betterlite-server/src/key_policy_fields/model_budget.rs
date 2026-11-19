use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

// the example LiteLLM appends to every rejection
pub const EXAMPLE_URL: &str = "https://docs.litellm.ai/docs/proxy/users";

#[derive(Debug, thiserror::Error)]
#[error("Invalid model_max_budget: {}. Example of valid model_max_budget: {}", .detail, EXAMPLE_URL)]
pub struct ModelBudgetError {
    pub detail: String,
}

// a None or empty map needs no license and no validation; a populated one must fit BudgetConfig
pub fn validate_model_max_budget(
    model_max_budget: Option<&BTreeMap<String, JsonData>>,
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
    // budget_limit is converted with float() before BudgetConfig sees it and then replaces max_budget
    if let Some(limit) = fields.get("budget_limit") {
        if !is_numeric(limit) {
            return Err(ModelBudgetError {
                detail: format!("{model}.budget_limit must be a number"),
            });
        }
    } else if let Some(max_budget) = fields.get("max_budget") {
        if !is_number_or_null(max_budget) {
            return Err(ModelBudgetError {
                detail: format!("{model}.max_budget must be a number"),
            });
        }
    }
    // time_period replaces budget_duration the same way
    if let Some(period) = fields.get("time_period") {
        if !is_string_or_null(period) {
            return Err(ModelBudgetError {
                detail: format!("{model}.time_period must be a string"),
            });
        }
    } else if let Some(duration) = fields.get("budget_duration") {
        if !is_string_or_null(duration) {
            return Err(ModelBudgetError {
                detail: format!("{model}.budget_duration must be a string"),
            });
        }
    }
    for limit_field in ["tpm_limit", "rpm_limit"] {
        if let Some(value) = fields.get(limit_field) {
            if !is_integer_or_null(value) {
                return Err(ModelBudgetError {
                    detail: format!("{model}.{limit_field} must be an integer"),
                });
            }
        }
    }
    Ok(())
}

// float() accepts numbers and numeric text; nothing else reaches BudgetConfig
fn is_numeric(value: &JsonData) -> bool {
    match value {
        JsonData::Number(_) => true,
        JsonData::String(text) => text.trim().parse::<f64>().is_ok(),
        _ => false,
    }
}

fn is_number_or_null(value: &JsonData) -> bool {
    match value {
        JsonData::Number(_) | JsonData::Null => true,
        _ => false,
    }
}

fn is_string_or_null(value: &JsonData) -> bool {
    match value {
        JsonData::String(_) | JsonData::Null => true,
        _ => false,
    }
}

fn is_integer_or_null(value: &JsonData) -> bool {
    match value {
        JsonData::Number(number) => number.as_i64().is_some() || number.as_u64().is_some(),
        JsonData::Null => true,
        _ => false,
    }
}
