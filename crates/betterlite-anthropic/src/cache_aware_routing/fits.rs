use betterlite_models::anthropic_pricing::ANTHROPIC_PROVIDER;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::tiered_pricing::Scalar;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::services::PredictionServices;
use crate::cache_prediction_targets::target::resolve_prediction_target;

const MAX_INPUT_TOKENS: &str = "max_input_tokens";
const MAX_OUTPUT_TOKENS: &str = "max_output_tokens";

impl AnthropicCacheRouting {
    #[must_use]
    pub fn fits(
        deployment: &PredictionDeployment<'_>,
        input_tokens: i64,
        output_tokens: i64,
        services: &PredictionServices<'_>,
        lookup: &dyn PriceLookup,
    ) -> bool {
        let Ok(target) = resolve_prediction_target(
            deployment.litellm_params,
            services.settings,
            services.resolution,
        ) else {
            return false;
        };
        let resolved = lookup.effective_model_info(&target.model, Some(ANTHROPIC_PROVIDER));
        let max_input = declared_limit(deployment.model_info, MAX_INPUT_TOKENS).or_else(|| {
            resolved
                .as_ref()
                .and_then(|record| record_limit(record, MAX_INPUT_TOKENS))
        });
        let max_output = declared_limit(deployment.model_info, MAX_OUTPUT_TOKENS).or_else(|| {
            resolved
                .as_ref()
                .and_then(|record| record_limit(record, MAX_OUTPUT_TOKENS))
        });
        let (Some(max_input), Some(max_output)) = (max_input, max_output) else {
            return false;
        };
        let Some(window) = input_tokens.checked_add(output_tokens) else {
            return false;
        };
        window <= max_input && output_tokens <= max_output
    }
}

fn declared_limit(model_info: &JsonObject, field: &str) -> Option<i64> {
    match model_info.get(field) {
        Some(Json::Number(number)) => number.as_i64(),
        _ => None,
    }
}

fn record_limit(record: &PriceRecord, field: &str) -> Option<i64> {
    match record.get(field) {
        Some(Scalar::Number(number)) => number.as_i64(),
        _ => None,
    }
}

mod tests;
