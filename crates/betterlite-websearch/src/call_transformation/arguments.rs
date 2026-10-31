use std::fmt::Write as _;

use indexmap::IndexMap;
use serde::Deserialize;
use serde::ser::Error as _;
use serde_json::value::{Number, RawValue};

use super::TransformationError;
use super::wire::{Arguments, Input};

const MAX_ARGUMENT_DEPTH: usize = 32;
const MAX_ARGUMENT_NODES: usize = 20_000;
const MAX_ARGUMENT_BYTES: usize = 0x0010_0000;

#[derive(Default)]
pub(super) struct Budget {
    depth: usize,
    nodes: usize,
    bytes: usize,
}

fn bump(counter: &mut usize, limit: usize) -> Result<(), TransformationError> {
    *counter = counter
        .checked_add(1)
        .filter(|&value| value <= limit)
        .ok_or(TransformationError::ArgumentsTooComplex)?;
    Ok(())
}

fn charge_bytes(budget: &mut Budget, bytes: usize) -> Result<(), TransformationError> {
    budget.bytes = budget
        .bytes
        .checked_add(bytes)
        .filter(|&value| value <= MAX_ARGUMENT_BYTES)
        .ok_or(TransformationError::ArgumentsTooComplex)?;
    Ok(())
}

fn quote_with_budget(
    value: &str,
    python: bool,
    budget: &mut Budget,
) -> Result<String, TransformationError> {
    let quoted_bytes = value
        .len()
        .checked_mul(6)
        .and_then(|bytes| bytes.checked_add(2))
        .ok_or(TransformationError::ArgumentsTooComplex)?;
    charge_bytes(budget, quoted_bytes)?;
    quote(value, python).map_err(TransformationError::from)
}

pub(super) fn normalize(raw: &Arguments, responses: bool) -> Input {
    let Arguments::Present(value) = raw else {
        return Input::default();
    };
    if let Ok(text) = serde_json::from_str::<String>(value.get()) {
        let input = serde_json::from_str::<Input>(&text).unwrap_or_else(|_| {
            tracing::warn!("Cannot parse search function arguments");
            Input::default()
        });
        return input;
    }
    serde_json::from_str::<Input>(value.get()).map_or_else(
        |_| Input::default(),
        |input| normalize_input(input, value.get(), responses),
    )
}

fn normalize_input(input: Input, raw: &str, responses: bool) -> Input {
    if (responses && !matches!(&input, Input::Object(_))) || (!responses && is_false_value(raw)) {
        Input::default()
    } else {
        input
    }
}

fn is_false_value(raw: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<ParsedValue>(raw) else {
        return false;
    };
    match parsed {
        ParsedValue::Null(value) => {
            let () = value;
            true
        }
        ParsedValue::Bool(flag) => !flag,
        ParsedValue::Number(number) => number.as_f64() == Some(0.0),
        ParsedValue::Array(items) => items.is_empty(),
        ParsedValue::Object(object) => {
            drop(object);
            false
        }
        ParsedValue::String(string) => {
            drop(string);
            false
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ParsedValue {
    Null(()),
    Bool(bool),
    Number(Number),
    Array(Vec<Box<RawValue>>),
    Object(IndexMap<String, Box<RawValue>>),
    String(String),
}

pub(super) fn wire_text(input: &Input) -> Result<String, TransformationError> {
    let mut budget = Budget::default();
    bump(&mut budget.nodes, MAX_ARGUMENT_NODES)?;
    match input {
        Input::Object(fields) => object_text(fields, false, &mut budget),
        Input::Other(raw) => {
            if raw.get().as_bytes().first() == Some(&b'"') {
                charge_bytes(&mut budget, raw.get().len())?;
            }
            if let Ok(text_value) = serde_json::from_str::<String>(raw.get()) {
                return Ok(text_value);
            }
            text(raw, true, &mut budget)
        }
    }
}

fn object_text(
    fields: &IndexMap<String, Box<RawValue>>,
    python: bool,
    budget: &mut Budget,
) -> Result<String, TransformationError> {
    let entries = fields
        .iter()
        .map(|(key, value)| {
            charge_bytes(budget, key.len())?;
            Ok(format!(
                "{}: {}",
                quote_with_budget(key, python, budget)?,
                text(value, python, budget)?
            ))
        })
        .collect::<Result<Vec<_>, TransformationError>>()?;
    Ok(format!("{{{}}}", entries.join(", ")))
}

pub(super) fn text(
    raw: &RawValue,
    python: bool,
    budget: &mut Budget,
) -> Result<String, TransformationError> {
    bump(&mut budget.nodes, MAX_ARGUMENT_NODES)?;
    charge_bytes(budget, raw.get().len())?;
    match raw.get().as_bytes().first() {
        Some(b'{') => {
            bump(&mut budget.depth, MAX_ARGUMENT_DEPTH)?;
            let fields = serde_json::from_str(raw.get())?;
            let rendered = object_text(&fields, python, budget);
            budget.depth = budget.depth.saturating_sub(1);
            rendered
        }
        Some(b'[') => {
            bump(&mut budget.depth, MAX_ARGUMENT_DEPTH)?;
            let items: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
            let rendered = items
                .iter()
                .map(|item| text(item, python, budget))
                .collect::<Result<Vec<_>, TransformationError>>()
                .map(|values| format!("[{}]", values.join(", ")));
            budget.depth = budget.depth.saturating_sub(1);
            rendered
        }
        Some(b'"') => {
            let value = serde_json::from_str::<String>(raw.get())?;
            quote_with_budget(&value, python, budget)
        }
        Some(b'n') if python => Ok("None".to_owned()),
        Some(b't') if python => Ok("True".to_owned()),
        Some(b'f') if python => Ok("False".to_owned()),
        _ => Ok(raw.get().to_owned()),
    }
}

pub(super) fn quote(value: &str, python: bool) -> Result<String, serde_json::Error> {
    let quoted = serde_json::to_string(value)?;
    if python {
        let delimiter = if value.contains('\'') && !value.contains('"') {
            '"'
        } else {
            '\''
        };
        let escaped = value
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
            .replace(delimiter, &format!("\\{delimiter}"));
        return Ok(format!("{delimiter}{escaped}{delimiter}"));
    }
    let mut ascii = String::new();
    for character in quoted.chars() {
        if character.is_ascii() && character != '\u{7f}' {
            ascii.push(character);
        } else {
            let mut units = [0_u16; 2];
            for unit in character.encode_utf16(&mut units) {
                write!(ascii, "\\u{unit:04x}").map_err(serde_json::Error::custom)?;
            }
        }
    }
    Ok(ascii)
}
