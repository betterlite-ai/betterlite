use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::tool_permission::types::{mapping_value, object_list, object_mapping};
use serde_json::Value;

use betterlite_oracle_helper_api::error::HelperError;

use super::support::failure;

fn text_of(value: &GuardrailValue) -> Value {
    match value {
        GuardrailValue::Text(text) => Value::String(text.clone()),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => Value::Null,
    }
}

fn field_text(fields: &Object, key: &str) -> Value {
    fields.get(key).map_or(Value::Null, text_of)
}

fn tool_call_view(call: &GuardrailValue) -> Value {
    let function = mapping_value(call, "function");
    serde_json::json!({
        "id": field_text_of(call, "id"),
        "type": field_text_of(call, "type"),
        "function": {
            "name": function.map_or(Value::Null, |function| field_text_of(function, "name")),
            "arguments": function.map_or(Value::Null, |function| field_text_of(function, "arguments")),
        },
    })
}

fn field_text_of(value: &GuardrailValue, key: &str) -> Value {
    mapping_value(value, key).map_or(Value::Null, text_of)
}

fn tool_calls_view(calls: Option<&GuardrailValue>) -> Value {
    let Some(calls) = calls.and_then(object_list) else {
        return Value::Null;
    };
    Value::Array(calls.iter().map(tool_call_view).collect())
}

fn function_view(function: Option<&GuardrailValue>) -> Value {
    let Some(function) = function else {
        return Value::Null;
    };
    serde_json::json!({
        "name": field_text_of(function, "name"),
        "arguments": field_text_of(function, "arguments"),
    })
}

fn message_view(message: &GuardrailValue) -> Value {
    serde_json::json!({
        "role": field_text_of(message, "role"),
        "content": field_text_of(message, "content"),
        "tool_calls": tool_calls_view(mapping_value(message, "tool_calls")),
        "function_call": function_view(mapping_value(message, "function_call")),
    })
}

pub(crate) fn choices_view(response: &Object) -> Value {
    let Some(choices) = response.get("choices").and_then(object_list) else {
        return Value::Array(Vec::new());
    };
    let projected: Vec<Value> = choices
        .iter()
        .map(|choice| {
            let message = mapping_value(choice, "message")
                .or_else(|| mapping_value(choice, "delta"))
                .map_or(Value::Null, message_view);
            serde_json::json!({
                "index": mapping_value(choice, "index").map_or(Value::Null, exact),
                "finish_reason": field_text_of(choice, "finish_reason"),
                "message": message,
            })
        })
        .collect();
    Value::Array(projected)
}

fn exact(value: &GuardrailValue) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn block_view(block: &GuardrailValue) -> Value {
    serde_json::json!({
        "type": field_text_of(block, "type"),
        "text": field_text_of(block, "text"),
        "id": field_text_of(block, "id"),
        "name": field_text_of(block, "name"),
    })
}

fn anthropic_view(response: &Object) -> Value {
    let content = response
        .get("content")
        .and_then(object_list)
        .map_or_else(Vec::new, |blocks| blocks.iter().map(block_view).collect());
    serde_json::json!({
        "content": Value::Array(content),
        "stop_reason": field_text(response, "stop_reason"),
    })
}

pub(crate) fn response_view(
    format: &str,
    response: &GuardrailValue,
) -> Result<Value, Box<HelperError>> {
    let Some(fields) = object_mapping(response) else {
        return Ok(Value::Null);
    };
    match format {
        "openai" => Ok(serde_json::json!({ "choices": choices_view(fields) })),
        "anthropic" => Ok(anthropic_view(fields)),
        other => Err(failure(
            "ValueError",
            format!("unknown response format {other}"),
        )),
    }
}

pub(crate) fn applied_guardrails(data: &Object) -> Option<Vec<String>> {
    let bucket = match data.get("litellm_metadata") {
        Some(value) => value,
        None => data.get("metadata")?,
    };
    let names = mapping_value(bucket, "applied_guardrails")?;
    Some(
        object_list(names)?
            .iter()
            .filter_map(|name| match name {
                GuardrailValue::Text(text) => Some(text.clone()),
                GuardrailValue::Null
                | GuardrailValue::Boolean(_)
                | GuardrailValue::SignedInteger(_)
                | GuardrailValue::UnsignedInteger(_)
                | GuardrailValue::Decimal(_)
                | GuardrailValue::Array(_)
                | GuardrailValue::Object(_) => None,
            })
            .collect(),
    )
}

pub(crate) fn blocked_view(error: &GuardrailFailure) -> Option<Value> {
    let GuardrailFailure::GuardrailRaised {
        status_code,
        blocked_content,
        ..
    } = error
    else {
        return None;
    };
    Some(serde_json::json!({
        "status": status_code,
        "message": error.to_string(),
        "blocked_content": blocked_content,
    }))
}
