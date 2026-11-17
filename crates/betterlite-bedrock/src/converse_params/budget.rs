use betterlite_anthropic::params::DEFAULT_MAX_TOKENS;
use betterlite_openai::base_helpers::thinking_request;
use serde_json::Number;

use crate::converse_options::budget::clamp_thinking_budget_tokens;
use crate::converse_params::error::ConverseParamsError;
use crate::converse_params::wire::transcode;
use crate::model_routes::{ParamMap, ParamValue};

const BUDGET_FIELD: &str = "budget_tokens";
const MAX_TOKENS_FIELD: &str = "maxTokens";
const THINKING_FIELD: &str = "thinking";
const MAX_TOKEN_KEYS: [&str; 2] = ["max_tokens", "max_completion_tokens"];

// LiteLLM's Bedrock override: clamp the budget, then fill in maxTokens when the caller sent none.
pub fn update_thinking_budget(
    non_default_params: &ParamMap,
    optional_params: &mut ParamMap,
    minimum_budget: i64,
) -> Result<(), ConverseParamsError> {
    clamp_thinking_budget_tokens(optional_params, minimum_budget);
    let enabled = thinking_request(&transcode(optional_params)?).thinking;
    let requested = MAX_TOKEN_KEYS
        .iter()
        .any(|key| non_default_params.contains_key(*key));
    if !enabled || requested {
        return Ok(());
    }
    let Some(ParamValue::Mapping(thinking)) = optional_params.get(THINKING_FIELD) else {
        return Ok(());
    };
    let Some(budget) = thinking
        .get(BUDGET_FIELD)
        .filter(|budget| !matches!(budget, ParamValue::Null))
    else {
        return Ok(());
    };
    let total = add_default_max_tokens(budget)?;
    optional_params.insert(MAX_TOKENS_FIELD.to_owned(), total);
    Ok(())
}

// Python adds to an int or a float budget and raises for anything else.
fn add_default_max_tokens(budget: &ParamValue) -> Result<ParamValue, ConverseParamsError> {
    match budget {
        ParamValue::Bool(flag) => Ok(ParamValue::Number(Number::from(
            i64::from(*flag)
                .checked_add(DEFAULT_MAX_TOKENS)
                .ok_or(ConverseParamsError::BudgetOverflow)?,
        ))),
        ParamValue::Number(number) => add_tokens(number).map(ParamValue::Number),
        ParamValue::Null | ParamValue::Text(_) | ParamValue::List(_) | ParamValue::Mapping(_) => {
            Err(ConverseParamsError::BudgetNotNumber)
        }
    }
}

fn add_tokens(number: &Number) -> Result<Number, ConverseParamsError> {
    if let Some(value) = number.as_i64()
        && let Some(total) = value.checked_add(DEFAULT_MAX_TOKENS)
    {
        return Ok(Number::from(total));
    }
    let float = number
        .as_f64()
        .ok_or(ConverseParamsError::BudgetNotNumber)?;
    let default = num_traits::cast::<i64, f64>(DEFAULT_MAX_TOKENS)
        .ok_or(ConverseParamsError::BudgetOverflow)?;
    Number::from_f64(float.mul_add(1.0, default)).ok_or(ConverseParamsError::BudgetOverflow)
}
