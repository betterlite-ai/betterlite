use rust_decimal::Decimal;
use thiserror::Error;

use super::schema::RateField;
use crate::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};

#[derive(Debug, Error)]
pub enum OffPeakRateError {
    #[error("off-peak {field} is not a finite price")]
    NonFinite { field: &'static str },
    #[error("off-peak {field} cannot be represented as an exact decimal price")]
    Unrepresentable {
        field: &'static str,
        #[source]
        source: DecimalLexemeError,
    },
}

pub(super) fn off_peak_rate(
    value: Option<&RateField>,
    field: &'static str,
    standard_rate: Decimal,
) -> Result<Decimal, OffPeakRateError> {
    if let Some(rate) = parse_rate(value, field)? {
        return Ok(rate);
    }
    Ok(standard_rate)
}

pub(super) fn parse_rate(
    value: Option<&RateField>,
    field: &'static str,
) -> Result<Option<Decimal>, OffPeakRateError> {
    match value {
        Some(RateField::Number(number)) => parse_numeric_text(&number.to_string(), field),
        Some(RateField::Text(text)) => parse_numeric_text(text.trim(), field),
        Some(RateField::Bool(_) | RateField::Other(_)) | None => Ok(None),
    }
}

fn parse_numeric_text(
    lexeme: &str,
    field: &'static str,
) -> Result<Option<Decimal>, OffPeakRateError> {
    // f64 only checks that the text is numeric; the stored rate always comes from parse_decimal_lexeme.
    let Ok(approximate) = lexeme.parse::<f64>() else {
        return Ok(None);
    };
    if !approximate.is_finite() {
        return Err(OffPeakRateError::NonFinite { field });
    }
    parse_decimal_lexeme(lexeme)
        .map(Some)
        .map_err(|source| OffPeakRateError::Unrepresentable { field, source })
}
