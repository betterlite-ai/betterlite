use std::ops::{Add as _, Mul as _};
use std::sync::Arc;

use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use rust_decimal::prelude::ToPrimitive as _;

use super::{
    ANTHROPIC_MESSAGES_CALL, ANTHROPIC_PROVIDER, CACHE_KEY_PREFIX, CacheObservation, Clock,
    LOCAL_COPY_SECONDS, ObservationCache, ObservationParser, ObservedCache, RETENTION_SECONDS,
    SUCCESS_STATUS_CODE, WireRequest, read_exact, unix_now,
};
use crate::fields::payload::StandardLoggingPayload;
use crate::fields::status::LoggingStatus;

const NANOS_PER_SECOND: f64 = 0.000_000_001;

// the strict event LiteLLM validates; the proxy fills it while the wire request is still known
pub struct SuccessEvent<'event> {
    pub payload: &'event StandardLoggingPayload,
    pub wire: WireRequest<'event>,
    pub response: &'event Json,
    pub first_api_call_start_time: DateTime<FixedOffset>,
    pub status_code: u16,
    pub cache_hit: Option<bool>,
    pub stream: bool,
    pub prompt_cache_response_complete: bool,
}

pub struct PromptCacheObserver {
    pub cache: Arc<dyn ObservationCache>,
    pub parser: Arc<dyn ObservationParser>,
    pub clock: Clock,
}

impl PromptCacheObserver {
    #[must_use]
    pub fn new(cache: Arc<dyn ObservationCache>, parser: Arc<dyn ObservationParser>) -> Self {
        Self {
            cache,
            parser,
            clock: Arc::new(unix_now),
        }
    }

    // every rejection returns silently, as LiteLLM does, since only a stored observation has an effect
    pub async fn async_log_success_event(&self, event: &SuccessEvent<'_>) {
        let Some((model_id, caller_key_hash)) = claimed_ids(event) else {
            return;
        };
        let parsed = self
            .parser
            .parse(&event.wire, event.response, &caller_key_hash, &model_id);
        let Some(observed) = parsed else {
            return;
        };
        let now = (self.clock)();
        let Some(started) = unix_seconds(event.first_api_call_start_time) else {
            return;
        };
        if started > now {
            return;
        }
        if observed.cache_creation_tokens == 0 && !self.has_prior_evidence(&observed).await {
            return;
        }
        self.store(&observed, now, started).await;
    }

    // a read-only hit refreshes evidence only when the same fingerprint already reported the same reads
    async fn has_prior_evidence(&self, observed: &ObservedCache) -> bool {
        let previous = read_exact(
            self.cache.as_ref(),
            &observed.scope,
            &observed.prefix.fingerprint,
        )
        .await;
        previous.is_some_and(|found| {
            found.fingerprint == observed.prefix.fingerprint
                && found.cached_tokens == observed.cached_tokens
        })
    }

    async fn store(&self, observed: &ObservedCache, now: f64, started: f64) {
        let observation = CacheObservation {
            fingerprint: observed.prefix.fingerprint.clone(),
            cached_tokens: observed.cached_tokens,
            observed_at: now,
            expires_at: started.add(f64::from(observed.ttl_seconds)),
        };
        let Some(valid) = observation.validated() else {
            return;
        };
        let scope = observed.scope.as_str();
        let fingerprint = valid.fingerprint.as_str();
        let key = format!("{CACHE_KEY_PREFIX}:{scope}:{fingerprint}");
        let Ok(encoded) = serde_json::to_string(&valid) else {
            return;
        };
        let entry = Json::Text(encoded);
        self.cache.set(&key, &entry, RETENTION_SECONDS, false).await;
        // a short local copy keeps Redis's longer evidence from hiding a change within one request
        if self.cache.has_redis() {
            self.cache.set(&key, &entry, LOCAL_COPY_SECONDS, true).await;
        }
    }
}

// the strict _Event checks LiteLLM runs before it looks at the wire request
fn claimed_ids(event: &SuccessEvent<'_>) -> Option<(String, String)> {
    let logged = event.payload;
    let foreign_call = logged.call_type != ANTHROPIC_MESSAGES_CALL
        || logged.custom_llm_provider.as_deref() != Some(ANTHROPIC_PROVIDER);
    let not_success = logged.status != LoggingStatus::Success;
    let unobservable = event.cache_hit == Some(true)
        || event.status_code != SUCCESS_STATUS_CODE
        || (event.stream && !event.prompt_cache_response_complete);
    if foreign_call || not_success || unobservable {
        return None;
    }
    let model_id = logged.model_id.as_deref().filter(|id| !id.is_empty())?;
    let caller_key_hash = logged
        .metadata
        .user_api_key_hash
        .as_deref()
        .filter(|hash| !hash.is_empty())?;
    Some((model_id.to_owned(), caller_key_hash.to_owned()))
}

// datetime.timestamp(): whole seconds plus the sub-second part as a float
fn unix_seconds(time: DateTime<FixedOffset>) -> Option<f64> {
    let seconds = time.timestamp().to_f64()?;
    let fraction = f64::from(time.timestamp_subsec_nanos()).mul(NANOS_PER_SECOND);
    Some(seconds.add(fraction))
}
