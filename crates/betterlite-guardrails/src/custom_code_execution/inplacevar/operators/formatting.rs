use super::super::super::value::Value;
use super::super::{InplaceError, InplaceOp};
use super::{integer, unsupported};
use indexmap::IndexMap;
use std::str::from_utf8;
mod reprs;
const MINUS: u8 = 0b00001;
const ZERO: u8 = 0b00010;
const PLUS: u8 = 0b00100;
const SPACE: u8 = 0b01000;
const ALTERNATE: u8 = 0b10000;
#[derive(Default)]
struct Spec {
    key: Option<String>,
    flags: u8,
    width: usize,
    precision: Option<usize>,
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
    mapped: bool,
    positional: bool,
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
            mapped: false,
            positional: false,
        }
    }
    fn take(
        &mut self,
        spec: &Spec,
        left: &Value,
        right: &Value,
    ) -> Result<&'source Value, InplaceError> {
        if let Some(key) = spec.key.as_deref() {
            self.mapped = true;
            return self
                .mapping
                .and_then(|map| map.get(key))
                .ok_or_else(|| unsupported(InplaceOp::Modulo, left, right));
        }
        self.positional = true;
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
    while index < characters.len() {
        let Some(character) = characters.get(index).copied() else {
            break;
        };
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
    let leftover = arguments.used < arguments.sequence.len()
        && (arguments.used > 0 || arguments.mapping.is_none());
    if arguments.mapped && arguments.positional || leftover {
        return Err(unsupported(InplaceOp::Modulo, left, right));
    }
    Ok(output)
}
fn parse_spec(characters: &[char], mut index: usize) -> Option<(Spec, usize)> {
    let mut spec = Spec::default();
    if characters.get(index) == Some(&'(') {
        let start = index.saturating_add(1);
        let length = characters
            .get(start..)?
            .iter()
            .position(|character| *character == ')')?;
        spec.key = Some(
            characters
                .get(start..start.saturating_add(length))?
                .iter()
                .collect(),
        );
        index = start.saturating_add(length).saturating_add(1);
    }
    loop {
        match characters.get(index) {
            Some('#') => spec.flags |= ALTERNATE,
            Some('0') => spec.flags |= ZERO,
            Some('-') => spec.flags |= MINUS,
            Some('+') => spec.flags |= PLUS,
            Some(' ') => spec.flags |= SPACE,
            _ => break,
        }
        index = index.saturating_add(1);
    }
    let (width, after_width) = number(characters, index);
    spec.width = width;
    index = after_width;
    if characters.get(index) == Some(&'.') {
        let (precision, after_precision) = number(characters, index.saturating_add(1));
        spec.precision = Some(precision);
        index = after_precision;
    }
    spec.kind = *characters.get(index)?;
    Some((spec, index.saturating_add(1)))
}
fn number(characters: &[char], mut index: usize) -> (usize, usize) {
    let mut value = 0_usize;
    while let Some(digit) = characters
        .get(index)
        .and_then(|character| character.to_digit(10))
    {
        value = value
            .saturating_mul(10)
            .saturating_add(usize::try_from(digit).unwrap_or_default());
        index = index.saturating_add(1);
    }
    (value, index)
}
fn convert(spec: &Spec, value: &Value) -> Option<String> {
    let body = match spec.kind {
        's' => truncate(reprs::display(value, false)?, spec.precision),
        'r' => truncate(reprs::display(value, true)?, spec.precision),
        'd' | 'i' | 'u' => {
            let number = integer(value)?;
            sign_prefix(
                spec,
                number,
                &precision_digits(spec, number.unsigned_abs().to_string()),
            )
        }
        'x' => radix(spec, integer(value)?, 16, false),
        'X' => radix(spec, integer(value)?, 16, true),
        'o' => radix(spec, integer(value)?, 8, false),
        'c' => match value {
            Value::Str(text) if text.chars().count() == 1 => text.clone(),
            Value::Str(_) | Value::Bool(_) | Value::Int(_) => {
                let number = integer(value)?;
                let printable = u32::try_from(number).ok()?;
                String::from(char::from_u32(printable)?)
            }
            Value::None
            | Value::Float(_)
            | Value::Bytes(_)
            | Value::List(_)
            | Value::Tuple(_)
            | Value::Dict(_) => return None,
        },
        _ => return None,
    };
    Some(pad(
        spec,
        body,
        matches!(spec.kind, 'd' | 'i' | 'u' | 'x' | 'X' | 'o'),
    ))
}
fn precision_digits(spec: &Spec, mut digits: String) -> String {
    if let Some(precision) = spec.precision {
        while digits.len() < precision {
            digits.insert(0, '0');
        }
    }
    digits
}
fn sign_prefix(spec: &Spec, number: i64, digits: &str) -> String {
    if number < 0 {
        return format!("-{digits}");
    }
    if (spec.flags & PLUS) != 0 {
        return format!("+{digits}");
    }
    if (spec.flags & SPACE) != 0 {
        return format!(" {digits}");
    }
    digits.to_owned()
}
fn radix(spec: &Spec, number: i64, base: u32, upper: bool) -> String {
    let magnitude = number.unsigned_abs();
    let lower = match base {
        16 => format!("{magnitude:x}"),
        _ => format!("{magnitude:o}"),
    };
    let cased = if upper { lower.to_uppercase() } else { lower };
    let digits = precision_digits(spec, cased);
    let prefix = match base {
        16 if (spec.flags & ALTERNATE) != 0 && upper => "0X",
        16 if (spec.flags & ALTERNATE) != 0 => "0x",
        16 => "",
        _ if (spec.flags & ALTERNATE) != 0 => "0o",
        _ => "",
    };
    sign_prefix(spec, number, &format!("{prefix}{digits}"))
}
fn truncate(text: String, precision: Option<usize>) -> String {
    match precision {
        Some(limit) if text.chars().count() > limit => text.chars().take(limit).collect(),
        Some(_) | None => text,
    }
}
fn pad(spec: &Spec, body: String, numeric: bool) -> String {
    let length = body.chars().count();
    if spec.width <= length {
        return body;
    }
    let padding = spec.width.saturating_sub(length);
    if (spec.flags & MINUS) != 0 {
        return format!("{body}{}", " ".repeat(padding));
    }
    if (spec.flags & ZERO) != 0 && numeric {
        return zero_pad(&body, padding);
    }
    format!("{}{body}", " ".repeat(padding))
}
fn zero_pad(body: &str, padding: usize) -> String {
    let split = body
        .find(|character: char| character.is_ascii_digit())
        .unwrap_or(0);
    let (prefix, digits) = body.split_at(split);
    format!("{prefix}{}{digits}", "0".repeat(padding))
}
