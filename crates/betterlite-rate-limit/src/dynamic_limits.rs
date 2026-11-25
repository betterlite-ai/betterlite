use std::sync::Arc;

use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_core::api::request_principal_metadata::PrincipalFacts;
use betterlite_core::api::request_principal_metadata::principal::logged_api_key;
use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use chrono::{DateTime, Utc};
use tokio_util::task::TaskTracker;

use crate::dynamic_limits::headers::write_usage_headers;
use crate::dynamic_limits::ports::{
    CacheReadError, CacheWriteError, DynamicLimitRouter, MinuteSetCache,
};
use crate::dynamic_limits::quota::{AvailableUsage, UsageError, available_quota, remaining_limit};
use crate::dynamic_limits::rejection::{
    DynamicLimitError, ExceededLimit, LimitCheck, dynamic_rate_limit_error,
};
use crate::dynamic_priority::settings::PriorityPolicy;
use crate::limiter_records::LimiterClock;
use crate::provider_attribution::{
    ModelGroupLimits, RateLimitErrorAttribution, convert_priority_to_percent,
};

pub mod headers;
pub mod ports;
pub mod quota;
pub mod rejection;

// check_available_usage's `weight: float = 1`: LiteLLM keeps the whole capacity unless the reservation names a share
const DEFAULT_WEIGHT: f64 = 1.0;

// user_api_key_dict.metadata's priority slot, the key check_available_usage reads
const PRIORITY_FIELD: &str = "priority";

// `user_api_key_dict.token or "default_key"`: a key with no identifier still registers as an active project
const DEFAULT_KEY: &str = "default_key";

// a minute of keys, expiring together as LiteLLM's ttl does
const CACHE_TTL_SECONDS: i64 = 60;

// DynamicRateLimiterCache: LiteLLM writes each minute's model set under strftime("%H-%M") with a ttl of 60 seconds
pub struct DynamicRateLimiterCache {
    pub cache: Arc<dyn MinuteSetCache>,
    pub clock: Arc<dyn LimiterClock>,
}

impl DynamicRateLimiterCache {
    // async_get_cache: the minute set's length, or None when the cache holds no set for this minute
    pub async fn async_get_cache(&self, model: &str) -> Result<Option<i64>, CacheReadError> {
        let key = self.minute_key(model);
        let stored = self.cache.async_get_cache(&key).await?;
        let Some(members) = stored else {
            return Ok(None);
        };
        Ok(i64::try_from(members.len()).ok())
    }

    // async_set_cache_sadd: the key joins this minute's model set and the cache expires it after CACHE_TTL_SECONDS
    pub async fn async_set_cache_sadd(
        &self,
        model: &str,
        values: &[String],
    ) -> Result<(), CacheWriteError> {
        let key = self.minute_key(model);
        let write = self
            .cache
            .async_set_cache_sadd(&key, values, CACHE_TTL_SECONDS);
        write.await
    }

    // f"{hour}-{minute}:{model}", with hour and minute read from UTC time as datetime.strftime("%H-%M") writes them
    fn minute_key(&self, model: &str) -> String {
        let moment = DateTime::<Utc>::from(self.clock.current_time());
        let minute = moment.format("%H-%M");
        format!("{minute}:{model}")
    }
}

// _PROXY_DynamicRateLimitHandler: the per-project share of each model group's limits and the 429 a spent share raises
pub struct DynamicRateLimitHandler {
    pub internal_usage_cache: Arc<DynamicRateLimiterCache>,
    pub llm_router: Option<Arc<dyn DynamicLimitRouter>>,
    pub policy: PriorityPolicy,
    pub native: Arc<dyn ProviderResolution>,
    pub router: Option<Arc<dyn DeploymentSnapshot>>,
    pub tasks: TaskTracker,
}

// the two arguments async_pre_call_hook reads: the model a request names and the key that asked
pub struct DynamicLimitRequest<'request> {
    pub model: Option<&'request str>,
    pub principal: &'request PrincipalFacts,
}

// the two arguments async_post_call_success_hook reads: the model id a response names and the key that asked
pub struct PostCallRequest<'request> {
    pub model_id: &'request str,
    pub principal: &'request PrincipalFacts,
}

impl DynamicRateLimitHandler {
    // ast-grep-ignore: no-one-line-functions - LiteLLM stores the router here and every later hook reads it back
    pub fn update_variables(&mut self, llm_router: Arc<dyn DynamicLimitRouter>) {
        self.llm_router = Some(llm_router);
    }

    // check_available_usage: LiteLLM's fail-open contract, where any failed step answers five Nones instead of raising
    pub async fn check_available_usage(
        &self,
        model: &str,
        priority: Option<&str>,
    ) -> AvailableUsage {
        match self.read_available_usage(model, priority).await {
            Ok(usage) => usage,
            Err(error) => {
                tracing::error!(
                    "dynamic_rate_limiter check_available_usage: Exception occurred - {error}"
                );
                AvailableUsage::default()
            }
        }
    }

    // the reads and the arithmetic check_available_usage wraps, in LiteLLM's order: group info, weight, active projects, usage
    async fn read_available_usage(
        &self,
        model: &str,
        priority: Option<&str>,
    ) -> Result<AvailableUsage, UsageError> {
        let Some(router) = self.llm_router.as_deref() else {
            return Err(UsageError::MissingRouter);
        };
        let model_group_info = router.get_model_group_info(model).await?;
        let weight = priority_weight(&self.policy, priority, model_group_info.as_ref());
        let active_projects = self.internal_usage_cache.async_get_cache(model).await?;
        let (current_tokens, current_requests) = router.get_model_group_usage(model).await?;
        let total_tokens = model_group_info.as_ref().and_then(|limits| limits.tpm);
        let total_requests = model_group_info.as_ref().and_then(|limits| limits.rpm);
        let model_tokens = remaining_limit(total_tokens, current_tokens);
        let model_requests = remaining_limit(total_requests, current_requests);
        Ok(AvailableUsage {
            available_tpm: available_quota(model_tokens, weight, active_projects)?,
            available_rpm: available_quota(model_requests, weight, active_projects)?,
            model_tpm: model_tokens,
            model_rpm: model_requests,
            active_projects,
        })
    }

    // LiteLLM raises RateLimitType::TOKENS when the token check refuses and RateLimitType::REQUESTS when the request check does
    pub async fn async_pre_call_hook(
        &self,
        request: &DynamicLimitRequest<'_>,
    ) -> Result<(), DynamicLimitError> {
        let Some(model) = request.model else {
            return Ok(());
        };
        let priority = key_priority(request.principal);
        let usage = self.check_available_usage(model, priority).await;
        if usage.available_tpm == Some(0) {
            return Err(self.rejection(request, ExceededLimit::Tokens, &usage));
        }
        if usage.available_rpm == Some(0) {
            return Err(self.rejection(request, ExceededLimit::Requests, &usage));
        }
        if usage.available_tpm.is_some() || usage.available_rpm.is_some() {
            self.spawn_registration(model, request.principal);
        }
        Ok(())
    }

    // the 429 each refused check answers, with LiteLLM's detail text and the resolved model pair
    fn rejection(
        &self,
        request: &DynamicLimitRequest<'_>,
        exceeded: ExceededLimit,
        usage: &AvailableUsage,
    ) -> DynamicLimitError {
        let (quota, model_limit) = match exceeded {
            ExceededLimit::Tokens => (usage.available_tpm, usage.model_tpm),
            ExceededLimit::Requests => (usage.available_rpm, usage.model_rpm),
        };
        let check = LimitCheck {
            exceeded,
            quota,
            model_limit,
            active_projects: usage.active_projects,
            api_key: logged_api_key(request.principal),
        };
        let attribution = RateLimitErrorAttribution {
            requested_model: request.model,
            native: self.native.as_ref(),
            router: self.router.as_deref(),
        };
        match dynamic_rate_limit_error(&check, &attribution) {
            Ok(error) => DynamicLimitError::Rejected(Box::new(error)),
            Err(source) => DynamicLimitError::Header(source),
        }
    }

    // asyncio.create_task(...async_set_cache_sadd(...)): the write runs on the tracker and only its failure is logged
    fn spawn_registration(&self, model: &str, principal: &PrincipalFacts) {
        let cache = Arc::clone(&self.internal_usage_cache);
        let model_name = model.to_owned();
        let member = logged_api_key(principal).map(str::to_owned);
        let values = vec![member.unwrap_or_else(|| String::from(DEFAULT_KEY))];
        self.tasks.spawn(async move {
            let write = cache.async_set_cache_sadd(&model_name, &values);
            if let Err(error) = write.await {
                tracing::error!(
                    "dynamic_rate_limiter async_set_cache_sadd: Exception occurred - {error}"
                );
            }
        });
    }

    // async_post_call_success_hook: the six debugging headers, added next to whatever another callback already wrote
    pub async fn async_post_call_success_hook(
        &self,
        request: &PostCallRequest<'_>,
        headers: &mut AdditionalHeaders,
    ) {
        let Some(router) = self.llm_router.as_deref() else {
            tracing::error!(
                "dynamic_rate_limiter: the router is not set, so no headers are written"
            );
            return;
        };
        let model_id = request.model_id;
        let Some(model_group) = router.get_model_info(model_id) else {
            tracing::error!("Model info for model with id={model_id} is None");
            return;
        };
        let priority = key_priority(request.principal);
        let usage = self.check_available_usage(&model_group, priority).await;
        write_usage_headers(headers, &model_group, &usage);
    }
}

// litellm.priority_reservation: a priority the loaded reservation does not name keeps LiteLLM's weight of 1.0
fn priority_weight(
    policy: &PriorityPolicy,
    priority: Option<&str>,
    model_info: Option<&ModelGroupLimits>,
) -> f64 {
    let Some(reservation) = &policy.reservation else {
        tracing::error!("Priority Reservation not set for the given priority.");
        return DEFAULT_WEIGHT;
    };
    let Some(value) = priority.and_then(|name| reservation.get(name)) else {
        tracing::error!("Priority Reservation not set for the given priority.");
        return DEFAULT_WEIGHT;
    };
    convert_priority_to_percent(*value, model_info)
}

// user_api_key_dict.metadata.get("priority"): only a text value names a reservation, and an absent one names none
fn key_priority(principal: &PrincipalFacts) -> Option<&str> {
    let value = principal.metadata.get(PRIORITY_FIELD)?;
    match value {
        Json::Text(name) => Some(name.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}
