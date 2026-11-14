use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use serde_json::Number;

#[must_use]
pub fn python_str(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            python_repr(value)
        }
    }
}

#[must_use]
pub fn python_repr(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Number(number) => number_repr(number),
        Json::Text(text) => text_repr(text),
        Json::List(items) => {
            let rendered: Vec<String> = items.iter().map(python_repr).collect();
            format!("[{}]", rendered.join(", "))
        }
        Json::Object(object) => {
            let rendered: Vec<String> = object
                .iter()
                .map(|(key, value)| format!("{}: {}", text_repr(key), python_repr(value)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

// Python's repr for a string picks the quote the text does not contain
#[must_use]
pub fn text_repr(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::new();
    out.push(quote);
    for raw in text.chars() {
        match raw {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if character == quote => {
                out.push('\\');
                out.push(character);
            }
            control if control.is_control() => push_control_escape(&mut out, control),
            printable => out.push(printable),
        }
    }
    out.push(quote);
    out
}

fn push_control_escape(out: &mut String, character: char) {
    let rendered = format!("\\x{:02x}", u32::from(character));
    out.push_str(&rendered);
}

fn number_repr(number: &Number) -> String {
    let dumped = python_json::dumps(&Json::Number(number.clone()));
    match dumped.as_str() {
        "Infinity" => "inf".to_owned(),
        "-Infinity" => "-inf".to_owned(),
        "NaN" => "nan".to_owned(),
        finite => finite.to_owned(),
    }
}
