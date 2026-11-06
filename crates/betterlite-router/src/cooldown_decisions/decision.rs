use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::cooldown_decisions::failure::{
    ErrorStatus, FailureClass, FailureFacts, Seconds, exception_class_name,
    is_background_cost_poll_404, is_caller_timeout_408,
};
use crate::cooldown_decisions::policy::{AllowedFailsPolicy, CooldownPolicy, PolicyOrder};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinuteSamples {
    pub successes: u64,
    pub failures: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentFacts {
    pub id: String,
    pub model_group_size: u32,
    pub has_alternatives: bool,
    pub provider_default: bool,
    #[serde(default)]
    pub allowed_fails_policy: Option<AllowedFailsPolicy>,
    #[serde(default)]
    pub allowed_fails: Option<u32>,
    #[serde(default)]
    pub model_info_cooldown_time: Option<Seconds>,
    #[serde(default)]
    pub litellm_params_cooldown_time: Option<Seconds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedFailsCounter {
    pub key: String,
    pub ttl_seconds: Seconds,
    pub fails: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CooldownDecision {
    pub cooldown: bool,
    pub counter: Option<AllowedFailsCounter>,
}

impl CooldownDecision {
    pub const NONE: Self = Self {
        cooldown: false,
        counter: None,
    };
    pub const COOLDOWN: Self = Self {
        cooldown: true,
        counter: None,
    };
}

#[derive(Debug, Clone, Copy)]
pub struct CooldownRequest<'facts> {
    pub policy: &'facts CooldownPolicy,
    pub deployment: Option<&'facts DeploymentFacts>,
    pub failure: &'facts FailureFacts,
    pub minute: MinuteSamples,
    pub allowed_fails_before: u64,
    pub time_to_cooldown: Option<Seconds>,
}

#[must_use]
pub fn decide(request: &CooldownRequest<'_>) -> CooldownDecision {
    let failure = request.failure;
    if failure.advisor_orchestration
        || is_background_cost_poll_404(&failure.status, failure.background_response_cost_poll)
        || is_caller_timeout_408(&failure.status, failure.caller_timeout.as_ref())
    {
        return CooldownDecision::NONE;
    }
    // a missing deployment and a deployment whose model group is unknown both stop cooldown logic
    let Some(deployment) = request.deployment else {
        return CooldownDecision::NONE;
    };
    if !should_run_cooldown(
        request.policy,
        deployment,
        failure,
        request.time_to_cooldown,
    ) {
        return CooldownDecision::NONE;
    }
    should_cooldown(
        request.policy,
        deployment,
        failure,
        request.minute,
        request.allowed_fails_before,
    )
}

fn should_run_cooldown(
    policy: &CooldownPolicy,
    deployment: &DeploymentFacts,
    failure: &FailureFacts,
    time_to_cooldown: Option<Seconds>,
) -> bool {
    if time_to_cooldown.is_some_and(|value| value.0.abs() <= 1e-9_f64) {
        return false;
    }
    if policy.disable_cooldowns {
        return false;
    }
    if !is_cooldown_required(failure) && !has_explicit_policy(deployment, failure.class) {
        return false;
    }
    !deployment.provider_default
}

fn is_cooldown_required(failure: &FailureFacts) -> bool {
    // LiteLLM skips the cooldown when "APIConnectionError" appears in str(exception); the typed family is the same case
    if failure.class == FailureClass::ApiConnection {
        return false;
    }
    match &failure.status {
        ErrorStatus::Text(text) if text.is_empty() => false,
        ErrorStatus::Text(text) => text
            .trim()
            .parse::<i64>()
            .map_or(true, status_requires_cooldown),
        ErrorStatus::Code(code) => status_requires_cooldown(*code),
    }
}

fn status_requires_cooldown(code: i64) -> bool {
    if (400..500).contains(&code) {
        return matches!(code, 401 | 404 | 408 | 429);
    }
    true
}

fn has_explicit_policy(deployment: &DeploymentFacts, class: FailureClass) -> bool {
    let Some(allowed_fails_policy) = deployment.allowed_fails_policy.as_ref() else {
        return false;
    };
    allowed_fails_policy
        .resolve(PolicyOrder::Deployment, class)
        .is_some()
}

fn should_cooldown(
    policy: &CooldownPolicy,
    deployment: &DeploymentFacts,
    failure: &FailureFacts,
    minute: MinuteSamples,
    allowed_fails_before: u64,
) -> CooldownDecision {
    let single = deployment.model_group_size == 1 && !deployment.has_alternatives;
    if deployment.allowed_fails_policy.is_some() || deployment.allowed_fails.is_some() {
        return deployment_decision(policy, deployment, failure, single, allowed_fails_before);
    }
    if policy.allowed_fails_policy.is_none() && policy.allowed_fails == policy.default_allowed_fails
    {
        return failure_ratio_decision(policy, failure, minute, single);
    }
    let router_allowed = policy
        .allowed_fails_policy
        .as_ref()
        .and_then(|allowed_fails_policy| {
            allowed_fails_policy.resolve(PolicyOrder::Router, failure.class)
        });
    let allowed = router_allowed.unwrap_or(policy.allowed_fails);
    count_fails(
        deployment,
        allowed,
        policy.cooldown_time,
        None,
        allowed_fails_before,
    )
}

fn deployment_decision(
    policy: &CooldownPolicy,
    deployment: &DeploymentFacts,
    failure: &FailureFacts,
    single: bool,
    allowed_fails_before: u64,
) -> CooldownDecision {
    let from_policy = deployment
        .allowed_fails_policy
        .as_ref()
        .and_then(|allowed_fails_policy| {
            allowed_fails_policy.resolve(PolicyOrder::Deployment, failure.class)
        });
    // a plain deployment allowed_fails keeps the single-deployment safety net that a named policy entry overrides
    if from_policy.is_none() && deployment.allowed_fails.is_some() && single {
        return CooldownDecision::NONE;
    }
    let router_allowed = policy
        .allowed_fails_policy
        .as_ref()
        .and_then(|allowed_fails_policy| {
            allowed_fails_policy.resolve(PolicyOrder::Router, failure.class)
        });
    let allowed = from_policy
        .or(deployment.allowed_fails)
        .or(router_allowed)
        .unwrap_or(policy.allowed_fails);
    let suffix = if from_policy.is_some() {
        Some(exception_class_name(failure.class))
    } else if deployment.allowed_fails.is_some() {
        Some("generic")
    } else {
        None
    };
    // LiteLLM reads cooldown_time from model_info first and falls back to litellm_params
    let deployment_ttl = deployment
        .model_info_cooldown_time
        .or(deployment.litellm_params_cooldown_time);
    let ttl = deployment_ttl.unwrap_or(policy.cooldown_time);
    count_fails(deployment, allowed, ttl, suffix, allowed_fails_before)
}

fn count_fails(
    deployment: &DeploymentFacts,
    allowed: u32,
    ttl: Seconds,
    cache_suffix: Option<&str>,
    allowed_fails_before: u64,
) -> CooldownDecision {
    let key = cache_suffix.map_or_else(
        || format!("deployment:{}:allowed_fails", deployment.id),
        |suffix| format!("deployment:{}:allowed_fails:{suffix}", deployment.id),
    );
    let fails = allowed_fails_before.saturating_add(1);
    CooldownDecision {
        cooldown: fails > u64::from(allowed),
        counter: Some(AllowedFailsCounter {
            key,
            ttl_seconds: ttl,
            fails,
        }),
    }
}

fn failure_ratio_decision(
    policy: &CooldownPolicy,
    failure: &FailureFacts,
    minute: MinuteSamples,
    single: bool,
) -> CooldownDecision {
    let total = minute.successes.saturating_add(minute.failures);
    let code = failure.status.cast_code();
    if code == 429 && !single {
        return CooldownDecision::COOLDOWN;
    }
    if minute.successes == 0
        && minute.failures > 0
        && total >= policy.thresholds.single_deployment_traffic
    {
        return CooldownDecision::COOLDOWN;
    }
    let ratio = Decimal::from(minute.failures).checked_div(Decimal::from(total));
    if total >= policy.thresholds.minimum_requests
        && !single
        && ratio.is_some_and(|value| value > policy.thresholds.ratio)
    {
        return CooldownDecision::COOLDOWN;
    }
    if should_retry(code) {
        return CooldownDecision::NONE;
    }
    CooldownDecision::COOLDOWN
}

pub(super) const fn should_retry(code: i64) -> bool {
    if code == 408 || code == 409 || code == 429 {
        return true;
    }
    code >= 500
}
