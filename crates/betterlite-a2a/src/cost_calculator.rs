use std::str::FromStr as _;

use betterlite_cost::a2a_cost::{A2aCostError, A2aCostParams, A2aUsage, calculate_a2a_cost};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use thiserror::Error;

mod tests;

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CostCalculatorError {
    #[error("the A2A cost parameter {field} is not a decimal number")]
    ParamType { field: &'static str },
    #[error("the A2A litellm_params must be an object")]
    ParamsNotObject,
    #[error("the A2A usage must be an object")]
    UsageNotObject,
    #[error("the A2A usage field {field} is not a token count")]
    UsageType { field: &'static str },
    #[error(transparent)]
    Overflow(#[from] A2aCostError),
}

pub fn a2a_call_cost(logging: Option<&LoggingState>) -> Result<Decimal, CostCalculatorError> {
    let Some(state) = logging else {
        return Ok(Decimal::ZERO);
    };
    let details = &state.model_call_details;
    let params = cost_params(details)?;
    // LiteLLM reads usage only in the token branch, which cost_params reaches only when both fixed prices are absent
    let usage = if params.input_cost_per_token.is_some() || params.output_cost_per_token.is_some() {
        usage(details.get("usage"))?
    } else {
        None
    };
    Ok(calculate_a2a_cost(&params, usage)?)
}

fn cost_params(details: &IndexMap<String, Json>) -> Result<A2aCostParams, CostCalculatorError> {
    let response_cost = money(details.get("response_cost"), "response_cost")?;
    if response_cost.is_some() {
        return Ok(A2aCostParams {
            response_cost,
            cost_per_query: None,
            input_cost_per_token: None,
            output_cost_per_token: None,
        });
    }
    let params = litellm_params(details)?;
    let cost_per_query = money(
        params.and_then(|values| values.get("cost_per_query")),
        "cost_per_query",
    )?;
    if cost_per_query.is_some() {
        return Ok(A2aCostParams {
            response_cost,
            cost_per_query,
            input_cost_per_token: None,
            output_cost_per_token: None,
        });
    }
    let input_cost_per_token = money(
        params.and_then(|values| values.get("input_cost_per_token")),
        "input_cost_per_token",
    )?;
    let output_cost_per_token = money(
        params.and_then(|values| values.get("output_cost_per_token")),
        "output_cost_per_token",
    )?;
    Ok(A2aCostParams {
        response_cost,
        cost_per_query,
        input_cost_per_token,
        output_cost_per_token,
    })
}

fn litellm_params(
    details: &IndexMap<String, Json>,
) -> Result<Option<&IndexMap<String, Json>>, CostCalculatorError> {
    match details.get("litellm_params") {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Object(values)) => Ok(Some(values)),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(CostCalculatorError::ParamsNotObject)
        }
    }
}

fn money(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<Decimal>, CostCalculatorError> {
    let text = match value {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Number(number)) => number.to_string(),
        Some(Json::Text(text)) => text.clone(),
        Some(Json::Bool(_) | Json::List(_) | Json::Object(_)) => {
            return Err(CostCalculatorError::ParamType { field });
        }
    };
    decimal(&text)
        .map(Some)
        .ok_or(CostCalculatorError::ParamType { field })
}

// LiteLLM reads prices with float(), which accepts a decimal or scientific notation string
fn decimal(text: &str) -> Option<Decimal> {
    let trimmed = text.trim();
    Decimal::from_str(trimmed)
        .or_else(|_error| Decimal::from_scientific(trimmed))
        .ok()
}

fn usage(value: Option<&Json>) -> Result<Option<A2aUsage>, CostCalculatorError> {
    let Some(record) = value.filter(|item| !matches!(item, Json::Null)) else {
        return Ok(None);
    };
    let Json::Object(fields) = record else {
        return Err(CostCalculatorError::UsageNotObject);
    };
    let prompt_tokens = tokens(fields.get("prompt_tokens"), "usage.prompt_tokens")?;
    let completion_tokens = tokens(fields.get("completion_tokens"), "usage.completion_tokens")?;
    Ok(Some(A2aUsage {
        prompt_tokens,
        completion_tokens,
    }))
}

fn tokens(value: Option<&Json>, field: &'static str) -> Result<u64, CostCalculatorError> {
    if matches!(value, None | Some(Json::Null)) {
        return Ok(0);
    }
    if let Some(Json::Number(number)) = value {
        return number
            .as_u64()
            .ok_or(CostCalculatorError::UsageType { field });
    }
    Err(CostCalculatorError::UsageType { field })
}
