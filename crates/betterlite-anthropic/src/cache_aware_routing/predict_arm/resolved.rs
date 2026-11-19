use betterlite_logging::cache_observer::CacheObservation;

use crate::cache_aware_routing::deployment::ArmRequest;
use crate::cache_aware_routing::minimum_tokens::disabled_arm;
use crate::cache_aware_routing::services::PredictionServices;
use crate::cache_aware_routing::services::ScenarioRequest;
use crate::cache_aware_routing::types::CacheCostScenario;
use crate::cache_aware_routing::types::CacheEvidence;
use crate::cache_aware_routing::types::CacheEvidenceConfidence;
use crate::cache_aware_routing::types::CacheEvidenceSource;
use crate::cache_aware_routing::types::CachePredictionArm;
use crate::cache_aware_routing::types::CacheState;
use crate::cache_aware_routing::types::CacheTokenBuckets;
use crate::cache_aware_routing::types::TokenCountSource;
use crate::cache_plan::PromptPrefix;
use crate::cache_prediction_targets::target::NativePredictionTarget;

use super::failed_arm;

const PRICING_UNAVAILABLE: &str = "pricing_unavailable";
const OBSERVATION_EXPIRED: &str = "observation_expired";
const NO_OBSERVATION: &str = "no_compatible_observation";

pub(super) struct ResolvedArm<'request, 'prefix> {
    pub(super) request: &'request ArmRequest<'request>,
    pub(super) prefix: &'prefix PromptPrefix,
    pub(super) target: NativePredictionTarget,
    pub(super) total: i64,
    pub(super) cacheable: i64,
    pub(super) suffix: i64,
    pub(super) observation: Option<CacheObservation>,
    pub(super) exact: bool,
    pub(super) checked_at: f64,
}

impl ResolvedArm<'_, '_> {
    pub(super) async fn finish(&self, services: &PredictionServices<'_>) -> CachePredictionArm {
        let deployment = self.request.deployment;
        let minimum = services
            .minimum
            .minimum_tokens(deployment.model.unwrap_or_default());
        if self.cacheable < minimum {
            let Some(arm) =
                disabled_arm(&deployment, &self.target.model, self.total, services).await
            else {
                return failed_arm(
                    deployment.deployment_id,
                    deployment.model,
                    PRICING_UNAVAILABLE,
                );
            };
            return arm;
        }
        self.scenarios(services)
    }

    fn scenarios(&self, services: &PredictionServices<'_>) -> CachePredictionArm {
        let window = self.window();
        let read = match window {
            CacheWindow::FreshExact | CacheWindow::FreshLookback => self
                .observation
                .as_ref()
                .map_or(0, |found| found.cached_tokens),
            CacheWindow::Absent | CacheWindow::Expired => 0,
        };
        let ttl = self.prefix.ttl_seconds;
        let cold_tokens = services.cost.buckets(self.cacheable, self.suffix, 0, ttl);
        let warm_tokens = services
            .cost
            .buckets(self.cacheable, self.suffix, self.cacheable, ttl);
        let estimate_tokens = services
            .cost
            .buckets(self.cacheable, self.suffix, read, ttl);
        let priced = (
            self.price(services, &cold_tokens),
            self.price(services, &warm_tokens),
            self.price(services, &estimate_tokens),
        );
        let (Some(cold), Some(warm), Some(estimate)) = priced else {
            let deployment = self.request.deployment;
            return failed_arm(
                deployment.deployment_id,
                deployment.model,
                PRICING_UNAVAILABLE,
            );
        };
        CachePredictionArm {
            deployment_id: self.request.deployment.deployment_id.to_owned(),
            model: Some(self.target.model.clone()),
            cache_state: cache_state(window),
            reason: cache_reason(window).map(str::to_owned),
            estimate: Some(estimate),
            cold: Some(cold),
            warm: Some(warm),
            evidence: self.evidence(),
            token_count_source: Some(TokenCountSource::AnthropicCountTokens),
        }
    }

    fn price(
        &self,
        services: &PredictionServices<'_>,
        tokens: &CacheTokenBuckets,
    ) -> Option<CacheCostScenario> {
        let request = ScenarioRequest {
            model: &self.target.model,
            deployment_id: self.request.deployment.deployment_id,
            tokens,
        };
        services.cost.scenario(&request, services.billing_time)
    }

    fn window(&self) -> CacheWindow {
        let Some(found) = &self.observation else {
            return CacheWindow::Absent;
        };
        if found.expires_at <= self.checked_at {
            return CacheWindow::Expired;
        }
        if self.exact {
            return CacheWindow::FreshExact;
        }
        CacheWindow::FreshLookback
    }

    fn evidence(&self) -> Option<CacheEvidence> {
        self.observation.as_ref().map(|found| CacheEvidence {
            observed_at: found.observed_at,
            expires_at: found.expires_at,
            source: CacheEvidenceSource::ProviderUsage,
            confidence: CacheEvidenceConfidence::Observed,
        })
    }
}

// LiteLLM reads the same observation twice: once for the cache state and once for the reason it names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheWindow {
    Absent,
    Expired,
    FreshExact,
    FreshLookback,
}

const fn cache_state(window: CacheWindow) -> CacheState {
    match window {
        CacheWindow::FreshExact => CacheState::Warm,
        CacheWindow::FreshLookback => CacheState::Partial,
        CacheWindow::Expired => CacheState::Stale,
        CacheWindow::Absent => CacheState::Unknown,
    }
}

const fn cache_reason(window: CacheWindow) -> Option<&'static str> {
    match window {
        CacheWindow::FreshExact | CacheWindow::FreshLookback => None,
        CacheWindow::Expired => Some(OBSERVATION_EXPIRED),
        CacheWindow::Absent => Some(NO_OBSERVATION),
    }
}
