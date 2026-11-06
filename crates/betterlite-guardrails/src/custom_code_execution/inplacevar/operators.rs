use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use super::super::value::Value;
use super::{InplaceError, InplaceOp};

const MAX_REPEATED_ITEMS: usize = 0x0010_0000;

pub(super) const fn unsupported(op: InplaceOp, left: &Value, right: &Value) -> InplaceError {
    InplaceError::UnsupportedOperand {
        symbol: op.symbol(),
        left: left.type_name(),
        right: right.type_name(),
    }
}

const fn integer(op: InplaceOp, value: &Value) -> Result<i64, InplaceError> {
    if let Value::Int(number) = value {
        return Ok(*number);
    }
    Err(InplaceError::UnsupportedOperand {
        symbol: op.symbol(),
        left: value.type_name(),
        right: "int",
    })
}

fn int_result(op: InplaceOp, value: Option<i64>) -> Result<Value, InplaceError> {
    let symbol = op.symbol();
    value
        .map(Value::Int)
        .ok_or(InplaceError::Overflow { symbol })
}

pub(super) fn add(left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Int(first), Value::Int(second)) = (left, right) {
        return int_result(InplaceOp::Add, first.checked_add(*second));
    }
    if let (Value::Str(first), Value::Str(second)) = (left, right) {
        return Ok(Value::Str(format!("{first}{second}")));
    }
    if let (Value::Bytes(first), Value::Bytes(second)) = (left, right) {
        let mut joined = first.clone();
        joined.extend_from_slice(second);
        return Ok(Value::Bytes(joined));
    }
    if let (Value::List(first), Value::List(second)) = (left, right) {
        let mut joined = first.clone();
        joined.extend(second.iter().cloned());
        return Ok(Value::List(joined));
    }
    if let (Value::Tuple(first), Value::Tuple(second)) = (left, right) {
        let mut joined = first.clone();
        joined.extend(second.iter().cloned());
        return Ok(Value::Tuple(joined));
    }
    Err(unsupported(InplaceOp::Add, left, right))
}

pub(super) fn subtract(left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Int(first), Value::Int(second)) = (left, right) {
        return int_result(InplaceOp::Subtract, first.checked_sub(*second));
    }
    Err(unsupported(InplaceOp::Subtract, left, right))
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

pub(super) fn multiply(left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Int(first), Value::Int(second)) = (left, right) {
        return int_result(InplaceOp::Multiply, first.checked_mul(*second));
    }
    if let (Value::Str(text), Value::Int(count)) | (Value::Int(count), Value::Str(text)) =
        (left, right)
    {
        return repeated_text(text, *count);
    }
    if let (Value::List(items), Value::Int(count)) | (Value::Int(count), Value::List(items)) =
        (left, right)
    {
        return Ok(Value::List(repeated_items(items, *count)?));
    }
    Err(unsupported(InplaceOp::Multiply, left, right))
}

pub(super) fn floor_divide(left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Int(first), Value::Int(second)) = (left, right) {
        if *second == 0 {
            return Err(InplaceError::DivideByZero);
        }
        let symbol = InplaceOp::FloorDivide.symbol();
        let quotient = first
            .checked_div(*second)
            .ok_or(InplaceError::Overflow { symbol })?;
        let remainder = first
            .checked_rem(*second)
            .ok_or(InplaceError::Overflow { symbol })?;
        if remainder != 0 && ((remainder < 0) != (*second < 0)) {
            let rounded = quotient
                .checked_sub(1)
                .ok_or(InplaceError::Overflow { symbol })?;
            return Ok(Value::Int(rounded));
        }
        return Ok(Value::Int(quotient));
    }
    Err(unsupported(InplaceOp::FloorDivide, left, right))
}

pub(super) fn modulo(left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Int(first), Value::Int(second)) = (left, right) {
        if *second == 0 {
            return Err(InplaceError::DivideByZero);
        }
        let symbol = InplaceOp::Modulo.symbol();
        let remainder = first
            .checked_rem(*second)
            .ok_or(InplaceError::Overflow { symbol })?;
        if remainder != 0 && ((remainder < 0) != (*second < 0)) {
            let normalized = remainder
                .checked_add(*second)
                .ok_or(InplaceError::Overflow { symbol })?;
            return Ok(Value::Int(normalized));
        }
        return Ok(Value::Int(remainder));
    }
    Err(unsupported(InplaceOp::Modulo, left, right))
}

pub(super) fn true_divide(left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Int(first), Value::Int(second)) = (left, right) {
        if *second == 0 {
            return Err(InplaceError::DivideByZero);
        }
        let quotient = Decimal::from(*first)
            .checked_div(Decimal::from(*second))
            .and_then(|value| value.to_f64())
            .map(Value::Float)
            .ok_or(InplaceError::Overflow {
                symbol: InplaceOp::TrueDivide.symbol(),
            })?;
        return Ok(quotient);
    }
    Err(unsupported(InplaceOp::TrueDivide, left, right))
}

pub(super) fn power(left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Int(first), Value::Int(second)) = (left, right) {
        let exponent =
            u32::try_from(*second).map_err(|_error| unsupported(InplaceOp::Power, left, right))?;
        return int_result(InplaceOp::Power, first.checked_pow(exponent));
    }
    Err(unsupported(InplaceOp::Power, left, right))
}

pub(super) fn bits(op: InplaceOp, left: &Value, right: &Value) -> Result<Value, InplaceError> {
    let first = integer(op, left)?;
    let second = integer(op, right)?;
    let result = match op {
        InplaceOp::BitAnd => Some(first & second),
        InplaceOp::BitOr => Some(first | second),
        InplaceOp::BitXor => Some(first ^ second),
        InplaceOp::ShiftLeft => u32::try_from(second)
            .ok()
            .and_then(|shift| first.checked_shl(shift)),
        InplaceOp::ShiftRight => u32::try_from(second)
            .ok()
            .and_then(|shift| first.checked_shr(shift)),
        InplaceOp::Add
        | InplaceOp::Subtract
        | InplaceOp::Multiply
        | InplaceOp::TrueDivide
        | InplaceOp::FloorDivide
        | InplaceOp::Modulo
        | InplaceOp::Power
        | InplaceOp::MatrixMultiply => None,
    };
    int_result(op, result)
}

// `x += y` compiles to `x = _inplacevar_("+=", x, y)`, so this is the function that call resolves to.
