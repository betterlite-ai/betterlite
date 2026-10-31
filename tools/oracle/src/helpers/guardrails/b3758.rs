use betterlite_guardrails::cisco_conversation::Fields;
use betterlite_guardrails::cisco_conversation::json::Json;
use betterlite_guardrails::cisco_verdict;
use serde::Deserialize;
use serde_json::{Value, json};

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
        Ok(json!({"result": outcome(input)}))
    }
}

fn outcome(input: &Input) -> Value {
    match input {
        Input::UnwrapVerdict { response } => to_value(&cisco_verdict::unwrap_verdict_envelope(
            &Json::parse(&response.to_string()),
        )),
        Input::HasDecisionFields { response } => {
            json!(cisco_verdict::decision::has_decision_fields(&Json::parse(
                &response.to_string()
            )))
        }
        Input::NormalizeAction { action } => {
            json!(cisco_verdict::normalize_action(action).as_str())
        }
        Input::ExtractJsonrpcError { response } => {
            cisco_verdict::extract_jsonrpc_error(&Json::parse(&response.to_string()))
                .map_or(Value::Null, |error| to_value(&error))
        }
        Input::ExtractSanitizedText { response } => {
            cisco_verdict::extract_sanitized_text(&Json::parse(&response.to_string()))
                .map_or(Value::Null, Value::String)
        }
        Input::ExtractSanitizedMessages { response } => {
            cisco_verdict::extract_sanitized_messages(&Json::parse(&response.to_string()))
                .map_or(Value::Null, |messages| {
                    Value::Array(messages.iter().map(fields_to_value).collect())
                })
        }
        Input::ExtractSanitizedMcpArguments { response } => {
            cisco_verdict::extract_mcp_args(&Json::parse(&response.to_string()))
                .map_or(Value::Null, |arguments| fields_to_value(&arguments))
        }
        Input::ExtractMaskedEntityCount { rules } => {
            cisco_verdict::extract_masked_entity_count(&Json::parse(&rules.to_string()))
                .map_or(Value::Null, |counts| json!(counts))
        }
    }
}

fn to_value(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(text) => serde_json::from_str(text).unwrap_or(Value::Null),
        Json::Text(text) => Value::String(text.clone()),
        Json::List(items) => Value::Array(
            items
                .iter()
                .map(|item| serde_json::from_str(item.get()).unwrap_or(Value::Null))
                .collect(),
        ),
        Json::Object(fields) => fields_to_value(fields),
    }
}

fn fields_to_value(fields: &Fields) -> Value {
    let text = serde_json::to_string(fields).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or(Value::Null)
}
