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
    if let Input::UnwrapVerdict { response } = input {
        return unwrap_value(response);
    }
    if let Input::HasDecisionFields { response } = input {
        return has_fields_value(response);
    }
    if let Input::NormalizeAction { action } = input {
        return Ok(json!(
            betterlite_guardrails::cisco_verdict::normalize_action(action)
        ));
    }
    if let Input::ExtractJsonrpcError { response } = input {
        return error_value(response);
    }
    if let Input::ExtractSanitizedText { response } = input {
        return text_value(response);
    }
    if let Input::ExtractSanitizedMessages { response } = input {
        return messages_value(response);
    }
    if let Input::ExtractSanitizedMcpArguments { response } = input {
        return arguments_value(response);
    }
    let Input::ExtractMaskedEntityCount { rules } = input else {
        return Err(serde_json::Error::io(std::io::Error::other(
            "unrecognized Cisco verdict input",
        )));
    };
    count_value(rules)
}

fn unwrap_value(response: &Value) -> Result<Value, serde_json::Error> {
    let parsed = as_json(response)?;
    let verdict = betterlite_guardrails::cisco_verdict::unwrap_verdict_envelope(&parsed);
    to_value(&verdict)
}

fn has_fields_value(response: &Value) -> Result<Value, serde_json::Error> {
    let parsed = as_json(response)?;
    let result = betterlite_guardrails::cisco_verdict::has_decision_fields(&parsed);
    Ok(json!(result))
}

fn error_value(response: &Value) -> Result<Value, serde_json::Error> {
    let parsed = as_json(response)?;
    betterlite_guardrails::cisco_verdict::extract_jsonrpc_error(&parsed)
        .map_or(Ok(Value::Null), |value| to_value(&value))
}

fn text_value(response: &Value) -> Result<Value, serde_json::Error> {
    let parsed = as_json(response)?;
    let result = betterlite_guardrails::cisco_verdict::extract_sanitized_text(&parsed)
        .map_or(Value::Null, Value::String);
    Ok(result)
}

fn messages_value(response: &Value) -> Result<Value, serde_json::Error> {
    let parsed = as_json(response)?;
    let result = betterlite_guardrails::cisco_verdict::extract_sanitized_messages(&parsed)
        .map_or(Value::Null, |messages| json!(messages));
    Ok(result)
}

fn arguments_value(response: &Value) -> Result<Value, serde_json::Error> {
    let parsed = as_json(response)?;
    let result = betterlite_guardrails::cisco_verdict::extract_mcp_args(&parsed)
        .map_or(Value::Null, |arguments| json!(arguments));
    Ok(result)
}

fn count_value(rules: &Value) -> Result<Value, serde_json::Error> {
    let parsed = as_json(rules)?;
    let result = betterlite_guardrails::cisco_verdict::extract_masked_entity_count(&parsed)
        .map_or(Value::Null, |counts| json!(counts));
    Ok(result)
}

fn as_json(value: &Value) -> Result<Json, serde_json::Error> {
    let text = serde_json::to_string(value)?;
    Ok(Json::parse(&text))
}

fn to_value(value: &Json) -> Result<Value, serde_json::Error> {
    if let Json::Null = value {
        return Ok(Value::Null);
    }
    if let Json::Bool(flag) = value {
        return Ok(json!(flag));
    }
    if let Json::Number(number) = value {
        return serde_json::from_str(number);
    }
    if let Json::Text(text) = value {
        return Ok(Value::String(text.clone()));
    }
    if let Json::List(items) = value {
        return items
            .iter()
            .map(|item| serde_json::from_str(item.get()))
            .collect::<Result<Vec<Value>, _>>()
            .map(Value::Array);
    }
    let Json::Object(fields) = value else {
        return Err(serde_json::Error::io(std::io::Error::other(
            "unrecognized Cisco JSON value",
        )));
    };
    fields
        .iter()
        .map(|(key, item)| serde_json::from_str(item.get()).map(|parsed| (key.clone(), parsed)))
        .collect::<Result<serde_json::Map<String, Value>, _>>()
        .map(Value::Object)
}
