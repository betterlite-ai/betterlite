use serde_json::Value;

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
    let name = fields
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
    name.map_or_else(
        || choice.clone(),
        |name| serde_json::json!({"type": kind, "name": name}),
    )
}
