use std::fmt::Debug;
use std::str::FromStr as _;

use rust_decimal::Decimal;
use serde_json::{Map, Number, Value};

use crate::api::container_contracts::error::ContainerError;

// B068 owns the billing arithmetic; the container config only asks its projection for the cost
pub trait ContainerCosts: Debug + Send + Sync {
    fn code_interpreter_cost(&self, sessions: u32, provider: &str) -> Decimal;
}

// A decoded record's hidden params are always empty, so the header object is built whole
pub fn cost_hidden_params(cost: Decimal) -> Result<Map<String, Value>, ContainerError> {
    let number = Number::from_str(&cost.normalize().to_string())
        .map_err(|error| ContainerError::Decode(error.to_string()))?;
    let mut headers = Map::new();
    headers.insert(
        String::from("llm_provider-x-litellm-response-cost"),
        Value::Number(number),
    );
    let mut hidden = Map::new();
    hidden.insert(String::from("additional_headers"), Value::Object(headers));
    Ok(hidden)
}
