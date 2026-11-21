use betterlite_logging::analytics::python::float_text;

use super::RecordJson;
use super::decode::utf8_code_points;
use super::text::RecordText;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rendering {
    // Every key and number as it was read
    AsRead,
    // json.dumps(sort_keys=True) of what json.loads built: keys in code point order, numbers as the int or float they became
    Canonical,
}

#[must_use]
pub fn dumps(value: &RecordJson, rendering: Rendering) -> String {
    match value {
        RecordJson::Null => "null".to_owned(),
        RecordJson::Bool(flag) => flag.to_string(),
        RecordJson::Number(literal) => match rendering {
            Rendering::AsRead => literal.clone(),
            Rendering::Canonical => number_str(literal),
        },
        RecordJson::Text(text) => text_json(text),
        RecordJson::List(items) => {
            let parts: Vec<String> = items.iter().map(|item| dumps(item, rendering)).collect();
            format!("[{}]", parts.join(","))
        }
        RecordJson::Object(members) => {
            let mut entries: Vec<(&RecordText, &RecordJson)> = members.iter().collect();
            if rendering == Rendering::Canonical {
                entries.sort_by_key(|(key, _)| *key);
            }
            let parts: Vec<String> = entries
                .into_iter()
                .map(|(key, item)| format!("{}:{}", text_json(key), dumps(item, rendering)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

// str() of the int or float json.loads read from the literal
#[must_use]
pub fn number_str(literal: &str) -> String {
    match literal {
        "NaN" => "nan".to_owned(),
        "Infinity" => "inf".to_owned(),
        "-Infinity" => "-inf".to_owned(),
        "-0" => "0".to_owned(),
        float if float.contains(['.', 'e', 'E']) => float
            .parse::<f64>()
            .map_or_else(|_| float.to_owned(), float_text),
        integer => integer.to_owned(),
    }
}

fn text_json(text: &RecordText) -> String {
    let code_points = utf8_code_points(text.as_ref()).unwrap_or_default();
    let escaped: String = code_points.into_iter().map(escaped_code_point).collect();
    format!("\"{escaped}\"")
}

// A lone surrogate has no UTF-8 form, so it is written as the escape it was read from
fn escaped_code_point(code_point: u32) -> String {
    match char::from_u32(code_point) {
        Some('"') => "\\\"".to_owned(),
        Some('\\') => "\\\\".to_owned(),
        Some(character @ ' '..) => character.to_string(),
        Some(_) | None => format!("\\u{code_point:04x}"),
    }
}
