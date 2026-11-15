use betterlite_models::decisions::Json;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use rust_decimal::Decimal;

use super::chat_text;

mod tests;

// What the public price map answers for a response; the cost engine that implements it lives outside this module
pub trait PriceMap {
    fn completion_cost(&self, response: &Json) -> Option<Decimal>;
}

#[must_use]
pub fn call_cost<Map>(response: &Json, prices: &Map) -> Decimal
where
    Map: PriceMap + ?Sized,
{
    stamped_cost(response)
        .unwrap_or_else(|| prices.completion_cost(response).unwrap_or(Decimal::ZERO))
}

fn stamped_cost(response: &Json) -> Option<Decimal> {
    let hidden = chat_text::field(response, "_hidden_params")?;
    match chat_text::field(hidden, "response_cost") {
        Some(Json::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        Some(Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}
