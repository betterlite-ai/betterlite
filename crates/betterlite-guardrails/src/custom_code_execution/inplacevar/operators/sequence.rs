use super::super::super::value::Value;
use super::super::{InplaceError, InplaceOp};
use super::{integer, unsupported};

const MAX_REPEATED_ITEMS: usize = 0x0010_0000;

fn repeated_length(length: usize, count: i64) -> Result<usize, InplaceError> {
    let times = usize::try_from(count).unwrap_or(0);
    let total = length
        .checked_mul(times)
        .ok_or(InplaceError::RepetitionTooLong)?;
    if total > MAX_REPEATED_ITEMS {
        return Err(InplaceError::RepetitionTooLong);
    }
    Ok(times)
}

fn repeated_items(items: &[Value], count: i64) -> Result<Vec<Value>, InplaceError> {
    let times = repeated_length(items.len(), count)?;
    let mut repeated = Vec::new();
    for _ in 0..times {
        repeated.extend(items.iter().cloned());
    }
    Ok(repeated)
}

pub(crate) fn sequence(
    op: InplaceOp,
    left: &Value,
    right: &Value,
) -> Option<Result<Value, InplaceError>> {
    if op == InplaceOp::Add {
        return concatenate(left, right);
    }
    if op == InplaceOp::Multiply {
        return repetition(left, right);
    }
    if op == InplaceOp::BitOr {
        return merged(left, right);
    }
    if op == InplaceOp::Modulo {
        return formatted(left, right);
    }
    None
}

fn merged(left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    let (Value::Dict(first), Value::Dict(second)) = (left, right) else {
        return None;
    };
    let mut merged = first.clone();
    for (key, value) in second {
        merged.insert(key.clone(), value.clone());
    }
    Some(Ok(Value::Dict(merged)))
}

fn formatted(left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    let Value::Str(text) = left else {
        return None;
    };
    Some(percent_format(text, right))
}

fn text_of(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.clone()),
        Value::Int(number) => Some(number.to_string()),
        Value::Float(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "True" } else { "False" }.to_owned()),
        Value::None | Value::Bytes(_) | Value::List(_) | Value::Tuple(_) | Value::Dict(_) => None,
    }
}

// `s %= value` formats like Python's single-value percent formatting: one conversion, %% stays literal.
fn percent_format(text: &str, value: &Value) -> Result<Value, InplaceError> {
    let left = Value::Str(text.to_owned());
    let mut out = String::new();
    let mut converted = false;
    let mut chars = text.chars();
    while let Some(character) = chars.next() {
        if character != '%' {
            out.push(character);
            continue;
        }
        if converted {
            return Err(InplaceError::UnsupportedOperand {
                symbol: InplaceOp::Modulo.symbol(),
                left: "str",
                right: value.type_name(),
            });
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('s') => match text_of(value) {
                Some(rendered) => {
                    out.push_str(&rendered);
                    converted = true;
                }
                None => return Err(unsupported(InplaceOp::Modulo, &left, value)),
            },
            Some('d' | 'i') => match integer(value) {
                Some(number) => {
                    out.push_str(&number.to_string());
                    converted = true;
                }
                None => return Err(unsupported(InplaceOp::Modulo, &left, value)),
            },
            Some(_) | None => {
                return Err(unsupported(InplaceOp::Modulo, &left, value));
            }
        }
    }
    Ok(Value::Str(out))
}

fn concatenate(left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    if let (Value::Str(first), Value::Str(second)) = (left, right) {
        return Some(Ok(Value::Str(format!("{first}{second}"))));
    }
    if let (Value::Bytes(first), Value::Bytes(second)) = (left, right) {
        let mut joined = first.clone();
        joined.extend_from_slice(second);
        return Some(Ok(Value::Bytes(joined)));
    }
    if let (Value::List(first), Value::List(second)) = (left, right) {
        let mut joined = first.clone();
        joined.extend(second.iter().cloned());
        return Some(Ok(Value::List(joined)));
    }
    if let (Value::Tuple(first), Value::Tuple(second)) = (left, right) {
        let mut joined = first.clone();
        joined.extend(second.iter().cloned());
        return Some(Ok(Value::Tuple(joined)));
    }
    if let (Value::List(items), Value::Tuple(extra)) = (left, right) {
        let mut joined = items.clone();
        joined.extend(extra.iter().cloned());
        return Some(Ok(Value::List(joined)));
    }
    None
}

fn repetition(left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    match (left, right) {
        (Value::Str(text), Value::Int(count)) | (Value::Int(count), Value::Str(text)) => {
            Some(repeated_length(text.len(), *count).map(|times| Value::Str(text.repeat(times))))
        }
        (Value::Bytes(text), Value::Int(count)) | (Value::Int(count), Value::Bytes(text)) => {
            Some(repeated_length(text.len(), *count).map(|times| Value::Bytes(text.repeat(times))))
        }
        (Value::List(items), Value::Int(count)) | (Value::Int(count), Value::List(items)) => {
            Some(repeated_items(items, *count).map(Value::List))
        }
        (Value::Tuple(items), Value::Int(count)) | (Value::Int(count), Value::Tuple(items)) => {
            Some(repeated_items(items, *count).map(Value::Tuple))
        }
        _ => None,
    }
}
