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
fn double_bits(numerator: u128, denominator: u128) -> Option<u64> {
    let scale = 56_i64
        .checked_add(i64::from(denominator.ilog2()))?
        .checked_sub(i64::from(numerator.ilog2()))?;
    let (scaled, divisor) = if scale >= 0 {
        (
            numerator.checked_shl(u32::try_from(scale).ok()?)?,
            denominator,
        )
    } else {
        let shift = u32::try_from(scale.unsigned_abs()).ok()?;
        (numerator, denominator.checked_shl(shift)?)
    };
    let quotient = scaled.checked_div(divisor)?;
    let sticky = scaled.checked_rem(divisor)? != 0;
    let drop = quotient.ilog2().checked_sub(52)?;
    let kept = quotient.checked_shr(drop)?;
    let tail = quotient.checked_sub(kept.checked_shl(drop)?)?;
    let half = 1_u128.checked_shl(drop.checked_sub(1)?)?;
    let round_up = tail > half || (tail == half && (sticky || kept & 1 == 1));
    let mut mantissa = kept.checked_add(u128::from(round_up))?;
    let mut exponent = i64::from(drop).checked_sub(scale)?;
    if mantissa == 1_u128 << 53_u32 {
        mantissa >>= 1_u32;
        exponent = exponent.checked_add(1)?;
    }
    let biased = u64::try_from(exponent.checked_add(1075)?).ok()?;
    let fraction = u64::try_from(mantissa.checked_sub(1_u128 << 52_u32)?).ok()?;
    Some(biased.checked_shl(52)? | fraction)
}
fn float_quotient(first: i64, second: i64) -> Result<Value, InplaceError> {
    let sign = u64::from((first < 0) != (second < 0)) << 63_u32;
    let magnitude = match (first.unsigned_abs(), second.unsigned_abs()) {
        (_, 0) => return Err(InplaceError::DivideByZero),
        (0, _) => Some(0),
        (numerator, divisor) => double_bits(u128::from(numerator), u128::from(divisor)),
    };
    magnitude
        .map(|bits| Value::Float(f64::from_bits(sign | bits)))
        .ok_or(InplaceError::Overflow {
            symbol: InplaceOp::TrueDivide.symbol(),
        })
}
fn nearest_double_integer(number: i64) -> Option<i64> {
    let magnitude = number.unsigned_abs();
    let Some(log) = magnitude.checked_ilog2().filter(|log| *log > 52) else {
        return Some(number);
    };
    let drop = log.checked_sub(52)?;
    let kept = magnitude.checked_shr(drop)?;
    let tail = magnitude.checked_sub(kept.checked_shl(drop)?)?;
    let half = 1_u64.checked_shl(drop.checked_sub(1)?)?;
    let round_up = tail > half || (tail == half && kept & 1 == 1);
    let rounded = kept
        .checked_add(u64::from(round_up))?
        .checked_mul(1_u64.checked_shl(drop)?)?;
    if number < 0 {
        return 0_i64.checked_sub_unsigned(rounded);
    }
    i64::try_from(rounded).ok()
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
            let base = nearest_double_integer(first).ok_or(InplaceError::Overflow { symbol })?;
            let denominator = base
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
