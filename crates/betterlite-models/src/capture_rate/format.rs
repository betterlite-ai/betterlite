use rust_decimal::serde::arbitrary_precision::serialize;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serializer;

// JSON writes money the way Python's float text does: trailing zeros trimmed, an integral value with one fractional zero.
pub fn serialize_money<S>(amount: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut value = amount.normalize();
    if value.scale() == 0 {
        value.rescale(1);
    }
    serialize(&value, serializer)
}

pub fn serialize_optional_money<S>(rate: &Option<Decimal>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(value) = rate {
        return serialize_money(value, serializer);
    }
    serializer.serialize_none()
}

// Python's {value:.1%} and {value:.0%}: percent with an exact half-even rounding of the decimal.
#[must_use]
pub fn format_percent(value: Decimal, decimals: u32) -> String {
    let scaled = value.saturating_mul(Decimal::ONE_HUNDRED);
    let mut rounded =
        scaled.round_dp_with_strategy(decimals, RoundingStrategy::MidpointNearestEven);
    rounded.rescale(decimals);
    if value.is_sign_negative() {
        format!("-{}%", rounded.abs())
    } else {
        format!("{rounded}%")
    }
}

// Python's {value:,.2f}: two fractional digits, half-even, with thousands separators.
#[must_use]
pub fn format_usd(amount: Decimal) -> String {
    let rounded = amount.round_dp_with_strategy(2, RoundingStrategy::MidpointNearestEven);
    let text = format!("{:.2}", rounded.abs());
    let whole_len = text
        .chars()
        .take_while(|character| *character != '.')
        .filter(char::is_ascii_digit)
        .count();
    let mut grouped = String::new();
    let mut seen = 0_usize;
    for character in text.chars() {
        if character.is_ascii_digit() && seen < whole_len {
            let remaining = whole_len.saturating_sub(seen);
            if seen > 0 && remaining.is_multiple_of(3) {
                grouped.push(',');
            }
            seen = seen.saturating_add(1);
        }
        grouped.push(character);
    }
    if amount.is_sign_negative() {
        format!("-{grouped}")
    } else {
        grouped
    }
}
