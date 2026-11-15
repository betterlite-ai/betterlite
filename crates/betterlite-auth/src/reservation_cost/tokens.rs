use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use super::ReservationTokens;
use super::cost_info::CostInfo;
use super::rates::int_of;

pub(crate) const DEFAULT_MAX_OUTPUT_TOKENS: i64 = 16384;
const REQUESTED_OUTPUT_FIELDS: [&str; 3] =
    ["max_completion_tokens", "max_tokens", "max_output_tokens"];
const INPUT_ONLY_ROUTE_PARTS: [&str; 3] = ["embeddings", "rerank", "moderations"];

pub(crate) async fn counted_input_tokens(
    body: &JsonObject,
    tokens: &dyn ReservationTokens,
    model: &str,
    provided: Option<&IndexMap<String, usize>>,
) -> Option<i64> {
    let count = match provided.and_then(|counts| counts.get(model).copied()) {
        Some(count) => Some(count),
        None => tokens.count_input_tokens_for_model(body, model).await,
    };
    count.and_then(|value| i64::try_from(value).ok())
}

pub(crate) fn estimated_output_tokens(body: &JsonObject, route: &str, info: &CostInfo) -> i64 {
    if INPUT_ONLY_ROUTE_PARTS
        .iter()
        .any(|route_part| route.contains(route_part))
    {
        return 0;
    }
    let ceiling = info
        .max_output_tokens
        .filter(|ceiling| *ceiling != 0)
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
    requested_output_tokens(body)
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
        .min(ceiling)
}

fn requested_output_tokens(body: &JsonObject) -> Option<i64> {
    for field in REQUESTED_OUTPUT_FIELDS {
        if let Some(tokens) = int_of(body.get(field)) {
            return Some(tokens);
        }
    }
    let Some(Json::Object(config)) = body.get("inferenceConfig") else {
        return None;
    };
    int_of(config.get("maxTokens"))
}

pub(crate) fn output_multiplier(body: &JsonObject) -> i64 {
    let mut multiplier = 1;
    for field in ["n", "best_of"] {
        if let Some(value) = int_of(body.get(field)) {
            multiplier = multiplier.max(value);
        }
    }
    multiplier
}
