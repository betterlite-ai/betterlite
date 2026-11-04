use super::{InputTokensDetails, OutputTokensDetails, ResponseApiUsage};
use betterlite_models::decisions::Json;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Deserialize)]
pub(super) struct ResponseApiUsageWire {
    input_tokens: i64,
    input_tokens_details: Option<InputTokensDetails>,
    output_tokens: i64,
    output_tokens_details: Option<OutputTokensDetails>,
    total_tokens: i64,
    cost: Option<Value>,
    #[serde(flatten)]
    extra: BTreeMap<String, Json>,
}

#[derive(Debug, Error)]
pub(super) enum CostError {
    #[error("cost must be a number or a mapping with numeric total_cost")]
    InvalidType,
    #[error("cost is not a valid decimal")]
    Decimal(#[from] rust_decimal::Error),
}

impl TryFrom<ResponseApiUsageWire> for ResponseApiUsage {
    type Error = CostError;

    fn try_from(usage: ResponseApiUsageWire) -> Result<Self, Self::Error> {
        let cost = parse_cost(usage.cost)?;
        Ok(Self {
            input_tokens: usage.input_tokens,
            input_tokens_details: usage.input_tokens_details,
            output_tokens: usage.output_tokens,
            output_tokens_details: usage.output_tokens_details,
            total_tokens: usage.total_tokens,
            cost,
            extra: usage.extra,
        })
    }
}

fn parse_cost(encoded_cost: Option<Value>) -> Result<Option<Decimal>, CostError> {
    let Some(cost_value) = encoded_cost else {
        return Ok(None);
    };
    let encoded_number = match cost_value {
        Value::Null => None,
        Value::Number(input_number) => Some(input_number),
        Value::Object(mut fields) => match fields.remove("total_cost") {
            None | Some(Value::Null) => None,
            Some(Value::Number(mapping_number)) => Some(mapping_number),
            Some(_) => return Err(CostError::InvalidType),
        },
        Value::Bool(_) | Value::String(_) | Value::Array(_) => {
            return Err(CostError::InvalidType);
        }
    };
    encoded_number
        .map(|cost_number| {
            Decimal::from_str_exact(&cost_number.to_string()).map_err(CostError::from)
        })
        .transpose()
}
