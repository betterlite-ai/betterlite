use serde_json::Value;

pub(super) fn text_block(text: &str, role: &str) -> (Value, String) {
    let kind = if matches!(role, "user" | "system" | "developer" | "tool") {
        "input_text"
    } else {
        "output_text"
    };
    let value = serde_json::json!({"type": kind, "text": text});
    let diagnostic = format!(
        "{{'type': '{}', 'text': {}}}",
        kind,
        format_diagnostic_value(&Value::String(text.to_owned()))
    );
    (value, diagnostic)
}

pub(super) fn display_text_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        other => other.to_string(),
    }
}

pub(super) fn format_diagnostic_value(value: &Value) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::String(text) => format!("'{}'", text.replace('\\', "\\\\").replace('\'', "\\'")),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(format_diagnostic_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(fields) => {
            let mut keys = fields.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.iter()
                    .map(|key| format!("'{}': {}", key, format_diagnostic_value(&fields[*key])))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        Value::Number(number) => number.to_string(),
    }
}
