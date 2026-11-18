use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serializer;

// JSON writes an integral money value with one fractional zero, the way Python's float text does.
pub fn serialize_money<S>(amount: &Decimal, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let mut value = *amount;
    if value.scale() == 0 {
        value.rescale(1);
    }
    rust_decimal::serde::arbitrary_precision::serialize(&value, serializer)
}

pub fn serialize_optional_money<S>(
    amount: &Option<Decimal>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match amount {
        Some(value) => serialize_money(value, serializer),
        None => serializer.serialize_none(),
    }
}

// Python's {value:.1%} and {value:.0%}: percent with an exact half-even rounding of the decimal.
pub fn format_percent(value: Decimal, decimals: u32) -> String {
    let scaled = value.saturating_mul(Decimal::ONE_HUNDRED);
    let rounded = scaled.round_dp_with_strategy(decimals, RoundingStrategy::MidpointNearestEven);
    format!("{rounded:.decimals$}%")
}

// Python's {value:,.2f}: two fractional digits, half-even, with thousands separators.
pub fn format_usd(amount: Decimal) -> String {
    let rounded = amount.round_dp_with_strategy(2, RoundingStrategy::MidpointNearestEven);
    let text = format!("{rounded:.2}");
    let whole_len = text
        .chars()
        .take_while(|character| *character != '.')
        .filter(char::is_ascii_digit)
        .count();
    let mut grouped = String::new();
    let mut seen = 0usize;
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
    grouped
}
