use std::num::FpCategory;

use serde_json::Number;
use unicode_general_category::{GeneralCategory, get_general_category};

use super::RoutingValue;

pub(super) fn python_str(value: &RoutingValue) -> String {
    let mut text = String::new();
    match value {
        RoutingValue::String(raw) => text.push_str(raw),
        RoutingValue::Null
        | RoutingValue::Bool(_)
        | RoutingValue::Number(_)
        | RoutingValue::Array(_)
        | RoutingValue::Object(_) => write_repr(value, &mut text),
    }
    text
}

pub(super) fn python_number(number: &Number) -> String {
    match number.as_f64() {
        Some(float) if number.is_f64() => python_float(float),
        Some(_) | None => number.to_string(),
    }
}

// Python's float repr stays in fixed notation only for decimal exponents -4 through 15
fn python_float(value: f64) -> String {
    if value.classify() == FpCategory::Zero || (1e-4..1e16).contains(&value.abs()) {
        let fixed = value.to_string();
        let point = if fixed.contains('.') { "" } else { ".0" };
        return format!("{fixed}{point}");
    }
    let scientific = format!("{value:e}");
    match scientific.split_once('e') {
        Some((mantissa, power)) => match power.strip_prefix('-') {
            Some(digits) => format!("{mantissa}e-{digits:0>2}"),
            None => format!("{mantissa}e+{power:0>2}"),
        },
        None => scientific,
    }
}

fn write_repr(value: &RoutingValue, out: &mut String) {
    let mut separator = "";
    match value {
        RoutingValue::Null => out.push_str("None"),
        RoutingValue::Bool(true) => out.push_str("True"),
        RoutingValue::Bool(false) => out.push_str("False"),
        RoutingValue::Number(number) => out.push_str(&python_number(number)),
        RoutingValue::String(text) => write_string(text, out),
        RoutingValue::Array(items) => {
            out.push('[');
            for item in items {
                out.push_str(separator);
                write_repr(item, out);
                separator = ", ";
            }
            out.push(']');
        }
        RoutingValue::Object(fields) => {
            out.push('{');
            for (key, item) in fields {
                out.push_str(separator);
                write_string(key, out);
                out.push_str(": ");
                write_repr(item, out);
                separator = ", ";
            }
            out.push('}');
        }
    }
}

fn write_string(text: &str, out: &mut String) {
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
            // Python's str.isprintable rejects these categories, so repr escapes them
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
                out.push_str(&hex_escape(character));
            }
            _ => out.push(character),
        }
    }
    out.push(quote);
}

fn hex_escape(character: char) -> String {
    let code = u32::from(character);
    match code {
        0..=0xff => format!("\\x{code:02x}"),
        0x100..=0xffff => format!("\\u{code:04x}"),
        _ => format!("\\U{code:08x}"),
    }
}
