use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::unified_dispatch::emit::{HttpErrorDetail, StreamHttpError};
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

pub(super) fn to_json(value: &GuardrailValue) -> HelperResult {
    match serde_json::to_value(value) {
        Ok(serialized) => Ok(serialized),
        Err(source) => Err(Box::new(HelperError {
            class: "SerializationError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: source.to_string(),
        })),
    }
}

pub(super) fn text_index(pairs: &[(i64, String)]) -> IndexMap<i64, String> {
    let mut map = IndexMap::new();
    for (index, text) in pairs {
        map.insert(*index, text.clone());
    }
    map
}

pub(super) fn int_index(pairs: &[(i64, i64)]) -> IndexMap<i64, i64> {
    let mut map = IndexMap::new();
    for (index, count) in pairs {
        map.insert(*index, *count);
    }
    map
}

pub(super) fn reason_index(pairs: &[(i64, Option<String>)]) -> IndexMap<i64, Option<String>> {
    let mut map = IndexMap::new();
    for (index, reason) in pairs {
        map.insert(*index, reason.clone());
    }
    map
}

pub(super) fn text_pairs(map: &IndexMap<i64, String>) -> Vec<Value> {
    map.iter()
        .map(|(index, text)| json!([index, text]))
        .collect()
}

pub(super) fn int_pairs(map: &IndexMap<i64, i64>) -> Vec<Value> {
    map.iter()
        .map(|(index, count)| json!([index, count]))
        .collect()
}

pub(super) fn reason_pairs(map: &IndexMap<i64, Option<String>>) -> Vec<Value> {
    map.iter()
        .map(|(index, reason)| json!([index, reason]))
        .collect()
}

pub(super) fn error_json(error: &StreamHttpError) -> HelperResult {
    let detail = match &error.detail {
        HttpErrorDetail::Text(text) => Value::String(text.clone()),
        HttpErrorDetail::Object(fields) => to_json(&GuardrailValue::Object(fields.clone()))?,
    };
    Ok(json!({
        "status_code": error.status_code,
        "detail": detail,
    }))
}
