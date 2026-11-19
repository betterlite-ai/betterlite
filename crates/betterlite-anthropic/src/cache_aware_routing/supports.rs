use betterlite_models::resolution::ResolutionContext;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_prediction_targets::target::resolve_prediction_target;
use crate::credentials::AnthropicSettings;

impl AnthropicCacheRouting {
    #[must_use]
    pub fn supports(
        deployment: &PredictionDeployment<'_>,
        settings: &AnthropicSettings,
        resolution: &ResolutionContext<'_>,
    ) -> bool {
        resolve_prediction_target(deployment.litellm_params, settings, resolution).is_ok()
    }
}

mod tests;
