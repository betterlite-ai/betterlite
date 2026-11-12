use betterlite_models::decisions::Json;
use serde_json::Number;
use unicode_general_category::{GeneralCategory, get_general_category};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Rendering {
    Repr,
    Json,
}

// Python prints a float in fixed notation for decimal exponents -4 through 15 and in scientific notation outside them.
fn float_text(value: f64) -> String {
    if value == 0.0_f64 || (1e-4_f64..1e16_f64).contains(&value.abs()) {
        let fixed = value.to_string();
        let point = if fixed.contains('.') { "" } else { ".0" };
        return format!("{fixed}{point}");
    }
    let scientific = format!("{value:e}");
    match scientific.split_once('e') {
        Some((mantissa, power)) => power.strip_prefix('-').map_or_else(
            || format!("{mantissa}e+{power:0>2}"),
            |digits| format!("{mantissa}e-{digits:0>2}"),
        ),
        None => scientific,
    }
}

fn unicode_escape(code: u32) -> String {
    match code {
        0..=0xff => format!("\\x{code:02x}"),
        0x100..=0xffff => format!("\\u{code:04x}"),
        _ => format!("\\U{code:08x}"),
    }
}

fn write_repr_text(text: &str, out: &mut String) {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    out.push(quote);
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ' ' => out.push(' '),
            _ if character == quote => out.extend(['\\', quote]),
            // str.isprintable rejects these categories, so repr escapes them
            _ if matches!(
                get_general_category(character),
                GeneralCategory::Control
                    | GeneralCategory::Format
                    | GeneralCategory::PrivateUse
                    | GeneralCategory::Unassigned
                    | GeneralCategory::LineSeparator
                    | GeneralCategory::ParagraphSeparator
                    | GeneralCategory::SpaceSeparator
            ) =>
            {
                out.push_str(&unicode_escape(u32::from(character)));
            }
            _ => out.push(character),
        }
    }
    out.push(quote);
}

// json.dumps with its default ensure_ascii: every character past ASCII becomes \uXXXX, in surrogate pairs above the BMP.
fn write_json_text(text: &str, out: &mut String) {
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
            _ if character.is_ascii() && !character.is_ascii_control() => out.push(character),
            _ => {
                for unit in character.encode_utf16(&mut [0; 2]) {
                    let escaped = format!("\\u{unit:04x}");
                    out.push_str(&escaped);
                }
            }
        }
    }
    out.push('"');
}

fn number_text(number: &Number) -> String {
    match number.as_f64() {
        Some(float) if number.is_f64() => float_text(float),
        Some(_) | None => number.to_string(),
    }
}

// The text Python's str() gives a nested value (its repr) or json.dumps gives it, with the default separators.
pub(super) fn write_value(value: &Json, rendering: Rendering, out: &mut String) {
    let repr = rendering == Rendering::Repr;
    match value {
        Json::Null => out.push_str(if repr { "None" } else { "null" }),
        Json::Bool(flag) => out.push_str(match (flag, repr) {
            (true, true) => "True",
            (false, true) => "False",
            (true, false) => "true",
            (false, false) => "false",
        }),
        Json::Number(number) => out.push_str(&number_text(number)),
        Json::Text(text) if repr => write_repr_text(text, out),
        Json::Text(text) => write_json_text(text, out),
        Json::List(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_value(item, rendering, out);
            }
            out.push(']');
        }
        Json::Object(fields) => {
            out.push('{');
            for (index, (key, item)) in fields.iter().enumerate() {
                if index > 0 {
                    out.push_str(", ");
                }
                write_value(&Json::Text(key.clone()), rendering, out);
                out.push_str(": ");
                write_value(item, rendering, out);
            }
            out.push('}');
        }
    }
}
