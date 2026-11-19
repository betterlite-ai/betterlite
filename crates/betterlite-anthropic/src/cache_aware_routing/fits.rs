use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;

pub(super) const MAX_INPUT_TOKENS: &str = "max_input_tokens";
pub(super) const MAX_OUTPUT_TOKENS: &str = "max_output_tokens";

pub(super) fn declared_limit(model_info: &JsonObject, field: &str) -> Option<i64> {
    match model_info.get(field) {
        Some(Json::Number(number)) => number.as_i64(),
        _ => None,
    }
}

pub(super) fn resolved_limit(resolved: Option<&PriceRecord>, field: &str) -> Option<i64> {
    let record = resolved?;
    match record.get(field) {
        Some(Scalar::Number(number)) => number.as_i64(),
        _ => None,
    }
}

mod tests;
