use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpanError {
    pub error_type: Option<String>,
    pub message: Option<String>,
    pub code: Option<String>,
    pub stack_trace: Option<String>,
    pub llm_provider: Option<String>,
}

impl SpanError {
    pub fn from_payload(payload: &BTreeMap<String, Value>) -> Option<Self> {
        let info = payload.get("error_information").and_then(Value::as_object);
        let message = info
            .and_then(|map| map.get("message"))
            .and_then(Value::as_str)
            .or_else(|| payload.get("error_str").and_then(Value::as_str));
        let error_type = info.and_then(|map| map.get("type")).and_then(Value::as_str);
        if message.is_none() && error_type.is_none() {
            return None;
        }
        Some(Self {
            error_type: error_type.map(str::to_owned),
            message: message.map(str::to_owned),
            code: info
                .and_then(|map| map.get("code"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            stack_trace: info
                .and_then(|map| map.get("stack_trace"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            llm_provider: info
                .and_then(|map| map.get("llm_provider"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        })
    }
}

#[path = "error/tests.rs"]
mod tests;
