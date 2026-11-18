use std::str::FromStr as _;

use betterlite_models::decisions::{HiddenParams, Json};
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_openai::api::responses_objects::{
    ResponseApiUsage, ResponseOutputItem, ResponsesApiResponse,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use super::error::FileSearchToolsError;
use super::outputs::{FileSearchCallOutput, MessageOutput};
use super::wire;

// getattr(original_response, "model", ""): an absent attribute reads as "", while a declared null stays null
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OriginalModel {
    #[default]
    Absent,
    Null,
    Text(String),
}

// the fields _synthesize_responses_api_response copies from the provider's response, with None for an absent attribute
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OriginalResponse {
    pub id: Option<String>,
    pub created_at: Option<i64>,
    pub model: OriginalModel,
    pub usage: Option<ResponseApiUsage>,
    pub hidden_params: Option<HiddenParams>,
}

// the two output mappings the emulated flow built, in the order the response carries them
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputItems {
    pub call: FileSearchCallOutput,
    pub message: MessageOutput,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizedResponse {
    pub response: ResponsesApiResponse,
    pub hidden_params: HiddenParams,
}

// uuid.uuid4().hex and int(time.time()) are evaluated as Python call defaults, so both sources run before the original fields are read
pub fn synthesize_response<U, C>(
    original: &OriginalResponse,
    outputs: &OutputItems,
    first_hidden: Option<&HiddenParams>,
    mut uuid: U,
    now: C,
) -> Result<SynthesizedResponse, FileSearchToolsError>
where
    U: FnMut() -> String,
    C: FnOnce() -> Decimal,
{
    let generated_id = uuid();
    let timestamp = now()
        .to_i64()
        .ok_or(FileSearchToolsError::ClockOutOfRange)?;
    let mut response =
        serde_json::from_str::<ResponsesApiResponse>(r#"{"id":"","created_at":0,"output":[]}"#)
            .map_err(|source| FileSearchToolsError::Wire { source })?;
    response.id = original
        .id
        .clone()
        .unwrap_or_else(|| format!("resp_{generated_id}"));
    response.created_at = original.created_at.unwrap_or(timestamp);
    response.model = model_value(&original.model);
    response.object = Some("response".to_owned());
    response.status = Some("completed".to_owned());
    response.error = None;
    response.usage.clone_from(&original.usage);
    let call_item: ResponseOutputItem = wire(&outputs.call)?;
    let message_item: ResponseOutputItem = wire(&outputs.message)?;
    response.output = vec![call_item, message_item];
    let hidden_params = hidden_with_first_cost(original.hidden_params.as_ref(), first_hidden)?;
    Ok(SynthesizedResponse {
        response,
        hidden_params,
    })
}

fn model_value(model: &OriginalModel) -> Option<String> {
    match model {
        OriginalModel::Absent => Some(String::new()),
        OriginalModel::Null => None,
        OriginalModel::Text(text) => Some(text.clone()),
    }
}

// a missing _hidden_params attribute copies nothing and adds no first-response cost, while a first cost is added on top of the current one
fn hidden_with_first_cost(
    original: Option<&HiddenParams>,
    first: Option<&HiddenParams>,
) -> Result<HiddenParams, FileSearchToolsError> {
    let Some(current_hidden) = original else {
        return Ok(HiddenParams::default());
    };
    let mut hidden = (*current_hidden).clone();
    let Some(first_hidden) = first else {
        return Ok(hidden);
    };
    let Some(first_cost) = first_hidden.get("response_cost") else {
        return Ok(hidden);
    };
    if matches!(first_cost, Json::Null) {
        return Ok(hidden);
    }
    let current = match hidden.get("response_cost") {
        None => Decimal::ZERO,
        Some(value) => cost_value(value)?,
    };
    let total = current
        .checked_add(cost_value(first_cost)?)
        .ok_or(FileSearchToolsError::CostOverflow)?;
    let number = Number::from_str(&total.normalize().to_string())
        .map_err(|source| FileSearchToolsError::Wire { source })?;
    hidden.insert("response_cost".to_owned(), Json::Number(number));
    Ok(hidden)
}

fn cost_value(value: &Json) -> Result<Decimal, FileSearchToolsError> {
    match value {
        Json::Number(number) => parse_decimal_lexeme(&number.to_string())
            .map_err(|source| FileSearchToolsError::ResponseCost { source }),
        Json::Null => Ok(Decimal::ZERO),
        Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            Err(FileSearchToolsError::WrongType {
                field: "response_cost",
                expected: "a number",
            })
        }
    }
}
