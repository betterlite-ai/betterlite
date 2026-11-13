use std::fmt::Write as _;

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Number;

// serde_json sorts object keys and Python prints a dict in insertion order, so printed content keeps its own order
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum PyValue {
    Null,
    Bool(bool),
    Number(Number),
    Text(String),
    List(Vec<Self>),
    Dict(IndexMap<String, Self>),
}

// LiteLLM sends str(content) for a tool result that is not a string, so the token count is of Python's dict text
#[must_use]
pub fn display_text(value: &PyValue) -> String {
    match value {
        PyValue::Text(text) => text.clone(),
        PyValue::Null
        | PyValue::Bool(_)
        | PyValue::Number(_)
        | PyValue::List(_)
        | PyValue::Dict(_) => printed(value),
    }
}

fn printed(value: &PyValue) -> String {
    match value {
        PyValue::Null => String::from("None"),
        PyValue::Bool(true) => String::from("True"),
        PyValue::Bool(false) => String::from("False"),
        PyValue::Number(number) => number.to_string(),
        PyValue::Text(text) => quoted(text),
        PyValue::List(items) => {
            let parts: Vec<String> = items.iter().map(printed).collect();
            format!("[{}]", parts.join(", "))
        }
        PyValue::Dict(entries) => {
            let parts: Vec<String> = entries
                .iter()
                .map(|(key, item)| format!("{}: {}", quoted(key), printed(item)))
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
    }
}

fn quoted(text: &str) -> String {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            found if found == quote => {
                out.push('\\');
                out.push(found);
            }
            control if control.is_control() => {
                let code = u32::from(control);
                let _written = if code <= 0xff {
                    write!(out, "\\x{code:02x}")
                } else {
                    write!(out, "\\u{code:04x}")
                };
            }
            plain => out.push(plain),
        }
    }
    out.push(quote);
    out
}
