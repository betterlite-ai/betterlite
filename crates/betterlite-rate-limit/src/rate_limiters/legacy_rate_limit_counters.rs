use std::collections::BTreeMap;
use std::ops::Div as _;
use std::sync::Arc;

use betterlite_auth::provider_resolution::resolve_rate_limit_provider;
use betterlite_auth::rejection_provider::{DeploymentSnapshot, ProviderResolution};
use betterlite_auth::resolved_key::auth::ParentOtelSpan;
use betterlite_error::proxy_error_types::common::CommonProxyErrors;
use betterlite_error::rate_limit::{
    ProxyRateLimitError, ProxyRateLimitErrorInit, RateLimitDimension,
};
use betterlite_logging::analytics::python::float_text;
use betterlite_redact::structured::Payload;
use chrono::{Datelike as _, Timelike as _};
use num_traits::ToPrimitive as _;

use self::clock::LegacyClock;
use self::error::LegacyRateLimitError;
use self::limits::KeyLimitCheck;
use self::objects::{CacheObject, CacheObjectKeys};
use self::usage_cache::{LegacyCacheValue, LegacyUsageCache, MinuteCounter};
use self::verbose::VerboseConsole;

pub mod clock;
pub mod error;
pub mod limits;
pub mod objects;
pub mod usage_cache;
pub mod verbose;

// litellm's ttl=60 on every write the legacy limiter makes
pub const COUNTER_TTL_SECONDS: u64 = 60;

// _PROXY_MaxParallelRequestsHandler: the per-minute counter checks and the 429 they raise
pub struct LegacyRateLimitHandler {
    pub(crate) internal_usage_cache: Arc<dyn LegacyUsageCache>,
    pub(crate) clock: Arc<dyn LegacyClock>,
    pub(crate) verbose: Arc<dyn VerboseConsole>,
    pub(crate) set_verbose: bool,
    pub(crate) native: Arc<dyn ProviderResolution>,
    pub(crate) router: Option<Arc<dyn DeploymentSnapshot>>,
}

impl LegacyRateLimitHandler {
    pub fn print_verbose(&self, statement: &str) {
        tracing::debug!("{statement}");
        if self.set_verbose && !self.verbose.print_line(statement) {
            tracing::debug!("litellm.set_verbose could not print the statement");
        }
    }

    #[must_use]
    pub fn precise_minute(&self) -> String {
        let now = self.clock.now();
        format!(
            "{:04}-{:02}-{:02}-{:02}-{:02}",
            now.year(),
            now.month(),
            now.day(),
            now.hour(),
            now.minute()
        )
    }

    #[must_use]
    pub fn time_to_next_minute(&self) -> f64 {
        let now = self.clock.now();
        let elapsed = u64::from(now.second())
            .saturating_mul(1_000_000)
            .saturating_add(u64::from(now.timestamp_subsec_micros()));
        let remaining = 60_000_000_u64.saturating_sub(elapsed);
        remaining.to_f64().unwrap_or(f64::MAX).div(1_000_000.0_f64)
    }

    #[must_use]
    pub fn raise_rate_limit_error(
        &self,
        additional_details: Option<&str>,
        rate_limit_type: Option<RateLimitDimension>,
        requested_model: Option<&str>,
    ) -> LegacyRateLimitError {
        let detail = additional_details.map_or_else(
            || String::from("Max parallel request limit reached"),
            |details| format!("Max parallel request limit reached {details}"),
        );
        let dimension = rate_limit_type.unwrap_or(RateLimitDimension::ConcurrentRequests);
        self.rate_limit_error(detail, dimension, requested_model)
    }

    pub async fn check_key_in_limits(
        &self,
        check: KeyLimitCheck<'_>,
        values_to_update_in_cache: &mut Vec<(String, LegacyCacheValue)>,
    ) -> Result<MinuteCounter, LegacyRateLimitError> {
        match check.current {
            None => tracing::info!(
                "Current Usage of {} in this minute: none",
                check.rate_limit_type.as_str()
            ),
            Some(usage) => tracing::info!(
                "Current Usage of {} in this minute: requests {}, tpm {}, rpm {}",
                check.rate_limit_type.as_str(),
                usage.current_requests,
                usage.current_tpm,
                usage.current_rpm
            ),
        }
        let new_val = match check.current {
            None => self.initial_counter(&check)?,
            Some(current) => self.updated_counter(&check, current)?,
        };
        let key = check.request_count_api_key;
        values_to_update_in_cache.push((key, LegacyCacheValue::Counter(new_val)));
        self.write_counters(values_to_update_in_cache, check.parent_otel_span)
            .await;
        Ok(new_val)
    }

    #[must_use]
    pub async fn get_all_cache_objects(
        &self,
        keys: CacheObjectKeys,
        parent_otel_span: Option<&ParentOtelSpan>,
    ) -> CacheObject {
        let key_list = keys.into_list();
        let read = self
            .internal_usage_cache
            .batch_get(&key_list, parent_otel_span)
            .await;
        match read {
            Err(error) => {
                tracing::error!("LiteLLM Cache: exception in async_batch_get_cache: {error}");
                CacheObject::default()
            }
            Ok(Some(results)) if results.len() == key_list.len() => {
                CacheObject::from_results(results)
            }
            Ok(Some(_)) => {
                tracing::error!("LiteLLM Cache: async_batch_get_cache answered the wrong length");
                CacheObject::default()
            }
            Ok(None) => CacheObject::default(),
        }
    }

    // the current-is-None branch: any zero limit blocks every request, and otherwise the counter starts at 1/0/1
    fn initial_counter(
        &self,
        check: &KeyLimitCheck<'_>,
    ) -> Result<MinuteCounter, LegacyRateLimitError> {
        let limits = check.limits;
        if limits.max_parallel_requests == 0 || limits.tpm_limit == 0 || limits.rpm_limit == 0 {
            let details = format!(
                "{}. Hit limit for {}. Current limits: max_parallel_requests: {}, tpm_limit: {}, rpm_limit: {}",
                CommonProxyErrors::MaxParallelRequestLimitReached.as_str(),
                check.rate_limit_type.as_str(),
                limits.max_parallel_requests,
                limits.tpm_limit,
                limits.rpm_limit
            );
            let dimension = limits.disabled_dimension();
            return Err(self.raise_rate_limit_error(
                Some(&details),
                Some(dimension),
                check.requested_model,
            ));
        }
        Ok(MinuteCounter {
            current_requests: 1,
            current_tpm: 0,
            current_rpm: 1,
        })
    }

    fn updated_counter(
        &self,
        check: &KeyLimitCheck<'_>,
        current: MinuteCounter,
    ) -> Result<MinuteCounter, LegacyRateLimitError> {
        let limits = check.limits;
        if current.current_requests < limits.max_parallel_requests
            && current.current_tpm < limits.tpm_limit
            && current.current_rpm < limits.rpm_limit
        {
            return Ok(MinuteCounter {
                current_requests: current.current_requests.saturating_add(1),
                current_tpm: current.current_tpm,
                current_rpm: current.current_rpm.saturating_add(1),
            });
        }
        Err(self.over_limit_error(check, current))
    }

    fn over_limit_error(
        &self,
        check: &KeyLimitCheck<'_>,
        current: MinuteCounter,
    ) -> LegacyRateLimitError {
        let limits = check.limits;
        let detail = format!(
            "LiteLLM Rate Limit Handler for rate limit type = {}. {}. current rpm: {}, rpm limit: {}, current tpm: {}, tpm limit: {}, current max_parallel_requests: {}, max_parallel_requests: {}",
            check.rate_limit_type.as_str(),
            CommonProxyErrors::MaxParallelRequestLimitReached.as_str(),
            current.current_rpm,
            limits.rpm_limit,
            current.current_tpm,
            limits.tpm_limit,
            current.current_requests,
            limits.max_parallel_requests
        );
        self.rate_limit_error(
            detail,
            limits.tripped_dimension(current),
            check.requested_model,
        )
    }

    async fn write_counters(
        &self,
        entries: &[(String, LegacyCacheValue)],
        parent_otel_span: Option<&ParentOtelSpan>,
    ) {
        let written = self
            .internal_usage_cache
            .batch_set(entries, COUNTER_TTL_SECONDS, parent_otel_span, true)
            .await;
        if let Err(error) = written {
            tracing::error!("LiteLLM Cache: exception in async add_cache: {error}");
        }
    }

    fn rate_limit_error(
        &self,
        detail: String,
        rate_limit_type: RateLimitDimension,
        requested_model: Option<&str>,
    ) -> LegacyRateLimitError {
        let (model, llm_provider) = resolve_rate_limit_provider(
            requested_model,
            self.native.as_ref(),
            self.router.as_deref(),
        );
        let retry_after = float_text(self.time_to_next_minute());
        let init = ProxyRateLimitErrorInit {
            detail: Payload::String(detail),
            headers: Some(BTreeMap::from([(
                String::from("retry-after"),
                Payload::String(retry_after),
            )])),
            category: None,
            rate_limit_type: Some(rate_limit_type),
            model: Some(model),
            llm_provider: Some(llm_provider),
        };
        match ProxyRateLimitError::new(init) {
            Ok(error) => LegacyRateLimitError::Rejected(Box::new(error)),
            Err(source) => LegacyRateLimitError::Header(source),
        }
    }
}
