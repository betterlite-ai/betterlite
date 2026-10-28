use rust_decimal::Decimal;
use std::num::ParseIntError;

#[derive(Debug, thiserror::Error)]
pub enum DecimalLexemeError {
    #[error("the exponent is not a valid integer")]
    Exponent {
        #[source]
        source: ParseIntError,
    },
    #[error("the exponent moves the decimal point out of the supported range")]
    ScaleOverflow,
    #[error(transparent)]
    Decimal(#[from] rust_decimal::Error),
}

pub fn parse_decimal_lexeme(lexeme: &str) -> Result<Decimal, DecimalLexemeError> {
    let Some((coefficient, exponent_text)) = lexeme.split_once(['e', 'E']) else {
        return Ok(Decimal::from_str_exact(lexeme)?);
    };
    let decimal = Decimal::from_str_exact(coefficient)?;
    let exponent = exponent_text
        .parse::<i32>()
        .map_err(|source| DecimalLexemeError::Exponent { source })?;
    let scale = i32::try_from(decimal.scale())
        .map_err(|_source| DecimalLexemeError::ScaleOverflow)?
        .checked_sub(exponent)
        .ok_or(DecimalLexemeError::ScaleOverflow)?;
    if !scale.is_negative() {
        return Ok(Decimal::try_from_i128_with_scale(
            decimal.mantissa(),
            u32::try_from(scale).map_err(|_source| DecimalLexemeError::ScaleOverflow)?,
        )
        .map(|value| value.normalize())?);
    }
    let multiplier = 10_i128
        .checked_pow(scale.unsigned_abs())
        .ok_or(rust_decimal::Error::ExceedsMaximumPossibleValue)?;
    let mantissa = decimal
        .mantissa()
        .checked_mul(multiplier)
        .ok_or(rust_decimal::Error::ExceedsMaximumPossibleValue)?;
    Ok(Decimal::try_from_i128_with_scale(mantissa, 0).map(|value| value.normalize())?)
}
