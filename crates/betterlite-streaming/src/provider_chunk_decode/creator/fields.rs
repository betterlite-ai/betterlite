use crate::stream_chunk_shaping::records::{
    JsonObject, JsonValue, StreamChoice, StreamChunk, Usage,
};

pub(super) fn apply_provider_fields(model_response: &mut StreamChunk, fields: &JsonObject) {
    for (key, value) in fields {
        if !apply_declared_field(model_response, key, value) {
            model_response.extra.insert(key.clone(), value.clone());
        }
    }
}

fn apply_declared_field(model_response: &mut StreamChunk, key: &str, value: &JsonValue) -> bool {
    match (key, value) {
        ("id", JsonValue::String(text)) => model_response.id.clone_from(text),
        ("object", JsonValue::String(text)) => model_response.object.clone_from(text),
        ("model", JsonValue::String(text)) => model_response.model.clone_from(text),
        ("created", JsonValue::Number(number)) => {
            model_response.created = number.as_i64().unwrap_or(model_response.created);
        }
        ("system_fingerprint", fingerprint) => {
            model_response.system_fingerprint = string_of(fingerprint);
        }
        ("usage", counted) => model_response.usage = usage_of_value(counted),
        ("provider_specific_fields", nested) => {
            model_response.provider_specific_fields = object_of_value(nested);
        }
        ("hidden_params", JsonValue::Object(hidden)) => {
            model_response.hidden_params.clone_from(hidden);
        }
        ("choices", JsonValue::Array(choices)) => model_response.choices = choices_of(choices),
        _ => return false,
    }
    true
}

fn string_of(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

fn object_of_value(value: &JsonValue) -> Option<JsonObject> {
    match value {
        JsonValue::Object(fields) => Some(fields.clone()),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn usage_of_value(value: &JsonValue) -> Option<Usage> {
    match value {
        JsonValue::Object(fields) => Some(usage_of(fields)),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::String(_)
        | JsonValue::Array(_) => None,
    }
}

fn usage_of(fields: &JsonObject) -> Usage {
    Usage {
        prompt_tokens: count_of(fields, "prompt_tokens"),
        completion_tokens: count_of(fields, "completion_tokens"),
        total_tokens: count_of(fields, "total_tokens"),
        cost: None,
        prompt_tokens_details: None,
        completion_tokens_details: None,
        extra: JsonObject::new(),
    }
}

fn count_of(fields: &JsonObject, key: &str) -> i64 {
    fields
        .get(key)
        .and_then(JsonValue::as_i64)
        .unwrap_or_default()
}

fn choices_of(items: &[JsonValue]) -> Vec<StreamChoice> {
    items
        .iter()
        .filter_map(|item| {
            let plain = serde_json::to_value(item).ok()?;
            serde_json::from_value(plain).ok()
        })
        .collect()
}
