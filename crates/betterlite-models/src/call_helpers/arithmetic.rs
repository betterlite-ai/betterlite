use rust_decimal::Decimal;

#[must_use]
pub fn safe_divide(numerator: Decimal, denominator: Decimal, default: Decimal) -> Decimal {
    if denominator.is_zero() {
        return default;
    }
    numerator.checked_div(denominator).unwrap_or(default)
}

#[must_use]
pub fn safe_divide_seconds(
    seconds: Decimal,
    denominator: Decimal,
    default: Option<Decimal>,
) -> Option<Decimal> {
    if denominator <= Decimal::ZERO {
        return default;
    }
    seconds.checked_div(denominator)
}
