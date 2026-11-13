use serde_json::{Map, Value};

#[must_use]
pub fn flat_tool_choice(choice: &Value) -> Value {
    let Value::Object(fields) = choice else {
        return choice.clone();
    };
    let Some(kind) = fields.get("type").and_then(Value::as_str) else {
        return choice.clone();
    };
    if matches!(kind, "auto" | "none" | "required") {
        return Value::String(kind.to_owned());
    }
    if !matches!(kind, "function" | "custom") {
        return choice.clone();
    }
    let selected_name = fields
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            fields
                .get(kind)
                .and_then(Value::as_object)
                .and_then(|nested| nested.get("name"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
        });
    let Some(tool_name) = selected_name else {
        return choice.clone();
    };
    let mut normalized = Map::new();
    normalized.insert("type".to_owned(), Value::String(kind.to_owned()));
    normalized.insert("name".to_owned(), Value::String(tool_name.to_owned()));
    Value::Object(normalized)
}
