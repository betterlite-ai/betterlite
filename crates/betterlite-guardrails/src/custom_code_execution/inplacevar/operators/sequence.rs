use super::super::super::value::Value;
use super::super::{InplaceError, InplaceOp};
use super::{formatting, integer, unsupported};
pub(super) const MAX_REPEATED_ITEMS: usize = 0x0010_0000;
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
        return formatting::format(left, right);
    }
    None
}
fn merged(left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    let Value::Dict(first) = left else {
        return None;
    };
    if let Value::Dict(second) = right {
        let mut merged = first.clone();
        merged.extend(
            second
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        return Some(Ok(Value::Dict(merged)));
    }
    let (Value::List(items) | Value::Tuple(items)) = right else {
        return None;
    };
    let mut merged = first.clone();
    for item in items {
        let (Value::List(pair) | Value::Tuple(pair)) = item else {
            return Some(Err(unsupported(InplaceOp::BitOr, left, right)));
        };
        let [Value::Str(key), value] = pair.as_slice() else {
            return Some(Err(unsupported(InplaceOp::BitOr, left, right)));
        };
        merged.insert(key.clone(), value.clone());
    }
    Some(Ok(Value::Dict(merged)))
}

fn concatenate(left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    match (left, right) {
        (Value::Str(first), Value::Str(second)) => Some(Ok(Value::Str(format!("{first}{second}")))),
        (Value::Bytes(first), Value::Bytes(second)) => Some(Ok(Value::Bytes(
            [first.as_slice(), second.as_slice()].concat(),
        ))),
        (Value::Tuple(first), Value::Tuple(second)) => Some(Ok(Value::Tuple(
            [first.as_slice(), second.as_slice()].concat(),
        ))),
        (Value::List(items), extra) => extended(items, extra).map(|values| Ok(Value::List(values))),
        _ => None,
    }
}
fn extended(items: &[Value], extra: &Value) -> Option<Vec<Value>> {
    let mut values = items.to_vec();
    match extra {
        Value::List(second) | Value::Tuple(second) => values.extend(second.iter().cloned()),
        Value::Str(text) => values.extend(
            text.chars()
                .map(|character| Value::Str(character.to_string())),
        ),
        Value::Bytes(text) => values.extend(text.iter().map(|byte| Value::Int(i64::from(*byte)))),
        Value::Dict(map) => values.extend(map.keys().map(|key| Value::Str(key.clone()))),
        Value::None | Value::Bool(_) | Value::Int(_) | Value::Float(_) => return None,
    }
    Some(values)
}

fn repetition(left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    if let Some(count) = integer(right) {
        return repeat_value(left, count);
    }
    let count = integer(left)?;
    repeat_value(right, count)
}
fn repeat_value(value: &Value, count: i64) -> Option<Result<Value, InplaceError>> {
    if let Value::Str(text) = value {
        return Some(
            repeated_length(text.len(), count).map(|times| Value::Str(text.repeat(times))),
        );
    }
    if let Value::Bytes(text) = value {
        return Some(
            repeated_length(text.len(), count).map(|times| Value::Bytes(text.repeat(times))),
        );
    }
    if let Value::List(items) = value {
        return Some(repeated_items(items, count).map(Value::List));
    }
    if let Value::Tuple(items) = value {
        return Some(repeated_items(items, count).map(Value::Tuple));
    }
    None
}
