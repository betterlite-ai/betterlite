use std::fmt::Debug;
use std::str::FromStr as _;

use rust_decimal::Decimal;
use serde_json::{Map, Number, Value};

// B068 owns the billing arithmetic; the container config only asks its projection for the cost
pub trait ContainerCosts: Debug + Send + Sync {
    fn code_interpreter_cost(&self, sessions: u32, provider: &str) -> Decimal;
}

pub fn insert_code_interpreter_cost(hidden_params: &mut Map<String, Value>, cost: Decimal) {
    let Ok(number) = Number::from_str(&cost.normalize().to_string()) else {
        return;
    };
    let entry = hidden_params
        .entry(String::from("additional_headers"))
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(fields) = entry else {
        return;
    };
    fields.insert(
        String::from("llm_provider-x-litellm-response-cost"),
        Value::Number(number),
    );
}
