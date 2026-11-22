use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use serde_json::Number;

// a number past Decimal's range is still a rate Python reads as nonzero, so only a parsed zero is free
#[must_use]
pub fn is_nonzero_rate(value: &Json) -> bool {
    let Json::Number(number) = value else {
        return false;
    };
    !is_zero_rate(number)
}

// an all-zero table is how a rate is expressed as free
#[must_use]
pub fn is_nonzero_price(value: &Json) -> bool {
    if let Json::Object(table) = value {
        return table.values().any(is_nonzero_rate);
    }
    is_nonzero_rate(value)
}

#[must_use]
pub fn is_zero_price(value: &Json) -> bool {
    match value {
        Json::Number(number) => is_zero_rate(number),
        Json::List(items) => items.is_empty(),
        Json::Object(table) => !table.is_empty() && !is_nonzero_price(value),
        Json::Null | Json::Bool(_) | Json::Text(_) => false,
    }
}

fn is_zero_rate(number: &Number) -> bool {
    match parse_decimal_lexeme(&number.to_string()) {
        Ok(rate) => rate.is_zero(),
        Err(_) => false,
    }
}
