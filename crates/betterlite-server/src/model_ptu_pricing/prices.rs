use betterlite_models::decisions::Json;
use serde_json::Number;

// Python reads the lexeme as a float, so only the mantissa decides zero: 0e-30 is 0.0 and 1e400 is inf
fn is_zero(number: &Number) -> bool {
    let text = number.to_string();
    text.split(['e', 'E']).next().is_some_and(|mantissa| {
        mantissa
            .chars()
            .all(|digit| matches!(digit, '0' | '.' | '-'))
    })
}

#[must_use]
pub fn is_nonzero_rate(value: &Json) -> bool {
    let Json::Number(number) = value else {
        return false;
    };
    !is_zero(number)
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
        Json::Number(number) => is_zero(number),
        Json::List(items) => items.is_empty(),
        Json::Object(table) => !table.is_empty() && !is_nonzero_price(value),
        Json::Null | Json::Bool(_) | Json::Text(_) => false,
    }
}
