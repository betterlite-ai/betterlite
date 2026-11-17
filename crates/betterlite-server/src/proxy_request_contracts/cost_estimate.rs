use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};

use super::{ContractError, decode, default_amount, require_object};
use crate::key_fields::values::Amount;

const CACHE_SUBSET: &str =
    "cache_read_input_tokens plus cache_creation_input_tokens cannot exceed input_tokens";
const REASONING_SUBSET: &str = "reasoning_tokens cannot exceed output_tokens";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CostEstimateRequest {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_read_input_tokens: i64,
    #[serde(default)]
    pub cache_creation_input_tokens: i64,
    #[serde(default)]
    pub reasoning_tokens: i64,
    #[serde(default)]
    pub num_requests_per_day: Option<i64>,
    #[serde(default)]
    pub num_requests_per_month: Option<i64>,
}

impl CostEstimateRequest {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        request.check_counts()?;
        Ok(request)
    }

    // The field constraints run before the model's after-validator, so a negative count wins over a broken subset.
    fn check_counts(&self) -> Result<(), ContractError> {
        let counts = [
            ("input_tokens", self.input_tokens),
            ("output_tokens", self.output_tokens),
            ("cache_read_input_tokens", self.cache_read_input_tokens),
            (
                "cache_creation_input_tokens",
                self.cache_creation_input_tokens,
            ),
            ("reasoning_tokens", self.reasoning_tokens),
            (
                "num_requests_per_day",
                self.num_requests_per_day.unwrap_or(0),
            ),
            (
                "num_requests_per_month",
                self.num_requests_per_month.unwrap_or(0),
            ),
        ];
        if let Some((name, _)) = counts.iter().find(|entry| entry.1 < 0) {
            return Err(ContractError::from(format!("{name} must be at least 0")));
        }
        // LiteLLM checks the cache counts first, so a body that breaks both subsets reports the cache message.
        let cache_total = self
            .cache_read_input_tokens
            .checked_add(self.cache_creation_input_tokens);
        if cache_total.is_none_or(|total| total > self.input_tokens) {
            return Err(ContractError::from(CACHE_SUBSET));
        }
        if self.reasoning_tokens > self.output_tokens {
            return Err(ContractError::from(REASONING_SUBSET));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CostEstimateResponse {
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_read_input_tokens: i64,
    #[serde(default)]
    pub cache_creation_input_tokens: i64,
    #[serde(default)]
    pub reasoning_tokens: i64,
    #[serde(default)]
    pub num_requests_per_day: Option<i64>,
    #[serde(default)]
    pub num_requests_per_month: Option<i64>,
    pub cost_per_request: Amount,
    pub input_cost_per_request: Amount,
    pub output_cost_per_request: Amount,
    #[serde(default = "default_amount")]
    pub margin_cost_per_request: Amount,
    #[serde(default = "default_amount")]
    pub cache_read_cost_per_request: Amount,
    #[serde(default = "default_amount", rename = "cache_creation_cost_per_request")]
    pub cache_creation_cost: Amount,
    #[serde(default = "default_amount")]
    pub reasoning_cost_per_request: Amount,
    #[serde(default)]
    pub daily_cost: Option<Amount>,
    #[serde(default)]
    pub daily_input_cost: Option<Amount>,
    #[serde(default)]
    pub daily_output_cost: Option<Amount>,
    #[serde(default)]
    pub daily_margin_cost: Option<Amount>,
    #[serde(default)]
    pub daily_cache_read_cost: Option<Amount>,
    #[serde(default)]
    pub daily_cache_creation_cost: Option<Amount>,
    #[serde(default)]
    pub daily_reasoning_cost: Option<Amount>,
    #[serde(default)]
    pub monthly_cost: Option<Amount>,
    #[serde(default)]
    pub monthly_input_cost: Option<Amount>,
    #[serde(default)]
    pub monthly_output_cost: Option<Amount>,
    #[serde(default)]
    pub monthly_margin_cost: Option<Amount>,
    #[serde(default)]
    pub monthly_cache_read_cost: Option<Amount>,
    #[serde(default)]
    pub monthly_cache_creation_cost: Option<Amount>,
    #[serde(default)]
    pub monthly_reasoning_cost: Option<Amount>,
    #[serde(default)]
    pub input_cost_per_token: Option<Amount>,
    #[serde(default)]
    pub output_cost_per_token: Option<Amount>,
    #[serde(default)]
    pub cache_read_input_token_cost: Option<Amount>,
    #[serde(default, rename = "cache_creation_input_token_cost")]
    pub cache_creation_token_cost: Option<Amount>,
    #[serde(default, rename = "output_cost_per_reasoning_token")]
    pub output_reasoning_token_cost: Option<Amount>,
    #[serde(default)]
    pub provider: Option<String>,
}

impl CostEstimateResponse {
    pub fn from_value(value: &JsonValue) -> Result<Self, ContractError> {
        let fields = require_object(value)?;
        let request: Self = decode(&fields)?;
        Ok(request)
    }
}

mod tests;
