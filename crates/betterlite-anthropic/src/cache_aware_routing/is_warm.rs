use betterlite_logging::cache_observer::ObservedPrefix;
use betterlite_logging::cache_observer::lookup;
use secrecy::ExposeSecret as _;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::WarmRequest;
use crate::cache_aware_routing::services::PredictionServices;
use crate::cache_prediction_targets::scope::cache_scope;
use crate::cache_prediction_targets::target::resolve_prediction_target;

impl AnthropicCacheRouting {
    pub async fn is_warm(
        &self,
        request: &WarmRequest<'_>,
        services: &PredictionServices<'_>,
    ) -> bool {
        let Ok(target) = resolve_prediction_target(
            request.deployment.litellm_params,
            services.settings,
            services.resolution,
        ) else {
            return false;
        };
        let scope = cache_scope(
            request.caller_key_hash,
            request.deployment.deployment_id,
            target.api_key.expose_secret(),
            &target.model,
        );
        let observed = ObservedPrefix {
            fingerprint: self.prefix.fingerprint.clone(),
            fingerprints: self.prefix.fingerprints.clone(),
        };
        let observation = lookup(services.cache, &scope, &observed, Some(request.now)).await;
        observation.is_some_and(|found| {
            found.fingerprint == self.prefix.fingerprint && found.expires_at > request.now
        })
    }
}

mod tests;
