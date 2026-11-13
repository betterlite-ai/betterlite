use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use betterlite_error::rate_limit::{RateLimitCategory, RateLimitDimension};
use thiserror::Error;

// LiteLLM's azure formula: 6 requests per minute per 1000 tokens per minute
const TPM_PER_REQUEST: i64 = 1000;
const REQUESTS_PER_TPM_UNIT: i64 = 6;

// A caller that finds every slot in use gets this instead of waiting for one to free up.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error(
    "deployment {model_group} (id {model_id}) already has max_parallel_requests={max_parallel_requests} requests in flight; raise max_parallel_requests (or the rpm/tpm it is derived from) for this deployment"
)]
pub struct MaxParallelRequestsExceeded {
    pub model_group: String,
    pub model_id: String,
    pub max_parallel_requests: i64,
}

impl MaxParallelRequestsExceeded {
    pub const STATUS_CODE: u16 = 429;
    pub const CODE: &'static str = "429";
    pub const ERROR_TYPE: &'static str = "throttling_error";
    pub const CATEGORY: RateLimitCategory = RateLimitCategory::LitellmRateLimit;
    pub const DIMENSION: RateLimitDimension = RateLimitDimension::ConcurrentRequests;
    pub const LLM_PROVIDER: &'static str = "";
}

// The deployment's slot counter, shared by every request the cache hands this deployment.
pub struct MaxParallelRequestsLimit {
    pub max_parallel_requests: i64,
    pub model_id: String,
    pub model_group: String,
    in_flight: AtomicI64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitSnapshot {
    pub max_parallel_requests: i64,
    pub in_flight: i64,
}

// The guard holds one slot until it is dropped, which is what makes a stream release it when it ends.
pub struct DeploymentSlot {
    limit: Arc<MaxParallelRequestsLimit>,
}

impl Drop for DeploymentSlot {
    fn drop(&mut self) {
        let _released =
            self.limit
                .in_flight
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |in_flight| {
                    Some(in_flight.saturating_sub(1))
                });
    }
}

impl MaxParallelRequestsLimit {
    #[must_use]
    pub fn new(max_parallel_requests: i64, model_id: String, model_group: String) -> Self {
        Self {
            max_parallel_requests,
            model_id,
            model_group,
            in_flight: AtomicI64::new(0),
        }
    }

    pub fn acquire(self: &Arc<Self>) -> Result<DeploymentSlot, MaxParallelRequestsExceeded> {
        let acquired =
            self.in_flight
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |in_flight| {
                    (in_flight < self.max_parallel_requests).then(|| in_flight.saturating_add(1))
                });
        acquired.map_or_else(
            |_| {
                Err(MaxParallelRequestsExceeded {
                    model_group: self.model_group.clone(),
                    model_id: self.model_id.clone(),
                    max_parallel_requests: self.max_parallel_requests,
                })
            },
            |_| {
                Ok(DeploymentSlot {
                    limit: Arc::clone(self),
                })
            },
        )
    }

    // the synchronous facade of __enter__/__exit__: the slot is held for the whole body
    pub fn with_slot<F, T>(self: &Arc<Self>, work: F) -> Result<T, MaxParallelRequestsExceeded>
    where
        F: FnOnce() -> T,
    {
        let slot = self.acquire()?;
        let output = work();
        drop(slot);
        Ok(output)
    }

    #[must_use]
    pub fn snapshot(&self) -> LimitSnapshot {
        LimitSnapshot {
            max_parallel_requests: self.max_parallel_requests,
            in_flight: self.in_flight.load(Ordering::SeqCst),
        }
    }
}

// max_parallel_requests > rpm > tpm / 1000 * 6 (at least one) > default_max_parallel_requests
#[must_use]
pub fn calculate_max_parallel_requests(
    max_parallel_requests: Option<i64>,
    rpm: Option<i64>,
    tpm: Option<i64>,
    default_max_parallel_requests: Option<i64>,
) -> Option<i64> {
    if let Some(limit) = max_parallel_requests {
        return Some(limit);
    }
    if let Some(limit) = rpm {
        return Some(limit);
    }
    if let Some(tokens) = tpm {
        let calculated = tokens.saturating_mul(REQUESTS_PER_TPM_UNIT) / TPM_PER_REQUEST;
        return Some(if calculated == 0 { 1 } else { calculated });
    }
    default_max_parallel_requests
}

// The limits a deployment declares and the router-wide default its calculation falls back to.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LimitSource {
    pub max_parallel_requests: Option<i64>,
    pub rpm: Option<i64>,
    pub tpm: Option<i64>,
    pub default_max_parallel_requests: Option<i64>,
}

// InitalizeCachedClient.set_max_parallel_requests_client: a falsy calculation installs no cap at all
#[must_use]
pub fn installable_limit(
    source: LimitSource,
    model_id: &str,
    model_group: &str,
) -> Option<Arc<MaxParallelRequestsLimit>> {
    let calculated = calculate_max_parallel_requests(
        source.max_parallel_requests,
        source.rpm,
        source.tpm,
        source.default_max_parallel_requests,
    )?;
    if calculated == 0 {
        return None;
    }
    Some(Arc::new(MaxParallelRequestsLimit::new(
        calculated,
        model_id.to_owned(),
        model_group.to_owned(),
    )))
}
