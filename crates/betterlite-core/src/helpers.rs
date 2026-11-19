use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::api::media_relay_logging::spend::decimal_json;

// The header the cost calculator reads before the price map, as core_helpers names it.
pub const RESPONSE_COST_HEADER: &str = "llm_provider-x-litellm-response-cost";

// Ports core_helpers.set_response_cost_in_hidden_params: no cost means no header, and no response_cost field.
pub fn set_response_cost_in_hidden_params(
    hidden_params: &mut IndexMap<String, Json>,
    cost: Option<Decimal>,
) {
    let Some(cost) = cost else {
        return;
    };
    let mut merged = match hidden_params.get("additional_headers") {
        Some(Json::Object(headers)) => headers.clone(),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => IndexMap::new(),
    };
    merged.insert(RESPONSE_COST_HEADER.to_owned(), decimal_json(cost));
    hidden_params.insert("additional_headers".to_owned(), Json::Object(merged));
}
