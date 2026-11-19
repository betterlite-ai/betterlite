use chrono::DateTime;
use chrono::Utc;
use rust_decimal::Decimal;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::services::CacheCostService;
use crate::cache_aware_routing::services::PriceRequest;
use crate::cache_aware_routing::types::CachePredictionArm;

impl AnthropicCacheRouting {
    #[must_use]
    pub fn cost(
        service: &dyn CacheCostService,
        arm: &CachePredictionArm,
        output_tokens: i64,
        moment: DateTime<Utc>,
    ) -> Option<Decimal> {
        let scenario = arm.estimate.as_ref()?;
        let request = PriceRequest {
            model: arm.model.as_deref().unwrap_or_default(),
            deployment_id: &arm.deployment_id,
            tokens: &scenario.tokens,
            completion_tokens: output_tokens,
        };
        service.price(&request, moment)
    }
}

mod tests;
