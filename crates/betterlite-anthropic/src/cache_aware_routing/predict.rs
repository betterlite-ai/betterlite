use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::ArmRequest;
use crate::cache_aware_routing::predict_arm;
use crate::cache_aware_routing::services::PredictionServices;
use crate::cache_aware_routing::types::CachePredictionArm;
use crate::cache_prediction_targets::counting::TokenCounter;

impl AnthropicCacheRouting {
    pub async fn predict<Counter: TokenCounter>(
        &self,
        request: &ArmRequest<'_>,
        counter: &Counter,
        services: &PredictionServices<'_>,
    ) -> CachePredictionArm {
        predict_arm::predict_arm_for_request(request, &self.body, &self.prefix, counter, services)
            .await
    }
}

mod tests;
