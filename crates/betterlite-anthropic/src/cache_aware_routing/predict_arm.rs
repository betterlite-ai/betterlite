use betterlite_logging::cache_observer::CacheObservation;
use betterlite_logging::cache_observer::ObservedPrefix;
use betterlite_logging::cache_observer::lookup;
use betterlite_models::decisions::Json;
use secrecy::ExposeSecret as _;

use crate::cache_aware_routing::deployment::ArmRequest;
use crate::cache_aware_routing::services::PredictionServices;
use crate::cache_aware_routing::types::CachePredictionArm;
use crate::cache_aware_routing::types::CacheState;
use crate::cache_plan::PromptPrefix;
use crate::cache_prediction_targets::counting::TokenCounter;
use crate::cache_prediction_targets::scope::cache_scope;
use crate::cache_prediction_targets::target::NativePredictionTarget;
use crate::cache_prediction_targets::target::resolve_prediction_target;

use resolved::ResolvedArm;

const BLOCKED_REASON: &str = "unsupported_deployment_configuration";
const TOKEN_COUNT_UNAVAILABLE: &str = "token_count_unavailable";
const INCONSISTENT_COUNTS: &str = "inconsistent_prefix_token_count";

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

mod resolved;
mod tests;
