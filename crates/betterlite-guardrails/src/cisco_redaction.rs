use serde::Serialize;
use serde_json::value::RawValue;

use crate::cisco_conversation::Fields;
use crate::cisco_conversation::json::Json;

pub mod chat_input;
pub mod chat_output;
pub mod mcp;
mod tests;

fn to_raw<Value>(value: &Value) -> Option<Box<RawValue>>
where
    Value: Serialize + ?Sized,
{
    let Ok(text) = serde_json::to_string(value) else {
        return None;
    };
    RawValue::from_string(text).ok()
}

fn set_serializable_field<Value>(fields: &mut Fields, key: &str, value: &Value)
where
    Value: Serialize + ?Sized,
{
    if let Some(raw) = to_raw(value) {
        fields.insert(key.to_owned(), raw);
    }
}

// Shared by chat-input and chat-output redaction to clear tool/function-call arguments the same way.
fn clear_tool_call_arguments(message: &mut Fields) {
    if let Json::List(tool_calls) = Json::field(message, "tool_calls") {
        let cleared: Vec<Box<RawValue>> = tool_calls
            .iter()
            .map(|call| clear_call_arguments(call))
            .collect();
        set_serializable_field(message, "tool_calls", &cleared);
    }
    if let Json::Object(mut function_call) = Json::field(message, "function_call") {
        set_serializable_field(&mut function_call, "arguments", &"{}");
        set_serializable_field(message, "function_call", &function_call);
    }
}

fn clear_call_arguments(raw_call: &RawValue) -> Box<RawValue> {
    let Json::Object(mut call) = Json::parse(raw_call.get()) else {
        return raw_call.to_owned();
    };
    if let Json::Object(mut function) = Json::field(&call, "function") {
        set_serializable_field(&mut function, "arguments", &"{}");
        set_serializable_field(&mut call, "function", &function);
    }
    to_raw(&call).unwrap_or_else(|| raw_call.to_owned())
}
