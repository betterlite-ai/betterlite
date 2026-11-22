use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;

// a number past Decimal's range is still a rate Python reads as nonzero, so only a parsed zero is free
#[must_use]
pub fn is_nonzero_rate(value: &Json) -> bool {
    match value {
        Json::Number(number) => {
            !parse_decimal_lexeme(&number.to_string()).is_ok_and(|rate| rate.is_zero())
        }
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => false,
    }
}

// an all-zero table is how a rate is expressed as free
#[must_use]
pub fn is_nonzero_price(value: &Json) -> bool {
    match value {
        Json::Object(table) => table.values().any(is_nonzero_rate),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            is_nonzero_rate(value)
        }
    }
}

#[must_use]
pub fn is_zero_price(value: &Json) -> bool {
    match value {
        Json::Object(table) => !table.is_empty() && !table.values().any(is_nonzero_rate),
        Json::List(items) => items.is_empty(),
        Json::Number(number) => {
            parse_decimal_lexeme(&number.to_string()).is_ok_and(|rate| rate.is_zero())
        }
        Json::Null | Json::Bool(_) | Json::Text(_) => false,
    }
}
