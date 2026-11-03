use std::str;

use rust_decimal::Decimal;
use serde::Deserialize;

use super::metadata::Metadata;
use crate::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use crate::tiered_pricing::Scalar;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RoutingDecision {
    pub classifier_cost: Option<Scalar>,
}

// A bool is a JSON Bool, never a Number, so it is excluded by the match alone.
#[must_use]
pub fn numeric_savings(value: Option<&Scalar>) -> Option<Decimal> {
    match value {
        Some(Scalar::Number(number)) => round_decimal_lexeme(&number.to_string()).ok(),
        _ => None,
    }
}

// rust_decimal holds at most 28 decimal places; a lexeme with more rounds half away from zero instead of failing, since LiteLLM keeps these as floats and a residue past that resolution is normal there.
pub fn round_decimal_lexeme(lexeme: &str) -> Result<Decimal, DecimalLexemeError> {
    let (coefficient, exponent) = match lexeme.split_once(['e', 'E']) {
        Some((coefficient, exponent_text)) => (
            coefficient,
            exponent_text
                .parse::<i64>()
                .map_err(|source| DecimalLexemeError::Exponent { source })?,
        ),
        None => (lexeme, 0),
    };
    let (is_negative, integer_part, fraction_part) = split_coefficient(coefficient);
    let frac_len = i64::try_from(fraction_part.len()).unwrap_or(i64::MAX);
    let Some(final_scale) = frac_len.checked_sub(exponent) else {
        return Ok(Decimal::ZERO);
    };
    if final_scale <= i64::from(Decimal::MAX_SCALE) {
        return parse_decimal_lexeme(lexeme);
    }
    let digits = format!("{integer_part}{fraction_part}");
    let excess_scale = final_scale.saturating_sub(i64::from(Decimal::MAX_SCALE));
    let drop = usize::try_from(excess_scale).unwrap_or(usize::MAX);
    let mantissa = rounded_mantissa(digits.as_bytes(), drop);
    if mantissa == 0 {
        return Ok(signed_zero(is_negative));
    }
    let mut decimal = Decimal::try_from_i128_with_scale(mantissa, Decimal::MAX_SCALE)?;
    decimal.set_sign_negative(is_negative);
    Ok(decimal.normalize())
}

// The first dropped digit decides half-away-from-zero rounding; a drop that reaches past every digit leaves nothing to round, so the magnitude is zero.
fn rounded_mantissa(digit_bytes: &[u8], drop: usize) -> i128 {
    if drop > digit_bytes.len() {
        return 0;
    }
    let kept_len = digit_bytes.len().saturating_sub(drop);
    let kept_bytes = digit_bytes.get(..kept_len).unwrap_or_default();
    let mantissa: i128 = if kept_bytes.is_empty() {
        0
    } else {
        str::from_utf8(kept_bytes)
            .ok()
            .and_then(|text| text.parse().ok())
            .unwrap_or(i128::MAX)
    };
    let rounds_up = digit_bytes.get(kept_len).is_some_and(|&byte| byte >= b'5');
    if rounds_up {
        mantissa.saturating_add(1)
    } else {
        mantissa
    }
}

// A magnitude of zero loses its sign through try_from_i128_with_scale, since -0_i128 is 0_i128, so the sign is set explicitly here and every later print or compare normalizes the result, turning -0 back into 0.
fn signed_zero(is_negative: bool) -> Decimal {
    let mut zero = Decimal::ZERO;
    zero.set_sign_negative(is_negative);
    zero
}

fn split_coefficient(coefficient: &str) -> (bool, &str, &str) {
    let (is_negative, unsigned) = coefficient.strip_prefix('-').map_or_else(
        || (false, coefficient.strip_prefix('+').unwrap_or(coefficient)),
        |rest| (true, rest),
    );
    unsigned.split_once('.').map_or(
        (is_negative, unsigned, ""),
        |(integer_part, fraction_part)| (is_negative, integer_part, fraction_part),
    )
}

// The lexeme's raw text, not its mathematical value, decides "exact integer": "2" and "2.0" stay distinct.
#[must_use]
pub fn recorded_autorouter_savings(metadata: &Metadata) -> Option<Decimal> {
    let estimate = metadata
        .autorouter_savings_estimate
        .as_ref()?
        .as_mapping()?;
    let exact_version = matches!(
        &estimate.version,
        Some(Scalar::Number(number)) if matches!(number.as_i64(), Some(1..=3))
    );
    let is_estimated =
        matches!(&estimate.status, Some(Scalar::Text(status)) if status == "estimated");
    if !exact_version || !is_estimated {
        return None;
    }
    numeric_savings(metadata.autorouter_savings.as_ref())
}

mod tests;
