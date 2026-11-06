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
fn floor_and_modulo(op: InplaceOp, first: i64, second: i64) -> Result<(i64, i64), InplaceError> {
    if second == 0 {
        return Err(InplaceError::DivideByZero);
    }
    let symbol = op.symbol();
    let overflow = || InplaceError::Overflow { symbol };
    let quotient = first.checked_div(second).ok_or_else(overflow)?;
    let remainder = first.checked_rem(second).ok_or_else(overflow)?;
    if remainder != 0 && ((remainder < 0) != (second < 0)) {
        let rounded = quotient.checked_sub(1).ok_or_else(overflow)?;
        let normalized = remainder.checked_add(second).ok_or_else(overflow)?;
        return Ok((rounded, normalized));
    }
    Ok((quotient, remainder))
}
fn float_quotient(first: i64, second: i64) -> Result<Value, InplaceError> {
    if second == 0 {
        return Err(InplaceError::DivideByZero);
    }
    let symbol = InplaceOp::TrueDivide.symbol();
    let negative = (first < 0) != (second < 0);
    let divisor = u128::from(second.unsigned_abs());
    let mut remainder = u128::from(first.unsigned_abs());
    let integer = remainder
        .checked_div(divisor)
        .ok_or(InplaceError::DivideByZero)?;
    remainder = remainder
        .checked_rem(divisor)
        .ok_or(InplaceError::DivideByZero)?;
    let mut digits = String::new();
    let exponent = if integer == 0 {
        first_fraction_digit(&mut digits, &mut remainder, divisor)?
    } else {
        digits.push_str(&integer.to_string());
        i64::try_from(digits.len())
            .unwrap_or(i64::MAX)
            .saturating_sub(1)
    };
    while remainder != 0 && digits.len() < 25 {
        let digit = next_digit(&mut remainder, divisor)?;
        digits.push(char::from(b'0'.saturating_add(digit)));
    }
    if digits.is_empty() {
        return Ok(Value::Float(0.0));
    }
    let sign = if negative { "-" } else { "" };
    let (head, tail) = digits.split_at(1);
    let text = if tail.is_empty() {
        format!("{sign}{head}e{exponent}")
    } else {
        format!("{sign}{head}.{tail}e{exponent}")
    };
    text.parse::<f64>()
        .map(Value::Float)
        .map_err(|_error| InplaceError::Overflow { symbol })
}
fn first_fraction_digit(
    digits: &mut String,
    remainder: &mut u128,
    divisor: u128,
) -> Result<i64, InplaceError> {
    let mut zeros = 0_i64;
    while *remainder != 0 && digits.is_empty() {
        let digit = next_digit(remainder, divisor)?;
        if digit == 0 {
            zeros = zeros.saturating_add(1);
            continue;
        }
        digits.push(char::from(b'0'.saturating_add(digit)));
        return Ok(0_i64.saturating_sub(zeros.saturating_add(1)));
    }
    Ok(0)
}
fn next_digit(remainder: &mut u128, divisor: u128) -> Result<u8, InplaceError> {
    let symbol = InplaceOp::TrueDivide.symbol();
    *remainder = remainder
        .checked_mul(10)
        .ok_or(InplaceError::Overflow { symbol })?;
    let digit = remainder
        .checked_div(divisor)
        .ok_or(InplaceError::DivideByZero)?;
    *remainder = remainder
        .checked_rem(divisor)
        .ok_or(InplaceError::DivideByZero)?;
    u8::try_from(digit).map_err(|_error| InplaceError::Overflow { symbol })
}
fn power(op: InplaceOp, first: i64, second: i64) -> Result<Value, InplaceError> {
    if second >= 0 {
        return u32::try_from(second).map_or_else(
            |_error| huge_power(op, first, second.unsigned_abs().is_multiple_of(2)),
            |exponent| int_result(op, first.checked_pow(exponent)),
        );
    }
    let magnitude = second.unsigned_abs();
    match u32::try_from(magnitude) {
        Ok(exponent) => {
            let symbol = op.symbol();
            let denominator = first
                .checked_pow(exponent)
                .ok_or(InplaceError::Overflow { symbol })?;
            float_quotient(1, denominator)
        }
        Err(_error) => match first {
            0 => Err(InplaceError::DivideByZero),
            1 => Ok(Value::Float(1.0)),
            -1 => Ok(Value::Float(if magnitude.is_multiple_of(2) {
                1.0
            } else {
                -1.0
            })),
            _ => Err(InplaceError::Overflow {
                symbol: op.symbol(),
            }),
        },
    }
}
const fn huge_power(op: InplaceOp, base: i64, even: bool) -> Result<Value, InplaceError> {
    match base {
        0 => Ok(Value::Int(0)),
        1 => Ok(Value::Int(1)),
        -1 => Ok(Value::Int(if even { 1 } else { -1 })),
        _ => Err(InplaceError::Overflow {
            symbol: op.symbol(),
        }),
    }
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
    match op {
        InplaceOp::FloorDivide => {
            floor_and_modulo(op, first, second).map(|(value, _)| Value::Int(value))
        }
        InplaceOp::Modulo => {
            floor_and_modulo(op, first, second).map(|(_, value)| Value::Int(value))
        }
        InplaceOp::Power => power(op, first, second),
        InplaceOp::TrueDivide => float_quotient(first, second),
        InplaceOp::Add
        | InplaceOp::Subtract
        | InplaceOp::Multiply
        | InplaceOp::BitAnd
        | InplaceOp::BitOr
        | InplaceOp::BitXor
        | InplaceOp::ShiftLeft
        | InplaceOp::ShiftRight
        | InplaceOp::MatrixMultiply => bitwise(op, left, right),
    }
}
fn bitwise(op: InplaceOp, left: &Value, right: &Value) -> Result<Value, InplaceError> {
    if let (Value::Bool(first), Value::Bool(second)) = (left, right) {
        if op == InplaceOp::BitAnd {
            return Ok(Value::Bool(*first && *second));
        }
        if op == InplaceOp::BitOr {
            return Ok(Value::Bool(*first || *second));
        }
        if op == InplaceOp::BitXor {
            return Ok(Value::Bool(*first ^ *second));
        }
    }
    let (first, second) = integer_pair(op, left, right)?;
    if op == InplaceOp::ShiftLeft || op == InplaceOp::ShiftRight {
        return shift(op, first, second);
    }
    if op == InplaceOp::BitAnd {
        return Ok(Value::Int(first & second));
    }
    if op == InplaceOp::BitOr {
        return Ok(Value::Int(first | second));
    }
    if op == InplaceOp::BitXor {
        return Ok(Value::Int(first ^ second));
    }
    Err(unsupported(op, left, right))
}
fn shift(op: InplaceOp, first: i64, second: i64) -> Result<Value, InplaceError> {
    let symbol = op.symbol();
    let amount = u32::try_from(second).map_err(|_error| InplaceError::Overflow { symbol })?;
    if op == InplaceOp::ShiftRight {
        if amount >= 64 {
            return Ok(Value::Int(if first < 0 { -1 } else { 0 }));
        }
        return first
            .checked_shr(amount)
            .map(Value::Int)
            .ok_or(InplaceError::Overflow { symbol });
    }
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
mod formatting;
pub(crate) mod sequence;
