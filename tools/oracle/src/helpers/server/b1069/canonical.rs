use serde_json::{Map, Value};

// the JSONB columns hold JSON text, so an answer parses them before comparing
const JSON_TEXT_FIELDS: [&str; 2] = ["litellm_params", "vector_store_metadata"];

// a JSONB column holds JSON text, so this parses it and drops nulls before the two answers compare
pub(super) fn canonical(value: Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut out = Map::new();
            for (key, field) in fields {
                if field.is_null() {
                    continue;
                }
                if JSON_TEXT_FIELDS.contains(&key.as_str())
                    && let Value::String(text) = &field
                    && let Ok(parsed) = serde_json::from_str::<Value>(text)
                {
                    out.insert(key, canonical(parsed));
                    continue;
                }
                out.insert(key, canonical(field));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(canonical).collect()),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)) => other,
    }
}
