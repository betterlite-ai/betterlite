use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::tool_permission::types::{mapping_value, object_list, object_mapping};
use serde_json::Value;

use super::Format;

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

fn tool_call_view(call: &GuardrailValue) -> Value {
    let id = mapping_value(call, "id").map_or(Value::Null, text_of);
    let kind = mapping_value(call, "type").map_or(Value::Null, text_of);
    serde_json::json!({
        "id": id,
        "type": kind,
        "function": function_view(mapping_value(call, "function")),
    })
}

fn tool_calls_view(calls: Option<&GuardrailValue>) -> Value {
    let Some(entries) = calls.and_then(object_list) else {
        return Value::Null;
    };
    Value::Array(entries.iter().map(tool_call_view).collect())
}

fn function_view(function: Option<&GuardrailValue>) -> Value {
    let Some(fields) = function.and_then(object_mapping) else {
        return Value::Null;
    };
    let name = fields.get("name").map_or(Value::Null, text_of);
    let arguments = fields.get("arguments").map_or(Value::Null, text_of);
    serde_json::json!({"name": name, "arguments": arguments})
}

fn message_view(message: &GuardrailValue) -> Value {
    let role = mapping_value(message, "role").map_or(Value::Null, text_of);
    let content = mapping_value(message, "content").map_or(Value::Null, text_of);
    let tool_calls = tool_calls_view(mapping_value(message, "tool_calls"));
    let function_call = function_view(mapping_value(message, "function_call"));
    serde_json::json!({
        "role": role,
        "content": content,
        "tool_calls": tool_calls,
        "function_call": function_call,
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
            let index = mapping_value(choice, "index").map_or(Value::Null, |value| {
                serde_json::to_value(value).unwrap_or(Value::Null)
            });
            let finish_reason = mapping_value(choice, "finish_reason").map_or(Value::Null, text_of);
            serde_json::json!({
                "index": index,
                "finish_reason": finish_reason,
                "message": message,
            })
        })
        .collect();
    Value::Array(projected)
}

fn block_view(block: &GuardrailValue) -> Value {
    let kind = mapping_value(block, "type").map_or(Value::Null, text_of);
    let text = mapping_value(block, "text").map_or(Value::Null, text_of);
    let id = mapping_value(block, "id").map_or(Value::Null, text_of);
    let name = mapping_value(block, "name").map_or(Value::Null, text_of);
    serde_json::json!({"type": kind, "text": text, "id": id, "name": name})
}

fn anthropic_view(response: &Object) -> Value {
    let content = response
        .get("content")
        .and_then(object_list)
        .map_or_else(Vec::new, |blocks| blocks.iter().map(block_view).collect());
    let stop_reason = response.get("stop_reason").map_or(Value::Null, text_of);
    serde_json::json!({
        "content": Value::Array(content),
        "stop_reason": stop_reason,
    })
}

pub(crate) fn response_view(format: Format, response: &GuardrailValue) -> Value {
    let Some(fields) = object_mapping(response) else {
        return Value::Null;
    };
    match format {
        Format::Openai => serde_json::json!({ "choices": choices_view(fields) }),
        Format::Anthropic => anthropic_view(fields),
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
