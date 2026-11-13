use std::str::FromStr as _;

use rust_decimal::Decimal;
use serde::Deserialize;

use super::metadata::Metadata;
use crate::price_fields::decimal_lexeme::DecimalLexemeError;
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
    let has_nonzero_digit = integer_part
        .bytes()
        .chain(fraction_part.bytes())
        .any(|byte| byte != b'0');
    let expanded = expanded_decimal(coefficient, exponent)?;
    let mut decimal = Decimal::from_str(&expanded)?.normalize();
    if decimal.is_zero() && is_negative && has_nonzero_digit {
        decimal.set_sign_negative(true);
    }
    Ok(decimal)
}

fn expanded_decimal(coefficient: &str, exponent: i64) -> Result<String, DecimalLexemeError> {
    let (is_negative, integer_part, fraction_part) = split_coefficient(coefficient);
    let digits = format!("{integer_part}{fraction_part}");
    if digits.bytes().all(|byte| byte == b'0') {
        return Ok("0".to_owned());
    }
    let integer_len = i64::try_from(integer_part.len())
        .map_err(|source| DecimalLexemeError::ScaleOverflow { source })?;
    let decimal_position = integer_len
        .checked_add(exponent)
        .ok_or(DecimalLexemeError::ScaleShiftOverflow)?;
    shifted_decimal(&digits, decimal_position, is_negative)
}

fn shifted_decimal(
    digits: &str,
    decimal_position: i64,
    is_negative: bool,
) -> Result<String, DecimalLexemeError> {
    let leading_zeroes = i64::try_from(digits.bytes().take_while(|byte| *byte == b'0').count())
        .map_err(|source| DecimalLexemeError::ScaleOverflow { source })?;
    let first_significant_place = decimal_position
        .checked_sub(leading_zeroes)
        .and_then(|position| position.checked_sub(1))
        .ok_or(DecimalLexemeError::ScaleShiftOverflow)?;
    if first_significant_place < -29 {
        return Ok("0".to_owned());
    }
    let integer_digits = decimal_position
        .checked_sub(leading_zeroes)
        .ok_or(DecimalLexemeError::ScaleShiftOverflow)?;
    if integer_digits > 29 {
        return Err(DecimalLexemeError::Decimal(
            rust_decimal::Error::ExceedsMaximumPossibleValue,
        ));
    }
    let unsigned = place_decimal(digits, decimal_position)?;
    let expanded = if is_negative {
        format!("-{unsigned}")
    } else {
        unsigned
    };
    Ok(expanded)
}

fn place_decimal(digits: &str, decimal_position: i64) -> Result<String, DecimalLexemeError> {
    let digit_len = i64::try_from(digits.len())
        .map_err(|source| DecimalLexemeError::ScaleOverflow { source })?;
    if decimal_position <= 0 {
        let zeros = usize::try_from(decimal_position.unsigned_abs())
            .map_err(|source| DecimalLexemeError::ScaleOverflow { source })?;
        Ok(format!("0.{}{digits}", "0".repeat(zeros)))
    } else if decimal_position >= digit_len {
        let zero_count = decimal_position
            .checked_sub(digit_len)
            .ok_or(DecimalLexemeError::ScaleShiftOverflow)?;
        let zeros = usize::try_from(zero_count)
            .map_err(|source| DecimalLexemeError::ScaleOverflow { source })?;
        Ok(format!("{digits}{}", "0".repeat(zeros)))
    } else {
        let split = usize::try_from(decimal_position)
            .map_err(|source| DecimalLexemeError::ScaleOverflow { source })?;
        let (whole, fraction) = digits.split_at(split);
        Ok(format!("{whole}.{fraction}"))
    }
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
