use std::collections::BTreeMap;

use betterlite_error::envelope::WireField;
use betterlite_error::error::ErrorClass;
use serde_json::{Value, json};

use super::harness::Harness;

// the request as the fixture saw it: the method and target of the request line, and the headers the cases compare
pub(super) fn request(head: &str, wanted: &[String]) -> Value {
    if head.is_empty() {
        return json!({"method": null, "path": null, "headers": BTreeMap::<String, String>::new()});
    }
    let mut lines = head.lines();
    let mut parts = lines.next().unwrap_or_default().split(' ');
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    let headers: BTreeMap<String, String> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_lowercase(), value.trim().to_owned()))
        .filter(|(name, _value)| wanted.contains(name))
        .collect();
    json!({"method": method, "path": path, "headers": headers})
}

// the ten model_call_details keys LiteLLM writes around one agents call
pub(super) fn logging(harness: &Harness) -> Value {
    let state = serde_json::to_value(&harness.state.model_call_details).unwrap_or_default();
    let record = serde_json::to_value(&harness.record.model_call_details).unwrap_or_default();
    let at = |details: &Value, name: &str| details.get(name).cloned().unwrap_or(Value::Null);
    json!({
        "model": at(&state, "model"),
        "name": at(&state, "name"),
        "optional_params": at(&state, "optional_params"),
        "custom_llm_provider": at(&state, "custom_llm_provider"),
        "litellm_call_id": at(&state, "litellm_call_id"),
        "user": at(&state, "user"),
        "stream": at(&state, "stream"),
        "messages": at(&state, "messages"),
        "original_response": at(&record, "original_response"),
        "additional_args": at(&record, "additional_args"),
    })
}

// what litellm.exception_type reads off the wrapped error: status, type, param and code
pub(super) fn error(class: &ErrorClass) -> Value {
    let metadata = class.metadata();
    json!({
        "status": metadata.status_code,
        "type": wire(&metadata.error_type),
        "param": wire(&metadata.param),
        "code": metadata.openai_code,
    })
}

fn wire(field: &WireField) -> Option<String> {
    match field {
        WireField::Text(text) => Some(text.clone()),
        WireField::Null | WireField::Missing(_) => None,
    }
}
