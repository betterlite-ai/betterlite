use std::fmt::Write as _;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use unicode_general_category::{GeneralCategory, get_general_category};

const NON_PRINTABLE: [GeneralCategory; 8] = [
    GeneralCategory::Control,
    GeneralCategory::Format,
    GeneralCategory::PrivateUse,
    GeneralCategory::Surrogate,
    GeneralCategory::Unassigned,
    GeneralCategory::LineSeparator,
    GeneralCategory::ParagraphSeparator,
    GeneralCategory::SpaceSeparator,
];

// Python's str(): a string as given, and every other value by its repr text
#[must_use]
pub fn str_text(value: &GuardrailValue) -> String {
    match value {
        GuardrailValue::Text(text) => text.clone(),
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_)
        | GuardrailValue::Array(_)
        | GuardrailValue::Object(_) => repr_text(value),
    }
}

// Python's repr(): strings are quoted and the rest is written the way Python prints the value
#[must_use]
pub fn repr_text(value: &GuardrailValue) -> String {
    match value {
        GuardrailValue::Null => "None".to_owned(),
        GuardrailValue::Boolean(true) => "True".to_owned(),
        GuardrailValue::Boolean(false) => "False".to_owned(),
        GuardrailValue::SignedInteger(number) => number.to_string(),
        GuardrailValue::UnsignedInteger(number) => number.to_string(),
        GuardrailValue::Decimal(number) if number.is_finite() && number.fract() == 0.0 => {
            format!("{number:.1}")
        }
        GuardrailValue::Decimal(number) => number.to_string(),
        GuardrailValue::Text(text) => quote_text(text),
        GuardrailValue::Array(items) => {
            let rendered = items.iter().map(repr_text).collect::<Vec<_>>();
            format!("[{}]", rendered.join(", "))
        }
        GuardrailValue::Object(fields) => {
            let rendered = fields
                .iter()
                .map(|(name, item)| format!("{}: {}", quote_text(name), repr_text(item)))
                .collect::<Vec<_>>();
            format!("{{{}}}", rendered.join(", "))
        }
    }
}

// Python repr() quotes with ' unless the text holds one and no ", and escapes every character str.isprintable() rejects
#[must_use]
pub fn quote_text(text: &str) -> String {
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
            escaped if escaped == delimiter => {
                rendered.push('\\');
                rendered.push(escaped);
            }
            other if other == ' ' || !NON_PRINTABLE.contains(&get_general_category(other)) => {
                rendered.push(other);
            }
            other => {
                let _written = write_escape(&mut rendered, other);
            }
        }
    }
    rendered.push(delimiter);
    rendered
}

fn write_escape(rendered: &mut String, character: char) -> std::fmt::Result {
    let code = u32::from(character);
    if code < 0x100 {
        write!(rendered, "\\x{code:02x}")
    } else if code < 0x1_0000 {
        write!(rendered, "\\u{code:04x}")
    } else {
        write!(rendered, "\\U{code:08x}")
    }
}
