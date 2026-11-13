use std::collections::BTreeMap;
use std::iter::Peekable;
use std::str::Chars;

use rust_decimal::Decimal;

const FLOAT_MAGNITUDE_MASK: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum LogValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    List(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl LogValue {
    // the source returns the attribute itself and its caller tests Python truthiness
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(value) => *value,
            Self::Integer(value) => *value != 0,
            Self::Float(value) => (value.to_bits() & FLOAT_MAGNITUDE_MASK) != 0,
            Self::Text(value) => !value.is_empty(),
            Self::List(values) => !values.is_empty(),
            Self::Object(values) => !values.is_empty(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogArguments {
    None,
    Positional(Vec<LogValue>),
    Named(BTreeMap<String, LogValue>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MessageError {
    #[error("the log message ends inside a format specifier")]
    IncompleteSpecifier,
    #[error("the log message has no argument for a format specifier")]
    MissingArgument,
    #[error("the log message leaves a positional argument unused")]
    UnusedArguments,
    #[error("the log message uses an unsupported format specifier")]
    UnsupportedSpecifier,
    #[error("the log message uses a format specifier on a value it cannot print")]
    UnsupportedValue,
}

pub fn format_message(template: &str, arguments: &LogArguments) -> Result<String, MessageError> {
    let (positional, named) = match arguments {
        LogArguments::None => return Ok(template.to_owned()),
        LogArguments::Positional(values) => {
            if values.is_empty() {
                return Ok(template.to_owned());
            }
            (Some(values.as_slice()), None)
        }
        LogArguments::Named(values) => {
            if values.is_empty() {
                return Ok(template.to_owned());
            }
            (None, Some(values))
        }
    };
    let mut output = String::with_capacity(template.len());
    let mut characters = template.chars().peekable();
    let mut argument_index = 0_usize;
    loop {
        let Some(character) = characters.next() else {
            break;
        };
        if character != '%' {
            output.push(character);
            continue;
        }
        if characters.peek() == Some(&'%') {
            characters.next();
            output.push('%');
            continue;
        }
        let name = if characters.peek() == Some(&'(') {
            characters.next();
            Some(read_name(&mut characters)?)
        } else {
            None
        };
        let Some(specifier) = characters.next() else {
            return Err(MessageError::IncompleteSpecifier);
        };
        let value = value_for(positional, named, name.as_deref(), &mut argument_index)?;
        output.push_str(&render_conversion(value, specifier)?);
    }
    if positional.is_some_and(|values| argument_index < values.len()) {
        return Err(MessageError::UnusedArguments);
    }
    Ok(output)
}

fn read_name(characters: &mut Peekable<Chars<'_>>) -> Result<String, MessageError> {
    let mut name = String::new();
    loop {
        let Some(character) = characters.next() else {
            return Err(MessageError::IncompleteSpecifier);
        };
        if character == ')' {
            return Ok(name);
        }
        name.push(character);
    }
}

fn value_for<'arguments>(
    positional: Option<&'arguments [LogValue]>,
    named: Option<&'arguments BTreeMap<String, LogValue>>,
    name: Option<&str>,
    index: &mut usize,
) -> Result<&'arguments LogValue, MessageError> {
    if let Some(name) = name {
        return named
            .and_then(|values| values.get(name))
            .ok_or(MessageError::MissingArgument);
    }
    let position = *index;
    let value = positional
        .and_then(|values| values.get(position))
        .ok_or(MessageError::MissingArgument)?;
    *index = index.saturating_add(1);
    Ok(value)
}

fn render_conversion(value: &LogValue, specifier: char) -> Result<String, MessageError> {
    match specifier {
        's' => Ok(display_value(value)),
        'r' => Ok(repr_value(value)),
        'd' | 'i' => integer_value(value),
        'f' => float_value(value),
        _ => Err(MessageError::UnsupportedSpecifier),
    }
}

fn display_value(value: &LogValue) -> String {
    match value {
        LogValue::Null => "None".to_owned(),
        LogValue::Bool(true) => "True".to_owned(),
        LogValue::Bool(false) => "False".to_owned(),
        LogValue::Integer(number) => number.to_string(),
        LogValue::Float(number) => float_text(*number),
        LogValue::Text(text) => text.clone(),
        LogValue::List(items) => repr_sequence(items),
        LogValue::Object(entries) => repr_mapping(entries),
    }
}

fn repr_value(value: &LogValue) -> String {
    match value {
        LogValue::Text(text) => format!("'{}'", quote(text)),
        LogValue::Null
        | LogValue::Bool(_)
        | LogValue::Integer(_)
        | LogValue::Float(_)
        | LogValue::List(_)
        | LogValue::Object(_) => display_value(value),
    }
}

fn quote(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '\'' => quoted.push_str("\\'"),
            other => quoted.push(other),
        }
    }
    quoted
}

fn repr_sequence(items: &[LogValue]) -> String {
    let mut rendered = String::from("[");
    rendered.push_str(&items.iter().map(repr_value).collect::<Vec<_>>().join(", "));
    rendered.push(']');
    rendered
}

fn repr_mapping(entries: &BTreeMap<String, LogValue>) -> String {
    let mut rendered = String::from("{");
    let parts: Vec<String> = entries
        .iter()
        .map(|(key, value)| format!("'{}': {}", quote(key), repr_value(value)))
        .collect();
    rendered.push_str(&parts.join(", "));
    rendered.push('}');
    rendered
}

fn float_text(number: f64) -> String {
    let text = number.to_string();
    let plain = text
        .chars()
        .all(|character| character.is_ascii_digit() || character == '-');
    if plain {
        return format!("{text}.0");
    }
    text
}

fn integer_value(value: &LogValue) -> Result<String, MessageError> {
    match value {
        LogValue::Integer(number) => Ok(number.to_string()),
        LogValue::Float(number) => {
            let scaled = Decimal::from_f64_retain(*number).ok_or(MessageError::UnsupportedValue)?;
            let truncated = scaled.trunc();
            i64::try_from(truncated)
                .map(|value| value.to_string())
                .map_err(|_source| MessageError::UnsupportedValue)
        }
        LogValue::Null
        | LogValue::Bool(_)
        | LogValue::Text(_)
        | LogValue::List(_)
        | LogValue::Object(_) => Err(MessageError::UnsupportedValue),
    }
}

fn float_value(value: &LogValue) -> Result<String, MessageError> {
    let number = match value {
        LogValue::Integer(integer) => Decimal::from(*integer),
        LogValue::Float(float) => {
            Decimal::from_f64_retain(*float).ok_or(MessageError::UnsupportedValue)?
        }
        LogValue::Null
        | LogValue::Bool(_)
        | LogValue::Text(_)
        | LogValue::List(_)
        | LogValue::Object(_) => return Err(MessageError::UnsupportedValue),
    };
    Ok(format!("{}", number.round_dp(6)))
}
