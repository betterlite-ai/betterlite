use rust_decimal::Decimal;

use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::price_lookup::{PriceLookup, PriceRecord};
use crate::tiered_pricing::Scalar;

pub const DEFAULT_CHEAPEST_COUNT: usize = 1;

// LiteLLM ranks a provider's chat models by input plus output cost per token and keeps the provider's own order on a tie, because its sort is stable.
#[must_use]
pub fn pick_cheapest_chat_models(
    custom_llm_provider: &str,
    count: usize,
    known_models: Option<&[String]>,
    price_lookup: &dyn PriceLookup,
) -> Vec<String> {
    let Some(models) = known_models else {
        return Vec::new();
    };
    let mut ranked: Vec<(&str, Decimal)> = Vec::new();
    for model in models {
        let Some(info) = price_lookup.effective_model_info(model, Some(custom_llm_provider)) else {
            continue;
        };
        let Some(Scalar::Text(mode)) = info.get("mode") else {
            continue;
        };
        if mode.as_str() != "chat" {
            continue;
        }
        let (Some(input_cost), Some(output_cost)) = (
            token_cost(&info, "input_cost_per_token"),
            token_cost(&info, "output_cost_per_token"),
        ) else {
            continue;
        };
        let Some(total) = input_cost.checked_add(output_cost) else {
            continue;
        };
        ranked.push((model.as_str(), total));
    }
    ranked.sort_by_key(|entry| entry.1);
    ranked
        .into_iter()
        .take(count)
        .map(|(model, _cost)| model.to_owned())
        .collect()
}

// A cost map entry with a price of a type the price fields never carry is unusable, so the model is left out rather than ranked as free.
fn token_cost(info: &PriceRecord, field: &str) -> Option<Decimal> {
    match info.get(field) {
        None | Some(Scalar::Null | Scalar::Bool(false)) => Some(Decimal::ZERO),
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        Some(Scalar::Bool(true) | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_)) => None,
    }
}

mod tests;
