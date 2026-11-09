use std::fmt::Write as _;

use indexmap::IndexMap;
use serde::Deserialize;

use super::float::render_float;

pub type KeyObject = IndexMap<String, KeyValue>;

// LiteLLM builds the key from the request kwargs, so a value keeps Python's str text; the caller's order is kept because the material concatenates params in request order.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum KeyValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Self>),
    Object(KeyObject),
}

impl KeyValue {
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Self::String(text) => text.clone(),
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::Array(_) | Self::Object(_) => {
                self.repr()
            }
        }
    }

    fn repr(&self) -> String {
        match self {
            Self::Null => "None".to_owned(),
            Self::Bool(true) => "True".to_owned(),
            Self::Bool(false) => "False".to_owned(),
            Self::Number(number) => number_text(number),
            Self::String(text) => quote(text),
            Self::Array(items) => {
                let rendered = items.iter().map(Self::repr).collect::<Vec<_>>();
                format!("[{}]", rendered.join(", "))
            }
            Self::Object(fields) => {
                let rendered = fields
                    .iter()
                    .map(|(name, value)| format!("{}: {}", quote(name), value.repr()))
                    .collect::<Vec<_>>();
                format!("{{{}}}", rendered.join(", "))
            }
        }
    }
}

#[must_use]
pub fn field<'source>(source: Option<&'source KeyObject>, name: &str) -> Option<&'source KeyValue> {
    source
        .and_then(|fields| fields.get(name))
        .filter(|value| !matches!(value, KeyValue::Null))
}

fn number_text(number: &serde_json::Number) -> String {
    if let Some(integer) = number.as_i64() {
        return integer.to_string();
    }
    if let Some(integer) = number.as_u64() {
        return integer.to_string();
    }
    number
        .as_f64()
        .map_or_else(|| number.to_string(), render_float)
}

// Python repr() quotes with ' unless the text holds one and no ", and escapes only what is not printable.
fn quote(text: &str) -> String {
    let delimiter = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(text.len().saturating_add(2));
    rendered.push(delimiter);
    for character in text.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            escaped if escaped == delimiter => rendered.extend(['\\', delimiter]),
            escaped if escaped.is_control() => {
                let _written = write!(&mut rendered, "\\x{:02x}", u32::from(escaped));
            }
            other => rendered.push(other),
        }
    }
    rendered.push(delimiter);
    rendered
}
