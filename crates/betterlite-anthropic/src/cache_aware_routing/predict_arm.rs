use betterlite_logging::cache_observer::CacheObservation;
use betterlite_logging::cache_observer::ObservedPrefix;
use betterlite_logging::cache_observer::lookup;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;

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
use crate::cache_prediction_targets::counting::TokenCounter;
use crate::cache_prediction_targets::scope::cache_scope;
use crate::cache_prediction_targets::target::NativePredictionTarget;
use crate::cache_prediction_targets::target::resolve_prediction_target;

const BLOCKED_REASON: &str = "unsupported_deployment_configuration";
const TOKEN_COUNT_UNAVAILABLE: &str = "token_count_unavailable";
const INCONSISTENT_COUNTS: &str = "inconsistent_prefix_token_count";
const PRICING_UNAVAILABLE: &str = "pricing_unavailable";
const OBSERVATION_EXPIRED: &str = "observation_expired";
const NO_OBSERVATION: &str = "no_compatible_observation";

pub async fn predict_arm_for_request<Counter>(
    request: &ArmRequest<'_>,
    body: &Json,
    prefix: &PromptPrefix,
    counter: &Counter,
    services: &PredictionServices<'_>,
) -> CachePredictionArm
where
    Counter: TokenCounter,
{
    let deployment = request.deployment;
    if deployment.blocked {
        return failed_arm(deployment.deployment_id, deployment.model, BLOCKED_REASON);
    }
    let target = match resolve_prediction_target(
        deployment.litellm_params,
        services.settings,
        services.resolution,
    ) {
        Ok(target) => target,
        Err(reason) => {
            return failed_arm(deployment.deployment_id, deployment.model, reason.reason());
        }
    };
    let Some(counts) = PromptCounts::measure(&target, body, prefix, counter).await else {
        return failed_arm(
            deployment.deployment_id,
            deployment.model,
            TOKEN_COUNT_UNAVAILABLE,
        );
    };
    predict_counted(request, prefix, &target, counts, services).await
}

async fn predict_counted(
    request: &ArmRequest<'_>,
    prefix: &PromptPrefix,
    target: &NativePredictionTarget,
    counts: PromptCounts,
    services: &PredictionServices<'_>,
) -> CachePredictionArm {
    let deployment = request.deployment;
    let scope = cache_scope(
        request.caller_key_hash,
        deployment.deployment_id,
        target.api_key.expose_secret(),
        &target.model,
    );
    let observed = ObservedPrefix {
        fingerprint: prefix.fingerprint.clone(),
        fingerprints: prefix.fingerprints.clone(),
    };
    let observation = lookup(services.cache, &scope, &observed, Some(request.checked_at)).await;
    let exact = observation
        .as_ref()
        .is_some_and(|found| found.fingerprint == prefix.fingerprint);
    let Some(cacheable) = cacheable_tokens(counts, exact, observation.as_ref()) else {
        return failed_arm(
            deployment.deployment_id,
            deployment.model,
            INCONSISTENT_COUNTS,
        );
    };
    let arm = ResolvedArm {
        request,
        prefix,
        target: target.clone(),
        total: counts.total,
        cacheable,
        suffix: counts.total.saturating_sub(cacheable),
        observation,
        exact,
        checked_at: request.checked_at,
    };
    arm.finish(services).await
}

fn cacheable_tokens(
    counts: PromptCounts,
    exact: bool,
    observation: Option<&CacheObservation>,
) -> Option<i64> {
    let cacheable = if exact {
        observation.map_or(counts.prefix, |found| found.cached_tokens)
    } else {
        counts.prefix
    };
    let observed_over = observation.is_some_and(|found| found.cached_tokens > cacheable);
    if cacheable > counts.total || observed_over {
        return None;
    }
    Some(cacheable)
}

#[derive(Clone, Copy, Debug)]
struct PromptCounts {
    total: i64,
    prefix: i64,
}

impl PromptCounts {
    async fn measure<Counter>(
        target: &NativePredictionTarget,
        body: &Json,
        prefix: &PromptPrefix,
        counter: &Counter,
    ) -> Option<Self>
    where
        Counter: TokenCounter,
    {
        let prefix_body: Json = serde_json::from_str(prefix.prefix_body.get()).ok()?;
        let total = counter.count(&target.model, &target.api_key, body).await?;
        let prefix_tokens = counter
            .count(&target.model, &target.api_key, &prefix_body)
            .await?;
        if total < prefix_tokens {
            return None;
        }
        Some(Self {
            total,
            prefix: prefix_tokens,
        })
    }
}

struct ResolvedArm<'request, 'prefix> {
    request: &'request ArmRequest<'request>,
    prefix: &'prefix PromptPrefix,
    target: NativePredictionTarget,
    total: i64,
    cacheable: i64,
    suffix: i64,
    observation: Option<CacheObservation>,
    exact: bool,
    checked_at: f64,
}

impl ResolvedArm<'_, '_> {
    async fn finish(&self, services: &PredictionServices<'_>) -> CachePredictionArm {
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

fn failed_arm(deployment_id: &str, model: Option<&str>, reason: &str) -> CachePredictionArm {
    CachePredictionArm {
        deployment_id: deployment_id.to_owned(),
        model: model.map(str::to_owned),
        cache_state: CacheState::Unknown,
        reason: Some(reason.to_owned()),
        estimate: None,
        cold: None,
        warm: None,
        evidence: None,
        token_count_source: None,
    }
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

mod tests;
