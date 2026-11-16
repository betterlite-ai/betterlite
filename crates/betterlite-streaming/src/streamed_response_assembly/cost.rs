use std::str::FromStr as _;

use rust_decimal::Decimal;
use thiserror::Error;

use crate::stream_chunk_shaping::records::{JsonObject, JsonValue};
use crate::stream_usage_assembly::wire::Usage;

pub const XAI_PROVIDER: &str = "xai";
pub(crate) const RESPONSE_COST_KEY: &str = "response_cost";
const PROVIDER_KEY: &str = "custom_llm_provider";

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{message}")]
pub struct CostError {
    pub message: String,
}

// The fields LiteLLM's logging cost calculator and completion_cost read from the assembled response.
pub struct CostInput<'input> {
    pub model: &'input str,
    pub usage: &'input Usage,
    pub hidden_params: &'input JsonObject,
}

// B068: completion_cost(completion_response=..., custom_llm_provider=...) and cost_per_token(model=..., usage_object=...).
pub trait StreamCostCalculator: Send + Sync {
    fn completion_cost(
        &self,
        input: &CostInput<'_>,
        provider: Option<&str>,
    ) -> Result<Option<Decimal>, CostError>;

    fn prompt_completion_cost(
        &self,
        model: &str,
        usage: &Usage,
    ) -> Result<(Decimal, Decimal), CostError>;
}

// B068: Logging._response_cost_calculator plus the model_call_details it reads.
pub trait StreamLogging: Send + Sync {
    fn provider(&self) -> Option<&str>;

    fn response_cost(&self, input: &CostInput<'_>) -> Result<Option<Decimal>, CostError>;
}

// The logging calculator only stamps a positive cost, and an existing reported cost is never recomputed.
pub fn stamp_usage_cost(
    input: &CostInput<'_>,
    logging: Option<&dyn StreamLogging>,
) -> Result<Option<Decimal>, CostError> {
    let Some(active_logging) = logging else {
        return Ok(None);
    };
    if input.usage.cost.is_some() {
        return Ok(None);
    }
    match active_logging.response_cost(input)? {
        Some(computed) if computed > Decimal::ZERO => Ok(Some(computed)),
        Some(_) | None => Ok(None),
    }
}

pub fn stream_builder_response_cost(
    input: &CostInput<'_>,
    logging: Option<&dyn StreamLogging>,
    costs: &dyn StreamCostCalculator,
) -> Result<Option<Decimal>, CostError> {
    if let Some(reported) = input.usage.cost {
        // xai's reported cost is priced by the logging calculator, and only while a logging object exists.
        if logging.and_then(|active| active.provider()) == Some(XAI_PROVIDER) {
            return Ok(None);
        }
        return Ok(Some(reported));
    }
    if logging.is_some() {
        return Ok(None);
    }
    let provider = input
        .hidden_params
        .get(PROVIDER_KEY)
        .and_then(JsonValue::as_str);
    match costs.completion_cost(input, provider) {
        Ok(price) => Ok(price),
        Err(_error) => Ok(model_map_cost(input, costs)),
    }
}

fn model_map_cost(input: &CostInput<'_>, costs: &dyn StreamCostCalculator) -> Option<Decimal> {
    if input.model.is_empty() {
        return None;
    }
    let Ok((prompt, completion)) = costs.prompt_completion_cost(input.model, input.usage) else {
        return None;
    };
    prompt.checked_add(completion)
}

#[must_use]
pub fn response_cost_value(cost: Decimal) -> Option<JsonValue> {
    serde_json::Number::from_str(&cost.to_string())
        .ok()
        .map(JsonValue::Number)
}
