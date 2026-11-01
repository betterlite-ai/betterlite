use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// A json.dumps(value, separators=(",", ":")) equivalent; python_json::dumps keeps json.dumps's default ", "/": " separators instead.
#[must_use]
pub fn compact_json(value: &Json) -> String {
    let mut out = String::new();
    push_value(&mut out, value);
    out
}

fn push_value(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(number) => out.push_str(&number.to_string()),
        Json::Text(text) => push_text(out, text),
        Json::List(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position != 0 {
                    out.push(',');
                }
                push_value(out, item);
            }
            out.push(']');
        }
        Json::Object(object) => push_object(out, object),
    }
}

fn push_object(out: &mut String, object: &IndexMap<String, Json>) {
    out.push('{');
    for (position, (key, item)) in object.iter().enumerate() {
        if position != 0 {
            out.push(',');
        }
        push_text(out, key);
        out.push(':');
        push_value(out, item);
    }
    out.push('}');
}

fn push_text(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(character),
            other => push_unicode_escape(out, other),
        }
    }
    out.push('"');
}

fn push_unicode_escape(out: &mut String, character: char) {
    let mut units = [0_u16; 2];
    for unit in character.encode_utf16(&mut units) {
        out.push_str("\\u");
        for byte in unit.to_be_bytes() {
            out.extend(char::from_digit(u32::from(byte.div_euclid(16)), 16));
            out.extend(char::from_digit(u32::from(byte.rem_euclid(16)), 16));
        }
    }
}
