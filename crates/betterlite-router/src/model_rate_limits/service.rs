use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_tokenizer::reservation::RequestBody;

use crate::deployment_identity::spec::DeploymentSpec;
use crate::io_token_reservations::clock::{IoClock, minute_label};
use crate::io_token_reservations::error::IoPreCallError;
use crate::io_token_reservations::limits::IoDeployment;
use crate::io_token_reservations::reservation::IoReservation;

use super::cache::{RateLimitCache, RemoteRead};
use super::deployment::{ModelLimits, io_limits, model, model_id, model_limits};
use super::error::{
    ModelRateLimitError, RATE_LIMIT_TTL_SECONDS, RateLimitChannel, RateLimitExceeded,
};
use super::keys::{RateKeys, cache_keys};

// io_token_pre_call_check and the refunds B019's ITPM/OTPM part composes ahead of the tpm/rpm checks
#[async_trait]
pub trait IoTokenReservationHook: Send + Sync {
    async fn reserve(
        &self,
        deployment: &IoDeployment<'_>,
        request: &mut RequestBody,
    ) -> Result<Option<IoReservation>, IoPreCallError>;

    // io_token_refund_failure
    fn refund(&self, request: &mut RequestBody);

    // async_io_token_refund_failure
    async fn async_refund(&self, request: &mut RequestBody);
}

// The usage one successful response reports: its model id, the resolved model name, and its total token count
pub struct SuccessUsage<'usage> {
    pub model_id: Option<&'usage str>,
    pub model: Option<&'usage str>,
    pub total_tokens: i64,
}

impl<'usage> SuccessUsage<'usage> {
    #[must_use]
    pub const fn new(
        model_id: Option<&'usage str>,
        model: Option<&'usage str>,
        total_tokens: i64,
    ) -> Self {
        Self {
            model_id,
            model,
            total_tokens,
        }
    }
}

// The answer a tpm read gives the check: a current count, or the observation is unusable and the request goes through
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CurrentTpm {
    Known(Option<i64>),
    Unavailable,
}

pub struct ModelRateLimits {
    pub cache: Arc<dyn RateLimitCache>,
    pub io: Option<Arc<dyn IoTokenReservationHook>>,
    pub clock: IoClock,
    warned: Mutex<BTreeSet<String>>,
}

impl ModelRateLimits {
    #[must_use]
    pub fn new(
        cache: Arc<dyn RateLimitCache>,
        io: Option<Arc<dyn IoTokenReservationHook>>,
        clock: IoClock,
    ) -> Self {
        Self {
            cache,
            io,
            clock,
            warned: Mutex::new(BTreeSet::new()),
        }
    }

    // _warn_io_token_and_tpm_rpm_coexist_once: true when this request logs the warning, once per deployment id
    pub fn logs_coexist_warning(
        &self,
        deployment: &DeploymentSpec,
    ) -> Result<bool, ModelRateLimitError> {
        let io = io_limits(deployment)?;
        if io.itpm.is_none() && io.otpm.is_none() {
            return Ok(false);
        }
        let limits = model_limits(deployment)?;
        if limits.tpm.is_none() && limits.rpm.is_none() {
            return Ok(false);
        }
        let Some(id) = model_id(deployment) else {
            return Ok(true);
        };
        Ok(self
            .warned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.to_owned()))
    }

    // async_pre_call_check: the io reservation first, then the tpm read and the increment-first rpm count
    pub async fn async_pre_call_check(
        &self,
        deployment: &DeploymentSpec,
        request: &mut RequestBody,
    ) -> Result<(), ModelRateLimitError> {
        let limits = model_limits(deployment)?;
        let io = io_limits(deployment)?;
        let mut reserved = false;
        if io.itpm.is_some() || io.otpm.is_some() {
            if self.logs_coexist_warning(deployment)? {
                tracing::warn!(
                    model_id = model_id(deployment).unwrap_or_default(),
                    "deployment configures itpm/otpm alongside tpm/rpm; both limit types are enforced"
                );
            }
            if let Some(hook) = &self.io {
                let target = IoDeployment {
                    model_id: model_id(deployment),
                    model: model(deployment),
                    limits: io,
                };
                reserved = hook.reserve(&target, request).await?.is_some();
            }
        }
        if limits.tpm.is_none() && limits.rpm.is_none() {
            return Ok(());
        }
        let keys = cache_keys(deployment, &minute_label((self.clock)()));
        match self.async_check_limits(deployment, limits, &keys).await {
            Ok(()) => Ok(()),
            Err(error) => {
                if reserved && let Some(hook) = &self.io {
                    hook.async_refund(request).await;
                }
                Err(ModelRateLimitError::from(error))
            }
        }
    }

    // pre_call_check: the blocking facade reads and counts the in-memory tier only, since no blocking store tier exists
    pub fn pre_call_check(&self, deployment: &DeploymentSpec) -> Result<(), ModelRateLimitError> {
        let limits = model_limits(deployment)?;
        if limits.tpm.is_none() && limits.rpm.is_none() {
            return Ok(());
        }
        let keys = cache_keys(deployment, &minute_label((self.clock)()));
        if let Some(limit) = limits.tpm
            && let Some(current) = self.cache.local(&keys.tpm)
            && current >= limit
        {
            return Err(ModelRateLimitError::from(exceeded(
                deployment,
                RateLimitChannel::Tpm,
                limit,
                current,
                None,
            )));
        }
        if let Some(limit) = limits.rpm {
            let Some(current) = self
                .cache
                .increment_local(&keys.rpm, 1, RATE_LIMIT_TTL_SECONDS)
            else {
                return Ok(());
            };
            if current > limit {
                return Err(ModelRateLimitError::from(exceeded(
                    deployment,
                    RateLimitChannel::Rpm,
                    limit,
                    current,
                    None,
                )));
            }
        }
        Ok(())
    }

    // async_log_success_event: a response that reported both a model name and tokens counts its tpm usage
    pub async fn async_log_success_event(&self, usage: &SuccessUsage<'_>) {
        if let Some(key) = self.success_key(usage) {
            let _counted = self
                .cache
                .increment(&key, usage.total_tokens, RATE_LIMIT_TTL_SECONDS)
                .await;
        }
    }

    // log_success_event: the blocking twin counts the in-memory tier only
    pub fn log_success_event(&self, usage: &SuccessUsage<'_>) {
        if let Some(key) = self.success_key(usage) {
            let _counted =
                self.cache
                    .increment_local(&key, usage.total_tokens, RATE_LIMIT_TTL_SECONDS);
        }
    }

    // async_log_failure_event: never fail the primary logging pipeline over an io-token refund error
    pub async fn async_log_failure_event(&self, request: &mut RequestBody) {
        if let Some(hook) = &self.io {
            hook.async_refund(request).await;
        }
    }

    // log_failure_event: the blocking twin
    pub fn log_failure_event(&self, request: &mut RequestBody) {
        if let Some(hook) = &self.io {
            hook.refund(request);
        }
    }

    fn success_key(&self, usage: &SuccessUsage<'_>) -> Option<String> {
        let model_id = usage.model_id?;
        let model = usage.model.filter(|name| !name.is_empty())?;
        if usage.total_tokens == 0 {
            return None;
        }
        Some(format!(
            "{model_id}:{model}:tpm:{}",
            minute_label((self.clock)())
        ))
    }

    async fn async_check_limits(
        &self,
        deployment: &DeploymentSpec,
        limits: ModelLimits,
        keys: &RateKeys,
    ) -> Result<(), RateLimitExceeded> {
        if let Some(limit) = limits.tpm {
            match self.async_current_tpm(&keys.tpm, limit).await {
                CurrentTpm::Unavailable => return Ok(()),
                CurrentTpm::Known(Some(current)) if current >= limit => {
                    return Err(exceeded(
                        deployment,
                        RateLimitChannel::Tpm,
                        limit,
                        current,
                        Some(0),
                    ));
                }
                CurrentTpm::Known(_) => {}
            }
        }
        if let Some(limit) = limits.rpm {
            let Some(current) = self
                .cache
                .increment(&keys.rpm, 1, RATE_LIMIT_TTL_SECONDS)
                .await
            else {
                return Ok(());
            };
            if current > limit {
                return Err(exceeded(
                    deployment,
                    RateLimitChannel::Rpm,
                    limit,
                    current,
                    Some(0),
                ));
            }
        }
        Ok(())
    }

    // _async_get_current_tpm: the in-memory value wins once it is at the limit, then Redis answers, and only an open breaker falls back
    async fn async_current_tpm(&self, key: &str, limit: i64) -> CurrentTpm {
        let local = self.cache.local(key);
        if !self.cache.has_remote() || local.is_some_and(|value| value >= limit) {
            return CurrentTpm::Known(local);
        }
        match self.cache.remote(key).await {
            RemoteRead::Value(value) => CurrentTpm::Known(value),
            RemoteRead::BreakerOpen => CurrentTpm::Known(local),
            RemoteRead::Failed => CurrentTpm::Unavailable,
        }
    }
}

// _raise_rate_limit_error builds the same body the async and blocking checks reject with
fn exceeded(
    deployment: &DeploymentSpec,
    channel: RateLimitChannel,
    limit: i64,
    current: i64,
    num_retries: Option<u32>,
) -> RateLimitExceeded {
    RateLimitExceeded {
        channel,
        limit,
        current,
        model_id: model_id(deployment).unwrap_or("None").to_owned(),
        model: model(deployment).unwrap_or("None").to_owned(),
        model_group: deployment.model_name.clone(),
        num_retries,
    }
}
