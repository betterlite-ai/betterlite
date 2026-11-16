use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::{Json, Object};
use serde_json::Number;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeliveryError {
    #[error("the value {value} is not a JSON number")]
    NotJsonNumber { value: f64 },
    #[error("the request data could not be rendered as JSON")]
    Serialize(#[source] serde_json::Error),
}

pub fn json_object(data: &BTreeMap<String, GuardrailValue>) -> Result<Object, DeliveryError> {
    data.iter()
        .map(|(key, value)| Ok((key.clone(), json_value(value)?)))
        .collect()
}

pub fn json_value(value: &GuardrailValue) -> Result<Json, DeliveryError> {
    match value {
        GuardrailValue::Null => Ok(Json::Null),
        GuardrailValue::Boolean(flag) => Ok(Json::Bool(*flag)),
        GuardrailValue::SignedInteger(number) => Ok(Json::Number(Number::from(*number))),
        GuardrailValue::UnsignedInteger(number) => Ok(Json::Number(Number::from(*number))),
        GuardrailValue::Decimal(number) => Number::from_f64(*number)
            .map(Json::Number)
            .ok_or(DeliveryError::NotJsonNumber { value: *number }),
        GuardrailValue::Text(text) => Ok(Json::Text(text.clone())),
        GuardrailValue::Array(items) => items
            .iter()
            .map(json_value)
            .collect::<Result<Vec<Json>, DeliveryError>>()
            .map(Json::List),
        GuardrailValue::Object(fields) => json_object(fields).map(Json::Object),
    }
}
