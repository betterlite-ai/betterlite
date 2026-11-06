use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::super::value::Value;
use super::{InplaceError, InplaceOp};

const MAX_REPEATED_ITEMS: usize = 0x0010_0000;

#[must_use]
pub(super) const fn unsupported(op: InplaceOp, left: &Value, right: &Value) -> InplaceError {
    InplaceError::UnsupportedOperand {
        symbol: op.symbol(),
        left: left.type_name(),
        right: right.type_name(),
    }
}

const fn integer_pair(
    op: InplaceOp,
    left: &Value,
    right: &Value,
) -> Result<(i64, i64), InplaceError> {
    match (left, right) {
        (Value::Int(first), Value::Int(second)) => Ok((*first, *second)),
        _ => Err(unsupported(op, left, right)),
    }
}

fn int_result(op: InplaceOp, value: Option<i64>) -> Result<Value, InplaceError> {
    let symbol = op.symbol();
    value
        .map(Value::Int)
        .ok_or(InplaceError::Overflow { symbol })
}

// Python rounds floor division towards negative infinity and gives the remainder the divisor's sign.
fn floor_and_modulo(op: InplaceOp, first: i64, second: i64) -> Result<(i64, i64), InplaceError> {
    if second == 0 {
        return Err(InplaceError::DivideByZero);
    }
    let symbol = op.symbol();
    let quotient = first
        .checked_div(second)
        .ok_or(InplaceError::Overflow { symbol })?;
    let remainder = first
        .checked_rem(second)
        .ok_or(InplaceError::Overflow { symbol })?;
    if remainder != 0 && ((remainder < 0) != (second < 0)) {
        let rounded = quotient
            .checked_sub(1)
            .ok_or(InplaceError::Overflow { symbol })?;
        let normalized = remainder
            .checked_add(second)
            .ok_or(InplaceError::Overflow { symbol })?;
        return Ok((rounded, normalized));
    }
    Ok((quotient, remainder))
}

fn float_quotient(first: i64, second: i64) -> Result<Value, InplaceError> {
    if second == 0 {
        return Err(InplaceError::DivideByZero);
    }
    let symbol = InplaceOp::TrueDivide.symbol();
    Decimal::from(first)
        .checked_div(Decimal::from(second))
        .and_then(|value| value.to_f64())
        .map(Value::Float)
        .ok_or(InplaceError::Overflow { symbol })
}

type IntOperation = fn(i64, i64) -> Option<i64>;

const ARITHMETIC: [(InplaceOp, IntOperation); 3] = [
    (InplaceOp::Add, i64::checked_add),
    (InplaceOp::Subtract, i64::checked_sub),
    (InplaceOp::Multiply, i64::checked_mul),
];

pub(super) fn numeric(op: InplaceOp, left: &Value, right: &Value) -> Result<Value, InplaceError> {
    let (first, second) = integer_pair(op, left, right)?;
    if let Some((_, operation)) = ARITHMETIC.iter().find(|(candidate, _)| *candidate == op) {
        return int_result(op, operation(first, second));
    }
    if op == InplaceOp::FloorDivide {
        let (quotient, _remainder) = floor_and_modulo(op, first, second)?;
        return Ok(Value::Int(quotient));
    }
    if op == InplaceOp::Modulo {
        let (_quotient, remainder) = floor_and_modulo(op, first, second)?;
        return Ok(Value::Int(remainder));
    }
    if op == InplaceOp::Power {
        let exponent = u32::try_from(second).map_err(|_error| unsupported(op, left, right))?;
        return int_result(op, first.checked_pow(exponent));
    }
    if op == InplaceOp::TrueDivide {
        return float_quotient(first, second);
    }
    bitwise(op, left, right)
}

fn bitwise(op: InplaceOp, left: &Value, right: &Value) -> Result<Value, InplaceError> {
    let (first, second) = integer_pair(op, left, right)?;
    let bits = match op {
        InplaceOp::BitAnd => first & second,
        InplaceOp::BitOr => first | second,
        InplaceOp::BitXor => first ^ second,
        InplaceOp::ShiftLeft => return shifted_value(op, first, second, i64::checked_shl),
        InplaceOp::ShiftRight => return shifted_value(op, first, second, i64::checked_shr),
        other @ (InplaceOp::Add
        | InplaceOp::Subtract
        | InplaceOp::Multiply
        | InplaceOp::TrueDivide
        | InplaceOp::FloorDivide
        | InplaceOp::Modulo
        | InplaceOp::Power
        | InplaceOp::MatrixMultiply) => return Err(unsupported(other, left, right)),
    };
    Ok(Value::Int(bits))
}

fn shifted_value(
    op: InplaceOp,
    first: i64,
    second: i64,
    shift: fn(i64, u32) -> Option<i64>,
) -> Result<Value, InplaceError> {
    let symbol = op.symbol();
    let amount = u32::try_from(second).map_err(|_error| InplaceError::Overflow { symbol })?;
    let value = shift(first, amount).ok_or(InplaceError::Overflow { symbol })?;
    Ok(Value::Int(value))
}

fn repeated_items(items: &[Value], count: i64) -> Result<Vec<Value>, InplaceError> {
    let times = usize::try_from(count).map_err(|_error| InplaceError::RepetitionTooLong)?;
    let total = items
        .len()
        .checked_mul(times)
        .ok_or(InplaceError::RepetitionTooLong)?;
    if total > MAX_REPEATED_ITEMS {
        return Err(InplaceError::RepetitionTooLong);
    }
    let mut repeated = Vec::with_capacity(total);
    for _ in 0..times {
        repeated.extend(items.iter().cloned());
    }
    Ok(repeated)
}

fn repeated_text(text: &str, count: i64) -> Result<Value, InplaceError> {
    let times = usize::try_from(count).map_err(|_error| InplaceError::RepetitionTooLong)?;
    let total = text
        .len()
        .checked_mul(times)
        .ok_or(InplaceError::RepetitionTooLong)?;
    if total > MAX_REPEATED_ITEMS {
        return Err(InplaceError::RepetitionTooLong);
    }
    Ok(Value::Str(text.repeat(times)))
}

fn sequence(op: InplaceOp, left: &Value, right: &Value) -> Option<Result<Value, InplaceError>> {
    if op == InplaceOp::Add {
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
    }
    if op == InplaceOp::Multiply {
        if let (Value::Str(text), Value::Int(count)) | (Value::Int(count), Value::Str(text)) =
            (left, right)
        {
            return Some(repeated_text(text, *count));
        }
        if let (Value::List(items), Value::Int(count)) | (Value::Int(count), Value::List(items)) =
            (left, right)
        {
            return Some(repeated_items(items, *count).map(Value::List));
        }
    }
    None
}

pub(super) fn sequence_or_numeric(
    op: InplaceOp,
    left: &Value,
    right: &Value,
) -> Result<Value, InplaceError> {
    if let Some(outcome) = sequence(op, left, right) {
        return outcome;
    }
    numeric(op, left, right)
}
