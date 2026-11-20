use betterlite_models::decisions::Json;
use serde_json::Number;

use crate::complexity_scoring::tier::Tier;
use crate::router_predicates::JsonMap;
use crate::tier_definitions::model::TierSet;

use super::TierPlacement;

const CEILING: [&str; 3] = ["max_tokens", "max_completion_tokens", "max_output_tokens"];
const MAX_TOKENS: &str = "max_tokens";

// _litellm_params_for_model: the tier entry's own params, and the model's output ceiling when the entry names none.
#[must_use]
pub fn params_for_model(
    placement: &TierPlacement<'_>,
    tier: Option<&Tier>,
    model: &str,
) -> JsonMap {
    let explicit = entry_params(placement.tiers, tier, model);
    if !placement.config.max_tokens_from_tier_model || has_ceiling(&explicit) {
        return explicit;
    }
    let Some(ceiling) = placement.context.group_output_ceiling(model) else {
        return explicit;
    };
    let mut params = explicit;
    params.insert(MAX_TOKENS.to_owned(), Json::Number(Number::from(ceiling)));
    params
}

fn entry_params(set: &TierSet, tier: Option<&Tier>, model: &str) -> JsonMap {
    let Some(tier) = tier else {
        return JsonMap::new();
    };
    let Some(pool) = set.pools.get(tier.name()) else {
        return JsonMap::new();
    };
    let Some(entry) = pool.models().iter().find(|entry| entry.model_name == model) else {
        return JsonMap::new();
    };
    entry
        .params
        .iter()
        .map(|(key, value)| (key.clone(), Json::from(value.clone())))
        .collect()
}

fn has_ceiling(params: &JsonMap) -> bool {
    params.keys().any(|key| CEILING.contains(&key.as_str()))
}
