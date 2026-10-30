use betterlite_guardrails::cisco_conversation::json::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::HelperError;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    UnwrapVerdict { response: Value },
    HasDecisionFields { response: Value },
    NormalizeAction { action: String },
    ExtractJsonrpcError { response: Value },
    ExtractSanitizedText { response: Value },
    ExtractSanitizedMessages { response: Value },
    ExtractSanitizedMcpArguments { response: Value },
    ExtractMaskedEntityCount { rules: Value },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        outcome(input).map_err(|error| {
            Box::new(HelperError {
                class: "InvalidInput".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

fn outcome(input: &Input) -> Result<Value, serde_json::Error> {
    match input {
        Input::UnwrapVerdict { response } => {
            let parsed = as_json(response)?;
            to_value(&betterlite_guardrails::cisco_verdict::unwrap_verdict_envelope(&parsed))
        }
        Input::HasDecisionFields { response } => Ok(json!(
            betterlite_guardrails::cisco_verdict::has_decision_fields(&as_json(response)?)
        )),
        Input::NormalizeAction { action } => Ok(json!(
            betterlite_guardrails::cisco_verdict::normalize_action(action)
        )),
        Input::ExtractJsonrpcError { response } => {
            let parsed = as_json(response)?;
            betterlite_guardrails::cisco_verdict::extract_jsonrpc_error(&parsed)
                .map_or(Ok(Value::Null), |value| to_value(&value))
        }
        Input::ExtractSanitizedText { response } => {
            let parsed = as_json(response)?;
            Ok(
                betterlite_guardrails::cisco_verdict::extract_sanitized_text(&parsed)
                    .map_or(Value::Null, Value::String),
            )
        }
        Input::ExtractSanitizedMessages { response } => {
            let parsed = as_json(response)?;
            Ok(
                betterlite_guardrails::cisco_verdict::extract_sanitized_messages(&parsed)
                    .map_or(Value::Null, |messages| json!(messages)),
            )
        }
        Input::ExtractSanitizedMcpArguments { response } => {
            let parsed = as_json(response)?;
            Ok(
                betterlite_guardrails::cisco_verdict::extract_mcp_args(&parsed)
                    .map_or(Value::Null, |arguments| json!(arguments)),
            )
        }
        Input::ExtractMaskedEntityCount { rules } => {
            let parsed = as_json(rules)?;
            Ok(
                betterlite_guardrails::cisco_verdict::extract_masked_entity_count(&parsed)
                    .map_or(Value::Null, |counts| json!(counts)),
            )
        }
    }
}

fn as_json(value: &Value) -> Result<Json, serde_json::Error> {
    let text = serde_json::to_string(value)?;
    Ok(Json::parse(&text))
}

fn to_value(value: &Json) -> Result<Value, serde_json::Error> {
    match value {
        Json::Null => Ok(Value::Null),
        Json::Bool(flag) => Ok(json!(flag)),
        Json::Number(number) => serde_json::from_str(number),
        Json::Text(text) => Ok(Value::String(text.clone())),
        Json::List(items) => items
            .iter()
            .map(|item| serde_json::from_str(item.get()))
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array),
        Json::Object(fields) => fields
            .iter()
            .map(|(key, item)| serde_json::from_str(item.get()).map(|parsed| (key.clone(), parsed)))
            .collect::<Result<serde_json::Map<String, Value>, _>>()
            .map(Value::Object),
    }
}
