use std::fmt::Write as _;

use betterlite_config::sections::settings::JsonValue;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[expect(
    clippy::derive_partial_eq_without_eq,
    reason = "a JSON number is a float, which has PartialEq but not Eq"
)]
pub struct OwnedValue {
    pub present: bool,
    #[serde(default)]
    pub value: JsonValue,
}

pub(super) const ABSENT: OwnedValue = OwnedValue {
    present: false,
    value: JsonValue::Null,
};

#[must_use]
pub(super) fn fingerprint(owned: &OwnedValue) -> String {
    let mut text = String::from("{\"present\": ");
    text.push_str(if owned.present { "true" } else { "false" });
    text.push_str(", \"value\": ");
    write_json(&owned.value, &mut text);
    text.push('}');
    hex::encode(Sha256::digest(text.as_bytes()))
}

fn write_json(value: &JsonValue, text: &mut String) {
    match value {
        JsonValue::Null => text.push_str("null"),
        JsonValue::Bool(flag) => text.push_str(if *flag { "true" } else { "false" }),
        JsonValue::Number(number) => text.push_str(&number.to_string()),
        JsonValue::String(content) => write_json_string(content, text),
        JsonValue::Array(items) => {
            text.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    text.push_str(", ");
                }
                write_json(item, text);
            }
            text.push(']');
        }
        JsonValue::Object(fields) => {
            text.push('{');
            let mut entries: Vec<(&String, &JsonValue)> = fields.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (index, (key, item)) in entries.iter().enumerate() {
                if index > 0 {
                    text.push_str(", ");
                }
                write_json_string(key, text);
                text.push_str(": ");
                write_json(item, text);
            }
            text.push('}');
        }
    }
}

fn write_json_string(content: &str, text: &mut String) {
    text.push('"');
    for character in content.chars() {
        match simple_escape(character) {
            Some(escaped) => text.push_str(escaped),
            None if character.is_ascii_graphic() || character == ' ' => text.push(character),
            None => write_unicode_escape(character, text),
        }
    }
    text.push('"');
}

const fn simple_escape(character: char) -> Option<&'static str> {
    match character {
        '"' => Some("\\\""),
        '\\' => Some("\\\\"),
        '\n' => Some("\\n"),
        '\r' => Some("\\r"),
        '\t' => Some("\\t"),
        '\u{08}' => Some("\\b"),
        '\u{0c}' => Some("\\f"),
        _ => None,
    }
}

fn write_unicode_escape(character: char, text: &mut String) {
    let mut units = [0_u16; 2];
    for unit in character.encode_utf16(&mut units) {
        text.push_str("\\u");
        let _unused = write!(text, "{unit:04x}");
    }
}
