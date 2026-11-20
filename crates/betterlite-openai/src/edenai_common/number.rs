use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;

// Python's float grammar minus the spellings Decimal cannot hold; anything else is no cost
pub(super) fn cost_decimal(text: &str) -> Option<Decimal> {
    let trimmed = text.trim();
    let (sign, rest) = trimmed.strip_prefix('-').map_or_else(
        || ("", trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| ("-", rest),
    );
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
    let mut lexeme = String::with_capacity(trimmed.len().saturating_add(2));
    lexeme.push_str(sign);
    lexeme.push_str(if whole.is_empty() { "0" } else { whole });
    if !fraction.is_empty() {
        lexeme.push('.');
        lexeme.push_str(fraction);
    }
    if let Some(found) = exponent {
        let normalized = normalized_exponent(found)?;
        lexeme.push('e');
        lexeme.push_str(&normalized);
    }
    parse_decimal_lexeme(&lexeme).ok()
}

fn normalized_exponent(exponent: &str) -> Option<String> {
    let unsigned = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
    if unsigned.is_empty() {
        return None;
    }
    if !unsigned.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    Some(exponent.to_owned())
}
