mod tests;

use std::str::FromStr as _;

use rust_decimal::Decimal;

#[must_use]
pub fn amount_from_text(text: &str) -> Option<Decimal> {
    if let Ok(amount) = Decimal::from_str(text) {
        return Some(amount);
    }
    Decimal::from_scientific(text).ok()
}
