use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;

// Python's float grammar minus the spellings Decimal cannot hold; anything else is no cost
pub(super) fn cost_decimal(text: &str) -> Option<Decimal> {
    let trimmed = text.trim();
    let (sign, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (rest, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (mantissa, ""),
    };
    let digits_only = whole
        .chars()
        .chain(fraction.chars())
        .all(|character| character.is_ascii_digit());
    if !digits_only || (whole.is_empty() && fraction.is_empty()) {
        return None;
    }
    let mut lexeme = String::with_capacity(trimmed.len() + 2);
    lexeme.push_str(sign);
    lexeme.push_str(if whole.is_empty() { "0" } else { whole });
    if !fraction.is_empty() {
        lexeme.push('.');
        lexeme.push_str(fraction);
    }
    if let Some(exponent) = exponent {
        let normalized = normalized_exponent(exponent)?;
        lexeme.push('e');
        lexeme.push_str(&normalized);
    }
    parse_decimal_lexeme(&lexeme).ok()
}

fn normalized_exponent(exponent: &str) -> Option<String> {
    let unsigned = match exponent.strip_prefix(['+', '-']) {
        Some(unsigned) => unsigned,
        None => exponent,
    };
    if unsigned.is_empty() {
        return None;
    }
    if !unsigned.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    Some(exponent.to_owned())
}
