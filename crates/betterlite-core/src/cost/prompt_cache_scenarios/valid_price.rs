use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::tiered_pricing::Scalar;

// Ports _valid_price: only a non-negative finite number counts, and a bool is not one even where Python says so.
#[must_use]
pub fn valid_price(value: Option<&Scalar>) -> bool {
    match value {
        Some(Scalar::Number(number)) => {
            parse_decimal_lexeme(&number.to_string()).is_ok_and(|amount| !amount.is_sign_negative())
        }
        Some(
            Scalar::Bool(_) | Scalar::Null | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_),
        )
        | None => false,
    }
}

mod tests;
