use super::super::super::value::Value;
use super::super::{InplaceError, InplaceOp};
use super::sequence::MAX_REPEATED_ITEMS;
use super::{integer, unsupported};
use indexmap::IndexMap;
use std::str::from_utf8;
struct Spec {
    key: Option<String>,
    width: usize,
    kind: char,
}
pub(super) fn format(left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    match left {
        Value::Str(text) => Some(render(text, left, right, false).map(Value::Str)),
        Value::Bytes(template) => Some(from_utf8(template).map_or_else(
            |_error| Err(unsupported(InplaceOp::Modulo, left, right)),
            |text| render(text, left, right, true).map(|out| Value::Bytes(out.into_bytes())),
        )),
        Value::None
        | Value::Bool(_)
        | Value::Int(_)
        | Value::Float(_)
        | Value::List(_)
        | Value::Tuple(_)
        | Value::Dict(_) => None,
    }
}
struct Arguments<'source> {
    mapping: Option<&'source IndexMap<String, Value>>,
    sequence: Vec<&'source Value>,
    used: usize,
}
impl<'source> Arguments<'source> {
    fn new(right: &'source Value) -> Self {
        let mapping = match right {
            Value::Dict(map) => Some(map),
            Value::None
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Str(_)
            | Value::Bytes(_)
            | Value::List(_)
            | Value::Tuple(_) => None,
        };
        let sequence = match right {
            Value::Tuple(items) => items.iter().collect(),
            Value::None
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Str(_)
            | Value::Bytes(_)
            | Value::List(_)
            | Value::Dict(_) => vec![right],
        };
        Self {
            mapping,
            sequence,
            used: 0,
        }
    }
    fn take(
        &mut self,
        spec: &Spec,
        left: &Value,
        right: &Value,
    ) -> Result<&'source Value, InplaceError> {
        if let Some(key) = spec.key.as_deref() {
            return self
                .mapping
                .and_then(|map| map.get(key))
                .ok_or_else(|| unsupported(InplaceOp::Modulo, left, right));
        }
        let value = self
            .sequence
            .get(self.used)
            .copied()
            .ok_or_else(|| unsupported(InplaceOp::Modulo, left, right))?;
        self.used = self.used.saturating_add(1);
        Ok(value)
    }
}
fn render(text: &str, left: &Value, right: &Value, bytes: bool) -> Result<String, InplaceError> {
    let mut arguments = Arguments::new(right);
    let characters: Vec<char> = text.chars().collect();
    let mut output = String::new();
    let mut index = 0_usize;
    while let Some(character) = characters.get(index).copied() {
        index = index.saturating_add(1);
        if character != '%' {
            output.push(character);
            continue;
        }
        if characters.get(index) == Some(&'%') {
            output.push('%');
            index = index.saturating_add(1);
            continue;
        }
        let (spec, next) = parse_spec(&characters, index, left, right)?;
        index = next;
        let value = arguments.take(&spec, left, right)?;
        let rendered = convert(&spec, value, bytes)
            .ok_or_else(|| unsupported(InplaceOp::Modulo, left, right))?;
        output.push_str(&rendered);
    }
    let lenient = matches!(right, Value::Dict(_) | Value::List(_) | Value::Bytes(_));
    if arguments.used < arguments.sequence.len() && !lenient {
        return Err(unsupported(InplaceOp::Modulo, left, right));
    }
    Ok(output)
}
fn parse_key(characters: &[char], index: usize) -> Option<(Option<String>, usize)> {
    if characters.get(index) != Some(&'(') {
        return Some((None, index));
    }
    let start = index.saturating_add(1);
    let length = characters
        .get(start..)?
        .iter()
        .position(|character| *character == ')')?;
    let end = start.saturating_add(length);
    let key = characters.get(start..end)?.iter().collect();
    Some((Some(key), end.saturating_add(1)))
}
fn parse_spec(
    characters: &[char],
    start: usize,
    left: &Value,
    right: &Value,
) -> Result<(Spec, usize), InplaceError> {
    let unsupported_template = || unsupported(InplaceOp::Modulo, left, right);
    let (key, mut index) = parse_key(characters, start).ok_or_else(unsupported_template)?;
    let digits_start = index;
    while characters.get(index).is_some_and(char::is_ascii_digit) {
        index = index.saturating_add(1);
    }
    let digits: String = characters
        .get(digits_start..index)
        .unwrap_or_default()
        .iter()
        .collect();
    if digits.starts_with('0') {
        return Err(unsupported_template());
    }
    let width = if digits.is_empty() {
        0
    } else {
        digits
            .parse::<usize>()
            .map_err(|_error| InplaceError::Overflow {
                symbol: InplaceOp::Modulo.symbol(),
            })?
    };
    if width > MAX_REPEATED_ITEMS {
        return Err(InplaceError::RepetitionTooLong);
    }
    while matches!(characters.get(index), Some('h' | 'l' | 'L')) {
        index = index.saturating_add(1);
    }
    let kind = *characters.get(index).ok_or_else(unsupported_template)?;
    Ok((Spec { key, width, kind }, index.saturating_add(1)))
}
fn convert(spec: &Spec, value: &Value, bytes: bool) -> Option<String> {
    let body = match spec.kind {
        's' if bytes => return None,
        's' => display(value, false)?,
        'r' => display(value, true)?,
        'd' | 'i' => integer(value)?.to_string(),
        'x' => hex(integer(value)?),
        _ => return None,
    };
    let padding = spec.width.saturating_sub(body.chars().count());
    Some(format!("{}{body}", " ".repeat(padding)))
}
fn hex(number: i64) -> String {
    if number < 0 {
        return format!("-{:x}", number.unsigned_abs());
    }
    format!("{number:x}")
}
fn display(value: &Value, repr: bool) -> Option<String> {
    match value {
        Value::Str(text) if repr => Some(format!(
            "'{}'",
            text.replace('\\', "\\\\").replace('\'', "\\'")
        )),
        Value::Str(text) => Some(text.clone()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.to_owned()),
        Value::None => Some("None".to_owned()),
        Value::Int(number) => Some(number.to_string()),
        Value::List(items) => Some(format!("[{}]", joined(items)?)),
        Value::Float(_) | Value::Bytes(_) | Value::Tuple(_) | Value::Dict(_) => None,
    }
}
fn joined(items: &[Value]) -> Option<String> {
    let mut parts = Vec::new();
    for item in items {
        parts.push(display(item, true)?);
    }
    Some(parts.join(", "))
}
