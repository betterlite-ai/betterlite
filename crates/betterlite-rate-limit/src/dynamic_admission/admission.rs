use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError};

use betterlite_auth::provider_resolution::resolve_rate_limit_provider;
use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::rate_limit::ProxyRateLimitError;

use crate::dynamic_priority::descriptors::{model_tracking_descriptor, priority_based_descriptors};
use crate::dynamic_priority::handler::DynamicRateLimitHandler;
use crate::limiter_records::RateLimitResponse;
use crate::provider_attribution::ModelGroupLimits;
use crate::request_stash::RequestStashSlot;
use crate::windowed_admission::OVER_LIMIT_CODE;

use super::denial::{DenialError, OverLimitFacts, build_over_limit_error};
use super::enforcement::{EnforcementPlan, plan_enforcement};
use super::limiter_port::{AdmissionLimiter, AdmissionLimiterError};

mod tests;

// the auth facts _check_rate_limits reads: the priority metadata is the hook's, the span reaches both limiter calls
pub struct AdmissionKey<'key> {
    pub team_metadata: Option<&'key BTreeMap<String, JsonData>>,
    pub key_metadata: &'key BTreeMap<String, JsonData>,
    pub parent_otel_span: Option<ParentOtelSpan>,
}

// one request's admission input: the model, its group capacities, the key and when the model is saturated
pub struct RateLimitCheck<'check> {
    pub model: &'check str,
    pub model_group_info: ModelGroupLimits,
    pub key: &'check AdmissionKey<'check>,
    pub priority: Option<&'check str>,
    pub saturation: f64,
    pub stash: &'check RequestStashSlot,
}

// resolve_llm_provider_for_rate_limit as the admission calls it: once per rejected request, never on the allowing path
pub trait RateLimitProviderResolver: Send + Sync {
    fn resolve(&self, model: &str) -> (String, String);
}

// B3030's native resolver and the router its alias fallback reads
pub struct AttributionProviderResolver {
    pub native: Arc<dyn ProviderResolution>,
    pub router: Option<Arc<dyn DeploymentSnapshot>>,
}

impl RateLimitProviderResolver for AttributionProviderResolver {
    fn resolve(&self, model: &str) -> (String, String) {
        resolve_rate_limit_provider(Some(model), self.native.as_ref(), self.router.as_deref())
    }
}

// Why _check_rate_limits stopped: the 429 it raises, the header failure it cannot turn into one, or a limiter failure
#[derive(Debug, thiserror::Error)]
pub enum CheckRateLimitError {
    #[error(transparent)]
    Rejection(Box<ProxyRateLimitError>),
    #[error(transparent)]
    Denial(#[from] DenialError),
    #[error(transparent)]
    Limiter(#[from] AdmissionLimiterError),
}

// The dynamic handler's admission: B3051's descriptors and saturation read, B5660's limiter behind the port, B3030's provider pair
pub struct DynamicAdmission<Router> {
    pub limiter: Arc<dyn AdmissionLimiter>,
    pub handler: DynamicRateLimitHandler<Router>,
    pub providers: Arc<dyn RateLimitProviderResolver>,
}

impl<Router: Send + Sync> DynamicAdmission<Router> {
    // _check_rate_limits: enforce the model-wide limit and, once saturated, the priority pools, then keep the answer for the post-call side
    pub async fn check_rate_limits(
        &self,
        check: &RateLimitCheck<'_>,
    ) -> Result<(), CheckRateLimitError> {
        let plan = self.enforcement_plan(check);
        let atomic = self
            .limiter
            .atomic_check_and_increment_by_n(
                &plan.enforced,
                &plan.increments,
                check.key.parent_otel_span.clone(),
            )
            .await?;
        tracing::debug!(
            "Atomic check+increment response: {}",
            serde_json::to_string(&atomic).unwrap_or_default()
        );
        if atomic.overall_code == OVER_LIMIT_CODE {
            let rejection = self.over_limit_error(&atomic, check)?;
            return Err(CheckRateLimitError::Rejection(Box::new(rejection)));
        }
        self.store_response(atomic, &plan, check).await
    }

    // the enforced set: B3051's model-wide descriptor first with the multiplier of 1, and the priority descriptors second when saturated
    fn enforcement_plan(&self, check: &RateLimitCheck<'_>) -> EnforcementPlan {
        let window_size = self.handler.v3_limiter.window_size;
        let model_wide =
            model_tracking_descriptor(check.model, &check.model_group_info, 1, window_size);
        let priority_descriptors = priority_based_descriptors(
            &self.handler.policy,
            check.model,
            check.priority,
            Some(&check.model_group_info),
            window_size,
        );
        plan_enforcement(
            model_wide,
            priority_descriptors,
            check.saturation,
            self.handler.policy.settings.saturation_threshold,
        )
    }

    // the 429 of the first OVER_LIMIT status we know, with the model pair resolved once and only here
    fn over_limit_error(
        &self,
        response: &RateLimitResponse,
        check: &RateLimitCheck<'_>,
    ) -> Result<ProxyRateLimitError, DenialError> {
        let (resolved_model, llm_provider) = self.providers.resolve(check.model);
        let facts = OverLimitFacts {
            model: check.model,
            priority: check.priority,
            saturation: check.saturation,
            window_size: self.limiter.window_size(),
            model_tpm: check.model_group_info.tpm,
            model_rpm: check.model_group_info.rpm,
            resolved_model,
            llm_provider,
        };
        build_over_limit_error(response, &facts)
    }

    // the answer the post-call side reads: the atomic one, or the atomic statuses plus the tracking ones when the pools were only tracked
    async fn store_response(
        &self,
        atomic: RateLimitResponse,
        plan: &EnforcementPlan,
        check: &RateLimitCheck<'_>,
    ) -> Result<(), CheckRateLimitError> {
        let stored = if plan.tracked_only.is_empty() {
            atomic
        } else {
            let tracking = self
                .limiter
                .should_rate_limit(
                    &plan.tracked_only,
                    check.key.parent_otel_span.clone(),
                    false,
                )
                .await?;
            let mut statuses = atomic.statuses;
            statuses.extend(tracking.statuses);
            RateLimitResponse {
                overall_code: atomic.overall_code,
                statuses,
                reservation_windows: None,
            }
        };
        let stash = check.stash.get_or_create();
        let mut state = stash.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.rate_limit_response = Some(stored);
        drop(state);
        Ok(())
    }
}
