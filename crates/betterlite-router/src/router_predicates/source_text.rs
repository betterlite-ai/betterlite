use std::fmt::Write as _;

use betterlite_models::decisions::Json;

// mock messages print through Python's str(), so a list or dict keeps Python's quotes, spacing and None
#[must_use]
pub fn source_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            repr_text(value)
        }
    }
}

fn repr_text(value: &Json) -> String {
    match value {
        Json::Null => String::from("None"),
        Json::Bool(true) => String::from("True"),
        Json::Bool(false) => String::from("False"),
        Json::Number(number) => number_text(number),
        Json::Text(text) => quoted_text(text),
        Json::List(items) => {
            let rendered: Vec<String> = items.iter().map(repr_text).collect();
            format!("[{}]", rendered.join(", "))
        }
        Json::Object(fields) => {
            let rendered: Vec<String> = fields
                .iter()
                .map(|(name, field)| format!("{}: {}", quoted_text(name), repr_text(field)))
                .collect();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

// repr prefers single quotes unless the text holds one and no double quote
fn quoted_text(text: &str) -> String {
    if text.contains('\'') && !text.contains('"') {
        return format!("\"{}\"", escaped_text(text, '"'));
    }
    format!("'{}'", escaped_text(text, '\''))
}

fn escaped_text(text: &str, quote: char) -> String {
    let mut escaped = String::new();
    for text_char in text.chars() {
        match text_char {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            code if u32::from(code) < 0x20 || u32::from(code) == 0x7f => {
                if write!(escaped, "\\x{:02x}", u32::from(code)).is_err() {
                    return escaped;
                }
            }
            current if current == quote => {
                escaped.push('\\');
                escaped.push(current);
            }
            other => escaped.push(other),
        }
    }
    escaped
}

fn number_text(number: &serde_json::Number) -> String {
    let literal = number.to_string();
    if !literal.contains('.') && !literal.contains('e') && !literal.contains('E') {
        return literal;
    }
    number.as_f64().map_or(literal, float_text)
}

// Python writes a float in exponent form as 1.5e-07, with a sign and at least two exponent digits
fn float_text(value: f64) -> String {
    let rendered = format!("{value:?}");
    let Some((mantissa, exponent)) = rendered.split_once('e') else {
        return rendered;
    };
    let (sign, digits) = exponent.strip_prefix('-').map_or_else(
        || ('+', exponent.strip_prefix('+').unwrap_or(exponent)),
        |rest| ('-', rest),
    );
    format!("{mantissa}e{sign}{digits:0>2}")
}
