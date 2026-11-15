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
    pub fn is_enabled(&self) -> bool {
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

#[derive(Debug, Clone, Copy)]
enum MessageSource<'arguments> {
    Positional(&'arguments [LogValue]),
    Named(&'arguments BTreeMap<String, LogValue>),
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
    let Some(source) = message_source(arguments) else {
        return Ok(template.to_owned());
    };
    let mut output = String::with_capacity(template.len());
    let mut characters = template.chars().peekable();
    let mut argument_index = 0_usize;
    while let Some(character) = characters.next() {
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
        let value = value_for(source, name.as_deref(), &mut argument_index)?;
        output.push_str(&render_conversion(value, specifier)?);
    }
    if matches!(source, MessageSource::Positional(values) if argument_index < values.len()) {
        return Err(MessageError::UnusedArguments);
    }
    Ok(output)
}

fn message_source(arguments: &LogArguments) -> Option<MessageSource<'_>> {
    match arguments {
        LogArguments::None => None,
        LogArguments::Positional(values) => {
            (!values.is_empty()).then_some(MessageSource::Positional(values.as_slice()))
        }
        LogArguments::Named(values) => (!values.is_empty()).then_some(MessageSource::Named(values)),
    }
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
    source: MessageSource<'arguments>,
    name: Option<&str>,
    index: &mut usize,
) -> Result<&'arguments LogValue, MessageError> {
    match (source, name) {
        (MessageSource::Named(values), Some(name)) => {
            values.get(name).ok_or(MessageError::MissingArgument)
        }
        (MessageSource::Positional(values), None) => {
            let position = *index;
            let value = values.get(position).ok_or(MessageError::MissingArgument)?;
            *index = index.saturating_add(1);
            Ok(value)
        }
        (MessageSource::Named(_), None) | (MessageSource::Positional(_), Some(_)) => {
            Err(MessageError::MissingArgument)
        }
    }
}

fn render_conversion(value: &LogValue, specifier: char) -> Result<String, MessageError> {
    match specifier {
        's' => Ok(display_value(value)),
        'r' => Ok(repr_value(value)),
        'd' | 'i' => integer_value(value),
        'f' => Ok(format!("{:.6}", decimal_of(value)?.round_dp(6))),
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
    if let LogValue::Text(text) = value {
        return format!("'{}'", quote(text));
    }
    display_value(value)
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

fn decimal_of(value: &LogValue) -> Result<Decimal, MessageError> {
    match value {
        LogValue::Integer(integer) => Ok(Decimal::from(*integer)),
        LogValue::Float(float) => {
            Decimal::from_f64_retain(*float).ok_or(MessageError::UnsupportedValue)
        }
        LogValue::Null
        | LogValue::Bool(_)
        | LogValue::Text(_)
        | LogValue::List(_)
        | LogValue::Object(_) => Err(MessageError::UnsupportedValue),
    }
}

fn integer_value(value: &LogValue) -> Result<String, MessageError> {
    i64::try_from(decimal_of(value)?.trunc())
        .map(|truncated| truncated.to_string())
        .map_err(|_source| MessageError::UnsupportedValue)
}
