use rust_decimal::Decimal;

use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

use super::GuardrailPricingError;

// NonFinite exists because Pydantic accepts inf/nan spellings; billing later treats any NonFinite as zero.
#[derive(Debug, Clone, Copy)]
pub(super) enum CostMagnitude {
    Finite(Decimal),
    NonFinite,
}

fn is_nonfinite_literal(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    body.eq_ignore_ascii_case("inf")
        || body.eq_ignore_ascii_case("infinity")
        || body.eq_ignore_ascii_case("nan")
}

fn coerce_finite_lexeme(lexeme: &str) -> Result<Option<CostMagnitude>, GuardrailPricingError> {
    match parse_decimal_lexeme(lexeme) {
        Ok(decimal) => Ok(Some(CostMagnitude::Finite(decimal))),
        Err(source) => Err(GuardrailPricingError::UnrepresentableCost { source }),
    }
}

pub(super) fn coerce_lax_magnitude(
    value: &Scalar,
) -> Result<Option<CostMagnitude>, GuardrailPricingError> {
    match value {
        Scalar::Bool(flag) => Ok(Some(CostMagnitude::Finite(Decimal::from(u8::from(*flag))))),
        Scalar::Number(number) => coerce_finite_lexeme(&number.to_string()),
        Scalar::Text(text) => {
            let trimmed = text.trim();
            if is_nonfinite_literal(trimmed) {
                Ok(Some(CostMagnitude::NonFinite))
            } else if trimmed.parse::<f64>().is_ok() {
                coerce_finite_lexeme(trimmed)
            } else {
                Ok(None)
            }
        }
        Scalar::Null | Scalar::Array(_) | Scalar::Object(_) => Ok(None),
    }
}

fn coerce_bool_spelling(spelling: &str) -> Option<bool> {
    if spelling == "0"
        || spelling == "f"
        || spelling == "n"
        || spelling == "no"
        || spelling == "off"
        || spelling == "false"
    {
        return Some(false);
    }
    if spelling == "1"
        || spelling == "t"
        || spelling == "y"
        || spelling == "on"
        || spelling == "yes"
        || spelling == "true"
    {
        return Some(true);
    }
    None
}

fn coerce_bool_number(number: &serde_json::Number) -> Option<bool> {
    match number.as_f64() {
        Some(0.0) => Some(false),
        Some(1.0) => Some(true),
        _ => None,
    }
}

// Mirrors Pydantic's lax bool validator: a real bool, 0/1, or one of these case-insensitive spellings.
pub(super) fn coerce_bool_like(value: &Scalar) -> Option<bool> {
    if let Scalar::Bool(flag) = value {
        return Some(*flag);
    }
    if let Scalar::Number(number) = value {
        return coerce_bool_number(number);
    }
    if let Scalar::Text(text) = value {
        return coerce_bool_spelling(&text.to_ascii_lowercase());
    }
    None
}

pub(super) enum CounterRate {
    Unpriced,
    Priced(Decimal),
    Invalid,
}

const fn counter_rate_from_magnitude(magnitude: Option<CostMagnitude>) -> CounterRate {
    match magnitude {
        Some(CostMagnitude::Finite(decimal)) if !decimal.is_sign_negative() => {
            CounterRate::Priced(decimal)
        }
        Some(CostMagnitude::Finite(_) | CostMagnitude::NonFinite) | None => CounterRate::Invalid,
    }
}

// Any single invalid counter invalidates the whole rollup entry rather than only that counter.
pub(super) fn coerce_nonnegative_counter(
    value: &Scalar,
) -> Result<CounterRate, GuardrailPricingError> {
    if matches!(value, Scalar::Null) {
        return Ok(CounterRate::Unpriced);
    }
    let magnitude = coerce_lax_magnitude(value)?;
    Ok(counter_rate_from_magnitude(magnitude))
}
