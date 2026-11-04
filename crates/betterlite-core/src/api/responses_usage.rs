use betterlite_models::modality_cost::error::ModalityCostError;
use betterlite_models::usage_sum::{CompletionTokensDetails, PromptTokensDetails};
use rust_decimal::Decimal;
use serde_json::{Map, Value};
use thiserror::Error;

mod conversion;
mod projection;
mod typed;
use projection::decimal_from_value;

#[derive(Debug, Error)]
pub enum ResponsesUsageError {
    #[error("usage JSON conversion failed: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("usage modality-cost conversion failed: {source}")]
    ModalityCost {
        #[source]
        source: ModalityCostError,
    },
}

impl From<serde_json::Error> for ResponsesUsageError {
    fn from(source: serde_json::Error) -> Self {
        Self::Json { source }
    }
}

impl From<ModalityCostError> for ResponsesUsageError {
    fn from(source: ModalityCostError) -> Self {
        Self::ModalityCost { source }
    }
}

const CHAT_FIELDS: [&str; 3] = ["prompt_tokens", "completion_tokens", "total_tokens"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub total_tokens: i64,
    pub prompt_tokens_details: Option<PromptTokensDetails>,
    pub completion_tokens_details: Option<CompletionTokensDetails>,
    pub cost: Option<Decimal>,
    pub extensions: Map<String, Value>,
}

#[derive(Clone, Copy)]
pub enum UsageInput<'usage> {
    Absent,
    Null,
    Existing(&'usage Usage),
    Mapping(&'usage Value),
}

pub fn normalize_usage(input: UsageInput<'_>) -> Result<Usage, ResponsesUsageError> {
    match input {
        UsageInput::Absent | UsageInput::Null => Ok(empty_usage()),
        UsageInput::Existing(usage) => Ok(usage.clone()),
        UsageInput::Mapping(value) => transform_response_api_usage(Some(value)),
    }
}

#[must_use]
pub fn is_response_api_usage(usage: &Map<String, Value>) -> bool {
    if !usage.contains_key("input_tokens") {
        return false;
    }
    usage.contains_key("output_tokens")
}

pub fn transform_response_api_usage(usage: Option<&Value>) -> Result<Usage, ResponsesUsageError> {
    let Some(input) = usage else {
        return Ok(empty_usage());
    };
    if input.is_null() {
        return Ok(empty_usage());
    }
    let Value::Object(fields) = input else {
        return Ok(ordinary_usage(&Map::new()));
    };
    if !is_response_api_usage(fields) {
        return Ok(ordinary_usage(fields));
    }
    conversion::from_response_usage(typed::parse(fields)?)
}

pub fn transform_value(usage: Option<&Value>) -> Result<Value, ResponsesUsageError> {
    match transform_response_api_usage(usage) {
        Ok(normalized) => Ok(projection::usage_value(&normalized)?),
        Err(error) => Err(error),
    }
}

fn empty_usage() -> Usage {
    Usage {
        prompt_tokens: 0,
        completion_tokens: 0,
        total_tokens: 0,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        cost: None,
        extensions: Map::new(),
    }
}

fn ordinary_usage(fields: &Map<String, Value>) -> Usage {
    let prompt = fields
        .get("prompt_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let completion = fields
        .get("completion_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let total = fields
        .get("total_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut extensions = fields.clone();
    for field in CHAT_FIELDS {
        extensions.remove(field);
    }
    let cost = fields.get("cost").and_then(decimal_from_value);
    extensions.remove("cost");
    Usage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: total,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        cost,
        extensions,
    }
}

#[path = "responses_usage/tests.rs"]
mod tests;
