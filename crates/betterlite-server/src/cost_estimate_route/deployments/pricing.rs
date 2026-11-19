use betterlite_core::cost::call_price::CustomCostPerToken;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_fields::decimal_lexeme::parse_decimal_lexeme;
use betterlite_models::price_lookup::{PriceLookup, PriceRecord};
use betterlite_models::tiered_pricing::Scalar;
use betterlite_provider::pricing::grounding::ProviderFormulas;
use rust_decimal::Decimal;

use crate::cost_service::CostService;

// _extract_custom_pricing: deployment fields answer first, then the backend's cost-map cache rates, then the input rate.
pub(super) fn extract_custom_pricing(
    params: &JsonObject,
    info: &JsonObject,
    service: &CostService<ProviderFormulas>,
    model: &str,
    provider: Option<&str>,
) -> Option<CustomCostPerToken> {
    let input_price = configured_price(params, info, "input_cost_per_token");
    let output_price = configured_price(params, info, "output_cost_per_token");
    if input_price.is_none() && output_price.is_none() {
        return None;
    }
    let builtin = service.lookup.effective_model_info(model, provider);
    Some(CustomCostPerToken {
        input_cost_per_token: input_price.unwrap_or_default(),
        output_cost_per_token: output_price.unwrap_or_default(),
        cache_read_input_token_cost: cache_price(
            params,
            info,
            builtin.as_ref(),
            "cache_read_input_token_cost",
        ),
        cache_creation_cost: cache_price(
            params,
            info,
            builtin.as_ref(),
            "cache_creation_input_token_cost",
        ),
    })
}

fn json_price(value: &Json) -> Option<Decimal> {
    let Json::Number(number) = value else {
        return None;
    };
    parse_decimal_lexeme(&number.to_string()).ok()
}

fn configured_price(params: &JsonObject, info: &JsonObject, key: &str) -> Option<Decimal> {
    params
        .get(key)
        .and_then(json_price)
        .or_else(|| info.get(key).and_then(json_price))
}

fn cache_price(
    params: &JsonObject,
    info: &JsonObject,
    builtin: Option<&PriceRecord>,
    key: &str,
) -> Option<Decimal> {
    if let Some(price) = configured_price(params, info, key) {
        return Some(price);
    }
    match builtin?.get(key)? {
        Scalar::Number(number) => parse_decimal_lexeme(&number.to_string()).ok(),
        Scalar::Null | Scalar::Bool(_) | Scalar::Text(_) | Scalar::Array(_) | Scalar::Object(_) => {
            None
        }
    }
}
