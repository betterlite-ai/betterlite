use betterlite_models::decisions::Json;
use serde_json::{Number, Value};

// numbers become text, since 3e-07 and 3e-7 are one float but two JSON spellings
fn number_text(found: &Number) -> String {
    let text = found.to_string();
    if !text.contains(['.', 'e', 'E']) {
        return format!("i:{text}");
    }
    let bits = found.as_f64().map_or(0, f64::to_bits);
    format!("f:{bits:016x}")
}

pub(super) fn project(value: &Json) -> Value {
    match value {
        Json::Null => Value::Null,
        Json::Bool(flag) => Value::Bool(*flag),
        Json::Number(found) => Value::String(number_text(found)),
        Json::Text(text) => Value::String(text.clone()),
        Json::List(items) => Value::Array(items.iter().map(project).collect()),
        Json::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, item)| (name.clone(), project(item)))
                .collect(),
        ),
    }
}
