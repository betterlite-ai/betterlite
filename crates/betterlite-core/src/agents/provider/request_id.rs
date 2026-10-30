use crate::agents::json::AgentJson;

pub(super) fn python_string(value: &AgentJson) -> String {
    match value {
        AgentJson::Null => String::from("None"),
        AgentJson::Bool(true) => String::from("True"),
        AgentJson::Bool(false) => String::from("False"),
        AgentJson::Number(number) => number.to_string(),
        AgentJson::String(text) => quoted(text),
        AgentJson::Array(values) => {
            let items = values
                .iter()
                .map(python_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{items}]")
        }
        AgentJson::Object(values) => {
            let items = values
                .iter()
                .map(|(key, item)| format!("{}: {}", quoted(key), python_string(item)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{items}}}")
        }
    }
}

fn quoted(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::from(quote);
    for character in text.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            control if control.is_control() => {
                result.push_str(&format!("\\x{:02x}", u32::from(control)))
            }
            delimiter if delimiter == quote => {
                result.push('\\');
                result.push(delimiter);
            }
            other => result.push(other),
        }
    }
    result.push(quote);
    result
}
