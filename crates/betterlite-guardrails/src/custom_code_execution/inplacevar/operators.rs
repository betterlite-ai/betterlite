use rust_decimal::Decimal;

use super::super::value::Value;
use super::{InplaceError, InplaceOp};

#[must_use]
pub(super) const fn unsupported(op: InplaceOp, left: &Value, right: &Value) -> InplaceError {
    InplaceError::UnsupportedOperand {
        symbol: op.symbol(),
        left: left.type_name(),
        right: right.type_name(),
    }
}

fn integer_pair(op: InplaceOp, left: &Value, right: &Value) -> Result<(i64, i64), InplaceError> {
    match (integer(left), integer(right)) {
        (Some(first), Some(second)) => Ok((first, second)),
        _ => Err(unsupported(op, left, right)),
    }
}

// Python treats bool as an integer in arithmetic; the bitwise operators keep bool results separately.
pub(super) fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::Int(number) => Some(*number),
        Value::Bool(flag) => Some(i64::from(*flag)),
        Value::None
        | Value::Float(_)
        | Value::Str(_)
        | Value::Bytes(_)
        | Value::List(_)
        | Value::Tuple(_)
        | Value::Dict(_) => None,
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
    let quotient = Decimal::from(first)
        .checked_div(Decimal::from(second))
        .ok_or(InplaceError::Overflow { symbol })?;
    // f64 parsing is correctly rounded, which Decimal::to_f64 is not.
    format!("{quotient}")
        .parse::<f64>()
        .map(Value::Float)
        .map_err(|_error| InplaceError::Overflow { symbol })
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
        return power(op, first, second, left, right);
    }
    if op == InplaceOp::TrueDivide {
        return float_quotient(first, second);
    }
    bitwise(op, left, right)
}

fn power(
    op: InplaceOp,
    first: i64,
    second: i64,
    left: &Value,
    right: &Value,
) -> Result<Value, InplaceError> {
    if second < 0 {
        let magnitude =
            u32::try_from(second.unsigned_abs()).map_err(|_error| unsupported(op, left, right))?;
        let symbol = op.symbol();
        let denominator = first
            .checked_pow(magnitude)
            .ok_or(InplaceError::Overflow { symbol })?;
        return float_quotient(1, denominator);
    }
    let exponent = u32::try_from(second).map_err(|_error| unsupported(op, left, right))?;
    int_result(op, first.checked_pow(exponent))
}

fn bitwise(op: InplaceOp, left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Bool(first), Value::Bool(second)) = (left, right)
        && matches!(op, InplaceOp::BitAnd | InplaceOp::BitOr | InplaceOp::BitXor)
    {
        let flag = match op {
            InplaceOp::BitAnd => *first && *second,
            InplaceOp::BitOr => *first || *second,
            InplaceOp::BitXor => *first ^ *second,
            InplaceOp::Add
            | InplaceOp::Subtract
            | InplaceOp::Multiply
            | InplaceOp::TrueDivide
            | InplaceOp::FloorDivide
            | InplaceOp::Modulo
            | InplaceOp::Power
            | InplaceOp::MatrixMultiply
            | InplaceOp::ShiftLeft
            | InplaceOp::ShiftRight => return Err(unsupported(op, left, right)),
        };
        return Ok(Value::Bool(flag));
    }
    let (first, second) = integer_pair(op, left, right)?;
    let bits = match op {
        InplaceOp::BitAnd => first & second,
        InplaceOp::BitOr => first | second,
        InplaceOp::BitXor => first ^ second,
        InplaceOp::ShiftLeft => return shift_left(op, first, second),
        InplaceOp::ShiftRight => return shift_right(op, first, second),
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

fn shift_left(op: InplaceOp, first: i64, second: i64) -> Result<Value, InplaceError> {
    let symbol = op.symbol();
    let amount = u32::try_from(second).map_err(|_error| InplaceError::Overflow { symbol })?;
    if amount >= 64 {
        return if first == 0 {
            Ok(Value::Int(0))
        } else {
            Err(InplaceError::Overflow { symbol })
        };
    }
    let shifted = first
        .checked_shl(amount)
        .ok_or(InplaceError::Overflow { symbol })?;
    if shifted >> amount == first {
        return Ok(Value::Int(shifted));
    }
    Err(InplaceError::Overflow { symbol })
}

fn shift_right(op: InplaceOp, first: i64, second: i64) -> Result<Value, InplaceError> {
    let symbol = op.symbol();
    let amount = u32::try_from(second).map_err(|_error| InplaceError::Overflow { symbol })?;
    if amount >= 64 {
        return Ok(Value::Int(if first < 0 { -1 } else { 0 }));
    }
    first
        .checked_shr(amount)
        .map(Value::Int)
        .ok_or(InplaceError::Overflow { symbol })
}

pub(crate) mod sequence;
