use super::super::super::value::Value;
use super::super::{InplaceError, InplaceOp};
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
        Value::Str(text) => Some(render(text, left, right).map(Value::Str)),
        Value::Bytes(template) => Some(render_bytes(template, left, right)),
        Value::None
        | Value::Bool(_)
        | Value::Int(_)
        | Value::Float(_)
        | Value::List(_)
        | Value::Tuple(_)
        | Value::Dict(_) => None,
    }
}
fn render_bytes(template: &[u8], left: &Value, right: &Value) -> Result<Value, InplaceError> {
    match from_utf8(template) {
        Ok(text) => render(text, left, right).map(|rendered| Value::Bytes(rendered.into_bytes())),
        Err(_error) => Err(unsupported(InplaceOp::Modulo, left, right)),
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
fn render(text: &str, left: &Value, right: &Value) -> Result<String, InplaceError> {
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
        let (spec, next) = parse_spec(&characters, index)
            .ok_or_else(|| unsupported(InplaceOp::Modulo, left, right))?;
        index = next;
        let value = arguments.take(&spec, left, right)?;
        let rendered =
            convert(&spec, value).ok_or_else(|| unsupported(InplaceOp::Modulo, left, right))?;
        output.push_str(&rendered);
    }
    let leftover = arguments.used < arguments.sequence.len() && arguments.mapping.is_none();
    if leftover {
        return Err(unsupported(InplaceOp::Modulo, left, right));
    }
    Ok(output)
}
fn parse_spec(characters: &[char], mut index: usize) -> Option<(Spec, usize)> {
    let mut key = None;
    if characters.get(index) == Some(&'(') {
        let start = index.saturating_add(1);
        let length = characters
            .get(start..)?
            .iter()
            .position(|character| *character == ')')?;
        key = Some(
            characters
                .get(start..start.saturating_add(length))?
                .iter()
                .collect(),
        );
        index = start.saturating_add(length).saturating_add(1);
    }
    let start = index;
    while characters.get(index).is_some_and(char::is_ascii_digit) {
        index = index.saturating_add(1);
    }
    let digits: String = characters.get(start..index)?.iter().collect();
    Some((
        Spec {
            key,
            width: digits.parse().unwrap_or(0),
            kind: *characters.get(index)?,
        },
        index.saturating_add(1),
    ))
}
fn convert(spec: &Spec, value: &Value) -> Option<String> {
    let body = match spec.kind {
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
