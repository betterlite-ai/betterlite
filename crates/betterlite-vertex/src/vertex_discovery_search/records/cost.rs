use betterlite_models::price_fields::PriceFields;
use rust_decimal::Decimal;

// LiteLLM's calculate_vector_store_cost: the catalog's per-query price, with no output or per-result charge
#[must_use]
pub fn calculate_vector_store_cost(prices: Option<&PriceFields>) -> (Decimal, Decimal) {
    let input = prices
        .and_then(|prices| prices.values.input.input_query)
        .unwrap_or(Decimal::ZERO);
    (input, Decimal::ZERO)
}
