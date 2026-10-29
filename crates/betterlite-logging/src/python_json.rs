use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

mod tests;

// The text of Python's json.dumps with its defaults: ", " and ": " separators, ASCII escapes, keys in given order
#[must_use]
pub fn dumps(value: &Json) -> String {
    let mut out = String::new();
    push_value(&mut out, value);
    out
}

#[must_use]
pub fn dumps_object(object: &IndexMap<String, Json>) -> String {
    let mut out = String::new();
    push_object(&mut out, object);
    out
}

fn push_value(out: &mut String, value: &Json) {
    match value {
        Json::Null => out.push_str("null"),
        Json::Bool(true) => out.push_str("true"),
        Json::Bool(false) => out.push_str("false"),
        Json::Number(number) => out.push_str(&python_number(number)),
        Json::Text(text) => push_text(out, text),
        Json::List(items) => {
            out.push('[');
            for (position, item) in items.iter().enumerate() {
                if position != 0 {
                    out.push_str(", ");
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
            out.push_str(", ");
        }
        push_text(out, key);
        out.push_str(": ");
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

// json.loads reads a number with a fraction or an exponent as a float and any other as an int
fn python_number(number: &Number) -> String {
    let text = number.to_string();
    if !text.contains(['.', 'e', 'E']) {
        return if text == "-0" { "0".to_owned() } else { text };
    }
    let Ok(value) = text.parse::<f64>() else {
        return text;
    };
    // Rust's shortest float text switches to an exponent where Python's repr does, without its sign and padding
    let shortest = format!("{value:?}");
    match (shortest.as_str(), shortest.split_once('e')) {
        ("inf", _) => "Infinity".to_owned(),
        ("-inf", _) => "-Infinity".to_owned(),
        (plain, None) => plain.to_owned(),
        (_, Some((mantissa, power))) => power.strip_prefix('-').map_or_else(
            || format!("{mantissa}e+{power:0>2}"),
            |digits| format!("{mantissa}e-{digits:0>2}"),
        ),
    }
}
