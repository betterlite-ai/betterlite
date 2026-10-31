use rust_decimal::Decimal;

use super::TokenRateError;
use super::rates::{matches_float_literal_grammar, parse_rate};

// A text that is not itself a valid float literal is a typed error, matching the sort it feeds.
pub(super) fn parse_token_threshold(text: &str, key: &str) -> Result<Decimal, TokenRateError> {
    let has_k = text.contains('k');
    let stripped: String = text.chars().filter(|&character| character != 'k').collect();
    let trimmed = stripped.trim();
    if !matches_float_literal_grammar(trimmed) {
        return Err(TokenRateError::InvalidThreshold {
            key: key.to_owned(),
        });
    }
    let value = parse_rate(key, &trimmed.replace('_', ""))?;
    if has_k {
        value
            .checked_mul(Decimal::from(1000_u16))
            .ok_or(TokenRateError::Overflow)
    } else {
        Ok(value)
    }
}

pub(super) fn prompt_exceeds(prompt_tokens: i64, limit: Decimal, inclusive: bool) -> bool {
    let tokens = Decimal::from(prompt_tokens);
    tokens > limit || (inclusive && tokens == limit)
}
