use std::fmt::Write as _;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;

// mirrors how json.loads keeps a dict in written order and json.dumps writes it back out
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum PyValue {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Object(IndexMap<String, Self>),
}

// json.dumps with its default separators (", ", ": ") and ensure_ascii=True
pub(super) fn python_json_text(value: &PyValue) -> String {
    match value {
        PyValue::Null => "null".to_owned(),
        PyValue::Bool(flag) => flag.to_string(),
        PyValue::Number(number) => python_number(number),
        PyValue::Text(text) => python_string(text),
        PyValue::List(items) => format!(
            "[{}]",
            items
                .iter()
                .map(python_json_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        PyValue::Object(entries) => format!(
            "{{{}}}",
            entries
                .iter()
                .map(|(key, entry)| format!("{}: {}", python_string(key), python_json_text(entry)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn python_string(text: &str) -> String {
    let mut encoded = String::from("\"");
    for character in text.chars() {
        write_escaped_char(&mut encoded, character);
    }
    encoded.push('"');
    encoded
}

fn write_escaped_char(encoded: &mut String, character: char) {
    match character {
        '"' => encoded.push_str("\\\""),
        '\\' => encoded.push_str("\\\\"),
        '\n' => encoded.push_str("\\n"),
        '\r' => encoded.push_str("\\r"),
        '\t' => encoded.push_str("\\t"),
        plain if plain.is_ascii() && !plain.is_ascii_control() => encoded.push(plain),
        other => write_unicode_escape(encoded, other),
    }
}

fn write_unicode_escape(encoded: &mut String, code_point: char) {
    let mut units = [0_u16; 2];
    let surrogate_pair = code_point.encode_utf16(&mut units);
    for unit in surrogate_pair {
        if write!(encoded, "\\u{unit:04x}").is_err() {
            return;
        }
    }
}

fn python_number(number: &Number) -> String {
    let text = number.to_string();
    // arbitrary_precision keeps the source digits, so a fraction or exponent still needs float formatting
    if text.contains(['.', 'e', 'E']) {
        return text.parse().map_or(text, python_float);
    }
    if text == "-0" { "0".to_owned() } else { text }
}

fn python_float(value: f64) -> String {
    if value.is_infinite() {
        return if value.is_sign_positive() {
            "Infinity"
        } else {
            "-Infinity"
        }
        .to_owned();
    }
    let shortest = format!("{value:?}");
    let Some((mantissa, exponent)) = shortest.split_once('e') else {
        return shortest;
    };
    let (sign, digits) = exponent
        .strip_prefix('-')
        .map_or(("+", exponent), |digits| ("-", digits));
    format!("{mantissa}e{sign}{digits:0>2}")
}
